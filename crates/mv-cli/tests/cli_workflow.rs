use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    Command::cargo_bin("mv-cli").unwrap()
}

// --- T015: CLI workflow integration tests ---

#[test]
fn workflow_run_missing_file() {
    cmd()
        .args(["workflow", "run", "nonexistent.yaml"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("workflow file not found"));
}

#[test]
fn workflow_run_missing_required_input() {
    let dir = tempfile::tempdir().unwrap();
    let wf_path = dir.path().join("test.yaml");
    std::fs::write(
        &wf_path,
        r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello {{topic}}"
"#,
    )
    .unwrap();

    cmd()
        .args(["workflow", "run", wf_path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("required input"));
}

#[test]
fn workflow_validate_missing_file() {
    cmd()
        .args(["workflow", "validate", "nonexistent.yaml"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("workflow file not found"));
}

#[test]
fn workflow_validate_valid_file() {
    let dir = tempfile::tempdir().unwrap();
    let wf_path = dir.path().join("test.yaml");
    std::fs::write(
        &wf_path,
        r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
"#,
    )
    .unwrap();

    cmd()
        .args(["workflow", "validate", wf_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
}

#[test]
fn workflow_validate_invalid_duplicate_ids() {
    let dir = tempfile::tempdir().unwrap();
    let wf_path = dir.path().join("test.yaml");
    std::fs::write(
        &wf_path,
        r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out1
    template: "hello"
  - id: s1
    type: prompt
    output: out2
    template: "world"
"#,
    )
    .unwrap();

    cmd()
        .args(["workflow", "validate", wf_path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("duplicate step id"));
}

// --- 009/WS3: the shipped branch example must validate ---

#[test]
fn shipped_branch_example_validates() {
    // Guards the example against rot — `workflow validate` runs the full
    // recursive branch validation (maybe-defined, condition syntax, …).
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../workflows/examples/branch-example.yaml"
    );
    cmd()
        .args(["workflow", "validate", path])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
}

#[test]
fn shipped_parallel_example_validates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../workflows/examples/parallel-example.yaml"
    );
    cmd()
        .args(["workflow", "validate", path])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
}

// --- 008/F2: workflow tool steps execute real built-in tools ---

#[test]
fn workflow_tool_step_executes_builtin_tool() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marker-file.txt"), "x").unwrap();

    let wf_path = dir.path().join("wf.yaml");
    std::fs::write(
        &wf_path,
        format!(
            r#"
name: tool-test
version: "1.0"
steps:
  - id: list
    type: tool
    output: listing
    tool: file_list
    inputs:
      path: "{}"
outputs:
  - name: listing
    from: list
"#,
            dir.path().display()
        ),
    )
    .unwrap();

    // No prompt step → no model backend needed; the tool runs locally.
    cmd()
        .args(["workflow", "run", wf_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("marker-file.txt"));
}

#[test]
fn workflow_tool_step_unknown_tool_fails_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let wf_path = dir.path().join("wf.yaml");
    std::fs::write(
        &wf_path,
        r#"
name: tool-test
version: "1.0"
steps:
  - id: t1
    type: tool
    output: out
    tool: no_such_tool
"#,
    )
    .unwrap();

    cmd()
        .args(["workflow", "run", wf_path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("no_such_tool"));
}

// --- 008/F6: unknown step model errors instead of silently using default ---

#[test]
fn workflow_unknown_model_errors_not_silent_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let wf_path = dir.path().join("wf.yaml");
    std::fs::write(
        &wf_path,
        r#"
name: model-test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    model: definitely-not-a-real-model
    template: "hello"
"#,
    )
    .unwrap();

    cmd()
        .args(["workflow", "run", wf_path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "Model 'definitely-not-a-real-model' not found in registry",
        ));
}

#[test]
fn workflow_subcommand_help() {
    cmd()
        .args(["workflow", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("validate"));
}
