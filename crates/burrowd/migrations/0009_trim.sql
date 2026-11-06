-- Removed: rooms (group chat, task board), the artifact store, the model router, and the
-- calendar and GitHub connectors. Email is the one connector.
DROP TABLE artifact_access;
DROP TABLE artifact_versions;
DROP TABLE artifacts;
DROP TABLE tasks;
DROP TABLE room_messages;
DROP TABLE room_members;
DROP TABLE rooms;
ALTER TABLE agents DROP COLUMN cheap_model_ref;
DELETE FROM schedules WHERE kind IN ('calendar', 'github');
DELETE FROM connections WHERE kind IN ('calendar', 'github');
