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

# Live TRT-LLM tests (require proxy on http://localhost:8003 with a model loaded)
test-trtllm:
    cargo test -p mv-cli -- --ignored
