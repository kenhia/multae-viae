//! Agent runtime: the single seam between a resolved [`ModelEntry`] and a rig
//! agent call, plus the concrete workflow executors that bridge the (rig-free)
//! workflow engine to rig. Both the prompt command and the workflow executor
//! route buffered completions through [`complete`]; fallback chains call it in
//! a loop via [`complete_chain`].
//!
//! This lives in `mv-core` so every consumer — `mv-cli` and `mv-server` — drives
//! the same routing, fallback, and tool-dispatch logic. The binary keeps only
//! its own concerns (CLI parsing, stdout formatting, terminal streaming).

use std::time::Duration;

use rig::completion::Prompt;
use rig::tool::server::ToolServerHandle;
use tracing::{Span, debug, info, warn};

use crate::preflight::{PreflightStatus, preflight};
use crate::providers::{SYSTEM_PREAMBLE, classify_prompt_error};
use crate::trtllm::START_HINT;
use crate::{ModelEntry, ModelRegistry, MvError, Provider};

/// Network timeout for a preflight probe — short, so the router never blocks on
/// a hung host before falling back.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(2);

/// The result of a (possibly chained) completion: the response text plus the
/// id of the model that actually served it — which may differ from the
/// requested model when a fallback chain was walked.
#[derive(Debug, Clone)]
pub struct CompletionOutcome {
    pub text: String,
    pub model_used: String,
}

/// Optional sampling parameters forwarded to the provider (set by workflow
/// steps; the plain prompt path uses provider defaults).
#[derive(Debug, Default, Clone, Copy)]
pub struct GenParams {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
}

impl GenParams {
    fn apply<M, P, T>(
        &self,
        mut builder: rig::agent::AgentBuilder<M, P, T>,
    ) -> rig::agent::AgentBuilder<M, P, T>
    where
        M: rig::completion::CompletionModel,
        P: rig::agent::PromptHook<M>,
    {
        if let Some(t) = self.temperature {
            builder = builder.temperature(t);
        }
        if let Some(m) = self.max_tokens {
            builder = builder.max_tokens(m);
        }
        builder
    }
}

