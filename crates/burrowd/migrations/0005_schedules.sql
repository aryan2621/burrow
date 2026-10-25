CREATE TABLE schedules (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,          -- cron | interval | file
  spec TEXT NOT NULL,          -- cron expression | seconds | watched path
  prompt TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  next_run_at INTEGER,         -- unix seconds; null for file triggers
  last_run_at INTEGER
);

-- What started a run: `chat` or `schedule:<id>`.
ALTER TABLE runs ADD COLUMN trigger TEXT NOT NULL DEFAULT 'chat';
