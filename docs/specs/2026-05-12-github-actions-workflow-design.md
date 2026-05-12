# GitHub Actions Workflow Design

**Date:** 2026-05-12  
**Branch:** `feat/github-action-worklow`  
**Status:** Approved

---

## Goal

Add a minimal, extensible GitHub Actions CI/CD pipeline to `ai-devops-toolkit` that:
1. Validates every PR with a build+test check and an AI-powered code review
2. Publishes cross-compiled release binaries on every merge to `main`
3. Dogfoods the `code-review` action — the pipeline uses the tool on itself

---

## Architecture

Two workflow files, each with a single trigger and clear responsibility:

```
.github/workflows/
  ci.yml       ← triggered on pull_request: build, test, AI review
  release.yml  ← triggered on push to main: cross-compile, publish release
```

No reusable workflows yet — premature until a third caller exists.

---

## `ci.yml` — Pull Request Checks

**Trigger:** `pull_request` (opened, synchronize, reopened) targeting `main`

### Job: `build-and-test`

- **Runner:** `ubuntu-latest`
- **Steps:**
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable`
  3. `cargo build --workspace`
  4. `cargo test --workspace`

### Job: `ai-code-review`

- **Runner:** `[self-hosted, dgx-spark]` — needs local network access to vLLM
- **Needs:** `build-and-test` — only runs if the build passes (avoids wasting LLM tokens on broken code)
- **Steps:**
  1. `actions/checkout@v4`
  2. `uses: ./code-review` with:
     - `vllm-url: ${{ secrets.VLLM_URL }}`
     - `github-token: ${{ secrets.GITHUB_TOKEN }}`

> Uses `./code-review` (local path) so it tests the version being merged — true dogfooding.

---

## `release.yml` — Release on Merge to Main

**Trigger:** `push` to `main`

### Job: `release`

- **Runner:** `ubuntu-latest`
- **Strategy:** matrix over two targets: `linux-amd64`, `linux-arm64`
- **Steps:**
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable` with cross-compilation targets
  3. Install `cross` CLI (Rust cross-compilation via Docker)
  4. `cross build --release --bin code-review-cli --target <matrix.target>`
  5. Rename binary to `code-review-cli-<matrix.platform>` (matches `action.yml` download logic)
  6. `softprops/action-gh-release@v2` — create/update floating `latest` tag, upload both binaries

> Floating `latest` tag matches what `action.yml` already expects at `releases/latest/download/...`.

---

## Secrets

| Secret | Source | Used by |
|---|---|---|
| `VLLM_URL` | Repo settings → Secrets | `ai-code-review` job |
| `GITHUB_TOKEN` | Built-in (automatic) | `ai-code-review` + `release` jobs |

No new secrets to configure beyond `VLLM_URL`.

---

## Runner Setup

The `ai-code-review` job targets `[self-hosted, dgx-spark]`. Setup:

```bash
mkdir ~/actions-runner-ai-devops-toolkit && cd ~/actions-runner-ai-devops-toolkit
# Download runner binaries (same version as other registered runners)
./config.sh --url https://github.com/WillIsback/ai-devops-toolkit --token <TOKEN> --labels dgx-spark
# Install as systemd service
sudo ./svc.sh install && sudo ./svc.sh start
```

One physical machine can host multiple runner processes — one per repo, each in its own directory.

---

## Future Extension Points

- Add `docgen.yml` to auto-generate docstrings on PR (once docgen is stable in CI)
- Add `security.yml` for security scanning after security-related changes
- Promote floating `latest` to semver tags when versioning strategy is decided
- Add `cargo clippy` and `cargo fmt --check` to `build-and-test` job
- Add build caching (`actions/cache` for `~/.cargo`) to speed up CI

---

## File Checklist

- [ ] `.github/workflows/ci.yml`
- [ ] `.github/workflows/release.yml`
- [ ] Repo secret `VLLM_URL` set in GitHub settings
- [ ] Self-hosted runner registered with label `dgx-spark`
