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
| 2026-06-13 | Architecture diagram assets: `docs/assets/architecture.svg` (README) + `docs/assets/architecture-workflow.svg` (docs/06) | Dark, color-coded visual overview for README readers. The high-level diagram draws the two-crate split with the `PromptExecutor`/`ToolExecutor` trait seam explicit and the workflow engine as a parse → validate → execute pipeline; the detail diagram covers step types, the typed `ExecutionContext`, and the executor seam. (Replaces a first-draft SVG that showed `mv-core` as undifferentiated boxes.) | `0a84c75` (on the `013-mv-server` branch) |
| 2026-06-12 | `default-run = "mv-cli"` in `crates/mv-cli/Cargo.toml` | The `fake_mcp_server` test fixture (added sprint 008) is a second bin target, making `cargo run -p mv-cli` / `just run` ambiguous. Pin the real CLI as the default-run target. | `3e39a3b` |
| 2026-06-12 | `models.yaml` default model `qwen3:8b` → `qwen3-coder:30b` | While verifying `just run`, `qwen3:8b`'s Ollama runner crashed on this host (`llama runner process has terminated`); `qwen3-coder:30b` runs cleanly. Repoint the registry default so the no-`-m` path works out of the box. (Host-specific model choice — adjust as needed.) | shipped in the sprint 012 PR |

## Resolved in sprint 012

- **Backend-error misclassification** (below) — fixed in sprint 012 WS1 as
  `MvError::BackendErrorResponse` (truthful, fallback-eligible; the bare
  `"HttpError"` substring no longer implies unreachable). See
  `specs/012-dsl-completion/`.

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

## Branch rollup note (closed)

The `default-run` + `models.yaml`-default commits originated on the
**`fix-default-run`** branch and were **not** shipped via their own PR. As
planned, **`012-dsl-completion`** branched off `fix-default-run` (carrying
those commits forward), fixed the misclassification, and shipped everything in
one sprint-012 PR. On merge, both local branches (`fix-default-run` and
`012-dsl-completion`) are deleted; the squash-merge on `main` is the single
record (the `3e39a3b` ref above was the pre-squash branch commit).
