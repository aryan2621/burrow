//! Artifact store: versioned files agents produce and share. An agent sees artifacts it owns,
//! ones shared with it, and those of rooms it's in; it adds versions where it may write.
use crate::tools::{self, Outcome};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use std::path::PathBuf;

const MAX_SIZE: usize = 20 * 1024 * 1024;
const MAX_READ: usize = 64 * 1024;

/// (id, name, owner, room, can write) of what `agent_id` can see.
async fn visible(db: &SqlitePool, agent_id: i64) -> Result<Vec<(i64, String, i64, Option<i64>, bool)>> {
    let rows = sqlx::query(
        "SELECT a.id, a.name, a.owner_id, a.room_id,
                (a.owner_id = ?1 OR a.room_id IN (SELECT room_id FROM room_members WHERE agent_id = ?1)
                 OR EXISTS (SELECT 1 FROM artifact_access x WHERE x.artifact_id = a.id AND x.agent_id = ?1 AND x.write = 1)) AS w
         FROM artifacts a
         WHERE a.owner_id = ?1 OR a.room_id IN (SELECT room_id FROM room_members WHERE agent_id = ?1)
            OR EXISTS (SELECT 1 FROM artifact_access x WHERE x.artifact_id = a.id AND x.agent_id = ?1)
         ORDER BY a.id")
        .bind(agent_id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get::<bool, _>(4))).collect())
}

pub const TOOLS: [&str; 3] = ["artifact_list", "artifact_read", "artifact_save"];

pub fn specs() -> Vec<Value> {
    let f = |name: &str, desc: &str, props: Value, req: &[&str]| json!({"type": "function", "function": {"name": name,
        "description": desc, "parameters": {"type": "object", "properties": props, "required": req}}});
    let s = |d: &str| json!({"type": "string", "description": d});
    vec![
        f("artifact_list", "List artifacts (shared, versioned files) you can see", json!({}), &[]),
        f("artifact_read", "Read an artifact's latest or a given version",
          json!({"id": {"type": "integer"}, "version": {"type": "integer"}}), &["id"]),
        f("artifact_save", "Save a new version of an artifact (creates it if new). Give `content`, or `path` of a file \
          in your folders. In a room it's shared with the room (undoable)",
          json!({"name": s("artifact name"), "id": {"type": "integer", "description": "existing artifact to add a version to"},
                 "content": s("text content"), "path": s("file to store instead of content")}), &[]),
    ]
}

pub async fn call(db: &SqlitePool, agent_id: i64, room: Option<i64>, folders: &[PathBuf], name: &str, args: &Value) -> Outcome {
    run(db, agent_id, room, folders, name, args).await.unwrap_or_else(|e| format!("error: {e:#}").into())
}

