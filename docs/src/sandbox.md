# The sandbox

Each agent's shell and browser run in their own container, picked in this order: Apple `container`
(macOS 26, one VM per agent), Podman, Docker.

- **VM isolation:** Apple `container` only. Podman and Docker share the host kernel, and the
  sidebar says so.
- **Size:** CPUs and RAM per agent in its settings (default 2 CPUs, 2 GB).
- **Windows:** Docker Desktop or Podman run inside a WSL2 VM, shared by all agents.
