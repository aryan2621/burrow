CREATE TABLE agents (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  role_prompt TEXT NOT NULL,
  model_ref TEXT NOT NULL,
  folders TEXT NOT NULL -- JSON array of absolute paths
);
CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  body TEXT NOT NULL, -- one OpenAI chat message as JSON
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
