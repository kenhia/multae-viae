<!--
SYNC IMPACT REPORT
Version: — → 1.0.0 (initial adoption)
Modified principles: N/A (initial adoption)
Added sections:
  - 7 Core Principles (I–VII)
  - Directory Structure
  - Pre-Commit Checks by Ecosystem (Rust)
  - Development Workflow
  - Governance
Removed sections: N/A
Templates requiring updates:
  - .specify/templates/plan-template.md ✅ no changes needed
  - .specify/templates/spec-template.md ✅ no changes needed
  - .specify/templates/tasks-template.md ✅ no changes needed
Follow-up TODOs: None
-->

# multae-viae Constitution

## Core Principles

### I. Spec-Driven Development (SDD)

All changes MUST be documented in `/specs/` before implementation
begins. Iteration-scoped changes live in their spec directory
(e.g., `/specs/001-foundation/spec.md`). Ad-hoc changes that fall
outside an active spec MUST be added to the current spec or to
`/specs/supplemental-spec.md`.

Architecture and design documents in `docs/` MUST be updated during
the polish phase of each spec and following any ad-hoc changes.

- No code change without a corresponding spec entry.
- Specs define acceptance criteria before implementation starts.
- Spec updates are part of the definition of done for every
  iteration.

### II. Architecture First

The architecture document at `docs/01-architecture-design.md` MUST
be maintained as the authoritative technical reference for the
system.

- The architecture document MUST be updated during the polish phase
  of each spec and following ad-hoc changes.
- Supporting documents (e.g., research docs, model routing design)
  MUST remain consistent with the architecture document.
- Architectural decisions MUST be recorded with rationale — either
  inline in the architecture doc or in a dedicated decision log.
- Implementation MUST NOT diverge from the documented architecture
  without updating the architecture document first.

### III. Test-Driven Development (TDD)

TDD is mandatory for all new code changes. The Red-Green-Refactor
cycle MUST be followed:

1. Write a failing test that captures the requirement.
2. Implement the minimum code to make the test pass.
3. Refactor while keeping tests green.

- Tests MUST exist before or alongside the code they validate.
- Test coverage MUST NOT decrease with new changes.
- Integration tests are required for cross-crate boundaries and
  external service interactions (model backends, MCP servers, RAG).

### IV. Code Standards Gate

All code MUST pass the following checks before commit:

1. **Formatted** — code is auto-formatted per ecosystem tooling.
2. **Linted** — no lint errors or warnings.
3. **Type-checked** — compiler/static analysis passes cleanly.
4. **Unit tests** — all tests pass.

The CI variant of each check (strict/non-interactive) MUST pass
clean. This applies to both new and existing code — no broken
windows.

See [Pre-Commit Checks by Ecosystem](#pre-commit-checks-by-ecosystem)
for specific tooling per language.

### V. Documentation from Day One

Each iteration (spec/sprint) MUST update relevant documentation:

- **README.md** — project overview and getting started
- **Architecture** — `docs/01-architecture-design.md`
- **Usage/setup guides** — as features become user-facing

Documentation updates are part of the definition of done for every
iteration, not a follow-up task. If a feature changes how the system
is built, configured, or used, the docs MUST reflect that before the
iteration is complete.

### VI. Quality & Observability

User experience MUST be consistent across all interfaces
(CLI, API). Specific standards:

- **CLI output**: Consistent formatting; errors to stderr, results
  to stdout; JSON output available for programmatic use.
- **API responses**: Consistent error shape, status codes, and
  pagination patterns across all endpoints.
- **Error messages**: Actionable — tell the user what went wrong and
  what they can do about it. Never expose raw stack traces in
  non-debug mode.
- **Telemetry**: All model calls, tool invocations, and workflow
  executions MUST emit OpenTelemetry traces and metrics. Telemetry
  is a first-class concern, not an afterthought.
- **Logging**: Structured via `tracing` crate, leveled, and
  sufficient for debugging without being noisy at default verbosity.

### VII. Simplicity & Intentional Design

Every addition MUST justify its complexity. YAGNI applies:

- Do not add features, abstractions, or configuration options for
  hypothetical future requirements.
- Prefer explicit over implicit behavior.
- Start with the simplest approach that meets the spec; refactor
  only when measured need arises.
- Defensive coding at system boundaries only (user input, external
  APIs, MCP messages, file I/O). Trust internal code and compiler
  guarantees.

## Directory Structure

The following directories MUST be used as specified. Create if
they do not exist.

| Directory | Purpose | Git tracked |
|-----------|---------|-------------|
| `.scratch-agent/` | Temporary workspace for agent use | No (`.gitignore`) |
| `.scratch/` | Temporary workspace for user use | No (`.gitignore`) |
| `docs/` | Project documentation (architecture, research, guides) | Yes |
| `specs/` | Iteration and supplemental specifications (SDD) | Yes |

- `README.md` lives at the project root, not in `docs/`.
- Spec directories use the pattern `specs/NNN-feature-name/`.

## Pre-Commit Checks by Ecosystem

### Rust (Primary)

```bash
# Standard
cargo fmt
cargo clippy --all-targets --all-features
cargo check
cargo test

# CI variant (must pass clean before commit)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Supplemental constitution files (e.g.,
`.specify/memory/constitution.svelte.md`) MUST be created for each
additional language used, containing the full pre-commit check
instructions.

## Development Workflow

1. **Spec** — Define or update the spec (`/specs/`).
2. **Plan** — Create implementation plan from spec.
3. **Implement** — Follow TDD; write tests first, then code.
4. **Check** — Run pre-commit checks (format, lint, type, test).
5. **Document** — Update `docs/` as needed.
6. **Review** — Verify constitution compliance before commit.

Ad-hoc changes follow the same workflow but reference
`/specs/supplemental-spec.md` instead of a feature spec.

## Governance

This constitution supersedes all other development practices for the
multae-viae project. All code changes, reviews, and architectural
decisions MUST verify compliance with these principles.

**Amendment procedure**:
1. Propose the change with rationale.
2. Document the amendment in this file.
3. Update the version number per semantic versioning:
   - **MAJOR**: Principle removal or backward-incompatible
     redefinition.
   - **MINOR**: New principle or materially expanded guidance.
   - **PATCH**: Clarifications, wording, or typo fixes.
4. Update `LAST_AMENDED_DATE`.
5. Propagate changes to dependent templates and documentation.

**Compliance review**: Every commit MUST pass the Code Standards Gate
(Principle IV). Architecture and spec alignment (Principles I, II)
are verified during iteration polish.

**Version**: 1.0.0 | **Ratified**: 2026-04-28 | **Last Amended**: 2026-04-28
