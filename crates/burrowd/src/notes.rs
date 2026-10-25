//! Per-agent notes on SQLite FTS5: `notes_add` and `notes_search`.
use anyhow::Result;
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};

const MAX_HITS: i64 = 10;

/// Returns the tool output and the inverse that deletes the note again.
pub async fn add(db: &SqlitePool, agent_id: i64, text: &str) -> Result<(String, Value)> {
    anyhow::ensure!(!text.trim().is_empty(), "note is empty");
    let id = sqlx::query("INSERT INTO notes (body, agent_id, created_at) VALUES (?, ?, datetime('now'))")
        .bind(text).bind(agent_id).execute(db).await?.last_insert_rowid();
    Ok((format!("saved note {id}"), json!({"note_id": id})))
}

/// Each word is quoted so model input can't hit FTS5 syntax errors; any word matches.
fn fts_query(query: &str) -> String {
    query.split_whitespace().map(|w| format!("\"{}\"", w.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR ")
}

/// Any-word match ranked by bm25.
pub async fn search(db: &SqlitePool, agent_id: i64, query: &str) -> Result<String> {
    let q = fts_query(query);
    if q.is_empty() {
        return Ok("empty query".into());
    }
    let rows = sqlx::query("SELECT rowid, body, created_at FROM notes WHERE notes MATCH ? AND agent_id = ? ORDER BY rank LIMIT ?")
        .bind(q).bind(agent_id).bind(MAX_HITS).fetch_all(db).await?;
    if rows.is_empty() {
        return Ok("no matching notes".into());
    }
    Ok(rows.iter().map(|r| format!("[{} {}] {}", r.get::<i64, _>(0), r.get::<String, _>(2), r.get::<String, _>(1)))
        .collect::<Vec<_>>().join("\n"))
}

/// The memory UI: all of an agent's notes, newest first, or those matching `query` by rank.
pub async fn list(db: &SqlitePool, agent_id: i64, query: &str) -> Result<Vec<Value>> {
    let q = fts_query(query);
    let rows = if q.is_empty() {
        sqlx::query("SELECT rowid, body, created_at FROM notes WHERE agent_id = ? ORDER BY rowid DESC").bind(agent_id).fetch_all(db).await?
    } else {
        sqlx::query("SELECT rowid, body, created_at FROM notes WHERE notes MATCH ? AND agent_id = ? ORDER BY rank")
            .bind(q).bind(agent_id).fetch_all(db).await?
    };
    Ok(rows.iter().map(|r| json!({"id": r.get::<i64, _>(0), "text": r.get::<String, _>(1), "created_at": r.get::<String, _>(2)})).collect())
}

pub async fn update(db: &SqlitePool, agent_id: i64, note_id: i64, text: &str) -> Result<()> {
    anyhow::ensure!(!text.trim().is_empty(), "note is empty");
    sqlx::query("UPDATE notes SET body = ? WHERE rowid = ? AND agent_id = ?").bind(text).bind(note_id).bind(agent_id).execute(db).await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, note_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM notes WHERE rowid = ?").bind(note_id).execute(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_search_scoped_and_undo() {
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::store::open(&tmp.path().join("t.db"), &"ab".repeat(32)).await.unwrap();
        let (_, inv) = add(&db, 1, "The quarterly report is due on Friday").await.unwrap();
        add(&db, 2, "agent two also has a report").await.unwrap();
        let hits = search(&db, 1, "report \"deadline").await.unwrap();
        assert!(hits.contains("Friday") && !hits.contains("agent two"), "{hits}");
        delete(&db, inv["note_id"].as_i64().unwrap()).await.unwrap();
        assert_eq!(search(&db, 1, "report").await.unwrap(), "no matching notes");
    }
}
