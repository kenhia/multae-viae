# kris Constitution

## Core Principles

### I. Spec-Driven Development (SDD)

All changes MUST be documented in `/specs/` before implementation begins.
Iteration-scoped changes live in their spec directory
(e.g., `/specs/001-scanner/spec.md`). Ad-hoc changes that fall outside
an active spec MUST be added to the current spec or to
`/specs/supplemental-spec.md`.

A combined specification at `/docs/specification.md` MUST be
created or updated during the polish phase of each spec and following
any ad-hoc changes. This serves as the canonical, up-to-date
reference for the full system.

- No code change without a corresponding spec entry.
- Specs define acceptance criteria before implementation starts.
- Spec updates are part of the definition of done for every iteration.

### II. Architecture First

An architecture document at `docs/architecture.md` MUST be maintained
as the authoritative technical reference for the system.

- The architecture document MUST be updated during the polish phase
  of each spec and following ad-hoc changes.
- Supporting documents (e.g., `docs/data-model.md`) MUST remain
  consistent with the architecture document.
- Architectural decisions MUST be recorded in
  `docs/clarifications-needed.md` (decision log section).
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
- Integration tests are required for cross-component boundaries
  (scanner↔catalog, catalog↔planner, retriever↔Qdrant).

### IV. Code Standards Gate

All code MUST pass the following checks before commit:

1. **Formatted** — code is auto-formatted per ecosystem tooling.
2. **Linted** — no lint errors or warnings.
3. **Type-checked** — static type analysis passes (where applicable).
4. **Unit tests** — all tests pass.

The CI variant of each check (strict/non-interactive) MUST pass
clean. This applies to both new and existing code — no broken
windows.

Use of `--unsafe-fixes` (e.g., `ruff check --fix --unsafe-fixes`)
MUST be approved by the user before execution.

See [Pre-Commit Checks by Ecosystem](#pre-commit-checks-by-ecosystem)
for specific tooling per language.

### V. User Documentation from Day One

Each iteration (spec/sprint) MUST update user-facing documentation:

- **Installation/setup guide** — `docs/setup.md`
- **Usage guide** — `docs/usage.md`

Documentation updates are part of the definition of done for every
iteration, not a follow-up task. If a feature changes how the user
installs, configures, or uses the system, the docs MUST reflect
that before the iteration is complete.

### VI. Quality & Accessibility

User experience MUST be consistent across all interfaces
(CLI, API, Web UI, TUI). Specific standards:

- **CLI output**: Consistent formatting via Rich; errors to stderr,
  results to stdout; JSON output available for programmatic use.
- **API responses**: Consistent error shape, status codes, and
  pagination patterns across all endpoints.
- **Error messages**: Actionable — tell the user what went wrong and
  what they can do about it. Never expose raw stack traces in
  non-debug mode.
- **Accessibility**: Web UI (when implemented) MUST meet WCAG 2.1 AA.
  CLI MUST respect `NO_COLOR` and terminal width. API MUST return
  properly structured error responses.
- **Logging**: Structured, leveled, and sufficient for debugging
  without being noisy at default verbosity.

### VII. Simplicity & Intentional Design

Every addition MUST justify its complexity. YAGNI applies:

- Do not add features, abstractions, or configuration options for
  hypothetical future requirements.
- Prefer explicit over implicit behavior.
- Start with the simplest approach that meets the spec; refactor
  only when measured need arises.
- Defensive coding at system boundaries only (user input, external
  APIs, file I/O). Trust internal code and framework guarantees.

## Directory Structure

The following directories MUST be used as specified. Create if
they do not exist.

| Directory | Purpose | Git tracked |
|-----------|---------|-------------|
| `.scratch-agent/` | Temporary workspace for agent use | No (`.gitignore`) |
| `.scratch/` | Temporary workspace for user use | No (`.gitignore`) |
| `docs/` | Project documentation (architecture, data model, guides) | Yes |
| `specs/` | Iteration and supplemental specifications (SDD) | Yes |
| `poc-ex/` | Proof of concept and exploration code | Yes |

- `README.md` lives at the project root, not in `docs/`.
- Spec directories use the pattern `specs/NNN-feature-name/`.

## Pre-Commit Checks by Ecosystem

Each language ecosystem has specific tooling. Supplemental
constitution files (e.g., `.specify/memory/constitution.python.md`)
MUST be created for each language used, containing the full
pre-commit check instructions.

### Python

```bash
# Standard
ruff format
ruff check            # or: ruff check --fix (no --unsafe-fixes without approval)
ty check
pytest

# CI variant (must pass clean before commit)
ruff format --check
ruff check
ty check
pytest -q
```

### Rust

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

### Svelte

```bash
# Standard
prettier --write .
eslint .
svelte-check
vitest run
npx playwright test   # as needed for e2e

# CI variant (must pass clean before commit)
prettier --check .
eslint .
svelte-check
vitest run
npx playwright test
```

### PowerShell

> **WARNING**: Pester, VS Code, and agent capture of terminal can
> easily lock up VS Code. If PowerShell is used, alert the user
> during planning (`/speckit.plan`) to provide "Pester Instructions"
> for the agent.

```powershell
Invoke-Formatter -ScriptDefinition (Get-Content -Raw -Path .\path\to\script.ps1) | Out-Null
Invoke-ScriptAnalyzer -Path . -Recurse
Invoke-Pester
```

### C# / .NET

```bash
# Standard
dotnet format
dotnet build
dotnet test

# CI variant (must pass clean before commit)
dotnet format --verify-no-changes
dotnet build -warnaserror
dotnet test --logger trx
```

> **Note**: PowerShell and C# are unlikely for most of this project
> but may apply if/when a Windows client is added.

## Development Workflow

1. **Spec** — Define or update the spec (`/specs/`).
2. **Plan** — Create implementation plan from spec.
3. **Implement** — Follow TDD; write tests first, then code.
4. **Check** — Run pre-commit checks (format, lint, type, test).
5. **Document** — Update `docs/setup.md`, `docs/usage.md`,
   `docs/architecture.md`, and `docs/specification.md` as needed.
6. **Review** — Verify constitution compliance before commit.

Ad-hoc changes follow the same workflow but reference
`/specs/supplemental-spec.md` instead of a feature spec.

## Governance

This constitution supersedes all other development practices for the
kris project. All code changes, reviews, and architectural decisions
MUST verify compliance with these principles.

**Amendment procedure**:
1. Propose the change with rationale.
2. Document the amendment in this file.
3. Update the version number per semantic versioning:
   - **MAJOR**: Principle removal or backward-incompatible redefinition.
   - **MINOR**: New principle or materially expanded guidance.
   - **PATCH**: Clarifications, wording, or typo fixes.
4. Update `LAST_AMENDED_DATE`.
5. Propagate changes to dependent templates and documentation.

**Compliance review**: Every commit MUST pass the Code Standards Gate
(Principle IV). Architecture and spec alignment (Principles I, II)
are verified during iteration polish.

**Version**: 1.0.0 | **Ratified**: 2026-03-19 | **Last Amended**: 2026-03-19
