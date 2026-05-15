# code-review

GitHub Composite Action that reviews a Pull Request diff using a self-hosted [vLLM](https://github.com/vllm-project/vllm) instance and posts a structured Markdown comment on the PR.

The action downloads a pre-compiled **Rust binary** from GitHub Releases and executes it directly — no `setup-python`, no `pip install`, startup time is ~1 second plus inference.

---

## Prerequisites

| Requirement        | Details                                                                                    |
| ------------------ | ------------------------------------------------------------------------------------------ |
| Self-hosted runner | Must have network access to your vLLM instance; architecture `x86_64` or `aarch64` (Linux) |
| vLLM server        | Running and reachable from the runner; endpoint exposed as `VLLM_URL` secret               |
| Repository secret  | `VLLM_URL` — your vLLM base URL (e.g. `http://192.168.1.10:30000/v1`)                      |
| Runner label       | The job must target your self-hosted runner via `runs-on: self-hosted` (or a custom label) |

---

## Workflow

Create `.github/workflows/ai-code-review.yml` in your project:

```yaml
name: AI Code Review on PR

on:
  pull_request:
    types: [opened, synchronize, reopened, labeled]
    paths:
      - "backend/**"
      - "frontend/**"
      - ".github/workflows/**"

permissions:
  pull-requests: write
  contents: read

jobs:
  # Guardrail: explain clearly when review is skipped (external/fork PRs)
  ai-review-skipped:
    name: AI Review Guardrails
    runs-on: ubuntu-latest
    if: >-
      ${{
        !contains(fromJson('["OWNER","MEMBER","COLLABORATOR"]'), github.event.pull_request.author_association)
        && !contains(github.event.pull_request.labels.*.name, 'safe-to-test')
      }}
    steps:
      - name: Explain why AI review is skipped
        run: |
          echo "AI review skipped: external contributor and 'safe-to-test' label not set."
          echo "A maintainer must add the 'safe-to-test' label to trigger the review."

  # Main review job: runs on your self-hosted runner with vLLM access
  ai-review:
    name: AI Code Review
    runs-on: self-hosted
    timeout-minutes: 10
    if: >-
      ${{
        contains(fromJson('["OWNER","MEMBER","COLLABORATOR"]'), github.event.pull_request.author_association)
        || contains(github.event.pull_request.labels.*.name, 'safe-to-test')
      }}
    steps:
      - name: Checkout code
        uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: AI Code Review
        uses: WillIsback/code-review@main
        with:
          vllm-url: ${{ secrets.VLLM_URL }}
          github-token: ${{ secrets.GITHUB_TOKEN }}
          vllm-model: ${{ vars.VLLM_MODEL || '' }}
          vllm-timeout: ${{ vars.VLLM_TIMEOUT || '120' }}
          vllm-retries: ${{ vars.VLLM_RETRIES || '2' }}
```

### Security guardrail explained

The two-job pattern prevents fork PRs from accessing your runner or secrets:

- **Internal contributors** (`OWNER`, `MEMBER`, `COLLABORATOR`) — review runs automatically
- **External contributors / forks** — review is skipped until a maintainer adds the `safe-to-test` label to the PR

> **Always pin to a commit SHA in production** instead of `@main` to prevent supply-chain attacks:
>
> ```yaml
> uses: WillIsback/code-review@<full-sha> # e.g. afa4841a12e85276e932982f0997b4480d3d9cfb
> ```

---

## Inputs

| Input          | Required | Default | Description                                                   |
| -------------- | -------- | ------- | ------------------------------------------------------------- |
| `vllm-url`     | yes      | —       | Base URL of the vLLM server (e.g. `http://<host>:30000/v1`)   |
| `github-token` | yes      | —       | Token for fetching the PR diff and posting the review comment |
| `vllm-model`   | no       | `""`    | Model ID override — auto-detected from `/v1/models` if empty  |
| `vllm-timeout` | no       | `120`   | Total request timeout in seconds                              |
| `vllm-retries` | no       | `2`     | Number of retries on LLM request failure                      |

Store `vllm-model`, `vllm-timeout`, and `vllm-retries` as **repository variables** (`vars.*`) so they can be tuned without editing the workflow file.

---

## How it works

1. **Download binary** — fetches the pre-compiled `code-review-cli` for `amd64` or `arm64` from GitHub Releases, verifies the SHA-256 checksum, and makes it executable
2. **Fetch PR diff** — retrieves all changed files via the GitHub REST API with automatic Link-header pagination; skips lock files, dotfiles, and Markdown
3. **Model detection** — uses `VLLM_MODEL` if set; otherwise queries `GET /v1/models` to auto-detect the loaded model
4. **Review strategy** (chosen automatically based on diff size):
   - **Single-round** (1 file changed) — sends the full diff in one `chat/completions` call, returns a structured Markdown report
   - **Two-round** (multiple files) — breaks the diff into ~2000-word chunks, reviews each in parallel bullet format, then feeds all findings into a reasoning summarisation call to produce the final report
5. **Post comment** — posts the structured Markdown report as a PR comment (truncated at 60 000 characters to respect GitHub's limit)

→ [Full documentation](docs/code-review.md)

---

## Local development

```bash
cp .env.example .env
# Fill in VLLM_BASE_URL and GITHUB_TOKEN in .env
cargo build --release
```
