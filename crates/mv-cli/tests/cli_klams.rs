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
use support::{
    FakeKlams, FakeProxy, KlamsChunk, dead_endpoint, dead_mcp_url, write_trtllm_models_yaml,
};

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

// --- WS4 (T008): graceful degradation when klams is unreachable ---

#[test]
fn dead_klams_does_not_break_a_non_rag_prompt() {
    // A live model, a dead klams MCP endpoint. The MCP connection fails and is
    // logged-and-skipped (the established MCP behavior); the prompt still
    // completes via the model.
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("Answered without retrieval.");

    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &dead_mcp_url());

    cmd()
        .current_dir(dir.path())
        .env("KLAMS_TOKEN", TOKEN)
        .args([
            "-vv",
            "--config",
            models.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "Say hi",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Answered without retrieval."))
        // The skip is surfaced: a warning names the unreachable klams server.
        .stderr(predicate::str::contains("klams"))
        .stderr(predicate::str::contains("failed to connect"));
}

#[test]
fn dead_klams_makes_a_rag_workflow_fail_loudly() {
    // The model registry must contain the workflow's model (the pre-run
    // reference check), but the model is never contacted — the tool step runs
    // first and fails because `memory_search` never merged in (klams is dead).
    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &dead_endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &dead_mcp_url());
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
            "question=anything",
        ])
        .assert()
        .failure()
        .code(1)
        // Loud failure naming the missing tool — no silent skip.
        .stderr(predicate::str::contains("memory_search"));
}

// --- WS5 (T009): live round-trips against the real klams on kubs0 ---
//
// `#[ignore]`d (run via `just test-klams`). Gated on `KLAMS_TOKEN`; the URL
// defaults to kubs0 and is overridable with `KLAMS_URL`. These are the only
// non-hermetic tests in the suite — they confirm the contract against the
// real service. They early-return (skip) when `KLAMS_TOKEN` is absent so the
// blanket `cargo test -- --ignored` sweep does not fail without credentials.

fn live_klams_url() -> String {
    std::env::var("KLAMS_URL").unwrap_or_else(|_| "http://kubs0:7777/mcp".to_string())
}

/// The repo's real `models.yaml` (absolute), so live tests resolve against the
/// project's actual registry — not the built-in fallback that only knows
/// `qwen3:4b`. This is why `KLAMS_MODEL` must name a model defined there.
fn live_models_config() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../models.yaml").to_string()
}

/// Which registered model to drive. `KLAMS_MODEL` overrides; otherwise the
/// registry's `default:` model is used (no `-m`). m-v is a controller — it
/// *calls* this backend (Ollama / TRT-LLM / cloud), it does not serve one — so
/// the backend must be reachable with the model loaded for these tests to run.
fn live_model() -> Option<String> {
    std::env::var("KLAMS_MODEL").ok()
}

/// Run a model-driven live invocation. If it fails purely because no model
/// backend is reachable, **skip** rather than fail — these tests exercise the
/// full prompt path, and a missing backend means "not applicable here", not a
/// klams regression. The model-free `live_klams_workflow_retrieval` test is the
/// klams-only check. Returns the captured stderr when the command ran, or
/// `None` when skipped.
fn run_live_model(cmd: &mut Command, what: &str) -> Option<String> {
    let out = cmd.output().expect("spawn mv-cli");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() && stderr.contains("Cannot reach model backend") {
        eprintln!(
            "SKIP {what}: no model backend reachable. These live tests drive a real \
             model (m-v calls out to Ollama/TRT-LLM/cloud — it does not serve one). \
             Start the backend, or set KLAMS_MODEL to a model your machine can reach."
        );
        return None;
    }
    assert!(out.status.success(), "{what} failed:\n{stderr}");
    Some(stderr)
}

