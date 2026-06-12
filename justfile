# justfile for multae-viae

set dotenv-load := true
set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
# set positional-arguments

# Build all workspace crates
build:
    cargo build --workspace

# Run all tests
test:
    cargo test --workspace

# Clippy lint check
check:
    cargo clippy --all-targets --all-features -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Lint: format + clippy
lint: fmt check

# Run the CLI with a prompt
run PROMPT *FLAGS:
    cargo run -p mv-cli -- {{FLAGS}} "{{PROMPT}}"

# CI: format check + clippy + tests
ci:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --workspace

# Live TRT-LLM tests (require proxy on http://localhost:8003 with a model loaded).
# Forced single-threaded: the tests share one GPU-backed proxy, so concurrent
# heavy generations contend and time out. Run serially for stable results.
test-trtllm:
    cargo test -p mv-cli -- --ignored --test-threads=1

# Live klams tests (require klams on kubs0 + KLAMS_TOKEN). KLAMS_URL overrides
# the endpoint. The workflow-retrieval test is model-free — it is the pure
# "is klams reachable / auth / retrieval working" check. The agentic and memory
# tests drive the full prompt path, so they need a reachable MODEL BACKEND
# (m-v calls out to Ollama/TRT-LLM/cloud — it does not serve a model);
# KLAMS_MODEL picks which registered model to drive (else the CLI default).
# Those two SKIP (not fail) if no backend is reachable. Hermetic suite (just ci)
# stays green without any of this.
test-klams:
    cargo test -p mv-cli --test cli_klams -- --ignored --test-threads=1 --nocapture
