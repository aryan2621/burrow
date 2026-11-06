-- Per-agent sandbox size (was fixed at 2 CPU / 2 GB).
ALTER TABLE agents ADD COLUMN cpus INTEGER NOT NULL DEFAULT 2;
ALTER TABLE agents ADD COLUMN memory_mb INTEGER NOT NULL DEFAULT 2048;
