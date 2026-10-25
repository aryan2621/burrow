# Burrow

Your private AI team, running on your machine. Spec: `../product-specs/burrow.md`. Phases 1–2 are done: skeleton and safety core.

## Run

```sh
brew install ollama && ollama serve &
ollama pull qwen3:8b            # any tool-calling model
cd apps/desktop && pnpm i && pnpm tauri dev
```

Sandbox runtime (for the `shell` tool), picked automatically in this order:
Apple `container` (macOS 26, one VM per agent: `brew install container && container system start`),
then Podman, then Docker (Linux/Windows). Override with `BURROW_SANDBOX=docker`, image with `BURROW_SANDBOX_IMAGE`.
The default image is `python:3.12-slim` until the base image is published; for uv and Node build
`sandbox-images/base` (`container build -t burrow-sandbox-base sandbox-images/base`) and set `BURROW_SANDBOX_IMAGE=burrow-sandbox-base`.

The app starts `burrowd` if it isn't already running. The daemon keeps running after the window closes.

- Data: `~/.burrow/` (override with `BURROW_HOME`): `burrow.db`, `snapshots/` (undo), `scratch/<agent>/`.
- IPC: Unix socket `~/.burrow/burrowd.sock` (0600) on macOS/Linux, named pipe `\\.\pipe\burrowd-<user>` on Windows.
- Built-in models (llama.cpp): `scripts/fetch-llama.sh` installs a pinned, checksummed llama-server into `~/.burrow/llama` (the app bundles it). `hardware.detect` recommends a model by RAM from `crates/burrowd/models.json` (Qwen3 4B / 8B / 30B-A3B); `models.download {id}` fetches it into `~/.burrow/models` with resume and a sha256 check. Use `llama:<id>` model refs: the daemon starts llama-server on a free loopback port (`--parallel 4`, prompt cache reused per slot) and restarts it when an agent needs another model. `BURROW_LLAMA_URL` points at your own server instead.
- Test: `cargo test -p burrowd`; with a sandbox runtime installed also `cargo test -p burrowd -- --ignored sandbox` (`sandbox_mcp_stdio` needs the base image).

## Daemon API (ND-JSON-RPC 2.0 over the socket)

`status`, `hardware.detect`, `models.list`, `models.pull {name}` (Ollama), `models.catalog`, `models.download {id}`, `models.cancel {id}`, `agents.list`, `agents.create {name, role_prompt, model_ref, folders, allowed_domains?, approval_rules?}`, `agents.update {agent_id, allowed_domains, approval_rules}`, `agents.kill {agent_id}`, `agents.delete {agent_id}`, `chat.history {agent_id}`, `chat.send {agent_id, text}`, `chat.busy`, `actions.list {agent_id}`, `actions.undo {action_id, force?}`, `runs.undo {run_id, force?}`, `journal.verify`, `model_calls.list {agent_id}`, `secrets.set {name, value?, hosts, agent_ids, header?}`, `secrets.list`, `secrets.delete {name}`, `search.configure {brave_key?, searxng_url?}` (null clears), `domains.presets`, `mcp.list {agent_id}`, `mcp.add {agent_id, name, transport: stdio|http, command?|url?, secrets?}`, `mcp.remove {agent_id, name}`, `approvals.pending`, `approvals.decide {approval_id, allow, remember?}`, `events.subscribe`.
After `events.subscribe` a connection receives `chat.delta`, `chat.tool`, `chat.done`, `approval.requested`, `models.progress`, `models.downloaded` and `models.download_failed`.

## Safety model

- **Tools:** `list_dir`, `read_file`, `write_file`, `delete_file` (host, scoped to the agent's folders + scratch), `shell` (sandbox), `http_fetch` (daemon, GET only), `notes_add`/`notes_search` (FTS5, per agent), `web_search` (Brave or SearXNG; hidden when neither is set), and MCP tools as `mcp__<server>__<tool>`.
- **Sandbox:** one container per agent, 2 CPU / 2 GB, the agent's folders mounted **read-only**, scratch read-write. Host files only change through journaled file tools.
- **Network:** each sandbox sits alone on an `--internal` network; its only way out is the daemon's HTTP/CONNECT proxy (per-agent token, ports 80/443). Docker/Podman add a small forwarder container. Same domain allowlist as `http_fetch`; new domains ask during a run. DNS is resolved by the daemon and private/loopback addresses are refused (also for `http_fetch`), unless allowlisted verbatim. No TLS interception. Every connection is journaled as `net`. Fetched and searched content is wrapped in `<untrusted>`.
- **MCP:** stdio servers run inside the agent's sandbox and install through the proxy; HTTP servers are called by the daemon on allowlisted hosts with headers from bound secrets. Default rule is ask, or allow when the tool declares `readOnlyHint`.
- **Journal:** every tool call (including denied ones) in an append-only, hash-chained `actions` table.
- **Undo:** file writes/deletes snapshot first; undo refuses if the file changed since (unless forced). Shell/fetch are marked irreversible.
- **Approvals:** default allow for list/read/write (undoable), ask for delete, shell and new domains. "Always allow" saves a rule or adds the domain.
- **Kill switch:** stops the run, denies its pending approvals, removes its sandbox.

Not yet: owner-only DACL on the Windows pipe, rootless Podman networking on Linux, secret injection into sandbox traffic (needs TLS interception, v0.2).
