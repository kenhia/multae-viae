pub mod health;
pub mod stop;
pub mod usage;

/// User-facing hint for starting a TRT-LLM backend. Built once here so the
/// buffered, streaming, and (future) server paths never drift.
pub const START_HINT: &str = "Start the server with: trtllm-serve <model-path>";

/// User-facing hint for loading a model on the TRT-LLM proxy (the
/// trt-llm-explore repo's justfile). Single source for the
/// `ModelNotLoaded` hint, shared by error classification and preflight.
pub fn load_hint(id: &str) -> String {
    format!("Run: just load {id}")
}
