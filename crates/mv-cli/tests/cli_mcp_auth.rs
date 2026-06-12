//! WS1 (T003): bearer-token secrecy for HTTP MCP servers.
//!
//! When an HTTP MCP server entry sets `auth_token_env`, the token's *value* is
//! read from the environment and attached as an `Authorization: Bearer` header.
//! It must never reach stderr — not in tracing logs, not in the connection
//! warning, not in reqwest/hyper diagnostics — even under maximum verbosity.

use assert_cmd::Command;
use predicates::prelude::*;

/// A token value distinctive enough that any leak into stderr is unmistakable.
const SECRET: &str = "SUPERSECRET-bearer-tok-abc123XYZ";

fn write_http_mcp_config(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("mcp-servers.yaml");
    // A dead endpoint: the connection is attempted (exercising token
    // resolution + header construction) then fails and is logged-and-skipped.
    let yaml = "servers:\n  - name: klams\n    transport: http\n    \
                url: http://127.0.0.1:1/mcp\n    auth_token_env: MV_TEST_SECRET_TOKEN\n";
    std::fs::write(&path, yaml).expect("write mcp-servers.yaml");
    path
}

#[test]
fn token_value_never_appears_in_verbose_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_http_mcp_config(dir.path());

    // -vv (verbose) + a dead model endpoint so it fails fast after MCP setup.
    let assert = Command::cargo_bin("mv-cli")
        .unwrap()
        .env("MV_TEST_SECRET_TOKEN", SECRET)
        .args([
            "-vv",
            "--endpoint",
            "http://127.0.0.1:1",
            "--mcp-config",
            mcp_config.to_str().unwrap(),
            "Hello",
        ])
        .assert()
        .failure();

    // The token value must be absent from stderr entirely.
    assert.stderr(predicate::str::contains(SECRET).not());
}

#[test]
fn missing_token_var_errors_naming_var_not_value() {
    // `auth_token_env` names a variable that is NOT set: the per-server
    // connection fails with a message naming the variable, and (since there is
    // no value) the failure is non-fatal — the run proceeds and fails only on
    // the dead model endpoint. The variable name appears; nothing more.
    let dir = tempfile::tempdir().unwrap();
    let mcp_config = write_http_mcp_config(dir.path());

    let assert = Command::cargo_bin("mv-cli")
        .unwrap()
        .env_remove("MV_TEST_SECRET_TOKEN")
        .args([
            "-vv",
            "--endpoint",
            "http://127.0.0.1:1",
            "--mcp-config",
            mcp_config.to_str().unwrap(),
            "Hello",
        ])
        .assert()
        .failure();

    // The actionable error names the variable and the server, and the server
    // is skipped (non-fatal) — the run continues to the model call.
    assert
        .stderr(predicate::str::contains("MV_TEST_SECRET_TOKEN"))
        .stderr(predicate::str::contains("klams"));
}
