use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use sqlx::Row;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone)]
pub struct Agent {
    pub id: i64,
    pub name: String,
    pub role_prompt: String,
    pub model_ref: String,
    pub folders: Vec<PathBuf>,
    pub allowed_domains: Vec<String>,
    /// tool name → `allow` | `ask` | `deny`; tools not listed use `policy::default_rule`
    pub approval_rules: BTreeMap<String, String>,
    /// Estimated tokens of history sent to the model before compaction kicks in.
    pub context_budget: i64,
    pub limits: Limits,
}

/// Runaway limits per run, checked before each model call (a pending approval doesn't get cut off).
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct Limits {
    pub max_steps: i64,
    pub max_run_secs: i64,
    /// Output tokens summed over the run's model calls.
    pub max_run_tokens: i64,
    /// A second model call checks auto-allowed side effects against the request (cloud models only).
    pub review: bool,
    /// Cloud spend cap per calendar month in USD; 0 means none.
    #[serde(default)]
    pub monthly_usd: f64,
    /// Sandbox size: CPUs and RAM in MB.
    #[serde(default = "two")]
    pub cpus: i64,
    #[serde(default = "two_gb")]
    pub memory_mb: i64,
}

fn two() -> i64 { 2 }
fn two_gb() -> i64 { 2048 }

impl Default for Limits {
    fn default() -> Self {
        Limits { max_steps: 10, max_run_secs: 30 * 60, max_run_tokens: 32_000, review: true, monthly_usd: 0.0, cpus: 2, memory_mb: 2048 }
    }
}

/// Open the SQLCipher-encrypted DB with a hex `key`, encrypting a plaintext DB from
/// before encryption existed.
pub async fn open(path: &Path, key: &str) -> Result<SqlitePool> {
    anyhow::ensure!(key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()), "bad database key");
    let raw_key = format!("\"x'{key}'\"");
    if is_plaintext(path)? {
        encrypt_in_place(path, &raw_key).await?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .pragma("key", raw_key) // sqlx applies `key` before any other pragma
        .create_if_missing(true)
        .foreign_keys(true);
    let db = SqlitePool::connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    Ok(db)
}

