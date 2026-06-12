//! klams-backed [`MemoryStore`]: drives the pinned klams memory tools through
//! the **existing MCP handle** — the same `ToolServerHandle::call_tool` path
//! `HandleToolExecutor` uses for workflow tool steps. No REST client, no second
//! protocol, no new auth (the bearer is already on the HTTP transport from
//! sprint 010). Wire shapes follow `specs/011-klams-memory/contracts/` (v1.1).

use rig::tool::server::ToolServerHandle;

use mv_core::MvError;
use mv_core::memory::{
    MemoryItem, MemoryStore, RecallOpts, SessionMeta, TurnRecord, truncate_field,
};

/// How many prior turns / recalled items to inject. Small on purpose: stale
/// context hurts answers, and small reads stay well under the tool-output cap.
const RECALL_EVENTS: u32 = 4;
const RECALL_TOPK: u32 = 3;

/// A [`MemoryStore`] over a connected klams MCP server.
pub struct KlamsMemory {
    handle: ToolServerHandle,
}

impl KlamsMemory {
    pub fn new(handle: ToolServerHandle) -> Self {
        Self { handle }
    }

    /// Call a klams tool, returning its text result. A transport failure or an
    /// `isError` tool result (rig maps that to `Err`) becomes a
    /// [`MvError::MemoryError`] whose `details` carry klams's code/message — so
    /// the caller's warning is actionable.
    async fn call(&self, tool: &str, args: &serde_json::Value) -> Result<String, MvError> {
        let args_str = serde_json::to_string(args).map_err(|e| MvError::MemoryError {
            op: tool.to_string(),
            details: format!("failed to encode args: {e}"),
        })?;
        self.handle
            .call_tool(tool, &args_str)
            .await
            .map_err(|e| MvError::MemoryError {
                op: tool.to_string(),
                details: e.to_string(),
            })
    }
}

/// Render one `PublicMemory` JSON value as a context line for the model.
fn render_item(v: &serde_json::Value) -> MemoryItem {
    let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("memory");
    let text = match kind {
        "knowledge" => v
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "event" => {
            // Conversation events carry {prompt, response, …} in payload.
            let p = v.get("payload");
            match (
                p.and_then(|p| p.get("prompt")).and_then(|s| s.as_str()),
                p.and_then(|p| p.get("response")).and_then(|s| s.as_str()),
            ) {
                (Some(q), Some(a)) => format!("Earlier — you asked: {q}\nyou answered: {a}"),
                _ => p.map(std::string::ToString::to_string).unwrap_or_default(),
            }
        }
        "fact" => {
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("fact");
            let payload = v
                .get("payload")
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            format!("{ty}: {payload}")
        }
        _ => v.to_string(),
    };
    MemoryItem {
        kind: kind.to_string(),
        text,
    }
}

/// Parse a `tools/call` text result into JSON, attributing parse failures to
/// `op` (a malformed result is a memory failure, not a panic).
fn parse(op: &str, text: &str) -> Result<serde_json::Value, MvError> {
    serde_json::from_str(text).map_err(|e| MvError::MemoryError {
        op: op.to_string(),
        details: format!("unparseable result: {e}"),
    })
}

impl MemoryStore for KlamsMemory {
    async fn register_session(&self, meta: SessionMeta) -> Result<String, MvError> {
        let mut args = serde_json::json!({
            "agent_name": meta.agent_name,
            "session_title": meta.session,
            "client_app": meta.client_app,
            "client_version": meta.client_version,
        });
        if let Some(model) = meta.model {
            args["model"] = serde_json::Value::String(model);
        }
        let result = self.call("register_author", &args).await?;
        let v = parse("register_author", &result)?;
        v.get("author_id")
            .and_then(|a| a.as_str())
            .map(std::string::ToString::to_string)
            .ok_or_else(|| MvError::MemoryError {
                op: "register_author".to_string(),
                details: "result missing author_id".to_string(),
            })
    }

