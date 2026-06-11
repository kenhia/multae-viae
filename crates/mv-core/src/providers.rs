//! Provider-shared behavior: the system preamble and backend error
//! classification. Lives in mv-core (not the CLI) because every front end —
//! mv-cli today, mv-server in Phase 6 — must classify identically.

use crate::MvError;

/// System preamble attached to every agent, regardless of provider or front end.
pub const SYSTEM_PREAMBLE: &str = "\
You are a helpful assistant with access to local tools. \
Use the available tools to answer questions that require interacting with the local environment. \
If a question can be answered from your own knowledge, respond directly without using tools. \
When you use a tool, incorporate the result into a clear, human-readable response.";

/// Classify a typed rig prompt error. Typed variants are matched first
/// (exact, future-proof against message rewording); everything else falls
/// back to string classification of the rendered message.
pub fn classify_prompt_error(
    error: &rig::completion::PromptError,
    model: &str,
    endpoint: &str,
    hint: &str,
    trtllm_load_id: Option<&str>,
) -> MvError {
    if let rig::completion::PromptError::MaxTurnsError { max_turns, .. } = error {
        return MvError::MaxTurnsExceeded {
            turns: *max_turns as u64,
        };
    }
    classify_backend_error(&error.to_string(), model, endpoint, hint, trtllm_load_id)
}

/// Classify a backend failure from its rendered message. Used directly by
/// paths that only see strings (the SSE streaming layer); buffered paths
/// should prefer [`classify_prompt_error`].
pub fn classify_backend_error(
    msg: &str,
    model: &str,
    endpoint: &str,
    hint: &str,
    trtllm_load_id: Option<&str>,
) -> MvError {
    // Order matters. The TRT-LLM 502 → not-loaded mapping is checked FIRST:
    // rig surfaces a live-proxy 502 as a string that also contains "HttpError"
    // (the BackendUnreachable branch) and Triton's "...is not found" (the
    // ModelNotFound branch), so either would otherwise shadow it and swallow
    // the `just load` hint. This is safe for the connection-refused case:
    // a genuine refusal carries no "502", so it falls through to
    // BackendUnreachable below.
    if let Some(id) = trtllm_load_id
        && msg.contains("502")
    {
        MvError::ModelNotLoaded {
            model: id.to_string(),
            hint: crate::trtllm::load_hint(id),
        }
    } else if msg.contains("MaxTurnError") || msg.contains("max turn limit") {
        // String fallback for layers that flatten PromptError to text.
        let turns = msg
            .rsplit("max turn limit:")
            .next()
            .and_then(|s| s.trim().trim_end_matches(')').trim().parse::<u64>().ok())
            .unwrap_or(10);
        MvError::MaxTurnsExceeded { turns }
    } else if msg.contains("connection")
        || msg.contains("Connection")
        || msg.contains("connect")
        || msg.contains("tcp")
        || msg.contains("error sending request")
        || msg.contains("HttpError")
    {
        MvError::BackendUnreachable {
            endpoint: endpoint.to_string(),
            hint: hint.to_string(),
        }
    } else if msg.contains("not found") || (msg.contains("model") && msg.contains("pull")) {
        MvError::ModelNotFound {
            model: model.to_string(),
        }
    } else {
        MvError::CompletionFailed {
            details: msg.to_string(),
        }
    }
}

impl MvError {
    /// Whether a routing layer may try the next model in a fallback chain
    /// after this error. Backend-dead and provider-misconfiguration failures
    /// are eligible; user-input and workflow-definition errors must fail fast
    /// (retrying them elsewhere would mask the real problem).
    pub fn is_fallback_eligible(&self) -> bool {
        matches!(
            self,
            MvError::BackendUnreachable { .. }
                | MvError::ModelNotLoaded { .. }
                | MvError::ModelNotFound { .. }
                | MvError::ApiKeyMissing { .. }
        )
    }

