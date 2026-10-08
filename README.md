# code-review

GitHub Composite Action that runs on **GitHub-hosted runners**, reviews **Pull Requests and Issues**, and posts a structured Markdown comment. It reaches a private self-hosted [vLLM](https://github.com/vllm-project/vllm) instance through a **Cloudflare Tunnel** (no inbound port).

The action downloads a pre-compiled **Rust binary** from GitHub Releases and executes it directly — no `setup-python`, no `pip install`, startup time is ~1 second plus inference.

---

## Prerequisites

| Requirement         | Details                                                                                   |
| ------------------- | ----------------------------------------------------------------------------------------- |
| vLLM server         | Running with `--api-key`; reachable from the internet via a tunnel                        |
| Cloudflare Tunnel   | `cloudflared` publishes e.g. `vllm.example.com` (outbound only — no inbound port)         |
| Cloudflare Access   | A **Service Auth** policy on the hostname; provides a client id + secret                  |
| Repository secrets  | `VLLM_URL`, `VLLM_API_KEY`, `CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`              |
| Repository variable | `VLLM_MODEL`, `VLLM_TIMEOUT`, `VLLM_RETRIES` (optional; consumed via `vars.*`)            |

→ Full setup guide: [**Remote vLLM via Cloudflare Tunnel + Access**](docs/setup-cloudflare-tunnel.md).

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
> want live. See the [setup guide](docs/setup-cloudflare-tunnel.md#notes).

### Security model

- Reviews run on **ephemeral GitHub-hosted runners** — no LAN access, no persistence.
- Fork PRs never receive secrets: the privileged review runs via `workflow_run`,
  whose definition comes from the default branch and cannot be modified by the PR.
- The action never checks out or executes PR code; the diff and source files are
  fetched through the GitHub API.
- Two independent layers protect vLLM: Cloudflare Access (service token) + the
  vLLM `--api-key`.
- Issue triage is gated on `author_association` to prevent inference spam.

---

## Inputs

| Input                     | Required | Default | Description                                                     |
| ------------------------- | -------- | ------- | --------------------------------------------------------------- |
| `vllm-url`                | yes      | —       | Base URL of the vLLM server (e.g. `https://vllm.example.com/v1`) |
| `github-token`            | yes      | —       | Token for fetching the diff/issue and posting the comment       |
| `target-type`             | no       | `pr`    | `pr` or `issue`                                                 |
| `target-number`           | no       | `""`    | PR/issue number; falls back to the event's number               |
| `head-sha`                | no       | `""`    | PR head SHA (source context / PR-number fallback)               |
| `vllm-model`              | no       | `""`    | Model override — auto-detected if empty                         |
| `vllm-timeout`            | no       | `120`   | Total request timeout in seconds                                |
| `vllm-retries`            | no       | `2`     | Number of retries on LLM request failure                        |
| `vllm-api-key`            | no       | `""`    | vLLM bearer key                                                 |
| `cf-access-client-id`     | no       | `""`    | Cloudflare Access service token id                              |
| `cf-access-client-secret` | no       | `""`    | Cloudflare Access service token secret                          |

Store `vllm-model`, `vllm-timeout`, and `vllm-retries` as **repository variables** (`vars.*`) so they can be tuned without editing the workflow file.

---

## How it works

1. **Download binary** — fetches the pre-compiled `code-review-cli` for `amd64` or `arm64` from GitHub Releases, verifies the SHA-256 checksum, and makes it executable
2. **Fetch context** — for PRs, retrieves all changed files via the GitHub REST API with automatic Link-header pagination; skips lock files, dotfiles, and `.md`/`.mdx` documentation files. For issues, fetches the issue title, body, and comments instead of a diff.
3. **Model detection** — uses `VLLM_MODEL` if set; otherwise queries `GET /v1/models` to auto-detect the loaded model
4. **Review strategy** (chosen automatically based on diff size):
   - **Single-round** (1 file changed) — the full diff is sent in one `chat/completions` call, returning a structured Markdown report
   - **Two-round** (multiple files) — the diff is split by file into ~2000-word chunks and reviewed **sequentially** (bullet findings); a **verification** pass then checks each finding against the actual source files (fetched from the repo at the PR head SHA via the GitHub Contents API) to confirm, downgrade, or filter it. If verification cannot run, it falls back to a summarization pass.
5. **Post comment** — posts the structured Markdown report as a PR comment (truncated at 60 000 characters to respect GitHub's limit)

→ [Full documentation](docs/code-review.md)

---

## Local development

```bash
cp .env.example .env
# Fill in GITHUB_REPOSITORY, GITHUB_TOKEN, and VLLM_BASE_URL in .env
cargo build --release
```
