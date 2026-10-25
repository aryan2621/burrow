-- Per-agent run budget (was fixed in code) and the reviewer pass before auto-allowed side effects.
ALTER TABLE agents ADD COLUMN max_steps INTEGER NOT NULL DEFAULT 10;
ALTER TABLE agents ADD COLUMN max_run_secs INTEGER NOT NULL DEFAULT 1800;
ALTER TABLE agents ADD COLUMN max_run_tokens INTEGER NOT NULL DEFAULT 32000;
ALTER TABLE agents ADD COLUMN review INTEGER NOT NULL DEFAULT 1;
