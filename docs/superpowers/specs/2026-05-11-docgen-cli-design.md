# Docgen CLI — Design Spec
**Date:** 2026-05-11

## Overview

A local CLI tool that generates and inserts docstrings into Python and TypeScript source files using a locally-hosted vLLM instance. Designed for use during development or at build time via `uv run` / `pnpm run` scripts. Operates safely by isolating all file edits in a short-lived git sub-branch, then merging back into the current working branch.

---

## File Layout

```
ai-devops-toolkit/
└── docgen/
    ├── docgen.py        # single script: CLI, parsing, LLM calls, git ops
    ├── pyproject.toml   # uv script entry point + dependencies
    └── .env.example     # VLLM_BASE_URL, BATCH_SIZE
```

---

## CLI Interface

Built with `typer`. Invoked via `uv run docgen`.

```bash
uv run docgen path/to/file.py
uv run docgen src/                         # flat folder, skip existing docstrings
uv run docgen src/ --recursive             # recurse into subdirectories
uv run docgen src/ --force                 # regenerate all docstrings
uv run docgen src/ --format mkdocs         # explicit format override
```

### Arguments & Flags

| Name | Type | Default | Description |
|---|---|---|---|
| `target` | path (required) | — | File or folder to process |
| `--format` | string | auto | Docstring format: `mkdocs`, `tsdoc`. Auto-detected from extension if omitted. |
| `--recursive` / `-r` | flag | false | Recurse into subdirectories when target is a folder |
| `--force` | flag | false | Regenerate docstrings even if they already exist |

### Format Auto-detection

| Extension | Default format |
|---|---|
| `.py` | `mkdocs` |
| `.ts`, `.tsx` | `tsdoc` |

---

## Configuration (`.env`)

```
VLLM_BASE_URL=http://192.168.x.x:30000/v1
BATCH_SIZE=4
```

- `VLLM_BASE_URL`: Base URL of the local vLLM server. Required.
- `BATCH_SIZE`: Number of files sent to the LLM concurrently. Default: `4`.

Model is auto-detected by querying `GET /v1/models` (same pattern as `reviewer.py`). No manual model specification needed.

---

## Data Flow

```
1. Parse CLI args (typer)
2. Load .env → VLLM_BASE_URL, BATCH_SIZE
3. PRE-FLIGHT CHECKS (see below)
4. Auto-detect model via GET /v1/models
5. Resolve target:
     file  → [file]
     folder → glob *.py / *.ts (+ subdirs if --recursive)
6. For each file:
     Parse → identify functions/classes missing docstrings
     If --force → include all regardless
     If nothing to annotate → skip file
7. Batch files by BATCH_SIZE → parallel LLM calls
     Each call: raw source + format instruction (no reasoning)
     Response: patched source with docstrings inserted
8. Git workflow:
     git checkout -b docgen/<timestamp>
     Write all patched files
     git add + git commit
     git merge docgen/<timestamp> into original branch
     git branch -d docgen/<timestamp>
```

---

## Pre-flight Checks

Run before any file parsing or LLM calls. Abort on failure.

| Check | Failure behavior |
|---|---|
| **Dirty working tree** (`git status --porcelain`) | Print list of modified files, exit code `1`. User must commit or stash. |
| **vLLM unreachable** (`GET /v1/models` timeout) | Print clear message with URL attempted, exit code `1`. |
| **No matching files found** | Warn and exit code `2`. |

---

## Error Handling During Processing

- **LLM call fails for a file** → log error, skip that file, continue with the rest of the batch. Never abort mid-run.
- **Git merge fails** → leave `docgen/<timestamp>` branch intact, print branch name for manual inspection. Exit code `1`.
- **File parse error** → skip the file, log a warning, continue.

---

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success (some files may have been skipped cleanly) |
| `1` | Pre-flight failure or git merge failure |
| `2` | Nothing to do (no files found, or all already documented and `--force` not set) |

---

## Git Workflow Detail

The tool always operates on a short-lived sub-branch:

```
current branch: feature/my-work
                    │
                    ├── git checkout -b docgen/20260511-143022
                    │       (edit files, insert docstrings)
                    │       git add + git commit
                    │
                    └── git merge docgen/20260511-143022
                        git branch -d docgen/20260511-143022
```

Since the docgen branch is always forked from the current HEAD, and the tool only modifies docstrings (never logic), merge conflicts are extremely unlikely. The pre-flight dirty-tree check eliminates the primary conflict vector.

---

## Dependencies

- `typer` — CLI framework
- `openai` — OpenAI-compatible client for vLLM
- `httpx` — vLLM reachability check + model detection
- `python-dotenv` — `.env` loading
- `gitpython` — git operations

---

## Integration as Scripts

**uv (`pyproject.toml`):**
```toml
[project.scripts]
docgen = "docgen:app"
```

**pnpm (`package.json`):**
```json
"scripts": {
  "docgen": "uv run docgen"
}
```
