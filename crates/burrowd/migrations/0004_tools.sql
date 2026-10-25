-- Per-agent notes with full-text search; also the base for v0.2 memory.
CREATE VIRTUAL TABLE notes USING fts5(body, agent_id UNINDEXED, created_at UNINDEXED);

-- Daemon-wide settings such as the SearXNG URL. Secret values go to the keychain instead.
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE mcp_servers (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  transport TEXT NOT NULL,             -- stdio | http
  command TEXT NOT NULL DEFAULT '[]',  -- JSON argv, run inside the agent's sandbox (stdio)
  url TEXT,                            -- http
  secrets TEXT NOT NULL DEFAULT '[]',  -- secret names whose headers go to the http server
  UNIQUE (agent_id, name)
);
