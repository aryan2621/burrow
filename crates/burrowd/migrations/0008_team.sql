-- Rooms: 2–6 agents talk with the user. The coordinator answers messages that mention nobody.
CREATE TABLE rooms (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  coordinator_id INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE room_members (
  room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
  agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  PRIMARY KEY (room_id, agent_id)
);
CREATE TABLE room_messages (
  id INTEGER PRIMARY KEY,
  room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
  agent_id INTEGER,            -- null: the user
  text TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
-- The room's shared task board.
CREATE TABLE tasks (
  id INTEGER PRIMARY KEY,
  room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  assignee_id INTEGER,
  status TEXT NOT NULL DEFAULT 'todo',   -- todo | doing | done
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Routines: a run saved for reuse. Only its tools are offered; tools approved in the original run
-- don't ask again (the injection guard and reviewer still apply).
CREATE TABLE routines (
  id INTEGER PRIMARY KEY,
  agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  prompt TEXT NOT NULL,
  tools TEXT NOT NULL DEFAULT '[]',
  approved TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
ALTER TABLE runs ADD COLUMN prompt TEXT;
ALTER TABLE schedules ADD COLUMN routine_id INTEGER REFERENCES routines(id) ON DELETE SET NULL;

-- Artifacts: versioned files agents share. The owner and agents in `artifact_access` (or the room's
-- members, for a room artifact) may read; `write` lets them add versions.
CREATE TABLE artifacts (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  owner_id INTEGER NOT NULL,
  room_id INTEGER REFERENCES rooms(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE UNIQUE INDEX artifacts_name ON artifacts (owner_id, COALESCE(room_id, 0), name);
CREATE TABLE artifact_versions (
  id INTEGER PRIMARY KEY,
  artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
  version INTEGER NOT NULL,
  author_id INTEGER NOT NULL,
  content BLOB NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (artifact_id, version)
);
CREATE TABLE artifact_access (
  artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
  agent_id INTEGER NOT NULL,
  write INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (artifact_id, agent_id)
);

-- Cost meter: USD per cloud model call (null when the price is unknown), monthly cap per agent,
-- and a cheaper model for simple steps (compaction summaries, reviewer checks).
ALTER TABLE model_calls ADD COLUMN cost_usd REAL;
ALTER TABLE agents ADD COLUMN monthly_usd REAL NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN cheap_model_ref TEXT;
