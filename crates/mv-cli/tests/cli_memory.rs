//! WS2 (T005): hermetic session continuity across CLI invocations.
//!
//! Two separate `mv-cli` processes share one stateful `FakeKlams`: the first
//! records its turn, the second recalls it before answering. Proves persistent
//! memory works without any always-on server — klams holds the state, the CLI
//! gains continuity.

mod support;

use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;
use support::{FakeKlams, FakeProxy, dead_mcp_url, write_trtllm_models_yaml};

const MODEL_ID: &str = "fake-llama";
const SERVED_NAME: &str = "fake-llama-served";
const TOKEN: &str = "klams-rw-token-xyz";

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

/// One `mv-cli` invocation against the shared fixtures. `session` is optional.
fn run(
    dir: &std::path::Path,
    models: &std::path::Path,
    mcp_config: &std::path::Path,
    session: Option<&str>,
    prompt: &str,
) -> assert_cmd::assert::Assert {
    let mut args = vec![
        "--config".to_string(),
        models.to_str().unwrap().to_string(),
        "--mcp-config".to_string(),
        mcp_config.to_str().unwrap().to_string(),
        "-m".to_string(),
        MODEL_ID.to_string(),
    ];
    if let Some(s) = session {
        args.push("--session".to_string());
        args.push(s.to_string());
    }
    args.push(prompt.to_string());

    cmd()
        .current_dir(dir)
        .env("KLAMS_TOKEN", TOKEN)
        .args(&args)
        .assert()
}

#[test]
fn session_continuity_across_invocations() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("klams uses the BGE embedding model.");

    // No seed: invocation 1 starts with a genuinely empty memory.
    let klams = FakeKlams::start(TOKEN, &[]);

    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &klams.mcp_url());

    // Invocation 1: ask, answer, record the turn.
    run(
        dir.path(),
        &models,
        &mcp_config,
        Some("research"),
        "What embedding model does klams use?",
    )
    .success();

    assert_eq!(klams.event_count(), 1, "first turn should be recorded");

    // Invocation 2 (new process): the recalled prior turn must reach the model.
    run(
        dir.path(),
        &models,
        &mcp_config,
        Some("research"),
        "And what dimension is that?",
    )
    .success();

    let bodies = proxy.chat_request_bodies();
    assert_eq!(bodies.len(), 2, "one completion per invocation");
    let second = serde_json::to_string(bodies.last().unwrap()).unwrap();
    assert!(
        second.contains("What embedding model does klams use?"),
        "invocation 2 must recall invocation 1's prompt: {second}"
    );

    // Attribution: agent_name/session_title/model carried; every write under
    // the registered author.
    let regs = klams.registrations();
    assert_eq!(regs.len(), 2, "one register_author per memory-active run");
    assert_eq!(regs[0].get("agent_name").unwrap(), "mv-cli");
    assert_eq!(regs[0].get("session_title").unwrap(), "research");
    assert_eq!(regs[0].get("model").unwrap(), MODEL_ID);
    let authors = klams.write_author_ids();
    assert!(!authors.is_empty());
    assert!(
        authors.iter().all(|a| a == support::FAKE_AUTHOR_ID),
        "every write carries the registered author_id: {authors:?}"
    );
}

#[test]
fn no_session_records_nothing_and_omits_addendum() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("a plain answer");

    let klams = FakeKlams::start(TOKEN, &[]);
    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &klams.mcp_url());

    run(dir.path(), &models, &mcp_config, None, "no memory please").success();

    assert_eq!(klams.event_count(), 0, "no --session ⇒ no turn recorded");
    assert!(
        klams.registrations().is_empty(),
        "no --session ⇒ no author registered"
    );
    // The capability addendum must be absent — no phantom memory.
    let body = serde_json::to_string(&proxy.chat_request_bodies()[0]).unwrap();
    assert!(
        !body.contains("persistent memory for this session"),
        "no-memory run must not carry the addendum: {body}"
    );
}

#[test]
fn agent_can_write_memory_with_attribution() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    // The model is told (via the addendum) it can save, and emits a memory_add
    // call carrying the run's author id, then answers.
    proxy.mount_tool_call_then_final(
        "memory_add",
        serde_json::json!({
            "author_id": support::FAKE_AUTHOR_ID,
            "kind": "knowledge",
            "text": "The user prefers the fish shell.",
        }),
        "Noted.",
    );

    let klams = FakeKlams::start(TOKEN, &[]);
    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &klams.mcp_url());

    run(
        dir.path(),
        &models,
        &mcp_config,
        Some("prefs"),
        "Remember that I prefer fish shell.",
    )
    .success()
    .stdout(predicate::str::contains("Noted."));

    // The addendum (with the author id) reached the model.
    let first = serde_json::to_string(&proxy.chat_request_bodies()[0]).unwrap();
    assert!(
        first.contains(support::FAKE_AUTHOR_ID),
        "the memory-active run must surface the author_id to the model"
    );

    // The agent's memory_add was attributed to the registered author. (Writes
    // include the turn-recording event plus the agent's memory_add — every one
    // carries the author id.)
    let authors = klams.write_author_ids();
    assert!(
        authors.len() >= 2 && authors.iter().all(|a| a == support::FAKE_AUTHOR_ID),
        "agent write + turn record both attributed: {authors:?}"
    );
}

// --- WS4 (T008): memory never blocks a prompt ---

#[test]
fn dead_klams_with_session_still_answers() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("answered despite no memory");

    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &dead_mcp_url());

    run(dir.path(), &models, &mcp_config, Some("research"), "hello")
        .success()
        .stdout(predicate::str::contains("answered despite no memory"))
        // The session could not be registered (klams dead); a warning says so and
        // the run proceeds.
        .stderr(predicate::str::contains(
            "memory unavailable for session 'research'",
        ));
}

#[test]
fn rejected_write_warns_with_code_and_still_answers() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("here is your answer");

    // klams is up (registration + recall work) but writes are rejected — e.g.
    // the daily backup maintenance window.
    let klams = FakeKlams::start(TOKEN, &[]);
    klams.reject_writes("MAINTENANCE_WINDOW_ACTIVE", "backups in progress");

    let dir = tempfile::tempdir().unwrap();
    let models = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    let mcp_config = write_klams_mcp_config(dir.path(), &klams.mcp_url());

    run(dir.path(), &models, &mcp_config, Some("research"), "hello")
        .success()
        .stdout(predicate::str::contains("here is your answer"))
        // The turn-record write is rejected; the warning carries klams's code.
        .stderr(predicate::str::contains("MAINTENANCE_WINDOW_ACTIVE"));

    assert_eq!(klams.event_count(), 0, "the rejected write stored nothing");
}
