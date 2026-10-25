# MCP tools

Add Model Context Protocol servers in an agent's **Tools** tab. Their tools appear as
`mcp__<server>__<tool>`.

- **stdio** servers (e.g. `npx -y @modelcontextprotocol/server-filesystem`) run inside the agent's
  sandbox, so they get the same folder and network limits. Their installs go through the proxy.
- **HTTP** servers are called by the daemon. The host must be in the agent's allowed domains, and
  you can bind secrets that are sent as headers.

MCP tools ask before running unless the server marks them read-only (`readOnlyHint`).
