//! Request handlers. Each returns `Result<Json<T>, ApiError>`, so an `MvError`
//! propagates through `?` into the shared error envelope. The actual work is
//! delegated to the `mv_core` runtime and workflow engine — handlers only do
//! HTTP-shaped translation (parse the request, resolve the model/path, call
//! core, shape the response).

use std::collections::HashMap;
use std::path::{Component, PathBuf};

use axum::Json;
use axum::extract::State;
use mv_core::runtime::{GenParams, HandleToolExecutor, RigPromptExecutor, complete_with_fallback};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

// --- GET /health ---

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub models: usize,
    pub mcp_servers_configured: usize,
    pub mcp_servers_live: usize,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        models: state.registry.entries().len(),
        mcp_servers_configured: state.mcp.configured_servers().len(),
        mcp_servers_live: state.mcp.live_servers().await.len(),
    })
}

// --- GET /v1/models ---

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub locality: String,
    pub endpoint: String,
    pub default: bool,
}

#[derive(Serialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

pub async fn list_models(State(state): State<AppState>) -> Json<ModelsResponse> {
    let models = state
        .registry
        .entries()
        .iter()
        .map(|e| ModelInfo {
            id: e.id.clone(),
            provider: e.provider.to_string(),
            locality: e.locality().to_string(),
            endpoint: e.endpoint(),
            default: e.default,
        })
        .collect();
    Json(ModelsResponse { models })
}

// --- POST /v1/prompt ---

#[derive(Deserialize)]
pub struct PromptRequest {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

#[derive(Serialize)]
pub struct PromptResponse {
    pub response: String,
    pub model_used: String,
}

pub async fn prompt(
    State(state): State<AppState>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, ApiError> {
    let prompt = mv_core::validate_prompt(&req.prompt)?;

    let entry = match &req.model {
        Some(id) => state
            .registry
            .get(id)
            .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                model: id.clone(),
                available: state.registry.available_ids().join(", "),
            })?,
        None => state.registry.default_model(),
    };

    let endpoint = entry.endpoint();
    let params = GenParams {
        temperature: req.temperature,
        max_tokens: req.max_tokens,
    };

    let outcome = complete_with_fallback(
        &state.registry,
        entry,
        &endpoint,
        prompt,
        state.agent_handle.clone(),
        &params,
    )
    .await?;

    Ok(Json(PromptResponse {
        response: outcome.text,
        model_used: outcome.model_used,
    }))
}

// --- POST /v1/workflows/run ---

#[derive(Deserialize)]
pub struct WorkflowRequest {
    /// Workflow file name, resolved inside the server's `--workflows-dir`.
    pub workflow: String,
    #[serde(default)]
    pub inputs: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct WorkflowResponse {
    pub workflow: String,
    pub outputs: HashMap<String, serde_json::Value>,
}

/// Resolve a caller-supplied workflow name strictly inside `root`. Rejects any
/// name that is absolute or contains a `..` / root / prefix component, before
/// any file is opened (FR-011). Returns the joined path on success.
fn resolve_workflow_path(root: &std::path::Path, name: &str) -> Result<PathBuf, ApiError> {
    let candidate = std::path::Path::new(name);
    let escapes = candidate.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if name.is_empty() || escapes {
        return Err(ApiError::bad_request(
            "WORKFLOW_PATH_OUTSIDE_ROOT",
            format!("workflow name '{name}' must be a plain path inside the workflows directory"),
        ));
    }
    Ok(root.join(candidate))
}

/// Coerce JSON inputs to the engine's `String` context values (CLI inputs are
/// strings too): a string passes through verbatim; anything else uses its
/// compact JSON form, so `{"n": 5}` becomes `"5"`.
fn stringify_inputs(inputs: HashMap<String, serde_json::Value>) -> HashMap<String, String> {
    inputs
        .into_iter()
        .map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            (k, s)
        })
        .collect()
}

pub async fn run_workflow(
    State(state): State<AppState>,
    Json(req): Json<WorkflowRequest>,
) -> Result<Json<WorkflowResponse>, ApiError> {
    let path = resolve_workflow_path(&state.workflows_dir, &req.workflow)?;

    let workflow = mv_core::workflow::parser::load_from_file(&path)?;

    let validation_errors =
        mv_core::workflow::validate::validate(&workflow, Some(&state.workflows_dir));
    if !validation_errors.is_empty() {
        let details = validation_errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(mv_core::MvError::WorkflowValidationError { details }.into());
    }

    // Validate model references up front (same as the CLI): a typo'd model
    // fails before execution rather than mid-run.
    for (_step_id, model_id) in workflow.model_references() {
        if state.registry.get(&model_id).is_none() {
            return Err(mv_core::MvError::ModelNotInRegistry {
                model: model_id,
                available: state.registry.available_ids().join(", "),
            }
            .into());
        }
    }

    let default_model = state.registry.default_model().id.clone();
    let prompt_exec = RigPromptExecutor {
        registry: (*state.registry).clone(),
        agent_handle: state.agent_handle.clone(),
    };
    let tool_exec = HandleToolExecutor {
        handle: state.agent_handle.clone(),
    };

    let result = mv_core::workflow::engine::execute_workflow(
        &workflow,
        stringify_inputs(req.inputs),
        &prompt_exec,
        &tool_exec,
        &state.workflows_dir,
        &default_model,
    )
    .await?;

    Ok(Json(WorkflowResponse {
        workflow: workflow.name,
        outputs: result.outputs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_traversal_and_absolute() {
        let root = std::path::Path::new("/srv/workflows");
        assert!(resolve_workflow_path(root, "../etc/passwd").is_err());
        assert!(resolve_workflow_path(root, "a/../../b.yaml").is_err());
        assert!(resolve_workflow_path(root, "/etc/passwd").is_err());
        assert!(resolve_workflow_path(root, "").is_err());
    }

    #[test]
    fn resolve_accepts_plain_names() {
        let root = std::path::Path::new("/srv/workflows");
        assert_eq!(
            resolve_workflow_path(root, "report.yaml").unwrap(),
            std::path::Path::new("/srv/workflows/report.yaml")
        );
        // A nested subdirectory inside the root is allowed.
        assert_eq!(
            resolve_workflow_path(root, "sub/report.yaml").unwrap(),
            std::path::Path::new("/srv/workflows/sub/report.yaml")
        );
    }

    #[test]
    fn stringify_inputs_passes_strings_and_jsonifies_rest() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), serde_json::json!("ken"));
        m.insert("count".to_string(), serde_json::json!(5));
        let out = stringify_inputs(m);
        assert_eq!(out["name"], "ken");
        assert_eq!(out["count"], "5");
    }
}
