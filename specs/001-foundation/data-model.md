# Data Model: Foundation

**Feature**: 001-foundation
**Date**: 2026-04-28

## Entities

### Prompt (value object)

The user's input text to be sent to the model.

| Field | Type | Constraints |
|-------|------|-------------|
| text | String | Non-empty, trimmed |

**Validation**: Must not be empty or whitespace-only after trimming.

### CompletionResponse (value object)

The model's generated response.

| Field | Type | Constraints |
|-------|------|-------------|
| text | String | May be empty (model chose to say nothing) |

### OutputFormat (enum)

Controls how the CLI formats its output.

| Variant | Behavior |
|---------|----------|
| Plain | Print response text directly to stdout |
| Json | Print `{"response": "..."}` on success, `{"error": "..."}` on failure |

### BackendConfig (configuration)

Connection settings for the inference backend.

| Field | Type | Default | Constraints |
|-------|------|---------|-------------|
| endpoint | URL string | `http://localhost:11434` | Valid URL |
| model | String | `qwen3:4b` | Non-empty |

### MvError (error enum)

Typed error for the `mv-core` library.

| Variant | When | User Message |
|---------|------|-------------|
| EmptyPrompt | User provides empty/whitespace prompt | "Prompt cannot be empty." |
| BackendUnreachable | Cannot connect to Ollama | "Cannot reach model backend at {endpoint}. Is Ollama running?" |
| ModelNotFound | Ollama returns model-not-found | "Model '{model}' not found. Run: ollama pull {model}" |
| CompletionFailed | Model returns an error | "Model returned an error: {details}" |

## Relationships

```
CLI Args → Prompt → BackendConfig → [Ollama via Rig] → CompletionResponse → stdout
                                         ↓ (on error)
                                      MvError → stderr
```

## State

No persistent state in this phase. Each CLI invocation is stateless:
parse args → connect → prompt → print → exit.
