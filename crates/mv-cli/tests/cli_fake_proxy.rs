//! Hermetic provider-path tests against the scripted fake proxy (T026, T029).
//!
//! Every test here drives the real binary end-to-end — health preflight,
//! rig agent loop, tool execution, output contract — with no live backend.

mod support;

use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;
use support::{FakeProxy, write_trtllm_models_yaml};

const MODEL_ID: &str = "fake-llama";
const SERVED_NAME: &str = "fake-llama-served";

fn cmd() -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(20));
    c
}

/// Fixture bundle: a fake proxy + a tempdir cwd holding a models.yaml that
/// points at it. Running from the tempdir keeps the repo-root models.yaml /
/// mcp-servers.yaml out of the picture.
struct Setup {
    proxy: FakeProxy,
    dir: tempfile::TempDir,
    config: std::path::PathBuf,
}

fn setup() -> Setup {
    let proxy = FakeProxy::start();
    let dir = tempfile::tempdir().unwrap();
    let config = write_trtllm_models_yaml(dir.path(), MODEL_ID, SERVED_NAME, &proxy.endpoint());
    Setup { proxy, dir, config }
}

// --- (1) buffered happy path ---

#[test]
fn buffered_completion_prints_assistant_text() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text("Hello from the fake proxy.");

    cmd()
        .current_dir(s.dir.path())
        .args([
            "--config",
            s.config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "--no-tools",
            "Say hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello from the fake proxy."));
}

