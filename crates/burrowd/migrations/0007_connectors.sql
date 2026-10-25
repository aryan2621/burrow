-- Accounts the user connects (email, calendar, GitHub); the password/token is in the keychain
-- under `conn-<name>`. `agent_ids` are the agents allowed to use it.
CREATE TABLE connections (
  name TEXT PRIMARY KEY,
  kind TEXT NOT NULL,              -- email | calendar | github
  config TEXT NOT NULL DEFAULT '{}',
  agent_ids TEXT NOT NULL DEFAULT '[]'
);

-- Approved emails wait here until send_at, so the user can still cancel (undo) them.
CREATE TABLE outbox (
  id INTEGER PRIMARY KEY,
  connection TEXT NOT NULL,
  agent_id INTEGER NOT NULL,
  message TEXT NOT NULL,
  send_at INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',  -- pending | sending | sent | cancelled | failed
  error TEXT
);

-- Event triggers (email, calendar, github) remember what they've already seen.
ALTER TABLE schedules ADD COLUMN cursor TEXT;
