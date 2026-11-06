-- Short-lived secret bindings, and bindings the kill switch suspended (restorable instead of lost).
ALTER TABLE secrets ADD COLUMN expires_at TEXT;
ALTER TABLE secrets ADD COLUMN suspended TEXT NOT NULL DEFAULT '[]';
