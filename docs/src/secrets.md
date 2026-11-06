# Secrets and identity

Secrets live in the OS keychain. An agent never sees a value: Burrow adds it to requests for the
hosts you bind it to and scrubs it from tool output.

- **Per agent:** a secret works only for the agents you bind it to.
- **Short-lived:** set *Expires after (hours)* and the binding stops working at that time.
- **Kill switch:** stopping an agent with the stop button suspends its secrets. Settings → Secrets
  shows *Restore* for each suspended agent when you want it to have them back.
- Secrets reach web fetches and HTTP MCP servers. HTTPS traffic from the sandbox is not
  intercepted, so shell commands can't use them.
