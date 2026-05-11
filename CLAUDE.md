# CLAUDE.md

## Context sources — read in order

1. `aidd_docs/memory/` — project context (architecture, testing, assertions, VCS)
2. `.claude/rules/` — auto-loaded coding rules and guardrails
3. `AGENTS.md` — roles, workflow, answering guidelines

## Project memory (`aidd_docs/memory/`)

| File | Content |
|------|---------|
| `architecture.md` | Tech stack, module graph, config system, IMAP semantics, tray patterns |
| `codebase_map.md` | Key files, entry points, module ownership |
| `coding_assertions.md` | Error handling patterns, key Rust idioms, pre-commit commands |
| `testing.md` | Test strategy, modules, execution commands |
| `deployment.md` | Build targets, feature flags |
| `vcs.md` | Branch model, commit conventions |
| `project_brief.md` | Vision, objectives, scope |

Update files in `aidd_docs/memory/` when architecture, conventions, or test coverage change.

## Rules & guardrails (`.claude/rules/`)

Auto-loaded by file-glob. Rules without `paths:` frontmatter apply globally.

| Category | Path |
|----------|------|
| Standards | `01-standards/` |
| Rust patterns | `02-programming-languages/` |
| Tooling | `04-tooling/` |
| Testing | `05-testing/` |
| Quality | `07-quality/` |
| Workflow | `09-other/` |

## Key constraints (not in rules)

- `tray` feature flag gates all GUI code (`tao`, `wry`, `tray-icon`, `rfd`) — CLI always builds without it
- Filesystem helpers: never follow symlinks — treat as opaque content
