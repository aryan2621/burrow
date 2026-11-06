# Models

## Built-in (llama.cpp)

Settings → Models lists the catalog with a recommendation for your RAM:

Only models that fit your memory are shown. All are Q4 GGUF builds with tool calling:

| RAM | Models (★ recommended) |
| --- | --- |
| 8 GB | Phi-4 mini 3.8B, Qwen3.5 4B ★ |
| 16 GB | Qwen2.5 Coder 7B, Llama 3.1 8B, Granite 3.3 8B, Gemma 4 E4B, Qwen3 8B, Qwen3.5 9B ★ |
| 18 GB | Gemma 4 12B, Mistral Nemo 12B |
| 24 GB | Qwen3 14B, gpt-oss 20B |
| 32 GB+ | Qwen3 30B-A3B ★ |

Downloads come from Hugging Face, resume after interruptions and are checked against a pinned
sha256. Files live in `~/.burrow/models`. The daemon starts llama-server on a loopback port when an
agent needs it and keeps each conversation's prompt cache warm.

GPU: Metal on macOS. On Linux and Windows the bundle is the CPU build; for Vulkan run
`scripts/fetch-llama.sh --vulkan`.

## Your own server

`BURROW_LLAMA_URL=http://host:port` sends `llama:` models to an OpenAI-compatible server you run.
Your conversations then go to that server.
