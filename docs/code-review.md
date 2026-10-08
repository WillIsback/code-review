# code-review

GitHub Composite Action that runs on **GitHub-hosted runners**, reviews **Pull Requests and Issues**, and posts a structured Markdown comment. It reaches a private self-hosted [vLLM](https://github.com/vllm-project/vllm) instance through a **Cloudflare Tunnel** (no inbound port).

The action downloads a pre-compiled **Rust binary** from GitHub Releases and executes it directly — no `setup-python`, no `pip install`, startup time is ~1 second plus inference.

---

## Usage

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

## Prerequisites

| Requirement         | Details                                                                                   |
| ------------------- | ----------------------------------------------------------------------------------------- |
| vLLM server         | Running with `--api-key`; reachable from the internet via a tunnel                        |
| Cloudflare Tunnel   | `cloudflared` publishes e.g. `vllm.example.com` (outbound only — no inbound port)         |
| Cloudflare Access   | A **Service Auth** policy on the hostname; provides a client id + secret                  |
| Repository secrets  | `VLLM_URL`, `VLLM_API_KEY`, `CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`              |
| Repository variable | `VLLM_MODEL` (optional; skips `/v1/models` auto-detection)                                |

## How it works

1. **Download binary** — the action fetches `code-review-cli-linux-amd64` (or `arm64`) from GitHub Releases, verifies the SHA-256 checksum, and makes it executable
2. **Fetch context** — for PRs, fetches all changed files via the GitHub REST API, following Link-header pagination with no fixed page cap; for issues, fetches the issue title, body, and comments instead of a diff
3. **Model detection** — uses `VLLM_MODEL` input if set; otherwise queries `GET /v1/models` to auto-detect the loaded model
4. **Review strategy** (chosen automatically based on diff size):
   - **Single-round** (1 file changed) — the full diff is sent in one `chat/completions` call
   - **Two-round** (multiple files) — the diff is split by file into ~2000-word chunks and reviewed **sequentially** (bullet findings); a **verification** pass then checks each finding against the actual source files (fetched from the repo at the PR head SHA via the GitHub Contents API) to confirm, downgrade, or filter it. If verification cannot run, it falls back to a summarization pass.
5. **Post comment** — aggregates the findings into a single Markdown comment and posts it on the PR (truncated safely at 60 000 characters)

## Issue triage

Set `target-type: issue` to triage an issue instead of a PR. The action fetches
the issue title, body, and comments and posts a structured Markdown comment
with these sections:

- **Summary** — a short restatement of the issue
- **Missing Information** — what the issue needs to be actionable
- **Possible Duplicates** — related or duplicate issues
- **Suggested Labels** — labels a maintainer might apply
- **Suggested Next Steps** — how to proceed

## Supported architectures

| Runner arch | Asset downloaded |
|---|---|
| `x86_64` | `code-review-cli-linux-amd64` |
| `aarch64` | `code-review-cli-linux-arm64` |

## Project structure

```
Cargo.toml                        # Workspace
code-review/
└── action.yml                    # Composite Action definition
crates/code-review-cli/src/
├── main.rs                       # Orchestration
├── config.rs                     # .env loading
├── github.rs                     # GitHub API (diff, issue, comment)
├── source.rs                     # Source-file context fetching
├── review.rs                     # Chunking, review strategy, comment posting
├── vllm.rs                       # vLLM client
└── error.rs                      # Error types
```
