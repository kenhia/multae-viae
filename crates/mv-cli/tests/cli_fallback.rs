//! Hermetic fallback-chain tests (WS1: T003–T005).
//!
//! Each test drives the real binary end-to-end. A dead backend is a reserved-
//! then-closed port; a working backend is the scripted fake proxy. No live
//! services are touched.

mod support;

use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;
use support::{FakeProxy, dead_endpoint};

const BACKUP_ID: &str = "backup-llama";
const BACKUP_SERVED: &str = "fake-llama-served";

fn cmd() -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(20));
    c
}

/// models.yaml with a primary that lists `backup` as its fallback. `primary_ep`
/// is where the primary points (dead, in these tests); the backup points at the
/// fake proxy. Both are trtllm so they share the proxy's Chat Completions
/// surface.
fn write_chain_config(
    dir: &std::path::Path,
    primary_id: &str,
    primary_ep: &str,
    backup_ep: &str,
) -> std::path::PathBuf {
    let path = dir.join("models.yaml");
    let yaml = format!(
        "models:\n  \
           - id: {primary_id}\n    provider: trtllm\n    served_name: prim-served\n    \
             endpoint: {primary_ep}\n    default: true\n    fallback: [{BACKUP_ID}]\n  \
           - id: {BACKUP_ID}\n    provider: trtllm\n    served_name: {BACKUP_SERVED}\n    \
             endpoint: {backup_ep}\n",
    );
    std::fs::write(&path, yaml).expect("write models.yaml");
    path
}

// --- US1.1: a dead primary transparently falls back to the backup ---

#[test]
fn dead_primary_falls_back_to_backup() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("Served by the backup.");

    let dir = tempfile::tempdir().unwrap();
    let config = write_chain_config(
        dir.path(),
        "primary-llama",
        &dead_endpoint(),
        &proxy.endpoint(),
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "primary-llama",
            "--no-tools",
            "ping",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Served by the backup."))
        // Text mode surfaces the substitution on stderr.
        .stderr(predicate::str::contains("fallback model"))
        .stderr(predicate::str::contains(BACKUP_ID));
}

// --- US1/FR-003: --json carries `model_used`, no stderr notice ---

#[test]
fn fallback_reports_model_used_in_json() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("Served by the backup.");

    let dir = tempfile::tempdir().unwrap();
    let config = write_chain_config(
        dir.path(),
        "primary-llama",
        &dead_endpoint(),
        &proxy.endpoint(),
    );

    let assert = cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "--json",
            "-m",
            "primary-llama",
            "--no-tools",
            "ping",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let obj: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json stdout");
    assert_eq!(obj["model_used"], BACKUP_ID);
    assert_eq!(obj["response"], "Served by the backup.");
}

// --- US1.3 / FR-004: when every entry fails, the error lists each attempt ---

#[test]
fn all_models_failed_enumerates_each_attempt() {
    // Primary and backup both point at dead ports.
    let dir = tempfile::tempdir().unwrap();
    let config = write_chain_config(
        dir.path(),
        "primary-llama",
        &dead_endpoint(),
        &dead_endpoint(),
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "primary-llama",
            "--no-tools",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "every model in the fallback chain failed",
        ))
        .stderr(predicate::str::contains("primary-llama"))
        .stderr(predicate::str::contains(BACKUP_ID));
}

// --- US1.2: an ineligible error fails fast — the backup is never tried ---

