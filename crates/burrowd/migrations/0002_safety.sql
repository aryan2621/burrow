ALTER TABLE agents ADD COLUMN allowed_domains TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agents ADD COLUMN approval_rules TEXT NOT NULL DEFAULT '{}';

CREATE TABLE runs (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  ended_at TEXT,
  status TEXT NOT NULL DEFAULT 'running' -- running | done | failed | killed | interrupted
);

-- The journal. Append-only and hash-chained; no FK so it outlives deleted agents.
CREATE TABLE actions (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL,
  run_id INTEGER NOT NULL,
  tool TEXT NOT NULL,
  input TEXT NOT NULL,
  output TEXT NOT NULL,
  inverse TEXT,             -- JSON, null when the action can't be undone
  decision TEXT NOT NULL,   -- auto | approved | denied
  created_at INTEGER NOT NULL, -- unix ms
  prev_hash TEXT NOT NULL,
  hash TEXT NOT NULL
);
CREATE TRIGGER actions_no_update BEFORE UPDATE ON actions BEGIN SELECT RAISE(ABORT, 'journal is append-only'); END;
CREATE TRIGGER actions_no_delete BEFORE DELETE ON actions BEGIN SELECT RAISE(ABORT, 'journal is append-only'); END;

CREATE TABLE undos (
  action_id INTEGER PRIMARY KEY REFERENCES actions(id),
  at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE approvals (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL,
  run_id INTEGER NOT NULL,
  tool TEXT NOT NULL,
  args TEXT NOT NULL,
  requested_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  decision TEXT,            -- null while pending | approved | denied | expired
  decided_at TEXT
);
