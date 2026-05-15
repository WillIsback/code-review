# code-review

GitHub Composite Action that reviews a Pull Request diff using a self-hosted [vLLM](https://github.com/vllm-project/vllm) instance and posts a structured Markdown comment on the PR.

The action downloads a pre-compiled **Rust binary** from GitHub Releases and executes it directly — no `setup-python`, no `pip install`, startup time is ~1 second plus inference.

---

## Usage

```yaml
- name: Checkout code
  uses: actions/checkout@v4
  with:
    fetch-depth: 0

- name: AI Code Review
  uses: WillIsback/code-review@main
  with:
    vllm-url: ${{ secrets.VLLM_URL }}
    github-token: ${{ secrets.GITHUB_TOKEN }}
```

## Inputs

| Input | Required | Default | Description |
|---|---|---|---|
| `vllm-url` | yes | — | Base URL of the vLLM server (e.g. `http://<host>:30000/v1`) |
| `github-token` | yes | — | GitHub token for fetching the PR diff and posting the review comment |
| `vllm-model` | no | `""` | Override model ID — auto-detected from `/v1/models` if empty |
| `vllm-timeout` | no | `120` | Total request timeout in seconds |
| `vllm-retries` | no | `2` | Number of retries on LLM request failure |

## Prerequisites

- A self-hosted GitHub Actions runner with network access to your vLLM instance
- A repository or organisation secret `VLLM_URL` set to your vLLM endpoint
- Runner architecture must be `x86_64` or `aarch64` (Linux)

## How it works

1. **Download binary** — the action fetches `code-review-cli-linux-amd64` (or `arm64`) from GitHub Releases and makes it executable
2. **Fetch PR diff** — fetches all changed files via the GitHub REST API, with automatic pagination (up to 50 pages of 100 files each)
3. **Model detection** — uses `VLLM_MODEL` input if set; otherwise queries `GET /v1/models` to auto-detect the loaded model
4. **Chunked review** — splits large diffs into ~2000-word chunks and sends each to the vLLM chat completions endpoint
5. **Post comment** — aggregates chunk reviews into a single Markdown comment and posts it on the PR (truncated safely at 60 000 characters)

## Supported architectures

| Runner arch | Asset downloaded |
|---|---|
| `x86_64` | `code-review-cli-linux-amd64` |
| `aarch64` | `code-review-cli-linux-arm64` |

## Project structure

```
Cargo.toml                        # Single crate
code-review/
└── action.yml                    # Composite Action definition
src/
├── main.rs                       # Orchestration
├── github.rs                     # PR diff fetch via octocrab
├── review.rs                     # Chunking, vLLM calls, PR comment posting
├── config.rs                     # .env loading
├── llm.rs                        # vLLM client
├── git.rs                        # Git helpers
└── error.rs                      # Error types
```
