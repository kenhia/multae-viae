use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;

fn cmd() -> Command {
    let mut c = Command::cargo_bin("mv-cli").unwrap();
    c.timeout(Duration::from_secs(20));
    c
}

/// models.yaml pinned to an instantly-refused endpoint (bind ephemeral port,
/// drop the listener) so tool-agent plumbing tests are deterministic — they
/// must never reach a live Ollama on :11434.
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

/// Queries that don't require tools should still work after adding tool support.
/// This test verifies that argument parsing and basic execution are unaffected.
#[test]
fn no_tool_query_still_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "What is Rust?"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

/// Verify JSON output mode still works with tool-capable agent.
#[test]
fn json_output_with_tool_agent() {
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

/// Verbose logging should still work with tool-capable agent.
#[test]
fn verbose_with_tool_agent() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_unreachable_config(dir.path());
    cmd()
        .current_dir(dir.path())
        .args(["--config", config.to_str().unwrap(), "-v", "Hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Cannot reach model backend"));
}

/// End-to-end test with a real filesystem MCP server.
///
/// Requires: `npx` on PATH and network access (to fetch the MCP server package).
/// Run with: `cargo test --test cli_tools -- --ignored mcp_filesystem`
#[test]
#[ignore = "requires npx and a running Ollama instance"]
fn mcp_filesystem_integration() {
    use std::io::Write;

    // Write a temporary MCP config pointing at the filesystem server
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("mcp-servers.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(
        f,
        "servers:\n  - name: filesystem\n    transport: stdio\n    command: npx\n    args: [\"-y\", \"@modelcontextprotocol/server-filesystem\", \"/tmp\"]"
    )
    .unwrap();

    // Create a known marker file so we can verify the tool saw it
    let marker = std::path::Path::new("/tmp/mv-cli-test-marker.txt");
    std::fs::write(marker, "integration-test").unwrap();

    let assert = cmd()
        .args([
            "--mcp-config",
            config_path.to_str().unwrap(),
            "-vv",
            "List the files in /tmp. Include every filename you see.",
        ])
        .timeout(std::time::Duration::from_secs(120))
        .assert();

    // The model should have used a tool and returned a listing that includes our marker
    assert
        .success()
        .stdout(predicate::str::contains("mv-cli-test-marker"));

    // Clean up marker
    let _ = std::fs::remove_file(marker);
}
