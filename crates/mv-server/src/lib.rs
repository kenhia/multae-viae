//! mv-server: a long-running REST controller over the shared `mv_core` runtime.
//!
//! The crate is a **library plus a thin binary**: all routing and state
//! construction live here so the router can be exercised in-process with
//! `tower::ServiceExt::oneshot` (no port binding, no subprocess), and
//! [`crate::main`] only parses flags, builds the state, binds, and serves.
//!
//! Handlers drive the same code paths the CLI uses — `mv_core::runtime` for
//! completions and the workflow engine for `workflow run` — so behavior is
//! identical across front ends; mv-server adds only the HTTP surface, sessions,
//! and scheduling.

pub mod error;
pub mod handlers;
pub mod state;

use axum::Router;
use axum::routing::{get, post};

pub use state::AppState;

/// Build the application router over a constructed [`AppState`].
///
/// Split out from binding/serving so tests can call
/// `build_router(state).oneshot(request)` directly.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/models", get(handlers::list_models))
        .route("/v1/prompt", post(handlers::prompt))
        .route("/v1/workflows/run", post(handlers::run_workflow))
        .with_state(state)
}
