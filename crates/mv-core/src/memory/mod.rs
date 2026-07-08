//! Persistent-memory seam.
//!
//! A provider-agnostic trait the prompt path uses to give a run continuity:
//! register a session author, recall prior context before the completion, and
//! record the turn after. The klams-backed implementation ([`klams`]) drives
//! the pinned MCP tools through a `ToolServerHandle`; author ids cross the
//! trait boundary as opaque `String`s, so the trait stays free of klams/MCP
//! specifics and is mock-tested independently of any backend.
//!
//! Memory is **best-effort**: every method can fail, and the prompt path is
//! expected to warn-and-continue rather than block the user's request. See
//! `specs/011-klams-memory/`.

use std::future::Future;

use crate::MvError;

pub mod klams;

pub use klams::{KlamsMemory, SessionMemory};

/// Identity + metadata for a memory-active run, recorded once at session start
/// via [`MemoryStore::register_session`]. Maps onto klams `register_author`
/// (`session` becomes `session_title`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    /// `"mv-cli"` for real runs, `"multae-viae"` for live tests
    /// (see contract §Attribution).
    pub agent_name: String,
    /// The `--session` name; distinguishes runs of the same conversation.
    pub session: String,
    /// The resolved model id behind this run, if known.
    pub model: Option<String>,
    pub client_app: String,
    pub client_version: String,
}

/// One completed prompt/response turn, recorded after the completion as a
/// klams `conversation` event. Oversized text fields are capped client-side
/// ([`truncate_field`]) — events are records, not archives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub session: String,
    pub prompt: String,
    pub response: String,
    pub model_used: String,
}

/// Knobs for a recall query. Kept small deliberately: stuffing a prompt with
/// stale context hurts answers, and small reads stay well under the MCP
/// tool-output cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallOpts {
    pub top_k: u32,
}

impl Default for RecallOpts {
    fn default() -> Self {
        Self { top_k: 5 }
    }
}

/// A recalled item, flattened to what the prompt path renders into context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryItem {
    /// `"knowledge"` | `"fact"` | `"event"`.
    pub kind: String,
    /// Human-readable rendering of the item for the model to read.
    pub text: String,
}

/// Persistent memory backing a session. Implementors are `Send + Sync` and
/// return `Send` futures (the prompt path is async); the binary's klams impl
/// drives the pinned MCP tools, but nothing here depends on that.
pub trait MemoryStore: Send + Sync {
    /// Register the run's author, returning an opaque author id all of this
    /// run's writes must carry. One call per memory-active run.
    fn register_session(
        &self,
        meta: SessionMeta,
    ) -> impl Future<Output = Result<String, MvError>> + Send;

    /// Append one completed turn under `author_id`.
    fn record_turn(
        &self,
        author_id: &str,
        turn: TurnRecord,
    ) -> impl Future<Output = Result<(), MvError>> + Send;

    /// Retrieve relevant memories for `query` (semantic + full-text on the
    /// klams side). Ordered most-relevant first.
    fn recall(
        &self,
        query: &str,
        opts: RecallOpts,
    ) -> impl Future<Output = Result<Vec<MemoryItem>, MvError>> + Send;

    /// Fetch the most recent events for `session`, newest first, capped at
    /// `limit` — the continuity backbone (what happened last time).
    fn session_events(
        &self,
        session: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<MemoryItem>, MvError>> + Send;
}

/// Max characters retained per recorded turn field. Events are records, not
/// archives; longer prompt/response text is truncated UTF-8-safely.
pub const FIELD_CAP: usize = 4_000;

/// Cap a turn field at [`FIELD_CAP`] without splitting a UTF-8 codepoint.
/// Delegates to the shared tool-output truncator so the boundary logic (and
/// the F1 panic lesson) lives in exactly one place.
#[must_use]
pub fn truncate_field(s: &str) -> String {
    crate::tools::truncate_output(s, FIELD_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory mock proving the trait is usable and the contract holds:
    /// register returns an id, writes accumulate under it, recall/events read
    /// back. Mirrors the shape the stateful FakeKlams will take in WS1/T002.
    #[derive(Default)]
    struct MockStore {
        registrations: Mutex<Vec<SessionMeta>>,
        turns: Mutex<Vec<(String, TurnRecord)>>,
    }

    impl MemoryStore for MockStore {
        async fn register_session(&self, meta: SessionMeta) -> Result<String, MvError> {
            self.registrations.lock().unwrap().push(meta);
            Ok("author-1".to_string())
        }

        async fn record_turn(&self, author_id: &str, turn: TurnRecord) -> Result<(), MvError> {
            self.turns
                .lock()
                .unwrap()
                .push((author_id.to_string(), turn));
            Ok(())
        }

        async fn recall(&self, query: &str, opts: RecallOpts) -> Result<Vec<MemoryItem>, MvError> {
            // Echo the query back as a single item, honoring top_k as a cap.
            let mut items = vec![MemoryItem {
                kind: "knowledge".to_string(),
                text: format!("recalled for: {query}"),
            }];
            items.truncate(opts.top_k as usize);
            Ok(items)
        }

        async fn session_events(
            &self,
            session: &str,
            _limit: u32,
        ) -> Result<Vec<MemoryItem>, MvError> {
            // Surface any recorded turns for this session as events.
            let turns = self.turns.lock().unwrap();
            Ok(turns
                .iter()
                .filter(|(_, t)| t.session == session)
                .map(|(_, t)| MemoryItem {
                    kind: "event".to_string(),
                    text: format!("Q: {} / A: {}", t.prompt, t.response),
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn register_then_record_then_read_back() {
        let store = MockStore::default();
        let author = store
            .register_session(SessionMeta {
                agent_name: "mv-cli".to_string(),
                session: "research".to_string(),
                model: Some("qwen3:8b".to_string()),
                client_app: "mv-cli".to_string(),
                client_version: "0.1.0".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(author, "author-1");

        store
            .record_turn(
                &author,
                TurnRecord {
                    session: "research".to_string(),
                    prompt: "what embedding model?".to_string(),
                    response: "BGE".to_string(),
                    model_used: "qwen3:8b".to_string(),
                },
            )
            .await
            .unwrap();

        let events = store.session_events("research", 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].text.contains("what embedding model?"));

        // A different session sees nothing.
        assert!(store.session_events("other", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recall_honors_top_k() {
        let store = MockStore::default();
        let items = store.recall("q", RecallOpts { top_k: 0 }).await.unwrap();
        assert!(items.is_empty());
        let items = store.recall("q", RecallOpts { top_k: 5 }).await.unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn truncate_field_caps_and_is_boundary_safe() {
        let short = "hello";
        assert_eq!(truncate_field(short), short);

        // A long multibyte string must not panic and must stay under cap+marker.
        let long = "é".repeat(FIELD_CAP);
        let out = truncate_field(&long);
        assert!(out.contains("truncated"));
    }
}
