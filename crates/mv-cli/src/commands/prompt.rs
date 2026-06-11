//! The `prompt` command: resolve a model, attach tools, run one completion.

use rig::tool::server::ToolServer;
use tracing::{debug, info};

use crate::cli::PromptArgs;
use crate::commands::connect_mcp_servers;
use crate::providers::{GenParams, complete, stream_trtllm};
use mv_core::Provider;

#[tracing::instrument(name = "mv_cli_request", skip(args), fields(
    prompt = %args.prompt,
    model = args.model.as_deref().unwrap_or("default"),
))]
pub async fn run_prompt(
    args: &PromptArgs,
    _json: bool,
    effective_stream: bool,
) -> std::result::Result<String, mv_core::MvError> {
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

    // Connect MCP servers to a separate handle, then register cleaned tools on
    // the agent handle (skipped entirely under --no-tools).
    let mcp_connections = if args.no_tools {
        Vec::new()
    } else {
        connect_mcp_servers(args.mcp_config.as_deref(), &agent_handle).await?
    };

    let result = if stream_trtllm_path {
        stream_trtllm(entry, &endpoint, prompt, agent_handle).await
    } else {
        complete(
            entry,
            &endpoint,
            prompt,
            agent_handle,
            &GenParams::default(),
        )
        .await
    };

    // Always shut down MCP connections, even on error
    mv_core::mcp::client::shutdown_all(mcp_connections).await;

    let response = result?;
    info!(len = response.len(), "received response");
    Ok(response)
}
