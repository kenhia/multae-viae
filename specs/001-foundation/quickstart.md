# Quickstart: Foundation

**Feature**: 001-foundation
**Date**: 2026-04-28

## Prerequisites

1. **Rust toolchain** (stable, recent — edition 2024 support):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup update stable
   ```

2. **Ollama** installed and running:
   ```bash
   # Install (Linux)
   curl -fsSL https://ollama.com/install.sh | sh

   # Pull a model
   ollama pull qwen3:4b

   # Verify it's running
   curl -s http://localhost:11434/api/tags | head
   ```

3. **just** task runner:
   ```bash
   cargo install just
   ```

## Build & Run

```bash
# Build the workspace
just build

# Run with a prompt
just run "What is Rust?"

# Or directly via cargo
cargo run -p mv-cli -- "What is Rust?"
```

## CLI Interface

```
mv-cli [OPTIONS] <PROMPT>

Arguments:
  <PROMPT>    The prompt to send to the model

Options:
  -m, --model <MODEL>      Model name [default: qwen3:4b]
  -e, --endpoint <URL>     Ollama endpoint [default: http://localhost:11434]
  -j, --json               Output response as JSON object
  -v, --verbose             Increase log verbosity (repeat for more: -vv)
  -h, --help               Print help
  -V, --version            Print version
```

### Examples

```bash
# Simple prompt
mv-cli "Explain Rust's ownership model in 3 sentences"

# Use a specific model
mv-cli --model qwen3:8b "Write a haiku about programming"

# Verbose output (logs on stderr, response on stdout)
mv-cli -vv "Hello world" 2>debug.log

# Pipe-friendly (only response on stdout)
mv-cli "List 5 Rust crates for HTTP" | head -5

# JSON output (for agents and scripts)
mv-cli --json "What is Rust?"
# → {"response": "Rust is a systems programming language..."}
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success — response printed |
| 1 | Error — message on stderr |

## Development Commands (justfile)

```bash
just build          # cargo build --workspace
just test           # cargo test --workspace
just check          # cargo clippy --all-targets --all-features -- -D warnings
just fmt            # cargo fmt --all
just lint           # fmt + clippy
just run "prompt"   # cargo run -p mv-cli -- "prompt"
just ci             # fmt --check + clippy -D warnings + test (what CI runs)
```
