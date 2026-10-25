//! Backup: one SQLCipher file keyed by a passphrase, holding the DB, the undo snapshots and the
//! DB key (snapshots are encrypted with a key derived from it). Keychain secrets stay out.
//! Import and wipe replace the live data, then shut the daemon down; the app starts a fresh one.
use crate::agent::{self, State};
use anyhow::{Context, Result};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};
use std::path::Path;
use std::sync::Arc;

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub async fn export(st: &State, path: &Path, pass: &str) -> Result<()> {
    anyhow::ensure!(pass.chars().count() >= 8, "use a passphrase of at least 8 characters");
    anyhow::ensure!(path.is_absolute() && !path.exists(), "{} must be a new absolute path", path.display());
    let key = st.vault.db_key().await?;
    let mut conn = st.db.acquire().await?;
    sqlx::query(&format!("ATTACH DATABASE {} AS bak KEY {}", quote(&path.display().to_string()), quote(pass)))
        .execute(&mut *conn).await?;
    let res = async {
        sqlx::query("SELECT sqlcipher_export('bak')").execute(&mut *conn).await?;
        sqlx::query("CREATE TABLE bak.backup_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)").execute(&mut *conn).await?;
        sqlx::query("INSERT INTO bak.backup_meta VALUES ('db_key', ?)").bind(&key).execute(&mut *conn).await?;
        sqlx::query("CREATE TABLE bak.backup_files (name TEXT PRIMARY KEY, data BLOB NOT NULL)").execute(&mut *conn).await?;
        for e in std::fs::read_dir(st.snapshots()).into_iter().flatten() {
            let e = e?;
            sqlx::query("INSERT INTO bak.backup_files VALUES (?, ?)")
                .bind(e.file_name().to_string_lossy().to_string()).bind(std::fs::read(e.path())?).execute(&mut *conn).await?;
        }
        anyhow::Ok(())
    }.await;
    sqlx::query("DETACH DATABASE bak").execute(&mut *conn).await?;
    if res.is_err() {
        let _ = std::fs::remove_file(path);
    }
    res
}

/// Stop every agent and close the DB so its files can be replaced.
async fn quiesce(st: &Arc<State>) -> Result<()> {
    for id in st.busy() {
        agent::kill(st, id).await?;
    }
    st.db.close().await;
    Ok(())
}

fn remove_db(home: &Path) {
    for f in ["burrow.db", "burrow.db-wal", "burrow.db-shm"] {
        let _ = std::fs::remove_file(home.join(f));
    }
}

/// Replace all agents, history and snapshots with the backup's.
pub async fn import(st: &Arc<State>, path: &Path, pass: &str) -> Result<()> {
    let mut bak = SqliteConnectOptions::new().filename(path).pragma("key", quote(pass)).read_only(true).connect().await?;
    let key: String = sqlx::query_scalar("SELECT value FROM backup_meta WHERE key = 'db_key'").fetch_one(&mut bak).await
        .context("wrong passphrase, or not a Burrow backup")?;
    anyhow::ensure!(key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()), "backup has a bad database key");
    bak.close().await?;
    // decrypt into a staged DB under the backup's own key, so its snapshots stay readable
    let staged = st.home.join("burrow.db.import");
    let _ = std::fs::remove_file(&staged);
    // ATTACH inherits the open flags, so allow create (the backup itself isn't written)
    let mut bak = SqliteConnectOptions::new().filename(path).pragma("key", quote(pass)).create_if_missing(true).connect().await?;
    sqlx::query(&format!("ATTACH DATABASE {} AS live KEY \"x'{key}'\"", quote(&staged.display().to_string()))).execute(&mut bak).await?;
    sqlx::query("SELECT sqlcipher_export('live')").execute(&mut bak).await?;
    sqlx::query("DROP TABLE live.backup_meta").execute(&mut bak).await?;
    let files: Vec<(String, Vec<u8>)> = sqlx::query_as("SELECT name, data FROM live.backup_files").fetch_all(&mut bak).await?;
    sqlx::query("DROP TABLE live.backup_files").execute(&mut bak).await?;
    sqlx::query("DETACH DATABASE live").execute(&mut bak).await?;
    bak.close().await?;
    for (name, _) in &files {
        anyhow::ensure!(!name.contains(['/', '\\']) && name != ".." , "bad snapshot name `{name}` in backup");
    }

    quiesce(st).await?;
    remove_db(&st.home);
    std::fs::rename(&staged, st.home.join("burrow.db"))?;
    let snaps = st.snapshots();
    let _ = std::fs::remove_dir_all(&snaps);
    std::fs::create_dir_all(&snaps)?;
    for (name, data) in files {
        std::fs::write(snaps.join(name), data)?;
    }
    st.vault.set("db-key", &key).await?;
    st.shutdown.notify_one();
    Ok(())
}

/// Delete everything Burrow stores about you: DB, snapshots, scratch folders, crash reports and
/// keychain entries. Downloaded models stay (they hold nothing personal).
pub async fn wipe(st: &Arc<State>) -> Result<()> {
    let mut keys: Vec<String> = sqlx::query_scalar("SELECT name FROM secrets").fetch_all(&st.db).await?
        .into_iter().map(|n: String| format!("secret-{n}")).collect();
    let conns: Vec<String> = sqlx::query_scalar("SELECT name FROM connections").fetch_all(&st.db).await?;
    keys.extend(conns.into_iter().map(|n| format!("conn-{n}")));
    keys.extend(["db-key", "search-brave", "provider-anthropic", "provider-openai"].map(String::from));
    quiesce(st).await?;
    remove_db(&st.home);
    for d in ["snapshots", "scratch", "crashes"] {
        let _ = std::fs::remove_dir_all(st.home.join(d));
    }
    for k in keys {
        st.vault.delete(&k).await?;
    }
    st.shutdown.notify_one();
    Ok(())
}