async fn run(db: &SqlitePool, agent_id: i64, room: Option<i64>, folders: &[PathBuf], name: &str, args: &Value) -> Result<Outcome> {
    let vis = visible(db, agent_id).await?;
    Ok(match name {
        "artifact_list" => {
            let mut lines = vec![];
            for (id, n, owner, room, w) in &vis {
                let v: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM artifact_versions WHERE artifact_id = ?").bind(id).fetch_one(db).await?;
                lines.push(format!("{id} | {n} | v{v} | owner agent {owner}{}{}", room.map(|r| format!(" | room {r}")).unwrap_or_default(),
                    if *w { "" } else { " | read-only" }));
            }
            if lines.is_empty() { "no artifacts".to_string().into() } else { lines.join("\n").into() }
        }
        "artifact_read" => {
            let id = args["id"].as_i64().context("missing `id`")?;
            let (_, n, owner, ..) = vis.iter().find(|a| a.0 == id).context("no such artifact, or not shared with you")?;
            let r = sqlx::query("SELECT version, author_id, content FROM artifact_versions WHERE artifact_id = ? AND (? IS NULL OR version = ?)
                                 ORDER BY version DESC LIMIT 1").bind(id).bind(args["version"].as_i64()).bind(args["version"].as_i64())
                .fetch_optional(db).await?.context("no such version")?;
            let (v, author, content): (i64, i64, Vec<u8>) = (r.get(0), r.get(1), r.get(2));
            let body = match String::from_utf8(content) {
                Ok(mut t) => {
                    if t.len() > MAX_READ { t.truncate(t.floor_char_boundary(MAX_READ)); t.push_str("\n[truncated]"); }
                    t
                }
                Err(e) => format!("(binary, {} bytes)", e.as_bytes().len()),
            };
            // written by another agent, which may have copied web content into it
            let tag = if *owner == agent_id && author == agent_id { body } else { format!("<untrusted source=\"artifact:{n}\">\n{body}\n</untrusted>") };
            format!("{n} v{v}\n{tag}").into()
        }
        _ => {
            let content: Vec<u8> = match (args["content"].as_str(), args["path"].as_str()) {
                (Some(c), _) => c.as_bytes().to_vec(),
                (None, Some(p)) => std::fs::read(tools::scoped(folders, p)?)?,
                _ => bail!("give `content` or `path`"),
            };
            anyhow::ensure!(content.len() <= MAX_SIZE, "artifacts are limited to 20 MB");
            let id = match args["id"].as_i64() {
                Some(id) => {
                    let a = vis.iter().find(|a| a.0 == id).context("no such artifact, or not shared with you")?;
                    anyhow::ensure!(a.4, "artifact {id} is read-only for you");
                    id
                }
                None => {
                    let n = args["name"].as_str().filter(|n| !n.trim().is_empty()).context("give `name` or `id`")?.trim();
                    match vis.iter().find(|a| a.1 == n && a.2 == agent_id && a.3 == room) {
                        Some(a) => a.0,
                        None => sqlx::query("INSERT INTO artifacts (name, owner_id, room_id) VALUES (?, ?, ?)")
                            .bind(n).bind(agent_id).bind(room).execute(db).await?.last_insert_rowid(),
                    }
                }
            };
            let v: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM artifact_versions WHERE artifact_id = ?").bind(id).fetch_one(db).await?;
            let vid = sqlx::query("INSERT INTO artifact_versions (artifact_id, version, author_id, content) VALUES (?, ?, ?, ?)")
                .bind(id).bind(v).bind(agent_id).bind(&content).execute(db).await?.last_insert_rowid();
            Outcome { output: format!("saved artifact {id} v{v} ({} bytes)", content.len()), inverse: Some(json!({"artifact_version_id": vid})) }
        }
    })
}

/// Undo a save: drop that version, and the artifact if it was the only one.
pub async fn undo(db: &SqlitePool, version_id: i64) -> Result<()> {
    let a: Option<i64> = sqlx::query_scalar("DELETE FROM artifact_versions WHERE id = ? RETURNING artifact_id").bind(version_id).fetch_optional(db).await?;
    if let Some(a) = a {
        sqlx::query("DELETE FROM artifacts WHERE id = ? AND NOT EXISTS (SELECT 1 FROM artifact_versions WHERE artifact_id = ?)")
            .bind(a).bind(a).execute(db).await?;
    }
    Ok(())
}

// ---- the UI ----

pub async fn list(db: &SqlitePool) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT a.*, (SELECT MAX(version) FROM artifact_versions WHERE artifact_id = a.id) AS latest,
                            (SELECT json_group_array(json_object('agent_id', agent_id, 'write', write)) FROM artifact_access WHERE artifact_id = a.id) AS access
                            FROM artifacts a ORDER BY a.id DESC").fetch_all(db).await?;
    rows.iter().map(|r| Ok(json!({"id": r.get::<i64, _>("id"), "name": r.get::<String, _>("name"), "owner_id": r.get::<i64, _>("owner_id"),
        "room_id": r.get::<Option<i64>, _>("room_id"), "latest": r.get::<Option<i64>, _>("latest"), "created_at": r.get::<String, _>("created_at"),
        "access": serde_json::from_str::<Value>(r.get("access"))?}))).collect()
}

pub async fn versions(db: &SqlitePool, id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT id, version, author_id, length(content) AS size, created_at FROM artifact_versions WHERE artifact_id = ? ORDER BY version DESC")
        .bind(id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| json!({"id": r.get::<i64, _>("id"), "version": r.get::<i64, _>("version"), "author_id": r.get::<i64, _>("author_id"),
        "size": r.get::<i64, _>("size"), "created_at": r.get::<String, _>("created_at")})).collect())
}

/// One version as text, or base64 when it isn't UTF-8.
pub async fn content(db: &SqlitePool, version_id: i64) -> Result<Value> {
    use base64::Engine;
    let c: Vec<u8> = sqlx::query_scalar("SELECT content FROM artifact_versions WHERE id = ?").bind(version_id).fetch_optional(db).await?.context("no such version")?;
    Ok(match String::from_utf8(c) {
        Ok(t) => json!({"text": t}),
        Err(e) => json!({"base64": base64::engine::general_purpose::STANDARD.encode(e.as_bytes())}),
    })
}

/// `write`: true/false grants read+write/read; null removes the agent's access.
pub async fn share(db: &SqlitePool, id: i64, agent_id: i64, write: &Value) -> Result<()> {
    match write.as_bool() {
        Some(w) => sqlx::query("INSERT INTO artifact_access (artifact_id, agent_id, write) VALUES (?, ?, ?)
                                ON CONFLICT DO UPDATE SET write = excluded.write").bind(id).bind(agent_id).bind(w).execute(db).await?,
        None => sqlx::query("DELETE FROM artifact_access WHERE artifact_id = ? AND agent_id = ?").bind(id).bind(agent_id).execute(db).await?,
    };
    Ok(())
}

pub async fn delete(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM artifacts WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}