#[test]
fn buffered_completion_without_usage_still_succeeds() {
    // A proxy that omits the usage block entirely must not break the
    // buffered path (usage is optional in the Chat Completions schema).
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text_without_usage("No usage attached.");

    cmd()
        .current_dir(s.dir.path())
        .args([
            "--config",
            s.config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "--no-tools",
            "Say hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("No usage attached."));
}

// --- (2) multi-turn tool round trip through the real agentic loop ---

#[test]
fn multi_turn_tool_round_trip_executes_real_tool() {
    let s = setup();
    s.proxy.mount_health_ok();

    // A directory with a marker file the real file_list tool will discover.
    let listing_dir = tempfile::tempdir().unwrap();
    std::fs::write(listing_dir.path().join("marker-roundtrip.txt"), "x").unwrap();

    s.proxy.mount_tool_call_then_final(
        "file_list",
        serde_json::json!({"path": listing_dir.path().to_str().unwrap()}),
        "FINAL: the directory contains marker-roundtrip.txt",
    );

    cmd()
        .current_dir(s.dir.path())
        .args([
            "--config",
            s.config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "List the files",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "FINAL: the directory contains marker-roundtrip.txt",
        ));

    // The second request must carry the real tool result: rig executed the
    // built-in file_list against the tempdir and sent its output back.
    let bodies = s.proxy.chat_request_bodies();
    assert!(
        bodies.len() >= 2,
        "expected a tool round trip (2+ chat requests), got {}",
        bodies.len()
    );
    let followup = serde_json::to_string(bodies.last().unwrap()).unwrap();
    assert!(
        followup.contains("\"role\":\"tool\""),
        "follow-up request should carry a tool result message: {followup}"
    );
    assert!(
        followup.contains("marker-roundtrip.txt"),
        "tool result should contain the real file_list output: {followup}"
    );
}

// --- (3) 502 + Triton body → ModelNotLoaded hint, buffered path ---

#[test]
fn buffered_502_maps_to_model_not_loaded_hint() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_502_triton(SERVED_NAME);

    cmd()
        .current_dir(s.dir.path())
        .args([
            "--config",
            s.config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "--no-tools",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("is not loaded"))
        .stderr(predicate::str::contains(format!(
            "Run: just load {MODEL_ID}"
        )));
}

// --- (3b) streaming preflight: model absent from /v1/models → same hint ---

#[test]
fn stream_unserved_model_maps_to_model_not_loaded_hint() {
    let s = setup();
    s.proxy.mount_health_ok();
    // The proxy serves some other model; ours is missing.
    s.proxy.mount_models(&["some-other-model"]);

    cmd()
        .current_dir(s.dir.path())
        .args([
            "--config",
            s.config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "--stream",
            "--no-tools",
            "ping",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("is not loaded"))
        .stderr(predicate::str::contains(format!(
            "Run: just load {MODEL_ID}"
        )));
}

// --- (4) streaming happy path: SSE accumulation + trailing newline ---

#[test]
fn stream_no_tools_accumulates_sse_chunks() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_models(&[SERVED_NAME]);
    s.proxy.mount_chat_sse(&["Hello", " from", " SSE"]);

    let assert = cmd()
        .current_dir(s.dir.path())
        .args([
            "--config",
            s.config.to_str().unwrap(),
            "-m",
            MODEL_ID,
            "--stream",
            "--no-tools",
            "Say hello",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(
        stdout.contains("Hello from SSE"),
        "expected accumulated stream text, got {stdout:?}"
    );
    assert!(
        stdout.ends_with('\n') && !stdout.ends_with("\n\n"),
        "expected exactly one trailing newline, got {stdout:?}"
    );
}

// --- (5) temperature/max_tokens propagation (closes the T008 deferred note) ---

#[test]
fn workflow_prompt_step_propagates_temperature_and_max_tokens() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text("hi");

    let wf_path = s.dir.path().join("wf.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: gen-params
version: "1.0"
defaults:
  model: {MODEL_ID}
steps:
  - id: s1
    type: prompt
    output: out
    temperature: 0.42
    max_tokens: 64
    template: "Say hi"
outputs:
  - name: final
    from: s1
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
        ])
        .assert()
        .success();

    let bodies = s.proxy.chat_request_bodies();
    assert!(!bodies.is_empty(), "expected at least one chat request");
    let body = &bodies[0];
    assert_eq!(
        body.get("temperature").and_then(|v| v.as_f64()),
        Some(0.42),
        "request body must carry the step temperature, got: {body}"
    );
    assert_eq!(
        body.get("max_tokens").and_then(|v| v.as_u64()),
        Some(64),
        "request body must carry the step max_tokens, got: {body}"
    );
}

// --- 009/WS3 (T014): branch workflow end-to-end through the CLI ---

/// Run the branch workflow with a given `style`; returns the chat request
/// bodies the proxy saw (one per executed prompt step).
fn run_branch_workflow(style: &str) -> Vec<serde_json::Value> {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text("ANALYSIS-TEXT");

    let wf_path = s.dir.path().join("branch.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: branch-e2e
version: "1.0"
defaults:
  model: {MODEL_ID}
inputs:
  - name: topic
    type: string
    required: true
  - name: style
    type: string
    required: true
steps:
  - id: route
    type: branch
    condition: "style == 'detailed'"
    then:
      - id: deep
        type: prompt
        output: analysis
        template: "THOROUGH analysis of {{{{topic}}}}"
    else:
      - id: quick
        type: prompt
        output: analysis
        template: "ONE-SENTENCE summary of {{{{topic}}}}"
  - id: format
    type: prompt
    output: formatted
    template: "Format: {{{{analysis}}}}"
outputs:
  - name: result
    from: format
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
            "--input",
            "topic=ownership",
            "--input",
            &format!("style={style}"),
        ])
        .assert()
        .success();

    s.proxy.chat_request_bodies()
}

#[test]
fn branch_workflow_takes_then_arm() {
    let bodies = run_branch_workflow("detailed");
    // Exactly two prompt steps ran: the `then` arm + the format step.
    assert_eq!(
        bodies.len(),
        2,
        "expected then-arm + format, got {}",
        bodies.len()
    );
    let first = serde_json::to_string(&bodies[0]).unwrap();
    assert!(first.contains("THOROUGH"), "then arm should run: {first}");
    assert!(
        !first.contains("ONE-SENTENCE"),
        "else arm must be skipped: {first}"
    );
    // The branch output flowed into the format step.
    let second = serde_json::to_string(&bodies[1]).unwrap();
    assert!(
        second.contains("Format: ANALYSIS-TEXT"),
        "branch output must reach the format step: {second}"
    );
}

#[test]
fn branch_workflow_takes_else_arm() {
    let bodies = run_branch_workflow("brief");
    assert_eq!(bodies.len(), 2);
    let first = serde_json::to_string(&bodies[0]).unwrap();
    assert!(
        first.contains("ONE-SENTENCE"),
        "else arm should run: {first}"
    );
    assert!(
        !first.contains("THOROUGH"),
        "then arm must be skipped: {first}"
    );
}

// --- 009/WS4 (T018): parallel workflow end-to-end through the CLI ---

#[test]
fn parallel_workflow_runs_children_and_merges() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text("PIECE");

    let wf_path = s.dir.path().join("parallel.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: parallel-e2e
version: "1.0"
defaults:
  model: {MODEL_ID}
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "Angle A on {{{{topic}}}}"
      - id: b
        type: prompt
        output: out_b
        template: "Angle B on {{{{topic}}}}"
  - id: combine
    type: prompt
    output: merged
    template: "Combine: {{{{out_a}}}} + {{{{out_b}}}}"
outputs:
  - name: result
    from: combine
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
            "--input",
            "topic=rust",
        ])
        .assert()
        .success();

    let bodies = s.proxy.chat_request_bodies();
    // Two parallel children + the combine step.
    assert_eq!(
        bodies.len(),
        3,
        "expected 2 children + combine, got {}",
        bodies.len()
    );

    // The combine step ran last and saw both merged outputs (both "PIECE").
    let combine = serde_json::to_string(bodies.last().unwrap()).unwrap();
    assert!(
        combine.contains("Combine: PIECE + PIECE"),
        "combine step should see both parallel outputs: {combine}"
    );
}

