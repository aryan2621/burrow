# Privacy

Burrow has no telemetry, no accounts and no Burrow servers. Your chats, files, notes, journal and
secrets stay on your machine: the database (`~/.burrow/burrow.db`) is encrypted with SQLCipher, its
key and your secrets live in the OS keychain (or a `0600` key file where no keychain is available).
Undo snapshots are encrypted with a key derived from it. Agent scratch folders are not encrypted,
because the sandbox mounts them directly. Crash reports are written to `~/.burrow/crashes/` and never
sent anywhere.

## Network calls Burrow makes

Only these, and only when the listed thing happens:

| When | To | What is sent |
| --- | --- | --- |
| You download a built-in model | `huggingface.co` (and its CDN) | A file request for that model |
| You pull an Ollama model | Your Ollama server (default `127.0.0.1:11434`) | The model name |
| An agent runs on a model | llama-server on loopback, or your Ollama / `BURROW_LLAMA_URL` server | The conversation |
| An agent uses `http_fetch` or its sandbox opens a connection | The site the agent asked for, if allowlisted or approved by you | That request |
| An agent uses `web_search` (only if you configured it) | Brave Search API or your SearXNG URL | The search query |
| An agent uses an HTTP MCP server you added | That server | The tool call |
| The sandbox starts for the first time | The container registry hosting the sandbox image | An image pull |

If you point an agent at a remote model server, the conversation goes there; Burrow's built-in
models never leave your machine.

Every connection an agent makes is recorded in its journal as a `net` entry, which you can see in
the Timeline.

## Deleting your data

Deleting an agent removes its chats, notes and schedules. Its journal entries stay, because the
journal is append-only and hash-chained. Delete `~/.burrow/` (or `BURROW_HOME`) and the keychain
entries under the service `dev.burrow` to remove everything.
