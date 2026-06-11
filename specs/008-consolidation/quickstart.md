# Quickstart: Verifying Sprint 008

No live backend needed — consolidation is verified hermetically.

```bash
just ci                      # the gate: fmt, clippy -D warnings, full test suite

# WS1 spot checks
cargo test -p mv-core truncate                      # F1 UTF-8 safety
cargo test -p mv-core shell_exec                    # F5/F8 timeout kill + status
cargo test -p mv-core retry                         # F4 no panic on max_attempts: 0
cargo test -p mv-cli classify                       # F7 max-turns classification

# Workflow tool steps now actually run (F2/F3/F6):
cargo run -p mv-cli -- workflow run workflows/examples/tool-example.yaml
# → real file listing flows into the summary prompt (requires Ollama for the
#   prompt step; the tool step itself runs without any backend)

# JSON error contract (F10): errors now on stderr
cargo run -p mv-cli -- --json -c /nonexistent.yaml "hi" 2>err.json; cat err.json
```

Live TRT-LLM behaviors remain covered by `just test-trtllm` (unchanged).
