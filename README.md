# code-review

GitHub Composite Action that runs on **GitHub-hosted runners**, reviews **Pull Requests and Issues**, and posts a structured Markdown comment. It talks to any **OpenAI-compatible** endpoint (e.g. a self-hosted [vLLM](https://github.com/vllm-project/vllm)), which you expose to the runners yourself.

The action downloads a pre-compiled **Rust binary** from GitHub Releases and executes it directly — no `setup-python`, no `pip install`, startup time is ~1 second plus inference.

---

## Prerequisites

| Requirement        | Details                                                                        |
| ------------------ | ------------------------------------------------------------------------------ |
| Endpoint           | Any OpenAI-compatible, HTTPS-reachable `…/v1` endpoint (self-hosted or hosted) |
| Authentication     | Optional bearer key (`vllm-api-key`) and/or extra headers (e.g. gateway tokens) |
| Repository secrets | `VLLM_URL`, `VLLM_API_KEY` (+ any header secrets your endpoint needs)          |
| Repository vars    | `VLLM_MODEL`, `VLLM_TIMEOUT`, `VLLM_RETRIES`, `VLLM_VERIFY_MAX_TOKENS` (optional; consumed via `vars.*`) |

The endpoint must be reachable from GitHub-hosted runners. If it is private,
publish it first — see [**Reaching a private inference endpoint**](docs/recipes/expose-private-endpoint.md)
(Cloudflare Tunnel, Tailscale Funnel, reverse proxy).

---

## Workflow

Copy the examples from [`examples/`](examples/):

- `code-review-gate.yml` — untrusted `pull_request` gate (no secrets)
- `code-review-pr.yml` — privileged `workflow_run` PR review
- `code-review-issue.yml` — issue triage on `issues: [opened]`

The gate exists because `workflow_run` only fires when a named workflow
completes. Repos that already run CI on PRs may instead point
`workflows: ["CI"]` at their existing workflow and skip the gate file.

> **Always pin to a full commit SHA in production** instead of a branch:
>
> ```yaml
> uses: WillIsback/code-review@<full-sha>
> ```
>
> The CLI binary is fetched from `releases/latest`, so a SHA pin freezes the
> action logic but **not** the binary — cut a tagged release for each change you
> want live. See the [endpoint guide](docs/recipes/expose-private-endpoint.md#notes).

### Security model

- Reviews run on **ephemeral GitHub-hosted runners** — no LAN access, no persistence.
- Fork PRs never receive secrets: the privileged review runs via `workflow_run`,
  whose definition comes from the default branch and cannot be modified by the PR.
- The action never checks out or executes PR code; the diff and source files are
  fetched through the GitHub API.
- The endpoint is protected by a bearer key and/or extra gateway headers
  (`extra-headers`), e.g. a Cloudflare Access service token.
- Issue triage is gated on `author_association` to prevent inference spam.

---

## Inputs

| Input                     | Required | Default | Description                                                     |
| ------------------------- | -------- | ------- | --------------------------------------------------------------- |
| `vllm-url`                | yes      | —       | OpenAI-compatible base URL (e.g. `https://vllm.example.com/v1`)  |
| `github-token`            | yes      | —       | Token for fetching the diff/issue and posting the comment       |
| `target-type`             | no       | `pr`    | `pr` or `issue`                                                 |
| `target-number`           | no       | `""`    | PR/issue number; falls back to the event's number               |
| `head-sha`                | no       | `""`    | PR head SHA (source context / PR-number fallback)               |
| `vllm-model`              | no       | `""`    | Model override — auto-detected if empty                         |
| `vllm-timeout`            | no       | `120`   | Total request timeout in seconds                                |
| `vllm-retries`            | no       | `2`     | Number of retries on LLM request failure                        |
| `vllm-api-key`            | no       | `""`    | Bearer key (`Authorization: Bearer …`)                          |
| `extra-headers`           | no       | `""`    | Extra headers, one `Name: Value` per line (e.g. gateway tokens) |

Store `vllm-model`, `vllm-timeout`, and `vllm-retries` as **repository variables** (`vars.*`) so they can be tuned without editing the workflow file.

### Endpoint examples

Any provider exposing OpenAI-style `POST /v1/chat/completions` and `GET /v1/models` works:

| Provider | `vllm-url` |
| --- | --- |
| Self-hosted vLLM (tunnel/proxy) | `https://vllm.example.com/v1` |
| OpenAI | `https://api.openai.com/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` |
| Groq | `https://api.groq.com/openai/v1` |
| Together | `https://api.together.xyz/v1` |
| Ollama | `http://<host>:11434/v1` |

> Azure OpenAI uses a different path and an `api-key` header — front it with a
> gateway that exposes OpenAI-style `/v1` if you need it.

---

## How it works

1. **Download binary** — fetches the pre-compiled `code-review-cli` for `amd64` or `arm64` from GitHub Releases, verifies the SHA-256 checksum, and makes it executable
2. **Fetch context** — for PRs, retrieves all changed files via the GitHub REST API with automatic Link-header pagination; skips lock files, dotfiles, and `.md`/`.mdx` documentation files. For issues, fetches the issue title, body, and comments instead of a diff.
3. **Model detection** — uses `VLLM_MODEL` if set; otherwise queries `GET /v1/models` to auto-detect the loaded model
4. **Review strategy** (chosen automatically based on diff size):
   - **Single-round** (1 file changed) — the full diff is sent in one `chat/completions` call, returning a structured Markdown report
   - **Two-round** (multiple files) — the diff is split by file into ~2000-word chunks and reviewed **sequentially** (bullet findings); a **verification** pass then checks each finding against the actual source files (fetched from the repo at the PR head SHA via the GitHub Contents API) to confirm, downgrade, or filter it. If verification cannot run, it falls back to a summarization pass.
5. **Deterministic report cleanup** — the two-round output is post-processed locally (no extra LLM call): only the final report is kept (verification reasoning and YAML front-matter are stripped), exact-duplicate table rows are merged, dangling headings left by a truncated generation are trimmed, and the claimed `findings_total` is reconciled with the actual number of rows. When some modified files could not be fetched, a coverage caveat is prepended to the report.
6. **Post comment** — posts the structured Markdown report as a PR comment (truncated at 60 000 characters to respect GitHub's limit). The comment carries a hidden marker and is **updated in place** on subsequent runs instead of stacking a new comment per run.

→ [Full documentation](docs/code-review.md)

---

## Local development

```bash
cp .env.example .env
# Fill in GITHUB_REPOSITORY, GITHUB_TOKEN, and VLLM_BASE_URL in .env
cargo build --release
```
