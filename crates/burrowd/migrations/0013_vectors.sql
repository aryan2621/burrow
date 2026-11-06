-- Embeddings for memory search (little-endian f32). Brute-force cosine in Rust: notes stay in the thousands.
CREATE TABLE note_vectors (note_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);
