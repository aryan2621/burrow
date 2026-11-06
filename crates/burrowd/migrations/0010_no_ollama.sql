-- Ollama support removed: models are built-in (llama.cpp) or cloud. Agents on an Ollama model
-- move to the 16 GB tier's built-in model; switch it in the agent's settings if that's not downloaded.
UPDATE agents SET model_ref = 'llama:qwen3-8b' WHERE model_ref LIKE 'ollama:%';
