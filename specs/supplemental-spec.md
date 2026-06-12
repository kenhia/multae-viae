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
