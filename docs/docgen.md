# docgen

CLI tool that generates and inserts docstrings into TypeScript and Python source files using a locally-hosted [vLLM](https://github.com/vllm-project/vllm) instance. Designed for use during development or at build time.

The engine is written in **Rust**: source files are parsed with [tree-sitter](https://tree-sitter.github.io/) AST queries (no regex), LLM calls run in parallel with a tokio semaphore, and all file edits are isolated in a short-lived `docgen/<timestamp>` git branch that is merged back and deleted — keeping your working branch clean.

---

## Installation

### npm / pnpm project

On `npm install` / `pnpm install` a postinstall script downloads the pre-compiled binary from GitHub Releases for your platform automatically.

```bash
pnpm add -D WillIsback/ai-devops-toolkit
# or
npm install --save-dev WillIsback/ai-devops-toolkit
```

Then run via `pnpm exec` / `npx`:

```bash
pnpm exec docgen src/
npx docgen src/
```

Or add a convenience script to your `package.json`:

```json
{
  "scripts": {
    "docgen": "docgen"
  }
}
```

```bash
pnpm run docgen -- src/
```

---

## Configuration

`docgen` loads `.env` files in two layers (project overrides global):

### Global config

Create `~/.config/docgen/.env` once — applies to every project on the machine:

```bash
mkdir -p ~/.config/docgen
cat > ~/.config/docgen/.env <<EOF
VLLM_BASE_URL=http://<your-host>:30000/v1
BATCH_SIZE=4
EOF
```

### Project-level config

Create a `.env` at the root of any project to override global values:

```
VLLM_BASE_URL=http://<your-host>:30000/v1
BATCH_SIZE=4
```

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `VLLM_BASE_URL` | `http://localhost:30000/v1` | vLLM server base URL |
| `BATCH_SIZE` | `4` | Max concurrent LLM requests |
| `VLLM_MODEL` | _(auto)_ | Override model ID; auto-detected from `/v1/models` if unset |

---

## Usage

```bash
# Single file
pnpm exec docgen path/to/file.ts

# Flat folder (top-level files only)
pnpm exec docgen src/

# Recurse into subdirectories
pnpm exec docgen src/ --recursive

# Regenerate existing docstrings
pnpm exec docgen src/ --force

# Explicit format override
pnpm exec docgen src/ --format tsdoc
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
2. **Model detection** — uses `VLLM_MODEL` env var if set; otherwise queries `GET /v1/models`
3. **File resolution** — collects `.py` / `.ts` / `.tsx` files under the target (flat or recursive)
4. **AST parsing** — Python files are parsed with `tree-sitter-python`; TypeScript with `tree-sitter-typescript`. Files with all functions/classes documented are skipped (unless `--force`). Parsing runs in parallel via `rayon`.
5. **Batch LLM calls** — files are sent to vLLM concurrently, up to `BATCH_SIZE` in-flight requests via a `tokio` semaphore
6. **Git workflow** — creates a `docgen/<timestamp>` branch, writes patched files, commits, merges back into the original branch, deletes the feature branch. On failure the branch is left intact for manual inspection.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Pre-flight failure (dirty tree, vLLM unreachable) or git error |
| `2` | Nothing to do (no files found or all already documented) |

## Project structure

```
Cargo.toml                        # Workspace root
crates/
├── toolkit-core/                 # Shared: vLLM client, config, git, errors
└── docgen-cli/                   # docgen binary
    ├── src/
    │   ├── main.rs               # Orchestration
    │   ├── cli.rs                # Clap argument definitions
    │   ├── resolver.rs           # File discovery
    │   ├── detect.rs             # tree-sitter AST detection
    │   ├── process.rs            # Parallel LLM calls
    │   └── apply.rs              # Git branch/merge workflow
    └── Cargo.toml
package.json                      # bin entry + npm postinstall binary download
scripts/npm-postinstall.js        # Downloads platform binary from GitHub Releases
.env.example                      # VLLM_BASE_URL, BATCH_SIZE
```
