# Agents and their settings

Open an agent's settings (the sliders icon) to change what it may do. The core of the agent (name
and role) stays as created.

- **Folders:** the only places on your disk it can list, read or change. Type a path (`~/Documents`
  works) or use *Choose…*, then press Save. Shell commands see folders read-only; writes go through
  journaled file tools. Changing folders restarts its sandbox.
- **Permissions:** allowed domains, and allow / ask / deny for each tool. A tool set to deny is not
  offered to the model at all.
- **Limits:** steps, minutes, output tokens and the monthly cloud budget per run, plus the sandbox's
  CPUs and RAM.
- **Stop all agents** (sidebar) kills every running agent at once.
