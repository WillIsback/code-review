# Reaching a private inference endpoint

The action runs on **ephemeral GitHub-hosted runners**, which have no access to
your LAN. If your OpenAI-compatible endpoint (vLLM, Ollama, LM Studio, a gateway…)
is private, expose it over HTTPS before the action can use it.

The action itself only needs:

| Input | Purpose |
| --- | --- |
| `vllm-url` | HTTPS base URL of the endpoint (e.g. `https://vllm.example.com/v1`) |
| `vllm-api-key` | Bearer key (`Authorization: Bearer …`) |
| `extra-headers` | Any additional auth headers, one `Name: Value` per line |

How you publish the endpoint is up to you. Below are three common options.

---

## Option A — Cloudflare Tunnel + Access (no inbound port)

1. **Zero Trust → Networks → Tunnels → Create a tunnel → Cloudflared**, name it.
2. Add a **Public hostname**: subdomain `vllm`, domain `example.com`, service
   `http://<host>:30000`. Copy the tunnel **token**.
3. Run the connector:

   ```yaml
   # docker-compose.yml
   services:
     cloudflared:
       image: cloudflare/cloudflared:latest
       container_name: cloudflared
       command: tunnel --no-autoupdate run
       environment:
         TUNNEL_TOKEN: "${CLOUDFLARE_TUNNEL_TOKEN}"
       restart: unless-stopped
   ```

   ```bash
   echo 'CLOUDFLARE_TUNNEL_TOKEN=...' >> .env   # gitignored
   docker compose up -d
   docker logs cloudflared --tail 20            # "Registered tunnel connection"
   ```

4. **Zero Trust → Access → Service Auth → Service Tokens → Create**. Copy the
   **Client ID** / **Client Secret** (shown once).
5. **Access → Applications → Add an application → Self-hosted** with domain
   `vllm.example.com` and a policy **Action = Service Auth** including that token.
6. Pass the token headers to the action:

   ```yaml
   extra-headers: |
     CF-Access-Client-Id: ${{ secrets.CF_ACCESS_CLIENT_ID }}
     CF-Access-Client-Secret: ${{ secrets.CF_ACCESS_CLIENT_SECRET }}
   ```

---

## Option B — Tailscale Funnel

If the machine already runs Tailscale:

```bash
tailscale funnel --bg 30000
```

Point `vllm-url` at `https://<node>.<tailnet>.ts.net/v1`. Funnel has no Access
layer — rely on the vLLM `--api-key`, or add a CNAME + Access elsewhere.

---

## Option C — HTTPS reverse proxy

Publish the endpoint behind nginx/Caddy/Traefik with TLS (and your existing
`--api-key`). Simplest when you already run a reverse proxy with a public
hostname.

---

## Configure the action

```bash
gh secret set VLLM_URL                --repo <owner>/<repo> --body "https://vllm.example.com/v1"
gh secret set VLLM_API_KEY            --repo <owner>/<repo>   # the endpoint's bearer key
# Only if the endpoint needs extra headers (e.g. Cloudflare Access):
gh secret set CF_ACCESS_CLIENT_ID     --repo <owner>/<repo>
gh secret set CF_ACCESS_CLIENT_SECRET --repo <owner>/<repo>
```

Optional repository variables: `VLLM_MODEL`, `VLLM_TIMEOUT`, `VLLM_RETRIES`.

---

## Verify

```bash
D=https://vllm.example.com; KEY=<VLLM_API_KEY>

# With the API key -> 200
curl -s -o /dev/null -w "%{http_code}\n" "$D/v1/models" \
  -H "Authorization: Bearer $KEY" \
  -H "CF-Access-Client-Id: <ID>" -H "CF-Access-Client-Secret: <SECRET>"  # if using Access
```

Expected `200` when the endpoint is reachable and authenticated; add or drop the
Access headers to confirm each layer blocks correctly (`403` without them, `401`
without the key).

---

## Notes

- **Binary source** — `action.yml` downloads the CLI from `releases/latest`.
  Pinning the action to a commit SHA freezes the *action logic* but **not** the
  binary; cut a tagged release for each change you want live.
- **Token rotation** — rotate any gateway/access token periodically and update
  the GitHub secret.
- **Cost / abuse** — add a rate-limiting rule at your edge if the endpoint is
  internet-facing.
- **Local DNS** — if a LAN machine cannot resolve the hostname while public
  resolvers can, its resolver cache is stale; flush it (`resolvectl flush-caches`)
  or the router's DNS. GitHub-hosted runners resolve normally.
