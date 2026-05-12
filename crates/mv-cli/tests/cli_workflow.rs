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

#[test]
fn workflow_subcommand_help() {
    cmd()
        .args(["workflow", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("validate"));
}
