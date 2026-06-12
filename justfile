# justfile for multae-viae

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

# Live klams RAG tests (require klams on kubs0 + KLAMS_TOKEN; KLAMS_URL and
# KLAMS_MODEL optionally override the endpoint / agentic model). The retrieval
# test needs only klams; the agentic test also needs a live model (skipped
# unless KLAMS_MODEL is set). Hermetic suite stays green without any of these.
test-klams:
    cargo test -p mv-cli --test cli_klams -- --ignored --test-threads=1
