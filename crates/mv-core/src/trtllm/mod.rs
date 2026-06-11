pub mod health;
pub mod stop;
pub mod usage;

/// User-facing hint for starting a TRT-LLM backend. Built once here so the
/// buffered, streaming, and (future) server paths never drift.
pub const START_HINT: &str = "Start the server with: trtllm-serve <model-path>";
