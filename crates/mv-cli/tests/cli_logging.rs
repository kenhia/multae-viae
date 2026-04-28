use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    Command::cargo_bin("mv-cli").unwrap()
}

#[test]
fn default_verbosity_no_stderr() {
    // With an empty prompt, we get an error on stderr — but that's the error message, not logging.
    // We test that no tracing-style log lines appear on stderr for a valid (but failing) request.
    // Use an unreachable endpoint so it fails fast without waiting for Ollama.
    let assert = cmd()
        .args(["--endpoint", "http://127.0.0.1:1", "Hello"])
        .assert();
    assert
        .failure()
        .stderr(predicate::str::contains("Error:"))
        .stderr(predicate::str::contains("INFO").not())
        .stderr(predicate::str::contains("DEBUG").not());
}

#[test]
fn verbose_flag_produces_log_lines() {
    // With -vv, we should see structured log lines on stderr.
    // Use an unreachable endpoint so it fails fast.
    let assert = cmd()
        .args(["-vv", "--endpoint", "http://127.0.0.1:1", "Hello"])
        .assert();
    assert
        .failure()
        .stderr(predicate::str::contains("INFO").or(predicate::str::contains("DEBUG")));
}
