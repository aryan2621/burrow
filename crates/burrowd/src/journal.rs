//! Append-only, hash-chained action log plus the file undo engine.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};

/// Key for undo snapshots, derived from the DB key at startup. Unset (tests), snapshots stay plaintext.
pub static SNAP_KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

fn aead() -> Option<aws_lc_rs::aead::LessSafeKey> {
    use aws_lc_rs::aead::{LessSafeKey, UnboundKey, CHACHA20_POLY1305};
    SNAP_KEY.get().map(|k| LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, k).unwrap()))
}

/// Encrypted snapshot file: 12-byte random nonce, then ciphertext + tag.
fn seal(key: &aws_lc_rs::aead::LessSafeKey, plain: &[u8]) -> Result<Vec<u8>> {
    use aws_lc_rs::aead::{Aad, Nonce};
    let mut n = [0u8; 12];
    getrandom::fill(&mut n).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut buf = plain.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(n), Aad::empty(), &mut buf).map_err(|_| anyhow::anyhow!("seal failed"))?;
    Ok([n.as_slice(), &buf].concat())
}

fn unseal(key: &aws_lc_rs::aead::LessSafeKey, mut b: Vec<u8>) -> Result<Vec<u8>> {
    use aws_lc_rs::aead::{Aad, Nonce};
    anyhow::ensure!(b.len() >= 12, "snapshot is corrupt");
    let n: [u8; 12] = b[..12].try_into()?;
    let plain = key.open_in_place(Nonce::assume_unique_for_key(n), Aad::empty(), &mut b[12..])
        .map_err(|_| anyhow::anyhow!("snapshot is corrupt or the key changed"))?;
    Ok(plain.to_vec())
}

/// Serializes appends so two agents can't both chain onto the same `prev_hash`.
static APPEND: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct Entry<'a> {
    pub agent_id: i64,
    pub run_id: i64,
    pub tool: &'a str,
    pub input: &'a Value,
    pub output: &'a str,
    pub inverse: Option<&'a Value>,
    pub decision: &'a str,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The hashed form of a row. serde_json maps are sorted, so this is deterministic.
fn body(agent_id: i64, run_id: i64, tool: &str, input: &str, output: &str, inverse: Option<&str>, decision: &str, at: i64) -> String {
    json!({"agent_id": agent_id, "run_id": run_id, "tool": tool, "input": input, "output": output,
           "inverse": inverse, "decision": decision, "created_at": at})
    .to_string()
}

pub async fn record(db: &SqlitePool, e: Entry<'_>) -> Result<i64> {
    let _g = APPEND.lock().await;
    let prev: String = sqlx::query_scalar("SELECT hash FROM actions ORDER BY id DESC LIMIT 1")
        .fetch_optional(db)
        .await?
        .unwrap_or_default();
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64;
    let input = e.input.to_string();
    let inverse = e.inverse.map(|v| v.to_string());
    let b = body(e.agent_id, e.run_id, e.tool, &input, e.output, inverse.as_deref(), e.decision, at);
    let hash = sha256(format!("{prev}\n{b}").as_bytes());
    let id = sqlx::query(
        "INSERT INTO actions (agent_id, run_id, tool, input, output, inverse, decision, created_at, prev_hash, hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(e.agent_id).bind(e.run_id).bind(e.tool).bind(&input).bind(e.output)
    .bind(&inverse).bind(e.decision).bind(at).bind(&prev).bind(&hash)
    .execute(db)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// Walk the whole chain. Returns the number of entries, or the first broken id.
pub async fn verify(db: &SqlitePool) -> Result<i64> {
    let mut prev = String::new();
    let mut n = 0;
    for r in sqlx::query("SELECT * FROM actions ORDER BY id").fetch_all(db).await? {
        let id: i64 = r.get("id");
        let b = body(r.get("agent_id"), r.get("run_id"), r.get("tool"), r.get("input"), r.get("output"),
                     r.get("inverse"), r.get("decision"), r.get("created_at"));
        let hash = sha256(format!("{prev}\n{b}").as_bytes());
        if r.get::<String, _>("prev_hash") != prev || r.get::<String, _>("hash") != hash {
            bail!("journal broken at action {id}");
        }
        prev = hash;
        n += 1;
    }
    Ok(n)
}

pub async fn list(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        "SELECT a.*, u.at AS undone_at FROM actions a LEFT JOIN undos u ON u.action_id = a.id
         WHERE a.agent_id = ? ORDER BY a.id",
    )
    .bind(agent_id)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| json!({
        "id": r.get::<i64, _>("id"), "run_id": r.get::<i64, _>("run_id"), "tool": r.get::<String, _>("tool"),
        "input": serde_json::from_str::<Value>(r.get("input")).unwrap_or_default(),
        "output": r.get::<String, _>("output"), "undoable": r.get::<Option<String>, _>("inverse").is_some(),
        "decision": r.get::<String, _>("decision"), "created_at": r.get::<i64, _>("created_at"),
        "undone_at": r.get::<Option<String>, _>("undone_at"),
    })).collect())
}