#[test]
fn ineligible_error_does_not_fall_back() {
    // The primary's proxy is healthy but returns a completion with no choices →
    // mapped to CompletionFailed, which is NOT fallback-eligible. The chain must
    // fail fast with that error, never touching the backup.
    let primary = FakeProxy::start();
    primary.mount_health_ok();
    primary.mount_chat_no_choices();

    let backup = FakeProxy::start();
    backup.mount_health_ok();
    backup.mount_chat_text("Backup should never run.");

    let dir = tempfile::tempdir().unwrap();
    let config = write_chain_config(
        dir.path(),
        "primary-llama",
        &primary.endpoint(),
        &backup.endpoint(),
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "primary-llama",
            "--no-tools",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Model returned an error"));

    // The backup proxy must have received no chat completions.
    let backup_chats = backup
        .received_requests()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/v1/chat/completions")
        .count();
    assert_eq!(
        backup_chats, 0,
        "backup must not be tried on an ineligible error"
    );
}

// --- US2.1: a dead Ollama primary is skipped by preflight, backup serves ---

#[test]
fn dead_ollama_primary_preflight_skips_to_backup() {
    // Ollama has no internal preflight in `complete`, so this exercises the
    // walker's preflight-skip specifically: the dead primary is skipped before
    // any agent build and the trtllm backup serves the request.
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("Backup served after preflight skip.");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.yaml");
    let yaml = format!(
        "models:\n  \
           - id: ollama-primary\n    provider: ollama\n    endpoint: {dead}\n    \
             default: true\n    fallback: [{BACKUP_ID}]\n  \
           - id: {BACKUP_ID}\n    provider: trtllm\n    served_name: {BACKUP_SERVED}\n    \
             endpoint: {backup}\n",
        dead = dead_endpoint(),
        backup = proxy.endpoint(),
    );
    std::fs::write(&path, yaml).unwrap();

    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            path.to_str().unwrap(),
            "-m",
            "ollama-primary",
            "--no-tools",
            "ping",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Backup served after preflight skip.",
        ))
        .stderr(predicate::str::contains(BACKUP_ID));
}

// --- US5 (WS5): step-level `prefer:` list resolves through the chain walker ---

/// Write a two-model config (no `fallback` fields) for prefer-list tests.
fn write_two_model_config(
    dir: &std::path::Path,
    a_id: &str,
    a_ep: &str,
    b_id: &str,
    b_served: &str,
    b_ep: &str,
) -> std::path::PathBuf {
    let path = dir.join("models.yaml");
    let yaml = format!(
        "models:\n  \
           - id: {a_id}\n    provider: trtllm\n    served_name: a-served\n    \
             endpoint: {a_ep}\n    default: true\n  \
           - id: {b_id}\n    provider: trtllm\n    served_name: {b_served}\n    \
             endpoint: {b_ep}\n",
    );
    std::fs::write(&path, yaml).expect("write models.yaml");
    path
}

#[test]
fn prefer_list_first_dead_second_serves() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("Served by the second preference.");

    let dir = tempfile::tempdir().unwrap();
    let config = write_two_model_config(
        dir.path(),
        "pref-primary",
        &dead_endpoint(),
        BACKUP_ID,
        BACKUP_SERVED,
        &proxy.endpoint(),
    );

    let wf_path = dir.path().join("prefer.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: prefer-e2e
version: "1.0"
steps:
  - id: ask
    type: prompt
    output: answer
    model:
      prefer: [pref-primary, {BACKUP_ID}]
    template: "ping"
outputs:
  - name: result
    from: ask
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Served by the second preference."));
}

#[test]
fn prefer_list_unknown_id_rejected_before_run() {
    let proxy = FakeProxy::start();
    proxy.mount_health_ok();
    proxy.mount_chat_text("never runs");

    let dir = tempfile::tempdir().unwrap();
    let config = write_two_model_config(
        dir.path(),
        "pref-primary",
        &proxy.endpoint(),
        BACKUP_ID,
        BACKUP_SERVED,
        &proxy.endpoint(),
    );

    let wf_path = dir.path().join("prefer-bad.yaml");
    std::fs::write(
        &wf_path,
        r#"
name: prefer-bad
version: "1.0"
steps:
  - id: ask
    type: prompt
    output: answer
    model:
      prefer: [pref-primary, ghost-model]
    template: "ping"
"#,
    )
    .unwrap();

    cmd()
        .current_dir(dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "Model 'ghost-model' not found in registry",
        ));
}

// --- FR-006: --stream keeps single-model semantics — no mid-stream fallback ---

#[test]
fn streaming_does_not_fall_back() {
    // Primary is dead; it has a fallback. Buffered would fall back, but
    // streaming must surface the primary's error and never touch the backup
    // (tokens already shown to the user can't be unshown).
    let backup = FakeProxy::start();
    backup.mount_health_ok();
    backup.mount_models(&[BACKUP_SERVED]);
    backup.mount_chat_sse(&["Backup", " should", " not", " stream"]);

    let dir = tempfile::tempdir().unwrap();
    let config = write_chain_config(
        dir.path(),
        "primary-llama",
        &dead_endpoint(),
        &backup.endpoint(),
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "primary-llama",
            "--stream",
            "--no-tools",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));

    let backup_requests = backup.received_requests().len();
    assert_eq!(
        backup_requests, 0,
        "streaming must not reach the fallback backend"
    );
}
