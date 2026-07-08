# justfile for multae-viae

set dotenv-load := true
set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
# set positional-arguments

# List available recipes
default:
    @just --list

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

# Run the REST controller daemon (mv-server). FLAGS go after, e.g.
# `just serve --bind 127.0.0.1:7077 --mcp-servers mcp-servers.yaml`.
serve *FLAGS:
    cargo run -p mv-server -- {{FLAGS}}

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
# (m-v calls out to Ollama/TRT-LLM/cloud — it does not serve a model). They use
# the repo's ./models.yaml; KLAMS_MODEL picks which model in it to drive (must
# be defined there — e.g. qwen3-coder:30b), else the registry default (qwen3:8b).
# Those two SKIP (not fail) if no backend is reachable. Hermetic suite (just ci)
# stays green without any of this.
test-klams:
    cargo test -p mv-cli --test cli_klams -- --ignored --test-threads=1 --nocapture
    cargo test -p mv-server --test api -- --ignored --test-threads=1 --nocapture
