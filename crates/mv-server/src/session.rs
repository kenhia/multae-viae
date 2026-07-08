//! Held-open conversation sessions.
//!
//! A [`Session`] keeps an [`AnyAgent`] built once and the running chat history,
//! so successive turns share context without rebuilding the agent. Turns within
//! a session are serialized by the per-session mutex in [`crate::state::SessionMap`];
//! distinct sessions run concurrently.
//!
//! Persistence reuses the sprint-011 memory seam: when a klams MCP server is
//! connected, each turn is recorded and the first turn of a (re)created session
//! recalls prior context — the same `SessionMemory<KlamsMemory>` path the CLI's
//! `--session` flag uses, so a session survives a server restart by name.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use mv_core::ModelEntry;
use mv_core::MvError;
use mv_core::memory::{KlamsMemory, SessionMemory, SessionMeta};
use mv_core::runtime::{AnyAgent, GenParams};
use rig::completion::Message;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::ApiError;
use crate::state::AppState;

/// One held-open conversation.
pub struct Session {
    pub name: String,
    /// The resolved model entry (carries id, provider, endpoint defaults).
    pub entry: ModelEntry,
    pub endpoint: String,
    agent: AnyAgent,
    history: Vec<Message>,
    /// Best-effort persistence; `None` when no klams server is connected.
    memory: Option<SessionMemory<KlamsMemory>>,
}

impl Session {
    /// Run one turn: prompt the held agent with the running history, append the
    /// exchange, and record it to memory (best-effort). On the first turn of a
    /// session backed by memory, prior context is recalled and prepended to the
    /// prompt — mirroring the CLI's `--session` behavior, which is what makes a
    /// session recoverable by name after a restart.
    pub async fn turn(&mut self, prompt: &str) -> Result<String, MvError> {
        let mut effective = prompt.to_string();
        if self.history.is_empty()
            && let Some(mem) = &self.memory
        {
            let mut prefix = mem.preamble_addendum();
            if let Some(block) = mem.recall_block(prompt).await {
                prefix.push_str("\n\n");
                prefix.push_str(&block);
            }
            effective = format!("{prefix}\n\n{prompt}");
        }

        let reply = self
            .agent
            .chat_turn(
                &self.entry,
                &self.endpoint,
                &effective,
                self.history.clone(),
            )
            .await?;

        // History records the original prompt (not the recall-augmented one),
        // so context grows naturally turn over turn.
        self.history.push(Message::user(prompt));
        self.history.push(Message::assistant(reply.clone()));

        if let Some(mem) = &self.memory
            && let Err(e) = mem.record(prompt, &reply, &self.entry.id).await
        {
            tracing::warn!(session = %self.name, error = %e, "failed to record turn to memory");
        }

        Ok(reply)
    }
}

// --- POST /v1/sessions ---

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub name: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub name: String,
    pub model: String,
}

pub async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionInfo>), ApiError> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request(
            "SESSION_NAME_EMPTY",
            "session name must not be empty",
        ));
    }

    let entry = match &req.model {
        Some(id) => state
            .registry
            .get(id)
            .ok_or_else(|| MvError::ModelNotInRegistry {
                model: id.clone(),
                available: state.registry.available_ids().join(", "),
            })?,
        None => state.registry.default_model(),
    }
    .clone();
    let endpoint = entry.endpoint();

    let agent = AnyAgent::build(
        &entry,
        &endpoint,
        state.agent_handle.clone(),
        &GenParams::default(),
    )?;

    // Best-effort memory: begins only if a klams server is connected and
    // registration succeeds. Failure leaves the session memory-less.
    let meta = SessionMeta {
        agent_name: "mv-server".to_string(),
        session: req.name.clone(),
        model: Some(entry.id.clone()),
        client_app: "mv-server".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let memory = SessionMemory::begin(KlamsMemory::new(state.agent_handle.clone()), meta)
        .await
        .ok();

    let session = Session {
        name: req.name.clone(),
        entry: entry.clone(),
        endpoint,
        agent,
        history: Vec::new(),
        memory,
    };

    let mut map = state.sessions.lock().await;
    if map.contains_key(&req.name) {
        return Err(ApiError::conflict(
            "SESSION_EXISTS",
            format!("session '{}' already exists", req.name),
        ));
    }
    map.insert(req.name.clone(), Arc::new(Mutex::new(session)));

    Ok((
        StatusCode::CREATED,
        Json(SessionInfo {
            name: req.name,
            model: entry.id,
        }),
    ))
}

// --- GET /v1/sessions ---

#[derive(Serialize)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionInfo>,
}

pub async fn list_sessions(State(state): State<AppState>) -> Json<SessionsResponse> {
    let map = state.sessions.lock().await;
    let mut sessions = Vec::with_capacity(map.len());
    for sess in map.values() {
        let s = sess.lock().await;
        sessions.push(SessionInfo {
            name: s.name.clone(),
            model: s.entry.id.clone(),
        });
    }
    Json(SessionsResponse { sessions })
}

// --- DELETE /v1/sessions/{name} ---

pub async fn delete_session(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut map = state.sessions.lock().await;
    if map.remove(&name).is_some() {
        // Dropping the Arc<Mutex<Session>> drops the held agent. Persisted klams
        // memory is retained — the name can be recreated and recall its history.
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(
            "SESSION_NOT_FOUND",
            format!("no session named '{name}'"),
        ))
    }
}

// --- POST /v1/sessions/{name}/turns ---

#[derive(Deserialize)]
pub struct TurnRequest {
    pub prompt: String,
}

#[derive(Serialize)]
pub struct TurnResponse {
    pub response: String,
    pub model_used: String,
}

pub async fn session_turn(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<TurnRequest>,
) -> Result<Json<TurnResponse>, ApiError> {
    let prompt = mv_core::validate_prompt(&req.prompt)?.to_string();

    // Clone the Arc and release the map lock before the (serialized) turn, so a
    // long turn in one session never blocks create/list/delete or other sessions.
    let session = {
        let map = state.sessions.lock().await;
        map.get(&name).cloned()
    };
    let Some(session) = session else {
        return Err(ApiError::not_found(
            "SESSION_NOT_FOUND",
            format!("no session named '{name}'; create it with POST /v1/sessions"),
        ));
    };

    let mut s = session.lock().await;
    let reply = s.turn(&prompt).await?;
    let model_used = s.entry.id.clone();
    Ok(Json(TurnResponse {
        response: reply,
        model_used,
    }))
}
