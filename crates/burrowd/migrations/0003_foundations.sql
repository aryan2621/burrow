-- Secret values live in the OS keychain; this only records names and where they may be used.
CREATE TABLE secrets (
  name TEXT PRIMARY KEY,
  agent_ids TEXT NOT NULL DEFAULT '[]', -- JSON array
  hosts TEXT NOT NULL DEFAULT '[]',     -- JSON array; subdomains included
  header TEXT NOT NULL,                 -- e.g. "Authorization: Bearer {}"
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE model_calls (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL,
  run_id INTEGER NOT NULL,
  model TEXT NOT NULL,
  prompt_tokens INTEGER,
  completion_tokens INTEGER,
  ms INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Compacted messages stay for the UI but are no longer sent to the model.
ALTER TABLE messages ADD COLUMN compacted_at TEXT;
ALTER TABLE agents ADD COLUMN context_budget INTEGER NOT NULL DEFAULT 8000;
