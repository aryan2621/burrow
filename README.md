# Burrow

Your private AI team, running on your machine. Spec: `../product-specs/burrow.md`. v0.1 is feature-complete; release signing waits on accounts. User docs: `docs/` (mdBook, `mdbook serve docs`). See also `SECURITY.md`, `PRIVACY.md`.

## Run

```sh
scripts/fetch-llama.sh          # built-in llama.cpp runtime (the release app bundles it)
cd apps/desktop && pnpm i && pnpm tauri dev   # then Settings → Models: download one, or add a cloud key
```

Sandbox runtime (for the `shell` tool), picked automatically in this order:
Apple `container` (macOS 26, one VM per agent: `brew install container && container system start`),
then Podman, then Docker (Linux/Windows). Override with `BURROW_SANDBOX=docker`, image with `BURROW_SANDBOX_IMAGE`.
Release builds default to `ghcr.io/<owner>/burrow-sandbox-base` (`BURROW_DEFAULT_IMAGE` at build time); dev builds use
`python:3.12-slim`. For uv and Node locally, build `sandbox-images/base` (`container build -t burrow-sandbox-base sandbox-images/base`) and set `BURROW_SANDBOX_IMAGE=burrow-sandbox-base`.

The app starts `burrowd` if it isn't already running. The daemon keeps running after the window closes.

- Data: `~/.burrow/` (override with `BURROW_HOME`): `burrow.db`, `snapshots/` (undo), `scratch/<agent>/`.
- IPC: Unix socket `~/.burrow/burrowd.sock` (0600) on macOS/Linux, named pipe `\\.\pipe\burrowd-<user>` on Windows.
- Built-in models (llama.cpp): the daemon downloads a pinned, checksummed llama-server into `~/.burrow/llama` with your first model (the release app bundles it; `scripts/fetch-llama.sh` does it by hand). `hardware.detect` recommends a model by RAM from `crates/burrowd/models.json` (13 models in 8 / 16 / 18 / 24 / 32 GB tiers; Settings shows the ones that fit); `models.download {id}` fetches it into `~/.burrow/models` with resume and a sha256 check. Use `llama:<id>` model refs: the daemon starts llama-server on a free loopback port (`--parallel 4`, prompt cache reused per slot) and restarts it when an agent needs another model. `BURROW_LLAMA_URL` points at your own server instead.
- CI/release: `.github/workflows/ci.yml` (3-OS daemon + desktop build), `release.yml` (on `v*` tag: sidecar `burrowd`, bundled llama.cpp, tauri-action publishes the release with installers for macOS, Linux and Windows; a tag with a `-` such as `v0.2.0-beta.1` is marked pre-release; signing secrets optional), `image.yml` (multi-arch sandbox image to GHCR).

## Daemon API (ND-JSON-RPC 2.0 over the socket)

`status`, `hardware.detect`, `models.list`, `models.catalog`, `models.download {id}`, `models.cancel {id}`, `agents.list`, `agents.create {name, role_prompt, model_ref, folders, allowed_domains?, approval_rules?}`, `agents.update {agent_id, allowed_domains, approval_rules}`, `agents.kill {agent_id}`, `agents.delete {agent_id}`, `chat.history {agent_id}`, `chat.send {agent_id, text}`, `chat.busy`, `actions.list {agent_id}`, `actions.undo {action_id, force?}`, `runs.undo {run_id, force?}`, `journal.verify`, `model_calls.list {agent_id}`, `secrets.set {name, value?, hosts, agent_ids, header?}`, `secrets.list`, `secrets.delete {name}`, `search.configure {brave_key?, searxng_url?}` (null clears), `domains.presets`, `mcp.list {agent_id}`, `mcp.add {agent_id, name, transport: stdio|http, command?|url?, secrets?}`, `mcp.remove {agent_id, name}`, `plugins.catalog`, `plugins.attach {agent_id, id, value?}` (OAuth plugins open the browser), `plugins.search {query}` (MCP Registry), `plugins.add_community {...}`, `team.propose {lead_id, model_ref, problem}`, `team.start {lead_id, problem}`, `approvals.pending`, `approvals.decide {approval_id, allow, remember?}`, `settings.get {key}`, `settings.set {key, value}`, `service.install`, `service.uninstall`, `runs.list {agent_id}`, `schedules.list {agent_id}`, `schedules.add {agent_id, kind: cron|interval|file, spec, prompt}`, `schedules.update {id, enabled?, spec?, prompt?}`, `schedules.delete {id}`, `events.subscribe`.
After `events.subscribe` a connection receives `chat.delta`, `chat.tool`, `chat.done`, `approval.requested`, `models.progress`, `models.downloaded`, `models.download_failed`, `schedule.fired` and `schedule.skipped`.

## Safety model

- **Tools:** `list_dir`, `read_file`, `write_file`, `delete_file` (host, scoped to the agent's folders + scratch), `shell` (sandbox), `http_fetch` (daemon, GET only), `notes_add`/`notes_search` (FTS5, per agent), `web_search` (Brave or SearXNG; hidden when neither is set), and MCP tools as `mcp__<server>__<tool>`.
- **Sandbox:** one container per agent, 2 CPU / 2 GB, the agent's folders mounted **read-only**, scratch read-write. Host files only change through journaled file tools.
- **Network:** each sandbox sits alone on an `--internal` network; its only way out is the daemon's HTTP/CONNECT proxy (per-agent token, ports 80/443). Docker/Podman add a small forwarder container. Same domain allowlist as `http_fetch`; new domains ask during a run. DNS is resolved by the daemon and private/loopback addresses are refused (also for `http_fetch`), unless allowlisted verbatim. No TLS interception. Every connection is journaled as `net`. Fetched and searched content is wrapped in `<untrusted>`.
- **Plugins:** `crates/burrowd/plugins.json` is the curated catalog (an MCP server plus the domains and key it needs). Attaching one is `mcp.add` plus allowed domains and a secret bound to that agent.
- **Teams:** an agent whose `delegate` rule isn't `deny` can call `delegate(agent, task)`: the specialist runs as its own run and its answer comes back as untrusted text. At most 6 per run; specialists can't delegate.
- **MCP:** stdio servers run inside the agent's sandbox and install through the proxy; HTTP servers are called by the daemon on allowlisted hosts with headers from bound secrets. Default rule is ask, or allow when the tool declares `readOnlyHint`.
- **Journal:** every tool call (including denied ones) in an append-only, hash-chained `actions` table.
- **Undo:** file writes/deletes snapshot first; undo refuses if the file changed since (unless forced). A shell command copies the agent's scratch folder first (up to 200 MB), so undo restores scratch and drops the container (installed packages go too). Fetch is marked irreversible.
- **Approvals:** default allow for list/read/write (undoable), ask for delete, shell and new domains. "Always allow" saves a rule or adds the domain.
- **Kill switch:** stops the run, denies its pending approvals, removes its sandbox.
- **Injection guard:** after `http_fetch`, `web_search`, `email_search`/`email_read` or an MCP tool, delete/shell/notes_add/email_send (and non-read-only MCP tools) ask for the rest of the run. Local undoable actions (write_file, email_move, email_draft) don't.
- **Limits:** 10 steps, 30 minutes, 32k output tokens per run. Schedules wait while on battery under 20%.
- **At rest:** SQLCipher DB and ChaCha20-Poly1305 undo snapshots (key in the keychain); scratch folders are plaintext. Crash reports go to `~/.burrow/crashes/`, local only.

Not yet: rootless Podman networking on Linux, secret injection into sandbox traffic (needs TLS interception, v0.2).
