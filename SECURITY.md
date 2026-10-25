# Security

Burrow runs AI agents on your machine. Its safety rests on a few boundaries; a way around any
of them is a security bug.

## In scope

- **Folder scoping:** a file tool reads or writes outside the agent's folders and scratch folder
  (including through symlinks).
- **Sandbox escape:** the `shell` tool or a stdio MCP server reaches the host, another agent's
  container, or writes to a mounted folder (mounted read-only).
- **Network egress:** a sandbox reaches the network other than through the daemon's proxy, the
  proxy connects to a domain that isn't allowlisted or approved, or a private/loopback address is
  reached without being allowlisted verbatim.
- **Approvals:** a tool whose rule is `ask` or `deny` runs without that decision.
- **Journal:** an action runs without being recorded, or the hash chain can be altered without
  `journal.verify` reporting it.
- **IPC:** another local user can talk to `burrowd` (the socket is `0600`; the Windows pipe has an
  owner-only DACL and refuses remote clients).
- **Secrets:** a secret value reaches the model, the journal, or a host it isn't bound to.

Out of scope: a model making bad decisions within the permissions you gave it, prompt injection
that only leads to actions you approved or allowed, and attacks that need your user account
already compromised.

## Reporting

Use GitHub's private vulnerability reporting on this repository ("Security" → "Report a
vulnerability"). Please include steps to reproduce and the OS and sandbox runtime. We aim to reply
within 7 days and to ship a fix for confirmed issues within 90 days, crediting you unless you ask
otherwise. Please don't open a public issue for an unfixed vulnerability.

## Known limits (v0.1)

- No TLS interception, so secrets can't be injected into sandbox HTTPS traffic; bound secrets
  only reach `http_fetch` and HTTP MCP servers, which the daemon calls itself.
- Rootless Podman networking on Linux isn't supported yet.
- Agent scratch folders aren't encrypted at rest (the sandbox mounts them); the database and undo
  snapshots are.
- Release builds aren't code-signed until the signing accounts exist.