    async fn record_turn(&self, author_id: &str, turn: TurnRecord) -> Result<(), MvError> {
        let args = serde_json::json!({
            "author_id": author_id,
            "category": "conversation",
            "payload": {
                "session": turn.session,
                "prompt": truncate_field(&turn.prompt),
                "response": truncate_field(&turn.response),
                "model_used": turn.model_used,
            },
        });
        self.call("memory_append_event", &args).await.map(|_| ())
    }

    async fn recall(&self, query: &str, opts: RecallOpts) -> Result<Vec<MemoryItem>, MvError> {
        let args = serde_json::json!({"query": query, "top_k": opts.top_k});
        let result = self.call("memory_search", &args).await?;
        let v = parse("memory_search", &result)?;
        let arr = v.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(render_item).collect())
    }

    async fn session_events(&self, session: &str, limit: u32) -> Result<Vec<MemoryItem>, MvError> {
        let args = serde_json::json!({
            "category": "conversation",
            "payload_match": {"session": session},
            "order": "desc",
            "limit": limit,
        });
        let result = self.call("event_search", &args).await?;
        let v = parse("event_search", &result)?;
        let events = v
            .get("events")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(events.iter().map(render_item).collect())
    }
}

/// Drives the register → recall → record sequence for one memory-active run,
/// keeping memory strictly at the edges of the completion. Construction
/// (`begin`) registers the author; `recall_block` runs before the completion;
/// `record` runs after. Every step is best-effort at the call site.
pub struct SessionMemory<M: MemoryStore> {
    store: M,
    session: String,
    author_id: String,
}

impl<M: MemoryStore> SessionMemory<M> {
    /// Begin a memory-active run by registering the author. Returns `Err` if
    /// registration fails (klams down, tools absent, write rejected) — the
    /// caller warns and proceeds without memory.
    pub async fn begin(store: M, meta: SessionMeta) -> Result<Self, MvError> {
        let session = meta.session.clone();
        let author_id = store.register_session(meta).await?;
        Ok(Self {
            store,
            session,
            author_id,
        })
    }

    /// A capability note telling the model it has memory and how to write to it.
    /// Without the `author_id` the model cannot fill `memory_add`'s required
    /// argument, so this is what makes agent-driven remembering possible. It
    /// rides the prompt prefix (alongside recalled context) because the system
    /// preamble is fixed at the provider call sites; if models prove to fumble
    /// the args, threading a real preamble suffix is the documented next step.
    pub fn preamble_addendum(&self) -> String {
        format!(
            "You have persistent memory for this session (author id: {}). \
             When the user shares a durable fact or preference worth keeping, \
             call the `memory_add` tool with that exact author_id (kind \
             \"knowledge\" for free text, or \"fact\" for structured user/task/env \
             facts). Do not announce that you are saving unless asked.",
            self.author_id
        )
    }

