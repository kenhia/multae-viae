//! The `prompt` command: resolve a model, attach tools, run one completion.

use rig::tool::server::ToolServer;
use tracing::{debug, info};

use crate::cli::PromptArgs;
use crate::stream::stream_trtllm;
use mv_core::Provider;
use mv_core::mcp::manager::McpManager;
use mv_core::runtime::{CompletionOutcome, GenParams, complete_with_fallback};

#[tracing::instrument(name = "mv_cli_request", skip(args), fields(
    prompt = %args.prompt,
    model = args.model.as_deref().unwrap_or("default"),
))]
pub async fn run_prompt(
    args: &PromptArgs,
    json: bool,
    effective_stream: bool,
) -> std::result::Result<CompletionOutcome, mv_core::MvError> {
    let prompt = mv_core::validate_prompt(&args.prompt)?;

    // Load model registry
    let registry = mv_core::ModelRegistry::resolve(args.config.as_deref())?;

    // Resolve the target model
    let entry = if let Some(ref model_id) = args.model {
        registry
            .get(model_id)
            .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                model: model_id.clone(),
                available: registry.available_ids().join(", "),
            })?
    } else {
        registry.default_model()
    };

    let endpoint = args.endpoint.clone().unwrap_or_else(|| entry.endpoint());
    let locality = entry.locality();

    debug!(
        model = %entry.id,
        provider = %entry.provider,
        endpoint = %endpoint,
        locality = %locality,
        "resolved model"
    );

    // Streaming is currently only implemented for TRT-LLM.
    if effective_stream && entry.provider != Provider::Trtllm {
        return Err(mv_core::MvError::StreamingNotSupported);
    }

    // The TRT-LLM proxy streams tool calls as plain text rather than as
    // executable `tool_calls`, so streaming with tools attached produces fake,
    // never-executed tool-call text. Prefer correctness: when tools are
    // attached, fall back to buffered (which performs the real tool round-trip).
    // `--no-tools` opts out and enables genuine streaming.
    let stream_trtllm_path =
        effective_stream && entry.provider == Provider::Trtllm && args.no_tools;
    if effective_stream && entry.provider == Provider::Trtllm && !args.no_tools {
        eprintln!(
            "note: --stream falls back to buffered output because tools are attached \
             (the TRT-LLM proxy cannot stream tool calls); re-run with --no-tools to stream"
        );
    }

    // Set up agent ToolServer. `--no-tools` attaches nothing (built-in or MCP),
    // which is what makes clean TRT-LLM streaming possible.
    let mut tool_server = ToolServer::new();
    if !args.no_tools {
        tool_server = tool_server
            .tool(mv_core::tools::file_list::FileList)
            .tool(mv_core::tools::file_read::FileRead)
            .tool(mv_core::tools::shell_exec::ShellExec)
            .tool(mv_core::tools::http_get::HttpGet);
    }
    let agent_handle = tool_server.run();

    // Connect MCP servers via the manager (one-shot mode), which registers
    // cleaned tools on the agent handle. `--no-tools` skips MCP entirely; an
    // empty manager shuts down as a no-op.
    let mcp_manager = if args.no_tools {
        None
    } else {
        Some(McpManager::connect(args.mcp_config.as_deref(), &agent_handle).await?)
    };

    // Begin a memory session if requested. Memory rides the same MCP tools, so
    // it needs them attached; `--no-tools` disables it. Best-effort throughout:
    // a failure here (no klams configured, server down) warns and proceeds with
    // no memory rather than failing the prompt. Recall context is prepended to
    // the prompt (the preamble is fixed at the provider call sites).
    let mut session_mem = None;
    let mut effective_prompt = prompt.to_string();
    if let Some(session) = &args.session {
        if args.no_tools {
            eprintln!("note: --session ignored with --no-tools (memory needs MCP tools)");
        } else {
            let meta = mv_core::memory::SessionMeta {
                agent_name: "mv-cli".to_string(),
                session: session.clone(),
                model: Some(entry.id.clone()),
                client_app: "mv-cli".to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            match mv_core::memory::SessionMemory::begin(
                mv_core::memory::KlamsMemory::new(agent_handle.clone()),
                meta,
            )
            .await
            {
                Ok(sm) => {
                    // The capability addendum is always present on a
                    // memory-active run (so the model knows it can write);
                    // recalled context is appended when there is any.
                    let mut prefix = sm.preamble_addendum();
                    if let Some(block) = sm.recall_block(prompt).await {
                        prefix.push_str("\n\n");
                        prefix.push_str(&block);
                    }
                    effective_prompt = format!("{prefix}\n\n{prompt}");
                    session_mem = Some(sm);
                }
                Err(e) => {
                    eprintln!("note: memory unavailable for session '{session}': {e}");
                }
            }
        }
    }

    // Streaming keeps single-model semantics — no mid-stream fallback (tokens
    // already shown can't be unshown), so it bypasses the chain walker.
    let result = if stream_trtllm_path {
        stream_trtllm(entry, &endpoint, &effective_prompt, agent_handle)
            .await
            .map(|text| CompletionOutcome {
                text,
                model_used: entry.id.clone(),
            })
    } else {
        complete_with_fallback(
            &registry,
            entry,
            &endpoint,
            &effective_prompt,
            agent_handle,
            &GenParams::default(),
        )
        .await
    };

    // Record the turn BEFORE tearing down MCP — recording talks to klams over
    // the same connections. Best-effort: a failure warns, never blocks. The
    // original prompt is recorded, not the recall-augmented one.
    if let (Some(sm), Ok(outcome)) = (&session_mem, &result)
        && let Err(e) = sm.record(prompt, &outcome.text, &outcome.model_used).await
    {
        eprintln!("note: failed to record turn to memory: {e}");
    }

    // Always shut down MCP connections, even on error
    if let Some(manager) = mcp_manager {
        manager.shutdown().await;
    }

    let outcome = result?;
    info!(len = outcome.text.len(), model_used = %outcome.model_used, "received response");

    // Surface a fallback in text mode (JSON carries `model_used` instead).
    if !json && outcome.model_used != entry.id {
        eprintln!(
            "note: '{}' was unavailable; this response was served by fallback model '{}'",
            entry.id, outcome.model_used
        );
    }

    Ok(outcome)
}
