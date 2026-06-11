//! Hermetic MCP integration tests against the fake stdio MCP server (T027).
//!
//! The fixture binary (`src/bin/fake_mcp_server.rs`) is spawned by the CLI
//! through a generated mcp-servers.yaml, exercising the full path: rmcp
//! handshake → tool list → merge/skip rules → workflow tool step calls →
//! graceful shutdown (each test would hang past its timeout otherwise).

use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;

fn cmd() -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(20));
    c
}

/// Write an mcp-servers.yaml in `dir` pointing at the fake server binary.
fn write_mcp_config(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("mcp-servers.yaml");
    let yaml = format!(
        "servers:\n  - name: fake\n    transport: stdio\n    command: {}\n",
        env!("CARGO_BIN_EXE_fake_mcp_server"),
    );
    std::fs::write(&path, yaml).expect("write mcp-servers.yaml");
    path
}

/// Write a single-tool-step workflow in `dir` and return its path.
fn write_tool_workflow(dir: &std::path::Path, tool: &str, inputs_yaml: &str) -> std::path::PathBuf {
    let path = dir.join(format!("wf-{tool}.yaml"));
    let yaml = format!(
        r#"
name: mcp-{tool}
version: "1.0"
steps:
  - id: s1
    type: tool
    output: out
    tool: {tool}
{inputs_yaml}
outputs:
  - name: result
    from: s1
"#
    );
    std::fs::write(&path, yaml).expect("write workflow");
    path
}

// --- echo_tool: a normal MCP tool merges in and is callable ---

#[test]
fn mcp_echo_tool_callable_via_workflow_step() {
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_mcp_config(dir.path());
    let wf = write_tool_workflow(
        dir.path(),
        "echo_tool",
        "    inputs:\n      text: \"hello-mcp-roundtrip\"",
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "workflow",
            "run",
            wf.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("echo: hello-mcp-roundtrip"));
}

// --- file_list: exact-name collision → built-in wins ---

#[test]
fn mcp_name_collision_keeps_builtin_tool() {
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_mcp_config(dir.path());

    let listing_dir = tempfile::tempdir().unwrap();
    std::fs::write(listing_dir.path().join("marker-collision.txt"), "x").unwrap();

    let wf = write_tool_workflow(
        dir.path(),
        "file_list",
        &format!(
            "    inputs:\n      path: \"{}\"",
            listing_dir.path().display()
        ),
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "workflow",
            "run",
            wf.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
        ])
        .assert()
        .success()
        // The real built-in lists the directory…
        .stdout(predicate::str::contains("marker-collision.txt"))
        // …and the fake's sentinel never appears: the MCP version was skipped.
        .stdout(predicate::str::contains("FAKE-MCP-FILE-LIST").not());
}

// --- read_file: SEMANTIC_OVERLAPS → skipped, so the name resolves nowhere ---

#[test]
fn mcp_semantic_overlap_tool_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_mcp_config(dir.path());
    let wf = write_tool_workflow(
        dir.path(),
        "read_file",
        "    inputs:\n      path: \"/tmp/whatever\"",
    );

    cmd()
        .current_dir(dir.path())
        .args([
            "workflow",
            "run",
            wf.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        // The overlap skip means read_file is not registered at all — the
        // tool step fails loudly instead of reaching the fake's sentinel.
        .stderr(predicate::str::contains("read_file"));
}

// --- big_tool: MCP output honors the 10k truncation cap ---

#[test]
fn mcp_tool_output_is_truncated_at_cap() {
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_mcp_config(dir.path());
    let wf = write_tool_workflow(dir.path(), "big_tool", "    inputs: {}");

    let assert = cmd()
        .current_dir(dir.path())
        .args([
            "workflow",
            "run",
            wf.to_str().unwrap(),
            "--mcp-config",
            mcp_config.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[truncated at 10000 chars]"));

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(
        stdout.len() < 11_000,
        "output should be capped near 10k chars, got {} chars",
        stdout.len()
    );
}
