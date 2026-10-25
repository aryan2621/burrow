//! Routines: a run worth repeating, saved with its prompt, the tools it used and the tools the
//! user approved in it. Running one offers only those tools and doesn't re-ask for the approved ones.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};

pub struct Routine {
    pub tools: Vec<String>,
    pub approved: Vec<String>,
}

pub async fn save(db: &SqlitePool, run_id: i64, name: &str) -> Result<i64> {
    anyhow::ensure!(!name.trim().is_empty(), "give the routine a name");
    let r = sqlx::query("SELECT agent_id, prompt, status FROM runs WHERE id = ?").bind(run_id).fetch_optional(db).await?.context("no such run")?;
    let prompt: String = r.get::<Option<String>, _>("prompt").context("this run is from before routines were recorded")?;
    anyhow::ensure!(r.get::<String, _>("status") == "done", "only a run that finished can become a routine");
    let tools: Vec<String> = sqlx::query_scalar("SELECT DISTINCT tool FROM actions WHERE run_id = ? AND decision != 'denied'").bind(run_id).fetch_all(db).await?;
    let approved: Vec<String> = sqlx::query_scalar("SELECT DISTINCT tool FROM actions WHERE run_id = ? AND decision = 'approved'").bind(run_id).fetch_all(db).await?;
    Ok(sqlx::query("INSERT INTO routines (agent_id, name, prompt, tools, approved) VALUES (?, ?, ?, ?, ?)")
        .bind(r.get::<i64, _>("agent_id")).bind(name.trim()).bind(prompt).bind(json!(tools).to_string()).bind(json!(approved).to_string())
        .execute(db).await?.last_insert_rowid())
}

pub async fn list(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM routines WHERE agent_id = ? ORDER BY id").bind(agent_id).fetch_all(db).await?;
    rows.iter().map(|r| Ok(json!({"id": r.get::<i64, _>("id"), "name": r.get::<String, _>("name"), "prompt": r.get::<String, _>("prompt"),
        "tools": serde_json::from_str::<Value>(r.get("tools"))?, "approved": serde_json::from_str::<Value>(r.get("approved"))?}))).collect()
}

pub async fn delete(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM routines WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}

/// (agent id, prompt) to start a routine.
pub async fn start(db: &SqlitePool, id: i64) -> Result<(i64, String)> {
    let r = sqlx::query("SELECT agent_id, prompt FROM routines WHERE id = ?").bind(id).fetch_optional(db).await?.context("no such routine")?;
    Ok((r.get(0), r.get(1)))
}

/// The routine a run belongs to: started as `routine:<id>`, or by a schedule that runs one.
pub async fn for_trigger(db: &SqlitePool, trigger: &str) -> Result<Option<Routine>> {
    let id: Option<i64> = if let Some(r) = trigger.strip_prefix("routine:") {
        r.parse().ok()
    } else if let Some(s) = trigger.strip_prefix("schedule:") {
        sqlx::query_scalar("SELECT routine_id FROM schedules WHERE id = ?").bind(s.parse::<i64>().unwrap_or(0)).fetch_optional(db).await?.flatten()
    } else {
        None
    };
    let Some(id) = id else { return Ok(None) };
    let Some(r) = sqlx::query("SELECT tools, approved FROM routines WHERE id = ?").bind(id).fetch_optional(db).await? else { return Ok(None) };
    Ok(Some(Routine { tools: serde_json::from_str(r.get("tools"))?, approved: serde_json::from_str(r.get("approved"))? }))
}
