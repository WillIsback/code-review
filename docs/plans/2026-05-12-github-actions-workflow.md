# GitHub Actions Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two GitHub Actions workflows — `ci.yml` (PR checks + AI code review) and `release.yml` (cross-compiled binary release on merge to main).

**Architecture:** Two independent workflow files under `.github/workflows/`. `ci.yml` runs on every PR: build+test on `ubuntu-latest`, then AI code review on `[self-hosted, dgx-spark]`. `release.yml` triggers on push to `main`, cross-compiles `code-review-cli` for amd64 and arm64, and publishes a floating `latest` GitHub Release.

**Tech Stack:** GitHub Actions, `dtolnay/rust-toolchain@stable`, `cross` (Rust cross-compilation), `softprops/action-gh-release@v2`, self-hosted runner with label `dgx-spark`.

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `.github/workflows/ci.yml` | PR build, test, AI review |
| Create | `.github/workflows/release.yml` | Cross-compile + publish binaries on merge to main |

---

## Task 1: Create `ci.yml`

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the workflows directory and `ci.yml`**

```yaml
# .github/workflows/ci.yml
name: CI

on:
  pull_request:
    branches: [main]
    types: [opened, synchronize, reopened]

jobs:
  build-and-test:
    name: Build & Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Build workspace
        run: cargo build --workspace

      - name: Test workspace
        run: cargo test --workspace

  ai-code-review:
    name: AI Code Review
    runs-on: [self-hosted, dgx-spark]
    needs: build-and-test
    steps:
      - uses: actions/checkout@v4

      - name: Run AI Code Review
        uses: ./code-review
        with:
          vllm-url: ${{ secrets.VLLM_URL }}
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

- [ ] **Step 2: Validate YAML syntax**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo "YAML valid"
```

Expected: `YAML valid`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "feat(ci): add PR build, test, and AI code review workflow"
```

---

## Task 2: Create `release.yml`

**Files:**
- Create: `.github/workflows/release.yml`

**Context:** `action.yml` downloads binaries named exactly `code-review-cli-linux-amd64` and `code-review-cli-linux-arm64` from `releases/latest/download/`. The matrix maps these platform names to Rust target triples. Both matrix jobs upload to the same floating `latest` release via `softprops/action-gh-release@v2`.

- [ ] **Step 1: Create `release.yml`**

```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    branches: [main]

jobs:
  release:
    name: Build & Release (${{ matrix.platform }})
    runs-on: ubuntu-latest
    strategy:
      matrix:
        include:
          - platform: linux-amd64
            target: x86_64-unknown-linux-gnu
          - platform: linux-arm64
            target: aarch64-unknown-linux-gnu

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable with cross-compilation target
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install cross
        run: cargo install cross --git https://github.com/cross-rs/cross

      - name: Build code-review-cli
        run: cross build --release --bin code-review-cli --target ${{ matrix.target }}

      - name: Rename binary
        run: cp target/${{ matrix.target }}/release/code-review-cli code-review-cli-${{ matrix.platform }}

      - name: Publish to GitHub Releases (latest)
        uses: softprops/action-gh-release@v2
        with:
          tag_name: latest
          name: Latest Release
          files: code-review-cli-${{ matrix.platform }}
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

- [ ] **Step 2: Validate YAML syntax**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))" && echo "YAML valid"
```

Expected: `YAML valid`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "feat(ci): add cross-compile release workflow for merge to main"
```

---

## Task 3: Push branch and open PR

**Files:** none — git operations only

- [ ] **Step 1: Verify both workflow files are committed**

```bash
git log --oneline -4
```

Expected: see both `feat(ci):` commits in the log.

- [ ] **Step 2: Push the feature branch**

```bash
git push -u origin feat/github-action-worklow
```

- [ ] **Step 3: Open a PR via GitHub CLI**

```bash
gh pr create \
  --title "feat(ci): add GitHub Actions CI and release workflows" \
  --body "$(cat <<'EOF'
## Summary

- `ci.yml`: build + test on every PR, then AI code review via self-hosted runner (dogfooding `./code-review`)
- `release.yml`: cross-compile `code-review-cli` for amd64 + arm64 and publish a floating `latest` GitHub Release on every merge to `main`

## Prerequisites before merging

- [ ] Add repo secret `VLLM_URL` in Settings → Secrets → Actions
- [ ] Register DGX Spark as self-hosted runner with label `dgx-spark` (see `docs/specs/2026-05-12-github-actions-workflow-design.md`)

## Test plan

- [ ] Confirm `CI / Build & Test` job passes on this PR
- [ ] Confirm `CI / AI Code Review` job runs on the self-hosted runner and posts a review comment
- [ ] After merge, confirm `Release` workflow creates/updates a `latest` GitHub Release with both binaries

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Verify the PR is open and CI triggered**

```bash
gh pr view --web
```

Expected: browser opens to the PR; `CI` workflow should appear under the Checks tab within ~30 seconds.

---

## Post-merge verification

After the PR is merged to `main`:

1. Go to **Actions → Release** — confirm both matrix jobs complete
2. Go to **Releases** — confirm a `latest` release exists with two assets:
   - `code-review-cli-linux-amd64`
   - `code-review-cli-linux-arm64`
3. The `code-review` action will now be fully functional for other repos

---

## Secrets setup reminder

If `VLLM_URL` is not yet set, the `ai-code-review` job will fail. Set it at:

```
https://github.com/WillIsback/ai-devops-toolkit/settings/secrets/actions
```

Name: `VLLM_URL`  
Value: your vLLM base URL (e.g. `http://192.168.x.x:30000/v1`)
