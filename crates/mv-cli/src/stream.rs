//! Terminal streaming for TRT-LLM. The one piece of provider dispatch that
//! stays in the binary: it writes tokens to stdout as they arrive, which is a
//! CLI concern (the server has no streaming this sprint). The agent itself is
//! built by the shared `mv_core::runtime::trtllm_agent`, so routing and stop
//! sequences match the buffered path exactly.

use rig::tool::server::ToolServerHandle;
use tracing::{debug, info};

use mv_core::preflight::{PreflightStatus, preflight};
use mv_core::providers::classify_backend_error;
use mv_core::runtime::{GenParams, trtllm_agent};
use mv_core::trtllm::START_HINT;
use mv_core::{ModelEntry, MvError};

/// Network timeout for the streaming preflight probe — short, matching the
/// buffered path's preflight budget.
const PREFLIGHT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

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
