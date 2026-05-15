use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn cmd() -> Command {
    Command::cargo_bin("mv-cli").unwrap()
}

/// Helper: write a models.yaml to a temp file and return the path.
fn write_models_yaml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

// --- T011: Config parsing with new fields ---

#[test]
fn trtllm_model_config_parsed_with_metadata() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
    served_name: meta-llama/Meta-Llama-3.1-8B-Instruct
    architecture: llama
    quant: fp8
    expected_vram_gb: 9
"#;
    let config = write_models_yaml(yaml);
    // The CLI should parse this config without errors.
    // It will fail at the health check (no server), but that's a runtime error, not a parse error.
    let assert = cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "Hello",
        ])
        .assert();
    // Should NOT be a config parse error (exit code 1, not 2)
    assert.code(predicate::ne(2));
}

#[test]
fn trtllm_model_config_minimal() {
    let yaml = r#"
models:
  - id: llama-3_1-8b
    provider: trtllm
"#;
    let config = write_models_yaml(yaml);
    let assert = cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-3_1-8b",
            "Hello",
        ])
        .assert();
    assert.code(predicate::ne(2));
}

// --- T015: Health check error output ---

#[test]
fn trtllm_unreachable_shows_hint() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
    endpoint: http://127.0.0.1:19999/v1
"#;
    let config = write_models_yaml(yaml);
    let assert = cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "Hello",
        ])
        .assert();
    assert
        .failure()
        .stderr(predicate::str::contains("TRT-LLM server not reachable"))
        .stderr(predicate::str::contains("trtllm-serve"));
}

// --- T011: unsupported provider still errors ---

#[test]
fn unsupported_provider_still_errors() {
    let yaml = r#"
models:
  - id: test-model
    provider: unknown_provider
"#;
    let config = write_models_yaml(yaml);
    let assert = cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "test-model",
            "Hello",
        ])
        .assert();
    assert
        .failure()
        .stderr(predicate::str::contains("unsupported provider"));
}

// --- T018: Telemetry span attributes (structural test) ---
// Note: We can't inspect OTLP spans in a CLI test, but we verify the
// tracing instrumentation compiles and runs without panicking.

#[test]
fn trtllm_with_verbose_does_not_panic() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
    endpoint: http://127.0.0.1:19999/v1
    architecture: llama
    quant: fp8
    expected_vram_gb: 9
"#;
    let config = write_models_yaml(yaml);
    let assert = cmd()
        .args([
            "-v",
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "Hello",
        ])
        .assert();
    // Should fail due to unreachable server, not panic
    assert.failure().code(1);
}

// --- T020: Tool calling graceful degradation ---
// When the server is unreachable, the health check should fail before
// any tool definitions are even sent. This verifies the health-check-first
// behavior works with tool-capable configs.

#[test]
fn trtllm_tool_calling_config_no_crash() {
    let yaml = r#"
models:
  - id: qwen3-trtllm
    provider: trtllm
    endpoint: http://127.0.0.1:19999/v1
    served_name: Qwen/Qwen3-8B
    architecture: qwen
"#;
    let config = write_models_yaml(yaml);
    let assert = cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "qwen3-trtllm",
            "List files in the current directory",
        ])
        .assert();
    // Should fail gracefully with health check error, not panic
    assert
        .failure()
        .stderr(predicate::str::contains("TRT-LLM server not reachable"));
}