#[test]
#[ignore = "requires a reachable klams on kubs0 and KLAMS_TOKEN"]
fn live_klams_workflow_retrieval_round_trips() {
    let Ok(token) = std::env::var("KLAMS_TOKEN") else {
        eprintln!("skipping: KLAMS_TOKEN not set");
        return;
    };

    // A model-free workflow: one memory_search tool step, output mapped out.
    // Proves real retrieval over authenticated HTTP without needing an LLM.
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_klams_mcp_config(dir.path(), &live_klams_url());
    let wf = dir.path().join("live-retrieve.yaml");
    std::fs::write(
        &wf,
        r#"
name: live-retrieve
version: "1.0"
inputs:
  - name: query
    type: string
    required: true
steps:
  - id: retrieve
    type: tool
    tool: memory_search
    inputs:
      query: "{{query}}"
      top_k: 3
    output: hits
outputs:
  - name: hits
    from: retrieve
"#,
    )
    .unwrap();

    cmd()
        .current_dir(dir.path())
        .env("KLAMS_TOKEN", token)
        .args([
            "workflow",
            "run",
            wf.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
            "--input",
            "query=klams memory service",
        ])
        .assert()
        .success();
}

/// Build a live model-driven `mv-cli` invocation: bearer token in env, the
/// repo's real model registry (`--config`), the klams MCP config, optional
/// `-m <KLAMS_MODEL>` (else the registry default), then `extra` args + prompt.
fn live_model_cmd(
    token: &str,
    mcp_config: &std::path::Path,
    extra: &[&str],
    prompt: &str,
) -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(60));
    c.env("KLAMS_TOKEN", token);
    let mut args: Vec<String> = vec![
        "--config".into(),
        live_models_config(),
        "--mcp-config".into(),
        mcp_config.to_string_lossy().into_owned(),
    ];
    if let Some(model) = live_model() {
        args.push("-m".into());
        args.push(model);
    }
    for a in extra {
        args.push((*a).to_string());
    }
    args.push(prompt.to_string());
    c.args(&args);
    c
}

#[test]
#[ignore = "requires klams on kubs0 + KLAMS_TOKEN; needs a reachable model backend (skips if none)"]
fn live_klams_agentic_retrieval_round_trips() {
    let Ok(token) = std::env::var("KLAMS_TOKEN") else {
        eprintln!("skipping: KLAMS_TOKEN not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_klams_mcp_config(dir.path(), &live_klams_url());

    let mut c = live_model_cmd(
        &token,
        &mcp_config,
        &[],
        "Search your memory and tell me one thing you know about klams.",
    );
    // Skips cleanly if no model backend is up — see run_live_model.
    run_live_model(&mut c, "live_klams_agentic_retrieval");
}

#[test]
#[ignore = "requires klams on kubs0 + KLAMS_TOKEN; needs a reachable model backend (skips if none)"]
fn live_klams_memory_round_trips() {
    // Sprint 011: exercise the full live memory path through the CLI —
    // register → recall → record across two `--session` invocations against the
    // real klams + a real model. Writes land under agent `mv-cli`,
    // session_title `mv-live-memory-test` (the CLI's fixed agent_name); the CLI
    // has no delete surface this sprint, so these writes are identifiable for
    // manual pruning by session.
    let Ok(token) = std::env::var("KLAMS_TOKEN") else {
        eprintln!("skipping: KLAMS_TOKEN not set");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_klams_mcp_config(dir.path(), &live_klams_url());
    let session = "mv-live-memory-test";

    // Invocation 1: record a turn. Skip the whole test if no model backend.
    let mut record = live_model_cmd(
        &token,
        &mcp_config,
        &["--session", session],
        "Briefly: what is klams?",
    );
    if run_live_model(&mut record, "live_klams_memory (record)").is_none() {
        return;
    }

    // Invocation 2 (new process), same session: must register, recall the prior
    // turn, answer, and record again — all live.
    let mut recall = live_model_cmd(
        &token,
        &mcp_config,
        &["--session", session],
        "What did I just ask you about?",
    );
    let stderr = match run_live_model(&mut recall, "live_klams_memory (recall)") {
        Some(s) => s,
        None => return,
    };

    // Success alone is weak (memory is best-effort and degrades silently); assert
    // memory actually engaged — registration against live klams did NOT warn.
    assert!(
        !stderr.contains("memory unavailable"),
        "live klams memory should have engaged, but registration warned:\n{stderr}"
    );
}
