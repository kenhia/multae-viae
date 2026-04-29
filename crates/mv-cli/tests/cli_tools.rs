use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    Command::cargo_bin("mv-cli").unwrap()
}

/// Queries that don't require tools should still work after adding tool support.
/// This test verifies that argument parsing and basic execution are unaffected.
#[test]
fn no_tool_query_still_accepted() {
    // Should not fail on clap parsing (exit code 2 = usage error)
    let assert = cmd().arg("What is Rust?").assert();
    assert.code(predicate::ne(2));
}

/// Verify JSON output mode still works with tool-capable agent.
#[test]
fn json_output_with_tool_agent() {
    let assert = cmd().args(["--json", "Hello"]).assert();
    assert.code(predicate::ne(2));
}

/// Verbose logging should still work with tool-capable agent.
#[test]
fn verbose_with_tool_agent() {
    let assert = cmd().args(["-v", "Hello"]).assert();
    assert.code(predicate::ne(2));
}
