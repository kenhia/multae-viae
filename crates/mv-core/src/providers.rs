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
    use rig::completion::{CompletionError, PromptError};

    if let PromptError::MaxTurnsError { max_turns, .. } = error {
        return MvError::MaxTurnsExceeded {
            turns: *max_turns as u64,
        };
    }
    // A backend that answered with an error *status* was reached — classify by
    // status (the typed shape is robust to message rewording), never as
    // unreachable. The string path below is the fallback for layers that have
    // already flattened the error to text (SSE streaming).
    if let PromptError::CompletionError(CompletionError::HttpError(he)) = error
        && let Some((status, body)) = http_status(he)
    {
        return classify_http_status(status, &body, model, endpoint, trtllm_load_id);
    }
    classify_backend_error(&error.to_string(), model, endpoint, hint, trtllm_load_id)
}

/// Extract `(status, body)` from a rig HTTP-layer error that carries one.
/// `None` for transport failures (no response — those are unreachable).
fn http_status(e: &rig::http_client::Error) -> Option<(u16, String)> {
    use rig::http_client::Error;
    match e {
        Error::InvalidStatusCodeWithMessage(s, body) => Some((s.as_u16(), body.clone())),
        Error::InvalidStatusCode(s) => Some((s.as_u16(), String::new())),
        _ => None,
    }
}

/// Find a `status code NNN` (100–599) in a flattened error string. `None` if
/// no status is present (i.e. the message is not a status-bearing response).
fn parse_http_status(msg: &str) -> Option<u16> {
    let lower = msg.to_lowercase();
    let idx = lower.find("status code")?;
    let after = &msg[idx + "status code".len()..];
    let digits: String = after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse::<u16>()
        .ok()
        .filter(|s| (100..=599).contains(s))
}