/// Walk `primary` and its fallback chain, returning the first success.
///
/// The chain is `[primary] + primary.fallback` (non-transitive — a fallback's
/// own `fallback` list is not followed). `primary_endpoint` lets the caller
/// override the primary's endpoint (the `--endpoint` flag / test injection);
/// fallback entries always use their own resolved endpoint. Advancement is
/// gated on [`MvError::is_fallback_eligible`]: a backend-dead or
/// misconfiguration error tries the next entry, anything else (empty prompt,
/// turn-limit, a completed-but-failed completion) fails fast. If every entry
/// is exhausted, returns [`MvError::AllModelsFailed`] enumerating each attempt.
pub async fn complete_with_fallback(
    registry: &ModelRegistry,
    primary: &ModelEntry,
    primary_endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<CompletionOutcome, MvError> {
    let chain = build_chain(registry, &[(primary, primary_endpoint.to_string())]);
    complete_chain(&chain, prompt, handle, params).await
}

/// Expand seed entries into the full candidate chain: each seed in order,
/// followed by its own `fallback` entries, deduped by id across the whole
/// chain. The single chain-construction rule shared by the CLI prompt path
/// (one seed: the requested model, possibly with a `--endpoint` override) and
/// workflow `prefer:` lists (one seed per preferred id). Seeds carry their
/// endpoint; fallback entries use their own resolved endpoint. Fallback ids
/// are registry-validated at load, so `get` is expected to hit; a missing
/// entry is skipped defensively.
pub fn build_chain<'a>(
    registry: &'a ModelRegistry,
    seeds: &[(&'a ModelEntry, String)],
) -> Vec<(&'a ModelEntry, String)> {
    let mut chain: Vec<(&ModelEntry, String)> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (entry, endpoint) in seeds {
        if seen.insert(entry.id.as_str()) {
            chain.push((entry, endpoint.clone()));
        }
        if let Some(ids) = &entry.fallback {
            for id in ids {
                if let Some(fe) = registry.get(id)
                    && seen.insert(fe.id.as_str())
                {
                    chain.push((fe, fe.endpoint()));
                }
            }
        }
    }
    chain
}

/// Walk a pre-built candidate chain, returning the first success. The single
/// fallback walker behind both [`complete_with_fallback`] (a model + its own
/// `fallback` list) and step-level `prefer:` lists. Advancement is gated on
/// [`MvError::is_fallback_eligible`]; a `Dead` preflight skips an entry without
/// building an agent (multi-entry chains only). If every entry is exhausted,
/// returns [`MvError::AllModelsFailed`] enumerating each attempt.
#[tracing::instrument(name = "model_routing", skip_all, fields(
    router.requested = chain.first().map(|(e, _)| e.id.as_str()).unwrap_or(""),
    router.selected = tracing::field::Empty,
    router.reason = tracing::field::Empty,
))]
pub async fn complete_chain(
    chain: &[(&ModelEntry, String)],
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<CompletionOutcome, MvError> {
    let chain_len = chain.len();
    let mut attempts: Vec<(String, String)> = Vec::new();
    for (entry, endpoint) in chain {
        // In a multi-entry chain, skip an entry whose backend preflights `Dead`
        // before building an agent. The big win is a dead Ollama: `complete`
        // has no internal preflight there and would wait out rig's connect
        // timeout. (Single-model chains skip this and go straight to `complete`
        // so their real classified error — and existing tests — are unchanged.
        // For TRT-LLM this overlaps `call_trtllm`'s own preflight; the extra
        // probe on a *healthy* backend is one cheap GET.)
        if chain_len > 1
            && let PreflightStatus::Dead(e) = preflight(entry, endpoint, PREFLIGHT_TIMEOUT).await
        {
            warn!(model = %entry.id, error = %e, "model preflight reported dead, skipping");
            attempts.push((entry.id.clone(), e.to_string()));
            continue;
        }
        match complete(entry, endpoint, prompt, handle.clone(), params).await {
            Ok(text) => {
                let span = Span::current();
                span.record("router.selected", entry.id.as_str());
                let reason = if attempts.is_empty() {
                    "primary"
                } else {
                    "fallback after primary failure"
                };
                span.record("router.reason", reason);
                if !attempts.is_empty() {
                    info!(
                        model = %entry.id,
                        skipped = attempts.len(),
                        "completion served by fallback model"
                    );
                }
                return Ok(CompletionOutcome {
                    text,
                    model_used: entry.id.clone(),
                });
            }
            Err(e) if e.is_fallback_eligible() && chain_len > 1 => {
                // Emit a span event per failed attempt so a trace shows the
                // full walk, then advance to the next candidate.
                warn!(model = %entry.id, error = %e, "model failed, trying next in chain");
                attempts.push((entry.id.clone(), e.to_string()));
                continue;
            }
            // Fail fast: either the error is not fallback-eligible, or there is
            // no chain to fall back to (single model → surface its real error).
            Err(e) => return Err(e),
        }
    }

    Err(MvError::AllModelsFailed { attempts })
}

/// Dispatch a buffered completion to the entry's provider.
pub async fn complete(
    entry: &ModelEntry,
    endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<String, MvError> {
    match entry.provider {
        Provider::Ollama => call_ollama(entry, endpoint, prompt, handle, params).await,
        Provider::Openai => {
            let env_var = entry.api_key_env();
            match std::env::var(env_var) {
                Ok(api_key) => call_openai(entry, endpoint, &api_key, prompt, handle, params).await,
                Err(_) => Err(MvError::ApiKeyMissing {
                    provider: entry.provider.to_string(),
                    env_var: env_var.to_string(),
                }),
            }
        }
        Provider::Trtllm => call_trtllm(entry, endpoint, prompt, handle, params).await,
    }
}

/// Build an Ollama agent (client + preamble + tools + sampling). Shared by the
/// one-shot path ([`call_ollama`]) and held sessions ([`AnyAgent`]).
pub fn ollama_agent(
    entry: &ModelEntry,
    endpoint: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<rig::agent::Agent<rig::providers::ollama::CompletionModel>, MvError> {
    use rig::client::{CompletionClient, Nothing};

    let client = rig::providers::ollama::Client::builder()
        .api_key(Nothing)
        .base_url(endpoint)
        .build()
        .map_err(|e| MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
            hint: "Is Ollama running?".to_string(),
        })?;

    let builder = client
        .agent(&entry.id)
        .preamble(SYSTEM_PREAMBLE)
        .tool_server_handle(handle)
        .default_max_turns(entry.effective_max_turns());
    Ok(params.apply(builder).build())
}

#[tracing::instrument(name = "llm_completion", skip(entry, handle, params), fields(
    gen_ai.system = "ollama",
    gen_ai.request.model = %entry.id,
))]
async fn call_ollama(
    entry: &ModelEntry,
    endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<String, MvError> {
    info!(model = %entry.id, endpoint = %endpoint, locality = "local", "connecting to Ollama");

    let agent = ollama_agent(entry, endpoint, handle, params)?;

    info!("sending prompt to model");
    let response = agent
        .prompt(prompt)
        .await
        .map_err(|e| classify_prompt_error(&e, &entry.id, endpoint, "Is Ollama running?", None))?;

    Ok(response)
}

/// Build an OpenAI (cloud, Responses API) agent. Shared by the one-shot path
/// ([`call_openai`]) and held sessions ([`AnyAgent`]).
pub fn openai_agent(
    entry: &ModelEntry,
    endpoint: &str,
    api_key: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<
    rig::agent::Agent<rig::providers::openai::responses_api::ResponsesCompletionModel>,
    MvError,
> {
    use rig::client::CompletionClient;

    let client = rig::providers::openai::Client::builder()
        .api_key(api_key)
        .base_url(endpoint)
        .build()
        .map_err(|e| MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
            hint: "Check the endpoint URL.".to_string(),
        })?;

    let builder = client
        .agent(&entry.id)
        .preamble(SYSTEM_PREAMBLE)
        .tool_server_handle(handle)
        .default_max_turns(entry.effective_max_turns());
    Ok(params.apply(builder).build())
}

#[tracing::instrument(name = "llm_completion", skip(entry, api_key, handle, params), fields(
    gen_ai.system = "openai",
    gen_ai.request.model = %entry.id,
))]
async fn call_openai(
    entry: &ModelEntry,
    endpoint: &str,
    api_key: &str,
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<String, MvError> {
    info!(model = %entry.id, endpoint = %endpoint, locality = "cloud", "connecting to OpenAI");

    let agent = openai_agent(entry, endpoint, api_key, handle, params)?;

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        debug!(raw_error = %e, "openai prompt failed");
        classify_prompt_error(&e, &entry.id, endpoint, "Check the endpoint URL.", None)
    })?;

    Ok(response)
}

