//! Per-agent notes: `notes_add` and `notes_search`, keyword search on SQLite FTS5.
use crate::agent::State;
use anyhow::Result;
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};

const MAX_HITS: usize = 10;

/// Returns the tool output and the inverse that deletes the note again.
pub async fn add(st: &State, agent_id: i64, text: &str) -> Result<(String, Value)> {
    anyhow::ensure!(!text.trim().is_empty(), "note is empty");
    let id = sqlx::query("INSERT INTO notes (body, agent_id, created_at) VALUES (?, ?, datetime('now'))")
        .bind(text).bind(agent_id).execute(&st.db).await?.last_insert_rowid();
    Ok((format!("saved note {id}"), json!({"note_id": id})))
}

/// Each word is quoted so model input can't hit FTS5 syntax errors; any word matches.
fn fts_query(query: &str) -> String {
    query.split_whitespace().map(|w| format!("\"{}\"", w.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR ")
}

/// Matching notes, best first.
async fn find(db: &SqlitePool, agent_id: i64, query: &str, limit: i64) -> Result<Vec<sqlx::sqlite::SqliteRow>> {
    let q = fts_query(query);
    if q.is_empty() {
        return Ok(vec![]);
    }
    Ok(sqlx::query("SELECT rowid, body, created_at FROM notes WHERE notes MATCH ? AND agent_id = ? ORDER BY rank LIMIT ?")
        .bind(q).bind(agent_id).bind(limit).fetch_all(db).await?)
}

pub async fn search(st: &State, agent_id: i64, query: &str) -> Result<String> {
    if query.trim().is_empty() {
        return Ok("empty query".into());
    }
    let found = find(&st.db, agent_id, query, MAX_HITS as i64).await?;
    if found.is_empty() {
        return Ok("no matching notes".into());
    }
    Ok(found.iter().map(|r| format!("[{} {}] {}", r.get::<i64, _>(0), r.get::<String, _>(2), r.get::<String, _>(1)))
        .collect::<Vec<_>>().join("\n"))
}

/// The memory UI: all of an agent's notes, newest first, or those matching `query` by rank.
pub async fn list(st: &State, agent_id: i64, query: &str) -> Result<Vec<Value>> {
    let found = if query.trim().is_empty() {
        sqlx::query("SELECT rowid, body, created_at FROM notes WHERE agent_id = ? ORDER BY rowid DESC").bind(agent_id).fetch_all(&st.db).await?
    } else {
        find(&st.db, agent_id, query, 50).await?
    };
    Ok(found.iter().map(|r| json!({"id": r.get::<i64, _>(0), "text": r.get::<String, _>(1), "created_at": r.get::<String, _>(2)})).collect())
}

pub async fn update(st: &State, agent_id: i64, note_id: i64, text: &str) -> Result<()> {
    anyhow::ensure!(!text.trim().is_empty(), "note is empty");
    sqlx::query("UPDATE notes SET body = ? WHERE rowid = ? AND agent_id = ?").bind(text).bind(note_id).bind(agent_id).execute(&st.db).await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, note_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM notes WHERE rowid = ?").bind(note_id).execute(db).await?;
    Ok(())
}

/// The UI's delete: only the agent's own notes.
pub async fn delete_for(db: &SqlitePool, agent_id: i64, note_id: i64) -> Result<()> {
    let own: Option<i64> = sqlx::query_scalar("SELECT rowid FROM notes WHERE rowid = ? AND agent_id = ?").bind(note_id).bind(agent_id).fetch_optional(db).await?;
    if let Some(id) = own {
        delete(db, id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_search_scoped_and_undo() {
        let tmp = tempfile::tempdir().unwrap();
        let st = State::new(tmp.path().to_path_buf(), String::new(), crate::sandbox::Sandbox::detect(), crate::secrets::Vault::File(tmp.path().join("keys"))).await.unwrap();
        let db = st.db.clone();
        let (_, inv) = add(&st, 1, "The quarterly report is due on Friday").await.unwrap();
        add(&st, 2, "agent two also has a report").await.unwrap();
        let hits = search(&st, 1, "report \"deadline").await.unwrap();
        assert!(hits.contains("Friday") && !hits.contains("agent two"), "{hits}");
        delete(&db, inv["note_id"].as_i64().unwrap()).await.unwrap();
        assert!(!search(&st, 1, "report").await.unwrap().contains("Friday"));
    }
}
