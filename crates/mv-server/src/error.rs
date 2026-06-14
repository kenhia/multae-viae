//! The HTTP error envelope.
//!
//! Every failing handler returns an [`ApiError`], which renders as
//! `{"error": {"code", "message", "hint"?}}` with an HTTP status derived from
//! the underlying [`mv_core::MvError`]. The orphan rule prevents implementing
//! axum's `IntoResponse` directly for the foreign `MvError`, so [`ApiError`] is
//! the local newtype that owns both the status mapping and the JSON shape —
//! handlers just `?` an `MvError` and the `From` impl does the rest.
//!
//! The `code` is `MvError::code()` verbatim — the same stable discriminant the
//! CLI's `--json` errors carry — so a caller branches on `MODEL_NOT_LOADED`
//! regardless of which front end produced it.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use mv_core::MvError;
use serde::Serialize;

/// A handler error: an HTTP status plus the machine-readable envelope fields.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
}

impl ApiError {
    /// A server-native `400 Bad Request` (e.g. a malformed request or a
    /// path-boundary violation) that does not originate from an `MvError`.
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
            hint: None,
        }
    }
}

/// Map an [`MvError`] to the HTTP status that best describes it.
///
/// Grouped by meaning, not by variant count: not-found shapes are 404, bad
/// caller input is 400, a backend that is down or has no usable model is 503,
/// a backend that answered with a server error is 502, and everything else —
/// genuine internal faults — is 500. The catch-all keeps this total without an
/// arm per variant; the `code` field still distinguishes them precisely.
fn status_for(err: &MvError) -> StatusCode {
    use MvError::*;
    match err {
        // Not found.
        ModelNotFound { .. }
        | ModelNotInRegistry { .. }
        | ConfigNotFound { .. }
        | McpConfigNotFound { .. }
        | WorkflowFileNotFound { .. } => StatusCode::NOT_FOUND,

        // Bad caller input / unsupported request.
        EmptyPrompt
        | StreamingNotSupported
        | WorkflowInputMissing { .. }
        | WorkflowInputInvalid { .. }
        | WorkflowValidationError { .. } => StatusCode::BAD_REQUEST,

        // Backend unavailable / no usable model.
        BackendUnreachable { .. } | ModelNotLoaded { .. } | AllModelsFailed { .. } => {
            StatusCode::SERVICE_UNAVAILABLE
        }

        // Backend was reached but returned a server error.
        BackendErrorResponse { .. } => StatusCode::BAD_GATEWAY,

        // Everything else is an internal fault.
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The structured hint a few variants carry, surfaced as the envelope's
/// optional `hint` field. (The hint text is also part of the `message` via
/// `Display`; exposing it separately lets a caller show it distinctly.)
fn hint_for(err: &MvError) -> Option<String> {
    match err {
        MvError::ModelNotLoaded { hint, .. } | MvError::BackendUnreachable { hint, .. } => {
            Some(hint.clone())
        }
        _ => None,
    }
}

impl From<MvError> for ApiError {
    fn from(err: MvError) -> Self {
        Self {
            status: status_for(&err),
            code: err.code(),
            message: err.to_string(),
            hint: hint_for(&err),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code,
                message: self.message,
                hint: self.hint,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_matrix() {
        // Each MvError moves straight into `ApiError` (no Clone needed); the
        // tuple pins the expected status + code together.
        let cases: Vec<(ApiError, StatusCode, &str)> = vec![
            (
                MvError::EmptyPrompt.into(),
                StatusCode::BAD_REQUEST,
                "EMPTY_PROMPT",
            ),
            (
                MvError::ModelNotInRegistry {
                    model: "m".into(),
                    available: "a".into(),
                }
                .into(),
                StatusCode::NOT_FOUND,
                "MODEL_NOT_IN_REGISTRY",
            ),
            (
                MvError::ModelNotFound { model: "m".into() }.into(),
                StatusCode::NOT_FOUND,
                "MODEL_NOT_FOUND",
            ),
            (
                MvError::ModelNotLoaded {
                    model: "m".into(),
                    hint: "Run: just load m".into(),
                }
                .into(),
                StatusCode::SERVICE_UNAVAILABLE,
                "MODEL_NOT_LOADED",
            ),
            (
                MvError::BackendUnreachable {
                    endpoint: "e".into(),
                    hint: "Is Ollama running?".into(),
                }
                .into(),
                StatusCode::SERVICE_UNAVAILABLE,
                "BACKEND_UNREACHABLE",
            ),
            (
                MvError::BackendErrorResponse {
                    endpoint: "e".into(),
                    model: "m".into(),
                    status: 500,
                    details: "d".into(),
                }
                .into(),
                StatusCode::BAD_GATEWAY,
                "BACKEND_ERROR_RESPONSE",
            ),
            (
                MvError::WorkflowValidationError {
                    details: "d".into(),
                }
                .into(),
                StatusCode::BAD_REQUEST,
                "WORKFLOW_VALIDATION_ERROR",
            ),
            (
                MvError::CompletionFailed {
                    details: "d".into(),
                }
                .into(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "COMPLETION_FAILED",
            ),
        ];

        for (api, want_status, want_code) in cases {
            assert_eq!(api.status, want_status, "status for {want_code}");
            assert_eq!(api.code, want_code, "code mismatch");
            assert!(!api.message.is_empty());
        }
    }

    #[test]
    fn not_loaded_carries_hint() {
        let api: ApiError = MvError::ModelNotLoaded {
            model: "m".into(),
            hint: "Run: just load m".into(),
        }
        .into();
        assert_eq!(api.hint.as_deref(), Some("Run: just load m"));
    }

    #[test]
    fn bad_request_constructor_is_400() {
        let api = ApiError::bad_request("WORKFLOW_PATH_OUTSIDE_ROOT", "nope");
        assert_eq!(api.status, StatusCode::BAD_REQUEST);
        assert_eq!(api.code, "WORKFLOW_PATH_OUTSIDE_ROOT");
        assert!(api.hint.is_none());
    }
}
