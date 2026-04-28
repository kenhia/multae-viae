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