    /// Build a context block from recent session turns + relevant memories, to
    /// prepend to the prompt. `None` when there is nothing to inject (a fresh
    /// session) or recall fails (best-effort — absence of memory is not an
    /// error).
    pub async fn recall_block(&self, query: &str) -> Option<String> {
        let mut lines = Vec::new();

        // Recent turns of this session, oldest-first for natural reading.
        if let Ok(mut events) = self
            .store
            .session_events(&self.session, RECALL_EVENTS)
            .await
        {
            events.reverse();
            lines.extend(events.into_iter().map(|e: MemoryItem| e.text));
        }
        // Relevant memories across sessions (knowledge/facts).
        if let Ok(items) = self
            .store
            .recall(query, RecallOpts { top_k: RECALL_TOPK })
            .await
        {
            lines.extend(items.into_iter().map(|i| i.text));
        }

        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "Context recalled from memory (session '{}'):\n{}",
            self.session,
            lines.join("\n")
        ))
    }

    /// Record this completed turn. Best-effort: returns the error for the
    /// caller to warn on, never blocks.
    pub async fn record(
        &self,
        prompt: &str,
        response: &str,
        model_used: &str,
    ) -> Result<(), MvError> {
        self.store
            .record_turn(
                &self.author_id,
                TurnRecord {
                    session: self.session.clone(),
                    prompt: prompt.to_string(),
                    response: response.to_string(),
                    model_used: model_used.to_string(),
                },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_knowledge_uses_text() {
        let v = serde_json::json!({"kind": "knowledge", "text": "klams uses BGE"});
        let item = render_item(&v);
        assert_eq!(item.kind, "knowledge");
        assert_eq!(item.text, "klams uses BGE");
    }

    #[test]
    fn render_conversation_event_shows_prior_turn() {
        let v = serde_json::json!({
            "kind": "event",
            "payload": {"session": "research", "prompt": "Q1", "response": "A1"},
        });
        let item = render_item(&v);
        assert_eq!(item.kind, "event");
        assert!(item.text.contains("Q1"), "{}", item.text);
        assert!(item.text.contains("A1"), "{}", item.text);
    }

    #[test]
    fn render_fact_summarizes_type_and_payload() {
        let v =
            serde_json::json!({"kind": "fact", "type": "UserFact", "payload": {"shell": "fish"}});
        let item = render_item(&v);
        assert_eq!(item.kind, "fact");
        assert!(item.text.contains("UserFact"));
        assert!(item.text.contains("fish"));
    }

    #[test]
    fn parse_rejects_malformed_result() {
        let err = parse("memory_search", "not json").unwrap_err();
        assert!(matches!(err, MvError::MemoryError { .. }));
        assert!(err.to_string().contains("memory_search"));
    }

    // --- SessionMemory driver, against an in-memory mock store ---

    use std::sync::Mutex;

    #[derive(Default)]
    struct MockStore {
        recorded: Mutex<Vec<(String, TurnRecord)>>,
        fail_register: bool,
    }

    impl MemoryStore for MockStore {
        async fn register_session(&self, _meta: SessionMeta) -> Result<String, MvError> {
            if self.fail_register {
                return Err(MvError::MemoryError {
                    op: "register_author".to_string(),
                    details: "down".to_string(),
                });
            }
            Ok("author-xyz".to_string())
        }
        async fn record_turn(&self, author_id: &str, turn: TurnRecord) -> Result<(), MvError> {
            self.recorded
                .lock()
                .unwrap()
                .push((author_id.to_string(), turn));
            Ok(())
        }
        async fn recall(&self, _q: &str, _o: RecallOpts) -> Result<Vec<MemoryItem>, MvError> {
            Ok(vec![])
        }
        async fn session_events(&self, session: &str, _l: u32) -> Result<Vec<MemoryItem>, MvError> {
            let recorded = self.recorded.lock().unwrap();
            Ok(recorded
                .iter()
                .filter(|(_, t)| t.session == session)
                .map(|(_, t)| MemoryItem {
                    kind: "event".to_string(),
                    text: format!("Earlier — you asked: {}", t.prompt),
                })
                .collect())
        }
    }

    fn meta() -> SessionMeta {
        SessionMeta {
            agent_name: "mv-cli".to_string(),
            session: "research".to_string(),
            model: Some("qwen3:8b".to_string()),
            client_app: "mv-cli".to_string(),
            client_version: "0.1.0".to_string(),
        }
    }

    #[tokio::test]
    async fn begin_failure_is_propagated() {
        let store = MockStore {
            fail_register: true,
            ..Default::default()
        };
        assert!(SessionMemory::begin(store, meta()).await.is_err());
    }

    #[tokio::test]
    async fn fresh_session_has_no_recall_block_but_has_addendum() {
        let sm = SessionMemory::begin(MockStore::default(), meta())
            .await
            .unwrap();
        // The capability addendum always carries the registered author id.
        assert!(sm.preamble_addendum().contains("author-xyz"));
        // A fresh session has nothing to recall.
        assert!(sm.recall_block("anything").await.is_none());
    }

    #[tokio::test]
    async fn recorded_turn_resurfaces_in_recall_block() {
        let sm = SessionMemory::begin(MockStore::default(), meta())
            .await
            .unwrap();
        sm.record("what embedding model?", "BGE", "qwen3:8b")
            .await
            .unwrap();
        let block = sm.recall_block("follow-up").await.expect("has context");
        assert!(block.contains("what embedding model?"), "{block}");
        assert!(block.contains("session 'research'"), "{block}");
    }
}