fn is_plaintext(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut head = [0u8; 16];
    match std::fs::File::open(path) {
        Ok(mut f) => Ok(f.read(&mut head)? == 16 && &head == b"SQLite format 3\0"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

async fn encrypt_in_place(path: &Path, raw_key: &str) -> Result<()> {
    let tmp = path.with_extension("db.enc");
    let _ = std::fs::remove_file(&tmp);
    use sqlx::{ConnectOptions, Connection};
    // one connection: ATTACH is per connection. It also inherits the open flags, so allow create.
    let mut plain = SqliteConnectOptions::new().filename(path).create_if_missing(true).connect().await?;
    sqlx::query(&format!("ATTACH DATABASE '{}' AS enc KEY {raw_key}", tmp.display().to_string().replace('\'', "''")))
        .execute(&mut plain).await?;
    sqlx::query("SELECT sqlcipher_export('enc')").execute(&mut plain).await?;
    sqlx::query("DETACH DATABASE enc").execute(&mut plain).await?;
    plain.close().await?;
    std::fs::rename(&tmp, path)?;
    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(path.with_extension(ext));
    }
    eprintln!("encrypted existing database {}", path.display());
    Ok(())
}

fn agent(r: sqlx::sqlite::SqliteRow) -> Result<Agent> {
    Ok(Agent {
        id: r.get("id"),
        name: r.get("name"),
        role_prompt: r.get("role_prompt"),
        model_ref: r.get("model_ref"),
        folders: serde_json::from_str(r.get("folders"))?,
        allowed_domains: serde_json::from_str(r.get("allowed_domains"))?,
        approval_rules: serde_json::from_str(r.get("approval_rules"))?,
        context_budget: r.get("context_budget"),
        limits: Limits { max_steps: r.get("max_steps"), max_run_secs: r.get("max_run_secs"),
            max_run_tokens: r.get("max_run_tokens"), review: r.get("review"), monthly_usd: r.get("monthly_usd"),
            cpus: r.get("cpus"), memory_mb: r.get("memory_mb") },
    })
}

pub async fn agents(db: &SqlitePool) -> Result<Vec<Agent>> {
    sqlx::query("SELECT * FROM agents ORDER BY id")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(agent)
        .collect()
}

pub async fn get_agent(db: &SqlitePool, id: i64) -> Result<Agent> {
    let r = sqlx::query("SELECT * FROM agents WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?
        .with_context(|| format!("no agent {id}"))?;
    agent(r)
}

pub async fn create_agent(db: &SqlitePool, a: &Agent) -> Result<i64> {
    // Never reuse a deleted agent's id: the journal and model_calls outlive it (no FK), so a
    // reused id would inherit its history.
    let id = sqlx::query(
        "INSERT INTO agents (id, name, role_prompt, model_ref, folders, allowed_domains, approval_rules, context_budget, max_steps, max_run_secs, max_run_tokens, review, monthly_usd, cpus, memory_mb)
         VALUES ((SELECT MAX(m) + 1 FROM (SELECT COALESCE(MAX(id), 0) m FROM agents
           UNION ALL SELECT COALESCE(MAX(agent_id), 0) FROM actions UNION ALL SELECT COALESCE(MAX(agent_id), 0) FROM model_calls)),
         ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&a.name)
    .bind(&a.role_prompt)
    .bind(&a.model_ref)
    .bind(serde_json::to_string(&a.folders)?)
    .bind(serde_json::to_string(&a.allowed_domains)?)
    .bind(serde_json::to_string(&a.approval_rules)?)
    .bind(a.context_budget)
    .bind(a.limits.max_steps).bind(a.limits.max_run_secs).bind(a.limits.max_run_tokens).bind(a.limits.review)
    .bind(a.limits.monthly_usd).bind(a.limits.cpus).bind(a.limits.memory_mb)
    .execute(db)
        .await?
        .last_insert_rowid();
    Ok(id)
}

pub async fn update_agent(db: &SqlitePool, a: &Agent) -> Result<()> {
    sqlx::query("UPDATE agents SET name = ?, role_prompt = ?, folders = ?, allowed_domains = ?, approval_rules = ?, context_budget = ?, model_ref = ?,
                 max_steps = ?, max_run_secs = ?, max_run_tokens = ?, review = ?, monthly_usd = ?, cpus = ?, memory_mb = ? WHERE id = ?")
        .bind(&a.name).bind(&a.role_prompt)
        .bind(serde_json::to_string(&a.folders)?)
        .bind(serde_json::to_string(&a.allowed_domains)?)
        .bind(serde_json::to_string(&a.approval_rules)?)
        .bind(a.context_budget)
        .bind(&a.model_ref)
        .bind(a.limits.max_steps).bind(a.limits.max_run_secs).bind(a.limits.max_run_tokens).bind(a.limits.review)
        .bind(a.limits.monthly_usd).bind(a.limits.cpus).bind(a.limits.memory_mb)
        .bind(a.id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn start_run(db: &SqlitePool, agent_id: i64, trigger: &str, prompt: &str) -> Result<i64> {
    Ok(sqlx::query("INSERT INTO runs (agent_id, trigger, prompt) VALUES (?, ?, ?)")
        .bind(agent_id)
        .bind(trigger)
        .bind(prompt)
        .execute(db)
        .await?
        .last_insert_rowid())
}

/// Close the agent's open run(s) with `status`.
pub async fn end_runs(db: &SqlitePool, agent_id: i64, status: &str) -> Result<()> {
    sqlx::query("UPDATE runs SET status = ?, ended_at = CURRENT_TIMESTAMP WHERE agent_id = ? AND status = 'running'")
        .bind(status)
        .bind(agent_id)
        .execute(db)
        .await?;
    Ok(())
}

/// After a restart nothing is running and nobody is waiting on an approval.
pub async fn recover(db: &SqlitePool) -> Result<()> {
    sqlx::query("UPDATE runs SET status = 'interrupted', ended_at = CURRENT_TIMESTAMP WHERE status = 'running'")
        .execute(db)
        .await?;
    sqlx::query("UPDATE approvals SET decision = 'expired', decided_at = CURRENT_TIMESTAMP WHERE decision IS NULL")
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete_agent(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM agents WHERE id = ?").bind(id).execute(db).await?;
    sqlx::query("DELETE FROM notes WHERE agent_id = ?").bind(id).execute(db).await?; // FTS tables have no FKs
    Ok(())
}

pub async fn add_message(db: &SqlitePool, agent_id: i64, msg: &Value) -> Result<()> {
    sqlx::query("INSERT INTO messages (agent_id, body) VALUES (?, ?)")
        .bind(agent_id)
        .bind(msg.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// Full history for the UI, including compacted messages and summaries.
pub async fn messages(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    sqlx::query("SELECT body FROM messages WHERE agent_id = ? ORDER BY id")
        .bind(agent_id)
        .fetch_all(db)
        .await?
        .iter()
        .map(|r| Ok(serde_json::from_str(r.get("body"))?))
        .collect()
}

/// What the model still sees: (id, message) pairs not yet compacted.
pub async fn live_messages(db: &SqlitePool, agent_id: i64) -> Result<Vec<(i64, Value)>> {
    sqlx::query("SELECT id, body FROM messages WHERE agent_id = ? AND compacted_at IS NULL ORDER BY id")
        .bind(agent_id)
        .fetch_all(db)
        .await?
        .iter()
        .map(|r| Ok((r.get("id"), serde_json::from_str(r.get("body"))?)))
        .collect()
}

pub async fn compact_upto(db: &SqlitePool, agent_id: i64, last_id: i64) -> Result<()> {
    sqlx::query("UPDATE messages SET compacted_at = CURRENT_TIMESTAMP WHERE agent_id = ? AND id <= ? AND compacted_at IS NULL")
        .bind(agent_id)
        .bind(last_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn runs(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM runs WHERE agent_id = ? ORDER BY id DESC LIMIT 200").bind(agent_id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| serde_json::json!({"id": r.get::<i64, _>("id"), "started_at": r.get::<String, _>("started_at"),
        "ended_at": r.get::<Option<String>, _>("ended_at"), "status": r.get::<String, _>("status"),
        "trigger": r.get::<String, _>("trigger")})).collect())
}

pub async fn add_model_call(db: &SqlitePool, agent_id: i64, run_id: i64, model: &str, usage: &Value, ms: i64, cost: Option<f64>) -> Result<()> {
    sqlx::query("INSERT INTO model_calls (agent_id, run_id, model, prompt_tokens, completion_tokens, ms, cost_usd) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(agent_id).bind(run_id).bind(model)
        .bind(usage["prompt_tokens"].as_i64()).bind(usage["completion_tokens"].as_i64()).bind(ms).bind(cost)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn model_calls(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM model_calls WHERE agent_id = ? ORDER BY id").bind(agent_id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| serde_json::json!({"run_id": r.get::<i64, _>("run_id"), "model": r.get::<String, _>("model"),
        "prompt_tokens": r.get::<Option<i64>, _>("prompt_tokens"), "completion_tokens": r.get::<Option<i64>, _>("completion_tokens"),
        "ms": r.get::<i64, _>("ms"), "cost_usd": r.get::<Option<f64>, _>("cost_usd"), "created_at": r.get::<String, _>("created_at")})).collect())
}

/// Cloud spend this calendar month (UTC), for the budget check and the cost meter.
pub async fn month_cost(db: &SqlitePool, agent_id: i64) -> Result<f64> {
    Ok(sqlx::query_scalar("SELECT COALESCE(SUM(cost_usd), 0.0) FROM model_calls WHERE agent_id = ? AND created_at >= strftime('%Y-%m-01', 'now')")
        .bind(agent_id).fetch_one(db).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encrypted_at_rest_and_migrates_plaintext() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("b.db");
        // a phase-2 style plaintext DB with data
        let plain = SqlitePool::connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(true)).await.unwrap();
        sqlx::query("CREATE TABLE marker (v TEXT); INSERT INTO marker VALUES ('kept')").execute(&plain).await.unwrap();
        plain.close().await;
        assert!(is_plaintext(&path).unwrap());

        let key = "ab".repeat(32);
        let db = open(&path, &key).await.unwrap();
        let v: String = sqlx::query_scalar("SELECT v FROM marker").fetch_one(&db).await.unwrap();
        assert_eq!(v, "kept");
        db.close().await;
        assert!(!is_plaintext(&path).unwrap());
        assert!(!std::fs::read(&path).unwrap().windows(4).any(|w| w == b"kept"));
        assert!(open(&path, &"cd".repeat(32)).await.is_err(), "wrong key must fail");
    }
}
