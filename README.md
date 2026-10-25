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

The app starts `burrowd` if it isn't already running. The daemon keeps running after the window closes.

- Data: `~/.burrow/` (override with `BURROW_HOME`): `burrow.db`, `snapshots/` (undo), `scratch/<agent>/`.
- IPC: Unix socket `~/.burrow/burrowd.sock` (0600) on macOS/Linux, named pipe `\\.\pipe\burrowd-<user>` on Windows.
- llama.cpp: set `BURROW_LLAMA_SERVER=/path/to/llama-server BURROW_LLAMA_MODEL=/path/to/model.gguf` and the daemon starts it on :8080. Then use `llama:<name>` model refs.
- Test: `cargo test -p burrowd`; with a sandbox runtime installed also `cargo test -p burrowd -- --ignored sandbox`.

## Daemon API (ND-JSON-RPC 2.0 over the socket)

`status`, `models.list`, `models.pull {name}`, `agents.list`, `agents.create {name, role_prompt, model_ref, folders, allowed_domains?, approval_rules?}`, `agents.update {agent_id, allowed_domains, approval_rules}`, `agents.kill {agent_id}`, `agents.delete {agent_id}`, `chat.history {agent_id}`, `chat.send {agent_id, text}`, `chat.busy`, `actions.list {agent_id}`, `actions.undo {action_id, force?}`, `runs.undo {run_id, force?}`, `journal.verify`, `approvals.pending`, `approvals.decide {approval_id, allow, remember?}`, `events.subscribe`.
After `events.subscribe` a connection receives `chat.delta`, `chat.tool`, `chat.done` and `approval.requested`.

## Safety model (phase 2)

- **Tools:** `list_dir`, `read_file`, `write_file`, `delete_file` (host, scoped to the agent's folders + scratch), `shell` (sandbox), `http_fetch` (daemon, GET only).
- **Sandbox:** one container per agent, `--network none`, 2 CPU / 2 GB, the agent's folders mounted **read-only**, scratch read-write. Host files only change through journaled file tools.
- **Network:** only via `http_fetch`: per-agent domain allowlist (subdomains included), every redirect re-checked, localhost/IP literals blocked, output wrapped in `<untrusted>`.
- **Journal:** every tool call (including denied ones) in an append-only, hash-chained `actions` table.
- **Undo:** file writes/deletes snapshot first; undo refuses if the file changed since (unless forced). Shell/fetch are marked irreversible.
- **Approvals:** default allow for list/read/write (undoable), ask for delete, shell and new domains. "Always allow" saves a rule or adds the domain.
- **Kill switch:** stops the run, denies its pending approvals, removes its sandbox.

Not yet: egress proxy (so no network inside the sandbox), secrets broker, encryption at rest, DNS-rebinding check, owner-only DACL on the Windows pipe.
