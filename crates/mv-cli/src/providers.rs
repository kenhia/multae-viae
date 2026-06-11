//! Provider dispatch: the single seam between a resolved `ModelEntry` and a
//! rig agent call. Both the prompt command and the workflow executor route
//! through [`complete`]; Phase 5 fallback chains will call it in a loop.

use std::time::Duration;

use mv_core::preflight::{PreflightStatus, preflight};
use mv_core::providers::{SYSTEM_PREAMBLE, classify_backend_error, classify_prompt_error};
use mv_core::trtllm::START_HINT;
use mv_core::{ModelEntry, ModelRegistry, MvError, Provider};
use rig::completion::Prompt;
use rig::tool::server::ToolServerHandle;
use tracing::{Span, debug, info, warn};

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
#[tracing::instrument(name = "model_routing", skip_all, fields(
    router.requested = %primary.id,
    router.selected = tracing::field::Empty,
    router.reason = tracing::field::Empty,
))]
pub async fn complete_with_fallback(
    registry: &ModelRegistry,
    primary: &ModelEntry,
    primary_endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
    params: &GenParams,
) -> Result<CompletionOutcome, MvError> {
    // Resolve the chain up front. Fallback ids are registry-validated at load,
    // so `get` is expected to hit; a missing entry is skipped defensively.
    let mut chain: Vec<(&ModelEntry, String)> = vec![(primary, primary_endpoint.to_string())];
    if let Some(ids) = &primary.fallback {
        for id in ids {
            if let Some(entry) = registry.get(id) {
                chain.push((entry, entry.endpoint()));
            }
        }
    }
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
            && let PreflightStatus::Dead(e) = preflight(entry, &endpoint, PREFLIGHT_TIMEOUT).await
        {
            warn!(model = %entry.id, error = %e, "model preflight reported dead, skipping");
            attempts.push((entry.id.clone(), e.to_string()));
            continue;
        }
        match complete(entry, &endpoint, prompt, handle.clone(), params).await {
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
    use rig::client::{CompletionClient, Nothing};

    info!(model = %entry.id, endpoint = %endpoint, locality = "local", "connecting to Ollama");

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
    let agent = params.apply(builder).build();

    info!("sending prompt to model");
    let response = agent
        .prompt(prompt)
        .await
        .map_err(|e| classify_prompt_error(&e, &entry.id, endpoint, "Is Ollama running?", None))?;

    Ok(response)
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
    use rig::client::CompletionClient;

    info!(model = %entry.id, endpoint = %endpoint, locality = "cloud", "connecting to OpenAI");

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
    let agent = params.apply(builder).build();

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        debug!(raw_error = %e, "openai prompt failed");
        classify_prompt_error(&e, &entry.id, endpoint, "Check the endpoint URL.", None)
    })?;

    Ok(response)
}

/// Build the TRT-LLM agent (Chat Completions client, stop sequences,
/// sampling params). Shared by the buffered and streaming paths.
fn trtllm_agent(
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
    if let Some(stop_value) = mv_core::trtllm::stop::request_stop_value(entry) {
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

    mv_core::trtllm::usage::Usage::from_counts(
        response.usage.input_tokens,
        response.usage.output_tokens,
    )
    .record_on(&tracing::Span::current());

    Ok(response.output)
}

#[tracing::instrument(name = "llm_completion", skip(entry, handle), fields(
    gen_ai.system = "trtllm",
    gen_ai.request.model = %entry.model_name(),
    trtllm.architecture = entry.architecture.as_deref().unwrap_or(""),
    trtllm.quant = entry.quant.as_deref().unwrap_or(""),
    trtllm.expected_vram_gb = entry.expected_vram_gb.unwrap_or(0),
    gen_ai.usage.input_tokens = tracing::field::Empty,
    gen_ai.usage.output_tokens = tracing::field::Empty,
))]
pub async fn stream_trtllm(
    entry: &ModelEntry,
    endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
) -> Result<String, MvError> {
    use futures_util::StreamExt;
    use rig::agent::{MultiTurnStreamItem, Text};
    use rig::streaming::StreamedAssistantContent;
    use std::io::Write as _;

    // One shared preflight covers both the health check and the served-model
    // list. The latter matters most on the streaming path: rig's streaming
    // layer swallows a proxy 502 (logs an SSE parse error, ends the turn
    // empty), so without it an unloaded model would yield silent empty output
    // and exit 0. A `Dead` status carries the `just load` / `trtllm-serve` hint.
    if let PreflightStatus::Dead(e) = preflight(entry, endpoint, PREFLIGHT_TIMEOUT).await {
        return Err(e);
    }

    let model_name = entry.model_name();
    info!(model = %model_name, endpoint = %endpoint, locality = "local", "streaming from TRT-LLM");

    let agent = trtllm_agent(entry, endpoint, handle, &GenParams::default())?;

    info!("sending prompt to model");
    use rig::streaming::StreamingPrompt as _;
    let mut response_stream = agent
        .stream_prompt(prompt)
        .multi_turn(entry.effective_max_turns())
        .await;

    let stdout = std::io::stdout();
    let mut accumulator = String::new();
    let mut stream_err: Option<MvError> = None;

    loop {
        let Some(chunk) = response_stream.next().await else {
            break;
        };
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                Text { text },
            ))) => {
                let mut handle = stdout.lock();
                if let Err(e) = handle
                    .write_all(text.as_bytes())
                    .and_then(|_| handle.flush())
                {
                    stream_err = Some(MvError::CompletionFailed {
                        details: format!("stdout write failed: {e}"),
                    });
                    break;
                }
                accumulator.push_str(&text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                let usage = final_response.usage();
                mv_core::trtllm::usage::Usage::from_counts(usage.input_tokens, usage.output_tokens)
                    .record_on(&tracing::Span::current());
            }
            Ok(_) => continue,
            Err(e) => {
                let msg = e.to_string();
                debug!(raw_error = %msg, "trtllm stream failed");
                stream_err = Some(classify_backend_error(
                    &msg,
                    model_name,
                    endpoint,
                    START_HINT,
                    Some(&entry.id),
                ));
                break;
            }
        }
    }

    if let Some(err) = stream_err {
        // Leave any partial output already written to stdout intact.
        return Err(err);
    }

    // Trailing newline after clean termination.
    let mut handle = stdout.lock();
    let _ = handle.write_all(b"\n");
    let _ = handle.flush();

    info!(len = accumulator.len(), "stream complete");
    // Caller's print_success will print an empty string; the stream itself
    // already wrote the full body + trailing newline to stdout.
    Ok(String::new())
}
