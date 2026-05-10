# ai-devops-toolkit

Centralized GitHub Composite Actions for WillIsback projects.

---

## Actions

### `code-review`

Reviews a Pull Request diff using a self-hosted [vLLM](https://github.com/vllm-project/vllm) instance and posts a structured Markdown comment on the PR.

#### Usage

```yaml
- name: Checkout code
  uses: actions/checkout@v4
  with:
    fetch-depth: 0

- name: AI Code Review
  uses: WillIsback/ai-devops-toolkit/code-review@main
  with:
    vllm-url: ${{ secrets.VLLM_URL }}
    github-token: ${{ secrets.GITHUB_TOKEN }}
```

#### Inputs

| Input | Required | Default | Description |
|---|---|---|---|
| `vllm-url` | yes | — | Base URL of the vLLM server |
| `github-token` | yes | — | GitHub token (use `secrets.GITHUB_TOKEN`) |
| `vllm-model` | no | `""` | Override model ID (auto-detected if empty) |
| `vllm-timeout` | no | `120` | Total request timeout (seconds) |
| `vllm-retries` | no | `2` | Retries on LLM request failure |

#### Prerequisites

- A self-hosted GitHub Actions runner with network access to your vLLM instance
- Repository/organization secret `VLLM_URL` (or equivalent) set to your vLLM endpoint

#### Model detection

The action auto-detects the loaded model by querying `/v1/models`. Set `vllm-model` only to override.

#### Qwen reasoning mode

For Qwen reasoning models, the action requests non-thinking mode per request so the final answer is returned directly in `message.content`:

- `reasoning_effort: "none"`
- `extra_body: {"chat_template_kwargs": {"enable_thinking": false}}`

This avoids responses that only contain reasoning traces without final review content.
