//! WS2 (T005): hermetic agentic RAG against a fake klams MCP server.
//!
//! Wires two fixtures: the fake proxy stands in for the LLM (scripted to call
//! `memory_search`, then answer), and `FakeKlams` stands in for the klams MCP
//! server over authenticated Streamable HTTP. The agent's tool call must reach
//! klams over HTTP with the bearer header and pull seeded content back into the
//! conversation. This is the Phase 5.5 deliverable in agentic form.

mod support;

use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;
use support::{FakeKlams, FakeProxy, KlamsChunk, write_trtllm_models_yaml};

const MODEL_ID: &str = "fake-llama";
const SERVED_NAME: &str = "fake-llama-served";
const TOKEN: &str = "klams-read-token-abc123";

/// A distinctive marker only retrievable from klams — its presence in the
/// model's follow-up request proves `memory_search` actually ran.
const MARKER: &str = "Mockville-is-the-capital-of-Fakeland";

fn cmd() -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(20));
    c
}

fn write_klams_mcp_config(dir: &std::path::Path, url: &str) -> std::path::PathBuf {
    let path = dir.join("mcp-servers.yaml");
    let yaml = format!(
        "servers:\n  - name: klams\n    transport: http\n    url: {url}\n    \
         auth_token_env: KLAMS_TOKEN\n",
    );
    std::fs::write(&path, yaml).expect("write mcp-servers.yaml");
    path
}

/// An ~800-char knowledge chunk (klams scanner-sized) carrying the marker.
fn seeded_chunk() -> KlamsChunk {
    let body = format!(
        "{MARKER}. {}",
        "Fakeland is a hermetic test nation used only in mv-cli integration \
         tests; its institutions, geography, and capital exist solely to give \
         retrieval something concrete to return. "
            .repeat(6)
    );
    KlamsChunk {
        text: body,
        source_path: "/home/ken/obsidian/fakeland.md".to_string(),
    }
}

#[test]
fn agent_retrieves_seeded_content_from_klams() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    // The model calls memory_search, then (seeing the tool result) answers.
    proxy.mount_tool_call_then_final(
        "memory_search",
        serde_json::json!({"query": "what is the capital of Fakeland?"}),
        "FINAL: the capital of Fakeland is Mockville.",
    );

    let klams = FakeKlams::start(TOKEN, &[seeded_chunk()]);

    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &klams.mcp_url());

    cmd()
        .current_dir(dir.path())
        .env("KLAMS_TOKEN", TOKEN)
        .args([
            "--config",
            models.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "What is the capital of Fakeland?",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "FINAL: the capital of Fakeland is Mockville.",
        ));

    // Proof the MCP tool actually ran over authenticated HTTP: the model's
    // follow-up request must carry the seeded marker as a tool result.
    let bodies = proxy.chat_request_bodies();
    assert!(
        bodies.len() >= 2,
        "expected a tool round trip (2+ chat requests), got {}",
        bodies.len()
    );
    let followup = serde_json::to_string(bodies.last().unwrap()).unwrap();
    assert!(
        followup.contains("\"role\":\"tool\""),
        "follow-up should carry a tool result message: {followup}"
    );
    assert!(
        followup.contains(MARKER),
        "the seeded klams content must reach the model: {followup}"
    );

    // And the bearer header genuinely reached klams on the wire.
    let saw_bearer = klams.received_requests().iter().any(|r| {
        r.headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == format!("Bearer {TOKEN}"))
            .unwrap_or(false)
    });
    assert!(saw_bearer, "klams must have received the bearer token");
}

fn write_rag_workflow(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("rag.yaml");
    // A tool step retrieves via memory_search; the prompt step consumes the
    // rendered {{context}}. Mirrors workflows/examples/rag-example.yaml but
    // pinned to the fake model id.
    let yaml = format!(
        r#"
name: rag-test
version: "1.0"
inputs:
  - name: question
    type: string
    required: true
steps:
  - id: retrieve
    type: tool
    tool: memory_search
    inputs:
      query: "{{{{question}}}}"
      top_k: 5
    output: context
  - id: answer
    type: prompt
    model: {MODEL_ID}
    output: answer
    template: |
      Context: {{{{context}}}}
      Question: {{{{question}}}}
outputs:
  - name: answer
    from: answer
"#
    );
    std::fs::write(&path, yaml).expect("write rag workflow");
    path
}

#[test]
fn workflow_retrieves_context_into_prompt_step() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    // The prompt step makes a single completion; its rendered template must
    // already contain the retrieved context.
    proxy.mount_chat_text("ANSWER: grounded in retrieved context.");

    let klams = FakeKlams::start(TOKEN, &[seeded_chunk()]);

    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &klams.mcp_url());
    let wf = write_rag_workflow(dir.path());

    cmd()
        .current_dir(dir.path())
        .env("KLAMS_TOKEN", TOKEN)
        .args([
            "workflow",
            "run",
            wf.to_str().unwrap(),
            "--config",
            models.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
            "--input",
            "question=What is the capital of Fakeland?",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "ANSWER: grounded in retrieved context.",
        ));

    // The prompt step's rendered template (sent to the model) must carry the
    // retrieved marker — proof the tool step's output flowed into {{context}}.
    let bodies = proxy.chat_request_bodies();
    assert_eq!(bodies.len(), 1, "the workflow makes exactly one completion");
    let body = serde_json::to_string(&bodies[0]).unwrap();
    assert!(
        body.contains(MARKER),
        "retrieved context must reach the prompt step: {body}"
    );
}

/// FR-006 decision gate: measure a realistic `memory_search` payload against
/// the 10,000-char tool-output cap. With top_k 5 × ~800-char chunks the
/// serialized result must stay well under the cap — otherwise the prompt step
/// would receive silently-truncated context. Documents the decision in code:
/// the universal cap stands; the example caps top_k at 5.
#[test]
fn realistic_search_payload_stays_under_tool_output_cap() {
    // Five scanner-sized chunks, the example's top_k.
    let chunks: Vec<KlamsChunk> = (0..5).map(|_| seeded_chunk()).collect();
    let serialized = support::public_memory_json(&chunks);
    assert!(
        serialized.len() < 10_000,
        "top_k=5 payload was {} chars — exceeds the 10k tool-output cap; \
         FR-006 would require a per-server tool_output_limit",
        serialized.len()
    );
}