    /// Whether an error class may be transient. The workflow retry handler
    /// re-attempts only these; permanent failures (validation, missing
    /// inputs, config mistakes) fail immediately regardless of
    /// `on_error: retry`. `ToolCallFailed` is included because tool failures
    /// are often transient (network, busy resource) — the cost is that a
    /// genuinely-unknown tool name is also retried until tool errors are
    /// differentiated.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            MvError::BackendUnreachable { .. }
                | MvError::CompletionFailed { .. }
                | MvError::ToolCallFailed { .. }
                | MvError::McpServerError { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HINT: &str = "Start the server with: trtllm-serve <model-path>";

    #[test]
    fn classify_502_maps_to_model_not_loaded() {
        let err = classify_backend_error(
            "HTTP error: status code: 502 Bad Gateway",
            "llama-fp8",
            "http://localhost:8003/v1",
            HINT,
            Some("llama-fp8"),
        );
        match err {
            MvError::ModelNotLoaded { model, hint } => {
                assert_eq!(model, "llama-fp8");
                assert_eq!(hint, "Run: just load llama-fp8");
            }
            other => panic!("expected ModelNotLoaded, got: {other:?}"),
        }
    }

    // Regression: this is the verbatim error rig surfaces from the live proxy
    // on a 502. It contains "HttpError" (matches the BackendUnreachable branch)
    // AND "...is not found" (matches the ModelNotFound branch) AND "502". The
    // TRT-LLM 502 → ModelNotLoaded mapping must win over both, or the
    // `just load` hint never fires against the real proxy.
    #[test]
    fn classify_502_with_not_found_body_maps_to_model_not_loaded() {
        let err = classify_backend_error(
            "CompletionError: HttpError: Invalid status code 502 Bad Gateway \
             with message: {\"detail\":\"Triton returned HTTP 404: \
             {\\\"error\\\":\\\"Request for unknown model: 'ensemble_llama-fp8' \
             is not found\\\"}\"}",
            "llama-fp8",
            "http://localhost:8003/v1",
            HINT,
            Some("llama-fp8"),
        );
        assert!(
            matches!(err, MvError::ModelNotLoaded { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn classify_500_does_not_map_to_model_not_loaded() {
        let err = classify_backend_error(
            "HTTP error: status code: 500 Internal Server Error",
            "llama-fp8",
            "http://localhost:8003/v1",
            HINT,
            Some("llama-fp8"),
        );
        assert!(
            !matches!(err, MvError::ModelNotLoaded { .. }),
            "got: {err:?}"
        );
        assert!(matches!(err, MvError::CompletionFailed { .. }));
    }

    #[test]
    fn classify_max_turns_string_fallback() {
        let err = classify_backend_error(
            "MaxTurnError: (reached max turn limit: 10)",
            "qwen3:8b",
            "http://localhost:11434",
            "Is Ollama running?",
            None,
        );
        assert!(matches!(err, MvError::MaxTurnsExceeded { turns: 10 }));
    }

    #[test]
    fn classify_max_turns_unparseable_count_defaults() {
        let err = classify_backend_error(
            "MaxTurnError: something unexpected",
            "qwen3:8b",
            "http://localhost:11434",
            "Is Ollama running?",
            None,
        );
        assert!(matches!(err, MvError::MaxTurnsExceeded { turns: 10 }));
    }

    #[test]
    fn classify_typed_max_turns_uses_exact_count() {
        let typed = rig::completion::PromptError::MaxTurnsError {
            max_turns: 7,
            chat_history: Box::new(vec![]),
            prompt: Box::new(rig::completion::Message::user("hi")),
        };
        let err = classify_prompt_error(
            &typed,
            "qwen3:8b",
            "http://localhost:11434",
            "Is Ollama running?",
            None,
        );
        assert!(matches!(err, MvError::MaxTurnsExceeded { turns: 7 }));
    }

    #[test]
    fn fallback_eligibility_taxonomy() {
        assert!(
            MvError::BackendUnreachable {
                endpoint: "e".into(),
                hint: "h".into()
            }
            .is_fallback_eligible()
        );
        assert!(
            MvError::ModelNotLoaded {
                model: "m".into(),
                hint: "h".into()
            }
            .is_fallback_eligible()
        );
        assert!(MvError::ModelNotFound { model: "m".into() }.is_fallback_eligible());
        assert!(
            MvError::ApiKeyMissing {
                provider: "openai".into(),
                env_var: "K".into()
            }
            .is_fallback_eligible()
        );

        assert!(!MvError::EmptyPrompt.is_fallback_eligible());
        assert!(!MvError::MaxTurnsExceeded { turns: 10 }.is_fallback_eligible());
        assert!(
            !MvError::CompletionFailed {
                details: "d".into()
            }
            .is_fallback_eligible()
        );
        assert!(
            !MvError::WorkflowValidationError {
                details: "d".into()
            }
            .is_fallback_eligible()
        );
    }
}
