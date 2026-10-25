# Models

## Built-in (llama.cpp)

Settings → Models lists the catalog with a recommendation for your RAM:

| Model | Good for |
| --- | --- |
| Qwen3 4B | 8 GB machines |
| Qwen3 8B | 16 GB |
| Qwen3 30B-A3B | 32 GB+ |

Downloads come from Hugging Face, resume after interruptions and are checked against a pinned
sha256. Files live in `~/.burrow/models`. The daemon starts llama-server on a loopback port when an
agent needs it and keeps each conversation's prompt cache warm.

GPU: Metal on macOS. On Linux and Windows the bundle is the CPU build; for Vulkan run
`scripts/fetch-llama.sh --vulkan`.

## Ollama

Agents can use any tool-calling Ollama model as `ollama:<name>`. Set `BURROW_OLLAMA_URL` if Ollama
isn't on `127.0.0.1:11434`.

## Your own server

`BURROW_LLAMA_URL=http://host:port` sends `llama:` models to an OpenAI-compatible server you run.
Your conversations then go to that server.
