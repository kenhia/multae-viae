# CLI Contract: mv-cli (Sprint 002 additions)

**Scope**: New flags and behaviors added in sprint 002.
Existing interface from sprint 001 is unchanged.

## New CLI Flags

```
mv-cli [OPTIONS] <PROMPT>

New Options:
  -c, --config <PATH>      Path to models.yaml config file [default: ./models.yaml]
      --otlp [<ENDPOINT>]  Enable OTLP trace export [default endpoint: http://localhost:4318]
```

## Environment Variables

| Variable | Purpose | Precedence |
|----------|---------|------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP endpoint URL | Overridden by `--otlp <endpoint>` flag |
| `OPENAI_API_KEY` | OpenAI API authentication | Required when using an `openai` provider model |

## Config File Schema (`models.yaml`)

```yaml
models:                          # Required: list of model definitions
  - id: <string>                 # Required: model identifier (used with --model)
    provider: ollama | openai    # Required: inference provider
    default: <bool>              # Optional: default model (at most one)
    endpoint: <string>           # Optional: custom provider endpoint
    api_key_env: <string>        # Optional: env var name for API key (required for cloud)
    locality: local | cloud      # Optional: inferred from provider if omitted
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (config parse, model not found, API key missing, backend unreachable, etc.) |
| 2 | CLI usage error (unchanged from clap) |

## Error Messages (new)

| Condition | stderr Output |
|-----------|--------------|
| Config parse failure | `Error: Failed to parse config './models.yaml': <yaml error details>` |
| Unknown model | `Error: Model 'foo' not found in registry. Available: qwen3:4b, qwen3:8b` |
| Missing API key | `Error: API key required for openai. Set OPENAI_API_KEY environment variable.` |

When `--json` is used, errors are JSON on stdout: `{"error": "..."}`.
