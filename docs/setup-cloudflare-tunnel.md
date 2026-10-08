# Remote vLLM via Cloudflare Tunnel + Access

How to expose a private vLLM instance (e.g. on your LAN at `http://192.168.1.87:30000`)
to GitHub-hosted runners over HTTPS — no self-hosted runner, no inbound port.

```
GitHub-hosted runner ──HTTPS──▶ Cloudflare ──▶ cloudflared ──▶ vLLM (LAN)
   (ephemeral)                    Access+WAF      (outbound)
```

Three conditions must hold for the action to reach vLLM:

1. vLLM is reachable from the machine running `cloudflared`.
2. vLLM is started with `--api-key`.
3. The public hostname is protected by a Cloudflare Access **Service Auth** policy.

---

## 1. Publish vLLM with a tunnel

1. Cloudflare dashboard → **Zero Trust → Networks → Tunnels → Create a tunnel** →
   **Cloudflared** → name it (e.g. `home-vllm`).
2. Add a **Public hostname**:
   - Subdomain `vllm`, domain `example.com`
   - **Service**: `http://<vllm-host>:30000`
3. Copy the tunnel **token**.

Run the connector with Docker:

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
echo 'CLOUDFLARE_TUNNEL_TOKEN=...' >> .env   # keep this file gitignored
docker compose up -d
docker logs cloudflared --tail 20            # expect "Registered tunnel connection"
```

---

## 2. Protect it with Cloudflare Access

1. **Zero Trust → Access → Service Auth → Service Tokens → Create**. Copy the
   **Client ID** and **Client Secret** (the secret is shown only once).
2. **Zero Trust → Access → Applications → Add an application → Self-hosted**:
   - Domain: `vllm.example.com`
   - Policy: **Action = Service Auth**, Include = the service token created above.

The action sends these headers on every vLLM request (`CF-Access-Client-Id`,
`CF-Access-Client-Secret`), alongside `Authorization: Bearer <VLLM_API_KEY>`.

---

## 3. Configure GitHub

Repository (or organisation) secrets:

```bash
gh secret set VLLM_URL                --repo <owner>/<repo> --body "https://vllm.example.com/v1"
gh secret set VLLM_API_KEY            --repo <owner>/<repo>   # the vLLM --api-key value
gh secret set CF_ACCESS_CLIENT_ID     --repo <owner>/<repo>
gh secret set CF_ACCESS_CLIENT_SECRET --repo <owner>/<repo>
```

Optional repository variables (consumed by the example workflows via `vars.*`):

```bash
gh variable set VLLM_MODEL   --repo <owner>/<repo> --body "qwen3.8-flash-next"
gh variable set VLLM_TIMEOUT --repo <owner>/<repo> --body "120"
gh variable set VLLM_RETRIES --repo <owner>/<repo> --body "2"
```

---

## 4. Verify

```bash
D=https://vllm.example.com
ID=<CF_ACCESS_CLIENT_ID>; SECRET=<CF_ACCESS_CLIENT_SECRET>; KEY=<VLLM_API_KEY>

# No Access headers -> blocked by Access (403)
curl -s -o /dev/null -w "%{http_code}\n" "$D/v1/models"

# Access + vLLM key -> 200
curl -s -o /dev/null -w "%{http_code}\n" "$D/v1/models" \
  -H "CF-Access-Client-Id: $ID" -H "CF-Access-Client-Secret: $SECRET" \
  -H "Authorization: Bearer $KEY"

# Access only -> 401 (vLLM rejects the missing key)
curl -s -o /dev/null -w "%{http_code}\n" "$D/v1/models" \
  -H "CF-Access-Client-Id: $ID" -H "CF-Access-Client-Secret: $SECRET"
```

Expected: `403`, then `200`, then `401`.

---

## Notes

- **Binary source** — `action.yml` downloads the CLI from `releases/latest`.
  Pinning the action to a commit SHA freezes the *action logic* but **not** the
  binary; cut a tagged release for each change you want live.
- **Token rotation** — rotate the Access service token periodically and update
  the GitHub secret.
- **Cost / abuse** — add a Cloudflare rate-limiting rule on the hostname if it is
  internet-facing.
- **Local DNS** — if a LAN machine cannot resolve the hostname while public
  resolvers can, its resolver cache is stale; flush it (`resolvectl flush-caches`)
  or the router's DNS. GitHub-hosted runners resolve normally.
