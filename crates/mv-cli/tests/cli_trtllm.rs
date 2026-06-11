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

/// Path to the repo-root `models.yaml` (the real registry).
///
/// Live, model-loaded `#[ignore]` tests resolve their served model name
/// through this rather than a fabricated inline config: an inline `llama-fp8`
/// entry with no `served_name` sends the literal id to the proxy, which serves
/// `llama-3_1-8b-fp8` and answers 502. Using the real registry means these
/// tests exercise whatever the operator actually deployed.
fn repo_models_yaml() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("models.yaml")
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

// --- T014 [US2]: live 502 maps to "Run: just load <id>" ---
// Requires only a running TRT-LLM proxy at http://localhost:8003. The inline
// config deliberately omits `served_name`, so the literal id `llama-fp8` is
// sent to the proxy — a name it never serves — which always answers 502.
// Keep the fabricated config (do NOT switch to repo_models_yaml(), whose
// served model is loaded and would answer 200).
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_502_emits_just_load_hint() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
"#;
    let config = write_models_yaml(yaml);
    cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Run: just load llama-fp8"));
}

// --- T017 [US1]: --stream rejected for non-trtllm providers ---
#[test]
fn stream_flag_rejected_for_non_trtllm_provider() {
    let yaml = r#"
models:
  - id: qwen3:4b
    provider: ollama
    endpoint: http://127.0.0.1:19999
"#;
    let config = write_models_yaml(yaml);
    cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "qwen3:4b",
            "--stream",
            "hi",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "streaming is only supported for TRT-LLM models in this release",
        ));
}

// --- T018 [US1]: --stream / --no-tools flags visible in help ---
#[test]
fn stream_flag_known_to_clap() {
    cmd()
        .args(["prompt", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--stream"))
        .stdout(predicate::str::contains("--no-tools"));
}

// --- US1: --stream + tools (default) falls back to buffered on TRT-LLM ---
// Offline: the note fires before any request, so an unreachable endpoint is
// enough. Tools are attached by default, so the fallback note must appear.
#[test]
fn stream_with_tools_emits_buffered_fallback_note() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
    endpoint: http://127.0.0.1:19999/v1
"#;
    let config = write_models_yaml(yaml);
    cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "--stream",
            "hi",
        ])
        .assert()
        .stderr(predicate::str::contains("--stream falls back to buffered"));
}

// --- US1: --stream --no-tools takes the genuine streaming path (no fallback) ---
#[test]
fn stream_no_tools_does_not_emit_fallback_note() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
    endpoint: http://127.0.0.1:19999/v1
"#;
    let config = write_models_yaml(yaml);
    cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "--stream",
            "--no-tools",
            "hi",
        ])
        .assert()
        .stderr(predicate::str::contains("falls back to buffered").not());
}

// --- T047 [US1]: --json overrides --stream with warning ---
#[test]
fn json_overrides_stream_with_warning() {
    let yaml = r#"
models:
  - id: qwen3:4b
    provider: ollama
    endpoint: http://127.0.0.1:19999
"#;
    let config = write_models_yaml(yaml);
    cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "--json",
            "-m",
            "qwen3:4b",
            "--stream",
            "hi",
        ])
        .assert()
        .stderr(predicate::str::contains(
            "warning: --json overrides --stream; falling back to buffered JSON output",
        ));
}

// --- T019 [US1]: live streaming emits incremental output ---
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_stream_emits_incremental_output() {
    let config = repo_models_yaml();
    // `--no-tools` is required for genuine streaming: with tools attached the
    // TRT-LLM path falls back to buffered (see trtllm_stream_with_tools_*).
    let assert = cmd()
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "llama-fp8",
            "--stream",
            "--no-tools",
            "Say hi and then stop.",
        ])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.len() > 4,
        "expected non-trivial stream output, got {stdout:?}"
    );
    assert!(
        stdout.ends_with('\n'),
        "expected single trailing newline, got {stdout:?}"
    );
    assert!(
        !stdout.ends_with("\n\n"),
        "expected exactly one trailing newline, got {stdout:?}"
    );
}

// --- T020 [US1]: streaming path inherits US2 502 hint ---
// Like T014, the fabricated `llama-fp8` id is never served, so the proxy
// returns 502 regardless of load state — keep the inline config. `--no-tools`
// forces the genuine streaming path so this exercises the streaming preflight
// (served_model_present) rather than the buffered fallback.
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_stream_inherits_just_load_hint_on_502() {
    let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
"#;
    let config = write_models_yaml(yaml);
    cmd()
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "-m",
            "llama-fp8",
            "--stream",
            "--no-tools",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Run: just load llama-fp8"));
}

// --- T026 [US3]: buffered call records gen_ai.usage.{input,output}_tokens ---
// Runs with -vv so the fmt subscriber emits span-close events (which include
// the recorded span fields). Requires a live proxy with the model loaded.
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_buffered_records_token_usage_attrs() {
    let config = repo_models_yaml();
    let assert = cmd()
        .args([
            "-vv",
            "--config",
            config.to_str().unwrap(),
            "-m",
            "llama-fp8",
            "Say hi and then stop.",
        ])
        .assert()
        .success();
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gen_ai.usage.input_tokens="),
        "expected gen_ai.usage.input_tokens in stderr, got:\n{stderr}"
    );
    assert!(
        stderr.contains("gen_ai.usage.output_tokens="),
        "expected gen_ai.usage.output_tokens in stderr, got:\n{stderr}"
    );
    // Both should be non-zero — the recorded values are present iff the
    // proxy supplied usage, and our gate skipped recording when both are 0.
    assert!(
        !stderr.contains("gen_ai.usage.input_tokens=0 ")
            && !stderr.contains("gen_ai.usage.input_tokens=0\n"),
        "input_tokens should be non-zero, stderr:\n{stderr}"
    );
}

