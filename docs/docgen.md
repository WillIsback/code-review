# docgen

CLI tool that generates and inserts docstrings into Python and TypeScript source files using a locally-hosted [vLLM](https://github.com/vllm-project/vllm) instance. Designed for use during development or at build time via `uv run` or `pnpm run`.

All file edits are isolated in a short-lived `docgen/<timestamp>` git branch that is merged back into the current branch and deleted — keeping your working branch clean.

---

## Installation

### Global install (use from anywhere)

Install `docgen` as a global tool with `uv`. It becomes available system-wide as a standalone command, without needing to be inside a specific project.

```bash
uv tool install git+https://github.com/WillIsback/ai-devops-toolkit.git
```

Then run from any directory:

```bash
docgen src/ --recursive
```

To update later:

```bash
uv tool upgrade docgen
```

---

### Local install — Python project

Add `docgen` as a development dependency of your Python project using `uv`:

```bash
uv add --dev git+https://github.com/WillIsback/ai-devops-toolkit.git
```

Then invoke it through `uv run` without installing it globally:

```bash
uv run docgen src/
```

Or register it as a script in your own `pyproject.toml`:

```toml
[tool.uv.scripts]
docgen = "docgen.docgen:app"
```

---

### Local install — TypeScript / Node project

Add the wrapper to your `package.json` scripts (requires `uv` to be available in the environment):

```bash
npm pkg set scripts.docgen="uv run --with git+https://github.com/WillIsback/ai-devops-toolkit.git docgen"
```

Or add it manually to `package.json`:

```json
{
  "scripts": {
    "docgen": "uv run --with git+https://github.com/WillIsback/ai-devops-toolkit.git docgen"
  }
}
```

Then run:

```bash
npm run docgen -- src/
pnpm run docgen -- src/
```

---

### Local install — Python + TypeScript monorepo

Clone or add the toolkit as a submodule, then wire both entry points:

```bash
# Clone alongside your project (or use as a git submodule)
git clone https://github.com/WillIsback/ai-devops-toolkit.git tools/ai-devops-toolkit
cd tools/ai-devops-toolkit && uv sync
```

In your root `package.json`:

```json
{
  "scripts": {
    "docgen": "uv run --project tools/ai-devops-toolkit docgen"
  }
}
```

In your root `pyproject.toml` (if using uv workspaces):

```toml
[tool.uv.workspace]
members = ["tools/ai-devops-toolkit"]
```

---

## Configuration

`docgen` loads configuration in two layers (project overrides global):

### Global config (recommended for the global install)

Create `~/.config/docgen/.env` once — applies to every project on the machine:

```bash
mkdir -p ~/.config/docgen
cat > ~/.config/docgen/.env <<EOF
VLLM_BASE_URL=http://<your-host>:30000/v1
BATCH_SIZE=4
EOF
```

### Project-level config

Create a `.env` at the root of any project to override the global values for that project:

```
VLLM_BASE_URL=http://<your-host>:30000/v1
BATCH_SIZE=4
```

`docgen` loads the global config first, then the project `.env` on top. Both files are optional — whichever is found wins; if neither is found, `VLLM_BASE_URL` defaults to `http://localhost:30000/v1`.

## Usage

```bash
# Single file
uv run docgen path/to/file.py

# Flat folder (top-level files only)
uv run docgen src/

# Recurse into subdirectories
uv run docgen src/ --recursive

# Regenerate existing docstrings
uv run docgen src/ --force

# Explicit format override
uv run docgen src/ --format tsdoc

# Via pnpm
pnpm run docgen -- src/
```

## Options

| Flag | Default | Description |
|---|---|---|
| `target` | — | File or folder to process (required) |
| `--format` | auto | Docstring format: `mkdocs` (Python) or `tsdoc` (TypeScript/TSX) |
| `--recursive` / `-r` | off | Recurse into subdirectories when target is a folder |
| `--force` | off | Regenerate docstrings even if they already exist |

## Format auto-detection

| Extension | Default format |
|---|---|
| `.py` | `mkdocs` |
| `.ts`, `.tsx` | `tsdoc` |

## How it works

1. **Pre-flight checks** — aborts if the working tree has uncommitted changes, or if vLLM is unreachable
2. **Model detection** — queries `GET /v1/models` to auto-detect the loaded model
3. **File resolution** — collects `.py` / `.ts` / `.tsx` files under the target (flat or recursive)
4. **Parsing** — Python files are parsed with the `ast` module; TypeScript files use regex. Files already fully documented are skipped (unless `--force`)
5. **Batch LLM calls** — files are sent to vLLM in parallel, up to `BATCH_SIZE` concurrent calls
6. **Git workflow** — creates a `docgen/<timestamp>` branch, writes patched files, commits, merges back, deletes the branch. On failure the branch is left intact for manual inspection

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Pre-flight failure (dirty tree, vLLM unreachable) or git error |
| `2` | Nothing to do (no files found or all already documented) |

## Files

```
docgen/
├── docgen.py          # CLI, parsing, LLM calls, git workflow
└── tests/
    └── test_docgen.py # 53 unit tests
pyproject.toml         # uv entry point: docgen = "docgen.docgen:app"
package.json           # pnpm scripts wrapper
.env.example           # VLLM_BASE_URL, BATCH_SIZE
```
