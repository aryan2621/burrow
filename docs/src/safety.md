# Safety model

- **Folders:** file tools resolve symlinks and refuse anything outside the agent's folders and
  scratch folder.
- **Sandbox:** one container per agent (2 CPU, 2 GB). Your folders are mounted read-only; only
  the journaled file tools change host files.
- **Network:** the sandbox has no direct route out. Its only exit is the daemon's proxy, which
  enforces the agent's domain allowlist, refuses private and loopback addresses unless you
  allowlist them exactly, and journals every connection.
- **Approvals:** reads and writes run (writes are undoable); deletes, shell, new domains and MCP
  tools ask by default. "Always allow" saves a rule.
- **Journal:** every tool call, including denied ones, goes into an append-only hash chain. The
  Timeline shows whether it's intact.
- **Undo:** writes and deletes snapshot first. Undo refuses if the file changed since, unless you
  force it. Shell commands and web requests can't be undone and are marked so.
- **Kill switch:** stops the run, denies its pending approvals and removes its sandbox.
- **Untrusted content:** fetched pages and search results are wrapped in `<untrusted>` tags, and
  the model is told never to follow instructions inside them.

See `SECURITY.md` in the repository for what counts as a vulnerability.
