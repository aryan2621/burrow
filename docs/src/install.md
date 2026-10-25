# Install

Download the installer for your OS from the releases page: `.dmg` (macOS), `.msi` (Windows),
`.AppImage` or `.deb` (Linux). The app bundles the `burrowd` daemon and llama.cpp.

v0.1 builds are not code-signed yet. On macOS right-click the app and choose **Open** the first
time; on Windows choose **More info → Run anyway** in SmartScreen.

## Sandbox runtime

The `shell` tool and stdio MCP servers run in a container. Burrow picks the first one it finds:

| OS | Runtime | Install |
| --- | --- | --- |
| macOS 26+ | Apple `container` (one VM per agent) | `brew install container && container system start` |
| macOS (older) | Docker Desktop or Podman | from their sites |
| Linux | Podman (rootful) or Docker | your package manager |
| Windows | Docker Desktop or Podman (WSL 2) | from their sites |

Force one with `BURROW_SANDBOX=docker|podman|container`. Without a runtime, file tools, notes and
`http_fetch` still work; only `shell` is unavailable. The onboarding screen checks this for you.

## From source

```sh
scripts/fetch-llama.sh            # pinned llama-server into ~/.burrow/llama
cd apps/desktop && pnpm i && pnpm tauri dev
```
