# Quickstart: Verifying Sprint 009

Everything verifies hermetically — fallback failure injection runs against the
wiremock fake proxy from 008.

```bash
just ci                      # the gate: fmt, clippy -D warnings, full test suite

# WS1/WS2 spot checks
cargo test -p mv-core fallback                      # chain validation + eligibility
cargo test -p mv-core preflight                     # per-provider Healthy/Dead/Unknown
cargo test -p mv-cli --test cli_fake_proxy fallback # dead primary → fallback serves

# WS3/WS4 spot checks
cargo test -p mv-core branch                        # condition eval + maybe-defined validation
cargo test -p mv-core parallel                      # snapshot isolation + rendezvous concurrency

# Live demo (needs Ollama; optional)
# models.yaml: give your trtllm entry `fallback: [qwen3:8b]`, leave the proxy down:
just run "Explain Rust ownership" -m llama-fp8
# → stderr notes the fallback, response served by qwen3:8b
just run "Explain Rust ownership" --json -m llama-fp8 | jq .model_used
# → "qwen3:8b"

# Branch / parallel examples
cargo run -p mv-cli -- workflow run workflows/examples/branch-example.yaml
cargo run -p mv-cli -- workflow run workflows/examples/parallel-example.yaml
```

Live TRT-LLM behaviors remain covered by `just test-trtllm` (unchanged).