/// Build the TRT-LLM agent (Chat Completions client, stop sequences,
/// sampling params). Shared by the buffered path here and the binary's
/// streaming path (`mv-cli`'s `stream_trtllm`).
pub fn trtllm_agent(
    entry: &ModelEntry,
    endpoint: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<rig::agent::Agent<rig::providers::openai::CompletionModel>, MvError> {
    use rig::client::CompletionClient;

    // Use the Chat Completions API (not the Responses API) because
    // OpenAI-compatible proxies typically only implement /v1/chat/completions.
    let client = rig::providers::openai::CompletionsClient::builder()
        .api_key("tensorrt_llm")
        .base_url(endpoint)
        .build()
        .map_err(|e| MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
            hint: START_HINT.to_string(),
        })?;

    let mut builder = client
        .agent(entry.model_name())
        .preamble(SYSTEM_PREAMBLE)
        .tool_server_handle(handle)
        .default_max_turns(entry.effective_max_turns());
    if let Some(stop_value) = crate::trtllm::stop::request_stop_value(entry) {
        builder = builder.additional_params(stop_value);
    }
    Ok(params.apply(builder).build())
}

#[tracing::instrument(name = "llm_completion", skip(entry, handle, params), fields(
    gen_ai.system = "trtllm",
    gen_ai.request.model = %entry.model_name(),
    trtllm.architecture = entry.architecture.as_deref().unwrap_or(""),
    trtllm.quant = entry.quant.as_deref().unwrap_or(""),
    trtllm.expected_vram_gb = entry.expected_vram_gb.unwrap_or(0),
    gen_ai.usage.input_tokens = tracing::field::Empty,
    gen_ai.usage.output_tokens = tracing::field::Empty,
))]
async fn call_trtllm(
    entry: &ModelEntry,
    endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<String, MvError> {
    // Shared preflight: health + served-model check, the single source of the
    // TRT-LLM reachability + not-loaded mapping (shared with the stream path).
    if let PreflightStatus::Dead(e) = preflight(entry, endpoint, PREFLIGHT_TIMEOUT).await {
        return Err(e);
    }

    let model_name = entry.model_name();
    info!(model = %model_name, endpoint = %endpoint, locality = "local", "connecting to TRT-LLM");

    let agent = trtllm_agent(entry, endpoint, handle, params)?;

    info!("sending prompt to model");
    let response = agent.prompt(prompt).extended_details().await.map_err(|e| {
        debug!(raw_error = %e, "trtllm prompt failed");
        classify_prompt_error(&e, model_name, endpoint, START_HINT, Some(&entry.id))
    })?;

    crate::trtllm::usage::Usage::from_counts(
        response.usage.input_tokens,
        response.usage.output_tokens,
    )
    .record_on(&tracing::Span::current());

    Ok(response.output)
}

/// A built, held-open agent — one per provider, as a **closed enum** over the
/// three concrete rig agent types (deliberately not `Box<dyn>`: the set of
/// providers is fixed, so static dispatch keeps the type information and avoids
/// a vtable). This is what a session holds between turns: built once via
/// [`AnyAgent::build`], then driven with [`AnyAgent::chat_turn`], which threads
/// the conversation history so context carries across turns.
pub enum AnyAgent {
    Ollama(rig::agent::Agent<rig::providers::ollama::CompletionModel>),
    OpenAi(rig::agent::Agent<rig::providers::openai::responses_api::ResponsesCompletionModel>),
    TrtLlm(rig::agent::Agent<rig::providers::openai::completion::CompletionModel>),
}

impl AnyAgent {
    /// Build the held agent for `entry` (the same client/preamble/tools/sampling
    /// the one-shot path uses), dispatching on provider. For TRT-LLM the caller
    /// should preflight separately (as the one-shot path does) — this only
    /// constructs the agent.
    pub fn build(
        entry: &ModelEntry,
        endpoint: &str,
        handle: ToolServerHandle,
        params: &GenParams,
    ) -> Result<Self, MvError> {
        match entry.provider {
            Provider::Ollama => Ok(Self::Ollama(ollama_agent(entry, endpoint, handle, params)?)),
            Provider::Openai => {
                let env_var = entry.api_key_env();
                let api_key = std::env::var(env_var).map_err(|_| MvError::ApiKeyMissing {
                    provider: entry.provider.to_string(),
                    env_var: env_var.to_string(),
                })?;
                Ok(Self::OpenAi(openai_agent(
                    entry, endpoint, &api_key, handle, params,
                )?))
            }
            Provider::Trtllm => Ok(Self::TrtLlm(trtllm_agent(entry, endpoint, handle, params)?)),
        }
    }

    /// Run one conversational turn: prompt the held agent with `prompt` and the
    /// prior `history`, returning the assistant's reply. Tool round-trips happen
    /// internally (bounded by the entry's max turns). Errors are classified with
    /// the same provider-specific hints as the one-shot path.
    pub async fn chat_turn(
        &self,
        entry: &ModelEntry,
        endpoint: &str,
        prompt: &str,
        history: Vec<rig::completion::Message>,
    ) -> Result<String, MvError> {
        let max_turns = entry.effective_max_turns();
        match self {
            Self::Ollama(a) => a
                .prompt(prompt)
                .with_history(history)
                .max_turns(max_turns)
                .await
                .map_err(|e| {
                    classify_prompt_error(&e, &entry.id, endpoint, "Is Ollama running?", None)
                }),
            Self::OpenAi(a) => a
                .prompt(prompt)
                .with_history(history)
                .max_turns(max_turns)
                .await
                .map_err(|e| {
                    classify_prompt_error(&e, &entry.id, endpoint, "Check the endpoint URL.", None)
                }),
            Self::TrtLlm(a) => a
                .prompt(prompt)
                .with_history(history)
                .max_turns(max_turns)
                .await
                .map_err(|e| {
                    classify_prompt_error(
                        &e,
                        entry.model_name(),
                        endpoint,
                        START_HINT,
                        Some(&entry.id),
                    )
                }),
        }
    }
}

/// Prompt executor that routes workflow prompt steps through the shared
/// provider dispatch seam.
pub struct RigPromptExecutor {
    pub registry: ModelRegistry,
    pub agent_handle: ToolServerHandle,
}

impl crate::workflow::engine::PromptExecutor for RigPromptExecutor {
    async fn execute_prompt(
        &self,
        prompt_text: &str,
        models: &[String],
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String, MvError> {
        // Resolve each preferred id (a typo'd model must fail loudly —
        // silently substituting the default would run the step elsewhere and
        // report success), then expand the seeds through the shared chain
        // builder: each id contributes itself + its own `fallback` entries,
        // deduped. So a bare `model:` behaves exactly like the CLI path, and
        // a `prefer:` list strings several such chains together.
        let mut seeds: Vec<(&ModelEntry, String)> = Vec::new();
        for id in models {
            let entry = self
                .registry
                .get(id)
                .ok_or_else(|| MvError::ModelNotInRegistry {
                    model: id.clone(),
                    available: self.registry.available_ids().join(", "),
                })?;
            seeds.push((entry, entry.endpoint()));
        }
        let chain = build_chain(&self.registry, &seeds);

        let params = GenParams {
            temperature,
            max_tokens,
        };
        // The engine only needs the text; `model_used` is recorded on the trace.
        complete_chain(&chain, prompt_text, self.agent_handle.clone(), &params)
            .await
            .map(|outcome| outcome.text)
    }
}

/// Tool executor that runs workflow tool steps against the shared agent
/// ToolServer — the same merged built-in + MCP tool set the agent sees.
pub struct HandleToolExecutor {
    pub handle: ToolServerHandle,
}

impl crate::workflow::engine::ToolExecutor for HandleToolExecutor {
    #[tracing::instrument(level = "info", skip(self, inputs), fields(tool.name = %tool_name))]
    async fn execute_tool(
        &self,
        tool_name: &str,
        inputs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, MvError> {
        let args = serde_json::to_string(inputs).map_err(|e| MvError::ToolCallFailed {
            tool: tool_name.to_string(),
            details: format!("failed to encode inputs: {e}"),
        })?;
        self.handle
            .call_tool(tool_name, &args)
            .await
            .map_err(|e| MvError::ToolCallFailed {
                tool: tool_name.to_string(),
                details: e.to_string(),
            })
    }
}