/// Decide the error for a backend that *responded* with `status`. Shared by
/// the typed and string classification paths so both agree.
fn classify_http_status(
    status: u16,
    body: &str,
    model: &str,
    endpoint: &str,
    trtllm_load_id: Option<&str>,
) -> MvError {
    // TRT-LLM 502 = proxy reached but the model is not loaded; keep the hint.
    // Checked first: the live proxy's 502 body also contains "is not found",
    // which would otherwise be read as ModelNotFound.
    if status == 502
        && let Some(id) = trtllm_load_id
    {
        return MvError::ModelNotLoaded {
            model: id.to_string(),
            hint: crate::trtllm::load_hint(id),
        };
    }
    if body.contains("not found") || (body.contains("model") && body.contains("pull")) {
        return MvError::ModelNotFound {
            model: model.to_string(),
        };
    }
    if (500..=599).contains(&status) {
        return MvError::BackendErrorResponse {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            status,
            details: body.to_string(),
        };
    }
    // 4xx (or any other status that reached us): the request was understood and
    // rejected — another model will not help. Fail fast.
    MvError::CompletionFailed {
        details: format!("backend returned HTTP {status}: {body}"),
    }
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
    } else if let Some(status) = parse_http_status(msg) {
        // The backend responded with an error status — reached, not unreachable.
        classify_http_status(status, msg, model, endpoint, trtllm_load_id)
    } else if msg.contains("connection")
        || msg.contains("Connection")
        || msg.contains("connect")
        || msg.contains("tcp")
        || msg.contains("error sending request")
    {
        // Genuine transport failure. NB: a bare "HttpError" no longer routes
        // here — rig stamps it on status responses too, which are *reached*.
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
                | MvError::BackendErrorResponse { .. }
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
                | MvError::BackendErrorResponse { .. }
                | MvError::CompletionFailed { .. }
                | MvError::ToolCallFailed { .. }
                | MvError::McpServerError { .. }
        )
    }

    /// A stable, machine-readable SCREAMING_SNAKE_CASE code for this error.
    ///
    /// The `Display` message is for humans (and may change wording); this code
    /// is the contract a programmatic caller branches on — most importantly the
    /// `mv-server` HTTP error envelope and the CLI's `--json` errors. The match
    /// is **exhaustive on purpose** (no `_` arm): adding an `MvError` variant
    /// without assigning it a code is a compile error, which is what keeps the
    /// code set complete as the taxonomy grows.
    pub fn code(&self) -> &'static str {
        match self {
            MvError::EmptyPrompt => "EMPTY_PROMPT",
            MvError::BackendUnreachable { .. } => "BACKEND_UNREACHABLE",
            MvError::ModelNotFound { .. } => "MODEL_NOT_FOUND",
            MvError::ModelNotLoaded { .. } => "MODEL_NOT_LOADED",
            MvError::StreamingNotSupported => "STREAMING_NOT_SUPPORTED",
            MvError::MaxTurnsExceeded { .. } => "MAX_TURNS_EXCEEDED",
            MvError::ToolCallFailed { .. } => "TOOL_CALL_FAILED",
            MvError::CompletionFailed { .. } => "COMPLETION_FAILED",
            MvError::ConfigNotFound { .. } => "CONFIG_NOT_FOUND",
            MvError::ConfigParseError { .. } => "CONFIG_PARSE_ERROR",
            MvError::ModelNotInRegistry { .. } => "MODEL_NOT_IN_REGISTRY",
            MvError::AllModelsFailed { .. } => "ALL_MODELS_FAILED",
            MvError::ApiKeyMissing { .. } => "API_KEY_MISSING",
            MvError::McpConfigNotFound { .. } => "MCP_CONFIG_NOT_FOUND",
            MvError::McpConfigParseError { .. } => "MCP_CONFIG_PARSE_ERROR",
            MvError::McpServerError { .. } => "MCP_SERVER_ERROR",
            MvError::McpDuplicateServer { .. } => "MCP_DUPLICATE_SERVER",
            MvError::WorkflowFileNotFound { .. } => "WORKFLOW_FILE_NOT_FOUND",
            MvError::WorkflowParseError { .. } => "WORKFLOW_PARSE_ERROR",
            MvError::WorkflowValidationError { .. } => "WORKFLOW_VALIDATION_ERROR",
            MvError::WorkflowStepFailed { .. } => "WORKFLOW_STEP_FAILED",
            MvError::WorkflowStepError { .. } => "WORKFLOW_STEP_ERROR",
            MvError::WorkflowInputMissing { .. } => "WORKFLOW_INPUT_MISSING",
            MvError::WorkflowInputInvalid { .. } => "WORKFLOW_INPUT_INVALID",
            MvError::WorkflowTemplateError { .. } => "WORKFLOW_TEMPLATE_ERROR",
            MvError::WorkflowParallelFailed { .. } => "WORKFLOW_PARALLEL_FAILED",
            MvError::MemoryError { .. } => "MEMORY_ERROR",
            MvError::WorkflowCycle { .. } => "WORKFLOW_CYCLE",
            MvError::WorkflowDepthExceeded { .. } => "WORKFLOW_DEPTH_EXCEEDED",
            MvError::BackendErrorResponse { .. } => "BACKEND_ERROR_RESPONSE",
        }
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
    fn classify_500_is_a_backend_error_response_not_unreachable() {
        // A 500 means the backend was *reached* and failed — not unreachable,
        // and not "model not loaded" (that is the 502 mapping).
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
        match err {
            MvError::BackendErrorResponse { status, .. } => assert_eq!(status, 500),
            other => panic!("expected BackendErrorResponse, got: {other:?}"),
        }
        // And it is fallback-eligible (try the next model).
        assert!(err_eligible(500));
    }

    fn err_eligible(status: u16) -> bool {
        classify_http_status(status, "", "m", "e", None).is_fallback_eligible()
    }

    #[test]
    fn typed_500_response_reports_truthfully_not_unreachable() {
        // The real bug: rig's CompletionError Display is "HttpError: ...", whose
        // bare substring used to route a *reached* 500 to BackendUnreachable
        // ("Is the server running?"). The typed path classifies by status.
        let typed = rig::completion::PromptError::CompletionError(
            rig::completion::CompletionError::HttpError(
                rig::http_client::Error::InvalidStatusCodeWithMessage(
                    reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                    "{\"error\":\"llama runner process has terminated\"}".to_string(),
                ),
            ),
        );
        let err = classify_prompt_error(
            &typed,
            "qwen3:8b",
            "http://localhost:11434",
            "Is Ollama running?",
            None,
        );
        match err {
            MvError::BackendErrorResponse {
                status,
                ref details,
                ..
            } => {
                assert_eq!(status, 500);
                assert!(details.contains("runner process has terminated"));
            }
            other => panic!("expected BackendErrorResponse, got: {other:?}"),
        }
        // The message must not lie about reachability.
        let msg = err.to_string();
        assert!(!msg.contains("Is Ollama running"), "{msg}");
        assert!(!msg.to_lowercase().contains("cannot reach"), "{msg}");
    }

    #[test]
    fn typed_4xx_fails_fast_not_eligible() {
        let typed = rig::completion::PromptError::CompletionError(
            rig::completion::CompletionError::HttpError(
                rig::http_client::Error::InvalidStatusCodeWithMessage(
                    reqwest::StatusCode::BAD_REQUEST,
                    "bad request".to_string(),
                ),
            ),
        );
        let err = classify_prompt_error(&typed, "m", "http://e", "h", None);
        assert!(
            matches!(err, MvError::CompletionFailed { .. }),
            "got: {err:?}"
        );
        assert!(!err.is_fallback_eligible());
    }

    #[test]
    fn transport_failure_is_still_unreachable() {
        // No status in the message → a genuine transport failure.
        let err = classify_backend_error(
            "error sending request for url (http://localhost:11434)",
            "qwen3:8b",
            "http://localhost:11434",
            "Is Ollama running?",
            None,
        );
        assert!(
            matches!(err, MvError::BackendUnreachable { .. }),
            "got: {err:?}"
        );
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

    /// One instance of every `MvError` variant. The `code()` match is
    /// exhaustive at compile time; this list is exhaustive at test time so the
    /// uniqueness/format pins below cover the whole taxonomy. Add a variant →
    /// `code()` won't compile without a code → add it here too.
    fn one_of_each() -> Vec<MvError> {
        let s = || "x".to_string();
        vec![
            MvError::EmptyPrompt,
            MvError::BackendUnreachable {
                endpoint: s(),
                hint: s(),
            },
            MvError::ModelNotFound { model: s() },
            MvError::ModelNotLoaded {
                model: s(),
                hint: s(),
            },
            MvError::StreamingNotSupported,
            MvError::MaxTurnsExceeded { turns: 10 },
            MvError::ToolCallFailed {
                tool: s(),
                details: s(),
            },
            MvError::CompletionFailed { details: s() },
            MvError::ConfigNotFound { path: s() },
            MvError::ConfigParseError {
                path: s(),
                details: s(),
            },
            MvError::ModelNotInRegistry {
                model: s(),
                available: s(),
            },
            MvError::AllModelsFailed { attempts: vec![] },
            MvError::ApiKeyMissing {
                provider: s(),
                env_var: s(),
            },
            MvError::McpConfigNotFound { path: s() },
            MvError::McpConfigParseError {
                path: s(),
                details: s(),
            },
            MvError::McpServerError {
                server: s(),
                details: s(),
            },
            MvError::McpDuplicateServer { name: s() },
            MvError::WorkflowFileNotFound { path: s() },
            MvError::WorkflowParseError {
                path: s(),
                details: s(),
            },
            MvError::WorkflowValidationError { details: s() },
            MvError::WorkflowStepFailed {
                step: s(),
                details: s(),
            },
            MvError::WorkflowStepError {
                step: s(),
                source: Box::new(MvError::EmptyPrompt),
            },
            MvError::WorkflowInputMissing { name: s() },
            MvError::WorkflowInputInvalid {
                name: s(),
                value: s(),
                allowed: s(),
            },
            MvError::WorkflowTemplateError {
                step: s(),
                details: s(),
            },
            MvError::WorkflowParallelFailed {
                step: s(),
                failures: vec![],
            },
            MvError::MemoryError {
                op: s(),
                details: s(),
            },
            MvError::WorkflowCycle { chain: s() },
            MvError::WorkflowDepthExceeded { max: 8 },
            MvError::BackendErrorResponse {
                endpoint: s(),
                model: s(),
                status: 500,
                details: s(),
            },
        ]
    }

    #[test]
    fn every_error_code_is_unique_and_well_formed() {
        let errs = one_of_each();
        let codes: Vec<&str> = errs.iter().map(MvError::code).collect();

        // Non-empty, SCREAMING_SNAKE_CASE (A–Z, 0–9, underscore; not leading/
        // trailing/doubled underscore).
        for code in &codes {
            assert!(!code.is_empty(), "empty code");
            assert!(
                code.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
                "code '{code}' is not SCREAMING_SNAKE_CASE"
            );
            assert!(
                !code.starts_with('_') && !code.ends_with('_') && !code.contains("__"),
                "code '{code}' has a misplaced underscore"
            );
        }

        // Unique across the whole taxonomy.
        let unique: std::collections::HashSet<&str> = codes.iter().copied().collect();
        assert_eq!(
            unique.len(),
            codes.len(),
            "duplicate error code(s): {codes:?}"
        );
    }

    #[test]
    fn key_error_codes_are_stable() {
        // Pin the codes downstream callers (mv-server, --json) branch on.
        assert_eq!(
            MvError::ModelNotLoaded {
                model: "m".into(),
                hint: "h".into()
            }
            .code(),
            "MODEL_NOT_LOADED"
        );
        assert_eq!(
            MvError::ModelNotInRegistry {
                model: "m".into(),
                available: "a".into()
            }
            .code(),
            "MODEL_NOT_IN_REGISTRY"
        );
        assert_eq!(MvError::EmptyPrompt.code(), "EMPTY_PROMPT");
        assert_eq!(
            MvError::BackendErrorResponse {
                endpoint: "e".into(),
                model: "m".into(),
                status: 500,
                details: "d".into()
            }
            .code(),
            "BACKEND_ERROR_RESPONSE"
        );
    }
}