// --- T031 [US4]: buffered tool-calling round-trip via TRT-LLM ---
// Creates a temp directory with a uniquely-named marker file, asks the
// model to list that directory using the built-in file_list tool, and
// asserts the final answer mentions the marker file name. Requires a
// live proxy with a tool-capable model loaded.
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_buffered_tool_call_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let marker = "mv_marker_abc123.txt";
    std::fs::write(dir.path().join(marker), "hi").unwrap();
    let config = repo_models_yaml();
    let prompt = format!(
        "Use the file_list tool to list the contents of {}, then tell me the file names you find.",
        dir.path().display()
    );
    let assert = cmd()
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "llama-fp8",
            &prompt,
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(
        stdout.contains(marker),
        "expected final answer to mention {marker}, got:\n{stdout}"
    );
}

// --- T032 [US4]: --stream + a tool prompt falls back to buffered ---
// The TRT-LLM proxy streams tool calls as plain text rather than executable
// `tool_calls`, so genuine streaming cannot complete a tool round-trip. With
// tools attached, `--stream` therefore falls back to the buffered path (which
// does perform the round-trip): the run emits the fallback note on stderr and
// still surfaces the real file name in stdout.
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_stream_with_tools_falls_back_to_buffered_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let marker = "mv_marker_xyz789.txt";
    std::fs::write(dir.path().join(marker), "hi").unwrap();
    let config = repo_models_yaml();
    let prompt = format!(
        "Use the file_list tool to list the contents of {}, then tell me the file names you find.",
        dir.path().display()
    );
    let assert = cmd()
        .args([
            "--config",
            config.to_str().unwrap(),
            "-m",
            "llama-fp8",
            "--stream",
            &prompt,
        ])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stderr.contains("--stream falls back to buffered"),
        "expected the buffered-fallback note on stderr, got:\n{stderr}"
    );
    assert!(
        stdout.contains(marker),
        "expected the buffered round-trip to mention {marker}, got:\n{stdout}"
    );
}

// --- T035 [US5]: every TRT-LLM model in models.yaml terminates cleanly ---
// Loops every `provider: trtllm` entry in the repo-root models.yaml, sends
// a fixed smoke prompt, and asserts the response does not contain literal
// provider-default stop tokens. Requires the proxy with the model loaded.
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn trtllm_registry_models_terminate_cleanly() {
    let registry_path = repo_models_yaml();
    let registry = mv_core::ModelRegistry::load(&registry_path).expect("load models.yaml");
    let trtllm_ids: Vec<String> = registry
        .available_ids()
        .into_iter()
        .filter_map(|id| {
            registry
                .get(id)
                .filter(|e| e.provider == "trtllm")
                .map(|e| e.id.clone())
        })
        .collect();
    assert!(
        !trtllm_ids.is_empty(),
        "expected at least one trtllm model in models.yaml"
    );

    let stop_tokens = ["</s>", "<|im_end|>", "<|eot_id|>"];
    for id in &trtllm_ids {
        let assert = cmd()
            .args([
                "--config",
                registry_path.to_str().unwrap(),
                "-m",
                id,
                "Say hi and then stop.",
            ])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
        for tok in &stop_tokens {
            assert!(
                !stdout.contains(tok),
                "model {id} leaked stop token {tok} in stdout:\n{stdout}"
            );
        }
    }
}

// --- T038 [US6]: 2-step workflow runs end-to-end via TRT-LLM ---
// Writes a minimal 2-step prompt workflow whose second step references the
// first step's output, runs `mv-cli workflow run` against it, and asserts
// the process exits 0 (downstream step received the captured value).
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn workflow_with_trtllm_prompt_step_runs_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let workflow_path = dir.path().join("wf.yaml");
    let workflow_yaml = r#"
name: trtllm-smoke
version: "1.0"
defaults:
  model: llama-fp8
inputs: []
steps:
  - id: first
    type: prompt
    output: greeting
    template: "Say 'hello' and stop."
  - id: second
    type: prompt
    output: echoed
    template: "Repeat this verbatim: {{greeting}}"
outputs:
  - name: final
    from: second
"#;
    std::fs::write(&workflow_path, workflow_yaml).unwrap();

    let config = repo_models_yaml();

    cmd()
        .args([
            "workflow",
            "run",
            workflow_path.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .assert()
        .success();
}

// --- T039 [US6]: workflow surfaces ModelNotLoaded hint on 502 ---
// As in T014, the inline `llama-fp8` model has no `served_name`, so the proxy
// never serves it and returns 502 — the workflow path must surface the hint.
#[test]
#[ignore = "requires TRT-LLM proxy"]
fn workflow_with_unloaded_trtllm_model_emits_just_load_hint() {
    let dir = tempfile::tempdir().unwrap();
    let workflow_path = dir.path().join("wf.yaml");
    let workflow_yaml = r#"
name: trtllm-unloaded
version: "1.0"
defaults:
  model: llama-fp8
inputs: []
steps:
  - id: only
    type: prompt
    output: out
    template: "ping"
outputs:
  - name: final
    from: only
"#;
    std::fs::write(&workflow_path, workflow_yaml).unwrap();

    let models_yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
"#;
    let config = write_models_yaml(models_yaml);

    cmd()
        .args([
            "workflow",
            "run",
            workflow_path.to_str().unwrap(),
            "--config",
            config.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Run: just load llama-fp8"));
}
