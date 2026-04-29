use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    Command::cargo_bin("mv-cli").unwrap()
}

#[test]
fn accepts_positional_prompt() {
    // Should not fail on argument parsing (may fail on implementation with todo!())
    let assert = cmd().arg("Hello world").assert();
    // We only check it didn't fail due to clap arg parsing errors
    // (exit code 2 = clap usage error)
    assert.code(predicate::ne(2));
}

#[test]
fn accepts_model_flag() {
    let assert = cmd().args(["--model", "llama3", "Hello"]).assert();
    assert.code(predicate::ne(2));
}

#[test]
fn accepts_endpoint_flag() {
    let assert = cmd()
        .args(["--endpoint", "http://localhost:9999", "Hello"])
        .assert();
    assert.code(predicate::ne(2));
}

#[test]
fn accepts_json_flag() {
    let assert = cmd().args(["--json", "Hello"]).assert();
    assert.code(predicate::ne(2));
}

#[test]
fn accepts_verbose_flag() {
    let assert = cmd().args(["-vv", "Hello"]).assert();
    assert.code(predicate::ne(2));
}

#[test]
fn missing_prompt_exits_with_usage_error() {
    cmd().assert().failure().code(2);
}

// --- US1: Config flag tests ---

#[test]
fn accepts_config_flag() {
    let assert = cmd().args(["--config", "models.yaml", "Hello"]).assert();
    assert.code(predicate::ne(2));
}

#[test]
fn unknown_model_error_message() {
    let assert = cmd()
        .args(["--model", "nonexistent-model-xyz", "Hello"])
        .assert();
    assert
        .failure()
        .code(1)
        .stderr(predicate::str::contains("not found in registry"));
}

// --- US2: OTLP flag tests ---

#[test]
fn accepts_otlp_flag() {
    let assert = cmd().args(["Hello", "--otlp"]).assert();
    assert.code(predicate::ne(2));
}

#[test]
fn otlp_graceful_without_collector() {
    // With --otlp pointing to a nonexistent collector, CLI should still
    // attempt the prompt (and fail for backend reasons, not OTel reasons).
    let assert = cmd()
        .args(["--otlp", "http://localhost:59999", "Hello"])
        .assert();
    // Should NOT exit 2 (clap error); may exit 1 (backend unreachable) or 0
    assert.code(predicate::ne(2));
}

// --- US3: API key missing test ---

#[test]
fn missing_api_key_error_message() {
    // Create a temp config with an openai model and no API key set
    let dir = std::env::temp_dir().join("mv-cli-test-apikey");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("models.yaml");
    std::fs::write(
        &config,
        "models:\n  - id: gpt-4o-mini\n    provider: openai\n    api_key_env: OPENAI_API_KEY\n    default: true\n",
    )
    .unwrap();

    let assert = cmd()
        .args(["--config", config.to_str().unwrap(), "Hello"])
        .env_remove("OPENAI_API_KEY")
        .assert();
    assert
        .failure()
        .code(1)
        .stderr(predicate::str::contains("API key required"));

    std::fs::remove_dir_all(&dir).ok();
}
