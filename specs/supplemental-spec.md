# Supplemental Spec

Home for **ad-hoc changes that fall outside an active sprint spec**, per
Constitution Principle I (Spec-Driven Development): "Ad-hoc changes that fall
outside an active spec MUST be added to the current spec or to
`/specs/supplemental-spec.md`."

Use this file when a change is too small or too urgent for a sprint of its
own and does not belong to the active `specs/NNN-name/` directory. Each entry
must state what changed, why, and which commit landed it. The same workflow
gates apply: failing test first (where testable), `just ci` green, docs
updated.

## Change Log

| Date | Change | Rationale | Commit |
|------|--------|-----------|--------|
| 2026-06-12 | `default-run = "mv-cli"` in `crates/mv-cli/Cargo.toml` | The `fake_mcp_server` test fixture (added sprint 008) is a second bin target, making `cargo run -p mv-cli` / `just run` ambiguous. Pin the real CLI as the default-run target. | `3e39a3b` |
| 2026-06-12 | `models.yaml` default model `qwen3:8b` → `qwen3-coder:30b` | While verifying `just run`, `qwen3:8b`'s Ollama runner crashed on this host (`llama runner process has terminated`); `qwen3-coder:30b` runs cleanly. Repoint the registry default so the no-`-m` path works out of the box. (Host-specific model choice — adjust as needed.) | _next-sprint roll-in_ |

## Deferred to sprint 012

- **Backend-error misclassification.** A model backend that *responds* with an
  HTTP error status is reported as `BackendUnreachable` ("Is Ollama running?")
  — misleading, because the backend was reached. Captured while diagnosing the
  `just run` failure above: rig surfaced
  `CompletionError(HttpError(InvalidStatusCodeWithMessage(500, "…llama runner
  process has terminated…")))`, and `classify_backend_error`
  (`crates/mv-core/src/providers.rs`) matched the bare `"HttpError"` substring
  → `BackendUnreachable`. A status-bearing response is **not** a transport
  failure; only connection-refused / timeout / "error sending request" should
  be `BackendUnreachable`. Fix in 012: distinguish a status-code response
  (route to `CompletionFailed`, or a new status-bearing variant, surfacing the
  real status + body) from a genuine transport failure; decide its
  fallback-eligibility deliberately (a crashed per-model runner is arguably a
  reasonable reason to fall back). Touches the shared classifier + its tests +
  the fallback taxonomy, so it is sprint work, not an ad-hoc patch.

## Branch rollup note

The `default-run` + `models.yaml`-default commits live on the **`fix-default-run`**
branch and are intentionally **not** shipped via their own PR. Plan: branch
**`012-*`** off `fix-default-run` (so 012 carries these commits forward), fix
the misclassification there, and ship one 012 PR. After that PR merges, delete
**both** local branches (`fix-default-run` and `012-*`); the squash-merge on
`main` is the single record. The commit refs above therefore point at the
`fix-default-run` branch commits (reachable until cleanup), not a `main` hash.