/// Copy `path` (if it exists) into the snapshot store before a change.
/// Returns the inverse that puts it back. `after` is the content the change will leave
/// (None = file removed), used to detect edits made after the agent's.
pub fn snapshot(snapshots: &Path, path: &Path, after: Option<&[u8]>) -> Result<Value> {
    let before = match std::fs::read(path) {
        Ok(bytes) => {
            let h = sha256(&bytes);
            std::fs::create_dir_all(snapshots)?;
            match aead() {
                Some(k) => {
                    let dst = snapshots.join(format!("{h}.enc"));
                    if !dst.exists() {
                        std::fs::write(&dst, seal(&k, &bytes)?)?;
                    }
                }
                None => {
                    let dst = snapshots.join(&h);
                    if !dst.exists() {
                        std::fs::copy(path, &dst)?; // APFS clonefile on macOS
                    }
                }
            }
            Some(h)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    Ok(json!({"path": path, "snapshot": before, "after_hash": after.map(sha256)}))
}

fn current_hash(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(sha256(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn undo(db: &SqlitePool, snapshots: &Path, action_id: i64, force: bool) -> Result<()> {
    let r = sqlx::query("SELECT a.inverse, u.action_id AS undone FROM actions a LEFT JOIN undos u ON u.action_id = a.id WHERE a.id = ?")
        .bind(action_id)
        .fetch_optional(db)
        .await?
        .with_context(|| format!("no action {action_id}"))?;
    if r.get::<Option<i64>, _>("undone").is_some() {
        bail!("action {action_id} is already undone");
    }
    let inv: Value = serde_json::from_str(&r.get::<Option<String>, _>("inverse").context("this action can't be undone")?)?;
    if let Some(note) = inv["note_id"].as_i64() {
        crate::notes::delete(db, note).await?;
        sqlx::query("INSERT INTO undos (action_id) VALUES (?)").bind(action_id).execute(db).await?;
        return Ok(());
    }
    let path = PathBuf::from(inv["path"].as_str().context("bad inverse")?);
    if !force && current_hash(&path)? != inv["after_hash"].as_str().map(String::from) {
        bail!("{} changed after the agent touched it; undo with force to overwrite", path.display());
    }
    match inv["snapshot"].as_str() {
        Some(h) => match (aead(), std::fs::read(snapshots.join(format!("{h}.enc")))) {
            (Some(k), Ok(b)) => std::fs::write(&path, unseal(&k, b)?)?,
            // snapshots taken before encryption existed
            _ => { std::fs::copy(snapshots.join(h), &path)?; }
        },
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        },
    }
    sqlx::query("INSERT INTO undos (action_id) VALUES (?)").bind(action_id).execute(db).await?;
    Ok(())
}

/// Undo a run's undoable actions newest first. Stops at the first failure.
pub async fn undo_run(db: &SqlitePool, snapshots: &Path, run_id: i64, force: bool) -> Result<usize> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT a.id FROM actions a LEFT JOIN undos u ON u.action_id = a.id
         WHERE a.run_id = ? AND a.inverse IS NOT NULL AND u.action_id IS NULL ORDER BY a.id DESC",
    )
    .bind(run_id)
    .fetch_all(db)
    .await?;
    for (n, id) in ids.iter().enumerate() {
        undo(db, snapshots, *id, force).await.with_context(|| format!("undid {n} of {} actions", ids.len()))?;
    }
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db(dir: &Path) -> SqlitePool {
        crate::store::open(&dir.join("t.db"), &"ab".repeat(32)).await.unwrap()
    }

    fn entry<'a>(input: &'a Value, inverse: Option<&'a Value>) -> Entry<'a> {
        Entry { agent_id: 1, run_id: 1, tool: "write_file", input, output: "ok", inverse, decision: "auto" }
    }

    #[tokio::test]
    async fn chain_verifies_and_is_append_only() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db(tmp.path()).await;
        let input = json!({"path": "x"});
        for _ in 0..3 {
            record(&db, entry(&input, None)).await.unwrap();
        }
        assert_eq!(verify(&db).await.unwrap(), 3);
        assert!(sqlx::query("UPDATE actions SET output = 'forged' WHERE id = 2").execute(&db).await.is_err());
        assert!(sqlx::query("DELETE FROM actions WHERE id = 3").execute(&db).await.is_err());

        // tampering that bypasses the triggers is still caught by the chain
        sqlx::query("DROP TRIGGER actions_no_update").execute(&db).await.unwrap();
        sqlx::query("UPDATE actions SET output = 'forged' WHERE id = 2").execute(&db).await.unwrap();
        assert!(verify(&db).await.unwrap_err().to_string().contains("action 2"));
    }

    #[tokio::test]
    async fn file_undo() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db(tmp.path()).await;
        let snaps = tmp.path().join("snaps");
        let input = json!({});
        let f = tmp.path().join("notes.txt");
        std::fs::write(&f, "old").unwrap();

        // overwrite → undo restores
        let inv = snapshot(&snaps, &f, Some(b"new")).unwrap();
        std::fs::write(&f, "new").unwrap();
        let a = record(&db, entry(&input, Some(&inv))).await.unwrap();
        undo(&db, &snaps, a, false).await.unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "old");
        assert!(undo(&db, &snaps, a, false).await.is_err(), "double undo");

        // new file → undo deletes
        let g = tmp.path().join("new.txt");
        let inv = snapshot(&snaps, &g, Some(b"hi")).unwrap();
        std::fs::write(&g, "hi").unwrap();
        let b = record(&db, entry(&input, Some(&inv))).await.unwrap();
        undo(&db, &snaps, b, false).await.unwrap();
        assert!(!g.exists());

        // someone edited after the agent → refuse unless forced
        let inv = snapshot(&snaps, &f, Some(b"agent")).unwrap();
        std::fs::write(&f, "user edit").unwrap();
        let c = record(&db, entry(&input, Some(&inv))).await.unwrap();
        assert!(undo(&db, &snaps, c, false).await.is_err());
        undo(&db, &snaps, c, true).await.unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "old");

        // run undo: two writes to one file undone newest first
        let inv1 = snapshot(&snaps, &f, Some(b"v1")).unwrap();
        std::fs::write(&f, "v1").unwrap();
        record(&db, Entry { run_id: 2, ..entry(&input, Some(&inv1)) }).await.unwrap();
        let inv2 = snapshot(&snaps, &f, Some(b"v2")).unwrap();
        std::fs::write(&f, "v2").unwrap();
        record(&db, Entry { run_id: 2, ..entry(&input, Some(&inv2)) }).await.unwrap();
        assert_eq!(undo_run(&db, &snaps, 2, false).await.unwrap(), 2);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "old");
    }
}