// --- T029: end-to-end 3-step workflow (tool → prompt → transform) ---

#[test]
fn workflow_tool_prompt_transform_end_to_end() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text(
        "Here is the summary:\n```json\n{\"files\": [\"marker-e2e.txt\"], \"count\": 1}\n```\nDone.",
    );

    let listing_dir = tempfile::tempdir().unwrap();
    std::fs::write(listing_dir.path().join("marker-e2e.txt"), "x").unwrap();

    let wf_path = s.dir.path().join("wf.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: e2e
version: "1.0"
defaults:
  model: {MODEL_ID}
steps:
  - id: list
    type: tool
    output: listing
    tool: file_list
    inputs:
      path: "{dir}"
  - id: ask
    type: prompt
    output: answer
    template: "Summarize this listing as JSON: {{{{listing}}}}"
  - id: extract
    type: transform
    output: final_json
    operation: extract_json
    input: "{{{{answer}}}}"
outputs:
  - name: result
    from: extract
"#,
            dir = listing_dir.path().display()
        ),
    )
    .unwrap();

    let assert = cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    // The final output is the parsed JSON — fences stripped, content intact.
    // Sprint 012: a non-string output value prints as pretty JSON
    // (`"count": 1`), not the old compact re-serialization.
    assert!(
        stdout.contains("marker-e2e.txt") && stdout.contains("\"count\": 1"),
        "expected extracted JSON in workflow output, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("```"),
        "extract_json must strip the markdown fences, got:\n{stdout}"
    );

    // The prompt step must have received the real tool output: {{listing}}
    // flowed from the file_list step into the rendered prompt.
    let bodies = s.proxy.chat_request_bodies();
    assert!(!bodies.is_empty(), "expected a chat request");
    let prompt_request = serde_json::to_string(&bodies[0]).unwrap();
    assert!(
        prompt_request.contains("marker-e2e.txt"),
        "prompt request should embed the tool step output: {prompt_request}"
    );
}

// --- 012/WS2: typed context — field access + numeric branch condition ---

#[test]
fn structured_value_field_access_and_numeric_branch() {
    // The model returns JSON with score 10. `extract_json` stores it as a typed
    // Value, so `result.score >= 8` is a NUMERIC comparison (10 >= 8 → true).
    // A string compare would give "10" >= "8" → false (lexicographic), so the
    // then-arm running proves typed evaluation. The then-arm template reaches
    // into `result.title`, proving field access.
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy
        .mount_chat_text("{\"title\": \"Rust Guide\", \"score\": 10}");

    let wf_path = s.dir.path().join("typed.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: typed
version: "1.0"
defaults:
  model: {MODEL_ID}
steps:
  - id: gen
    type: prompt
    output: raw
    template: "Return JSON"
  - id: extract
    type: transform
    output: result
    operation: extract_json
    input: "{{{{raw}}}}"
  - id: route
    type: branch
    condition: "result.score >= 8"
    then:
      - id: deep
        type: prompt
        output: summary
        template: "DETAILED about {{{{result.title}}}}"
    else:
      - id: quick
        type: prompt
        output: summary
        template: "QUICK only"
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The second completion request is the branch arm. It must be the THEN arm
    // (numeric 10 >= 8) and must carry the field-accessed title.
    let bodies = s.proxy.chat_request_bodies();
    let arm_req = serde_json::to_string(bodies.last().unwrap()).unwrap();
    assert!(
        arm_req.contains("DETAILED about Rust Guide"),
        "then-arm with field access expected; got: {arm_req}"
    );
    assert!(
        !arm_req.contains("QUICK only"),
        "else-arm must not run (numeric 10 >= 8 is true): {arm_req}"
    );
}

// --- 012/WS3: loop step exits early on a typed body-output condition ---

#[test]
fn loop_exits_when_body_score_meets_condition() {
    // Both prompts get the same canned JSON (score 9). Iteration 1: draft +
    // critique → score 9 → `review.score >= 8` true → exit. So exactly 2 chat
    // completions, not 6 (which a run-to-cap of 3 would produce).
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text("{\"score\": 9}");

    let wf_path = s.dir.path().join("loop.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: loop-e2e
version: "1.0"
defaults:
  model: {MODEL_ID}
steps:
  - id: refine
    type: loop
    max_iterations: 3
    exit_condition: "review.score >= 8"
    steps:
      - id: draft
        type: prompt
        output: draft
        template: "Write about cats"
      - id: critique
        type: prompt
        output: review_raw
        template: "Score: {{{{draft}}}}"
      - id: score
        type: transform
        operation: extract_json
        input: "{{{{review_raw}}}}"
        output: review
outputs:
  - name: result
    from: draft
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            wf_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
        ])
        .assert()
        .success();

    let chats = s.proxy.chat_request_bodies().len();
    assert_eq!(
        chats, 2,
        "loop should exit after one iteration (2 prompts), got {chats}"
    );
}

// --- 012/WS4: nested workflow — child outputs reachable downstream ---

#[test]
fn subworkflow_child_outputs_flow_into_parent() {
    let s = setup();
    s.proxy.mount_health_ok();
    s.proxy.mount_chat_text("FACTS-ABOUT-CATS");

    // Child gathers notes; parent summarizes via field access {{research.notes}}.
    std::fs::write(
        s.dir.path().join("child.yaml"),
        format!(
            r#"
name: child
version: "1.0"
defaults:
  model: {MODEL_ID}
inputs:
  - name: subject
    type: string
    required: true
steps:
  - id: gather
    type: prompt
    output: notes
    template: "Facts about {{{{subject}}}}"
outputs:
  - name: notes
    from: gather
"#
        ),
    )
    .unwrap();
    let parent_path = s.dir.path().join("parent.yaml");
    std::fs::write(
        &parent_path,
        format!(
            r#"
name: parent
version: "1.0"
defaults:
  model: {MODEL_ID}
steps:
  - id: research
    type: workflow
    file: child.yaml
    inputs:
      subject: "cats"
    output: research
  - id: summarize
    type: prompt
    output: summary
    template: "Summarize: {{{{research.notes}}}}"
outputs:
  - name: summary
    from: summarize
"#
        ),
    )
    .unwrap();

    cmd()
        .current_dir(s.dir.path())
        .args([
            "workflow",
            "run",
            parent_path.to_str().unwrap(),
            "--config",
            s.config.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The parent's summarize request must carry the child's output, reached via
    // `research.notes` — proof the child's outputs object flowed back typed.
    let bodies = s.proxy.chat_request_bodies();
    let last = serde_json::to_string(bodies.last().unwrap()).unwrap();
    assert!(
        last.contains("Summarize: FACTS-ABOUT-CATS"),
        "parent should see the child output via field access: {last}"
    );
}
