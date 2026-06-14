use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;

fn cmd() -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(20));
    c
}

/// Write a models.yaml in `dir` pinned to an instantly-refused endpoint:
/// bind an ephemeral port, drop the listener, point the model at it.
///
/// Tests that only exercise flag plumbing still need a backend target; an
/// explicit unreachable config makes them deterministic (no accidental live
/// Ollama on :11434) and fast (connection refused, not a timeout).
fn write_unreachable_config(dir: &std::path::Path) -> std::path::PathBuf {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let path = dir.join("models.yaml");
    std::fs::write(
        &path,
        format!(
            "models:\n  - id: test-model\n    provider: ollama\n    endpoint: http://127.0.0.1:{port}\n    default: true\n"
        ),
    )
    .unwrap();
    path
}

#[test]
fn accepts_positional_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "Hello world"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

#[test]
fn accepts_model_flag() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "--model",
            "test-model",
            "Hello",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

#[test]
fn accepts_endpoint_flag() {
    // The --endpoint flag overrides the registry endpoint; the error must
    // mention the overridden endpoint, proving the flag took effect.
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "--endpoint",
            &format!("http://127.0.0.1:{port}"),
            "Hello",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(format!("http://127.0.0.1:{port}")));
}

#[test]
fn accepts_json_flag() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "--json", "Hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(r#"{"error""#))
        .stdout(predicate::str::is_empty());
}

#[test]
fn accepts_verbose_flag() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "-vv", "Hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

#[test]
fn missing_prompt_exits_with_usage_error() {
    cmd().assert().failure().code(2);
}

// --- US1: Config flag tests ---

#[test]
fn accepts_config_flag() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    // The config was parsed (no parse error) and its endpoint was used
    // (backend-unreachable, not a clap or config failure).
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "Hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

#[test]
fn unknown_model_error_message() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "--model",
            "nonexistent-model-xyz",
            "Hello",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("not found in registry"));
}

// --- 008/F10: --json errors go to stderr, not stdout ---

#[test]
fn json_error_goes_to_stderr_not_stdout() {
    cmd()
        .args([
            "--json",
            "prompt",
            "-c",
            "/nonexistent/models.yaml",
            "Hello",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(r#"{"error""#))
        .stderr(predicate::str::contains("Config file not found"))
        // 013/WS1: the --json error envelope carries a stable machine code
        // additively alongside the unchanged human-readable message.
        .stderr(predicate::str::contains(r#""code":"CONFIG_NOT_FOUND""#))
        .stdout(predicate::str::is_empty());
}

// --- US2: OTLP flag tests ---

#[test]
fn accepts_otlp_flag() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "Hello", "--otlp"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

#[test]
fn otlp_graceful_without_collector() {
    // With --otlp pointing at a nonexistent collector, the CLI still
    // attempts the prompt and fails for backend reasons, not OTel reasons.
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "--otlp",
            "http://127.0.0.1:1",
            "Hello",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

// --- US3: API key missing test ---

#[test]
fn missing_api_key_error_message() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("models.yaml");
    std::fs::write(
        &config,
        "models:\n  - id: gpt-4o-mini\n    provider: openai\n    api_key_env: OPENAI_API_KEY\n    default: true\n",
    )
    .unwrap();

    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "Hello"])
        .env_remove("OPENAI_API_KEY")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("API key required"));
}
