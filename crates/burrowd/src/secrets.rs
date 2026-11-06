//! Secrets: values live in the OS keychain (or a 0600 file store as a fallback), the DB
//! only records which agents and hosts may use them. The model never sees a value: the
//! daemon adds it to outgoing requests and scrubs it from anything a tool returns.
use crate::fetch;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use std::path::PathBuf;

const SERVICE: &str = "dev.burrow";

pub enum Vault {
    /// macOS Keychain, Windows Credential Manager, Linux Secret Service.
    Keychain,
    /// One 0600 file per entry. For machines without a keychain, CI and tests.
    File(PathBuf),
}

impl Vault {
    /// `BURROW_KEYSTORE=file` forces the file store; otherwise use the keychain if it works.
    pub async fn detect(home: &std::path::Path) -> Vault {
        let file = Vault::File(home.join("keys"));
        if std::env::var("BURROW_KEYSTORE").as_deref() == Ok("file") {
            return file;
        }
        let probe = tokio::task::spawn_blocking(|| {
            let e = keyring::Entry::new(SERVICE, "probe")?;
            e.set_password("ok")?;
            e.delete_credential()
        })
        .await;
        match probe {
            Ok(Ok(())) => Vault::Keychain,
            _ => {
                eprintln!("warning: no OS keychain available; secrets are stored in {}/keys (0600)", home.display());
                file
            }
        }
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        match self {
            Vault::Keychain => {
                let key = key.to_string();
                tokio::task::spawn_blocking(move || match keyring::Entry::new(SERVICE, &key)?.get_password() {
                    Ok(v) => Ok(Some(v)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(e.into()),
                })
                .await?
            }
            Vault::File(dir) => match std::fs::read_to_string(dir.join(file_name(key))) {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
        }
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        match self {
            Vault::Keychain => {
                let (key, value) = (key.to_string(), value.to_string());
                tokio::task::spawn_blocking(move || keyring::Entry::new(SERVICE, &key)?.set_password(&value)).await??;
            }
            Vault::File(dir) => {
                std::fs::create_dir_all(dir)?;
                let path = dir.join(file_name(key));
                std::fs::write(&path, value)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                }
            }
        }
        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        match self {
            Vault::Keychain => {
                let key = key.to_string();
                tokio::task::spawn_blocking(move || match keyring::Entry::new(SERVICE, &key)?.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(anyhow::Error::from(e)),
                })
                .await??;
            }
            Vault::File(dir) => match std::fs::remove_file(dir.join(file_name(key))) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            },
        }
        Ok(())
    }

    /// The database key, created on first use.
    pub async fn db_key(&self) -> Result<String> {
        if let Some(k) = self.get("db-key").await? {
            return Ok(k);
        }
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("{e}"))?;
        let k: String = b.iter().map(|x| format!("{x:02x}")).collect();
        self.set("db-key", &k).await?;
        Ok(k)
    }
}

fn file_name(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

fn secret_key(name: &str) -> String {
    format!("secret-{name}")
}

pub struct Bound {
    pub name: String,
    pub value: String,
    pub hosts: Vec<String>,
    /// `Name: template` with `{}` where the value goes.
    pub header: String,
}

impl Bound {
    pub fn header_pair(&self) -> Option<(String, String)> {
        let (n, v) = self.header.split_once(':')?;
        Some((n.trim().to_string(), v.trim().replace("{}", &self.value)))
    }
}

pub async fn set(db: &SqlitePool, vault: &Vault, p: &Value) -> Result<()> {
    let name = p["name"].as_str().context("missing `name`")?;
    anyhow::ensure!(!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
        "secret names use letters, digits, - and _");
    let header = p["header"].as_str().unwrap_or("Authorization: Bearer {}");
    anyhow::ensure!(header.contains(':') && header.contains("{}"), "header must look like `Name: ...{{}}...`");
    let hosts = p["hosts"].as_array().context("missing `hosts`")?;
    let ids: Vec<i64> = serde_json::from_value(p["agent_ids"].clone()).context("`agent_ids` must be a list of agent ids")?;
    anyhow::ensure!(!hosts.is_empty(), "bind the secret to at least one host");
    if let Some(v) = p["value"].as_str() {
        anyhow::ensure!(v.len() >= 4, "secret value too short");
        vault.set(&secret_key(name), v).await?;
    } else if vault.get(&secret_key(name)).await?.is_none() {
        bail!("missing `value`");
    }
    // short-lived: the binding stops working after this many hours (a leaked key is one agent, one scope, a short time)
    let expires = match p["expires_in_hours"].as_f64() {
        Some(h) if h > 0.0 => Some(format!("+{} seconds", (h * 3600.0) as i64)),
        _ => None,
    };
    sqlx::query("INSERT INTO secrets (name, agent_ids, hosts, header, expires_at) VALUES (?, ?, ?, ?, datetime('now', ?))
                 ON CONFLICT(name) DO UPDATE SET agent_ids = excluded.agent_ids, hosts = excluded.hosts, header = excluded.header,
                 expires_at = excluded.expires_at")
        .bind(name).bind(json!(ids).to_string()).bind(json!(hosts).to_string()).bind(header).bind(expires)
        .execute(db).await?;
    Ok(())
}

/// Names and bindings only, never values.
pub async fn list(db: &SqlitePool) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM secrets ORDER BY name").fetch_all(db).await?;
    rows.iter().map(|r| Ok(json!({"name": r.get::<String, _>("name"),
        "agent_ids": serde_json::from_str::<Value>(r.get("agent_ids"))?,
        "hosts": serde_json::from_str::<Value>(r.get("hosts"))?, "header": r.get::<String, _>("header"),
        "expires_at": r.get::<Option<String>, _>("expires_at"),
        "suspended": serde_json::from_str::<Value>(r.get("suspended"))?}))).collect()
}

pub async fn delete(db: &SqlitePool, vault: &Vault, name: &str) -> Result<()> {
    vault.delete(&secret_key(name)).await?;
    sqlx::query("DELETE FROM secrets WHERE name = ?").bind(name).execute(db).await?;
    Ok(())
}

/// Kill switch: move an agent's bindings to `suspended`; they do nothing until `restore`.
pub async fn revoke(db: &SqlitePool, agent_id: i64) -> Result<()> {
    shift(db, agent_id, "agent_ids", "suspended").await
}

/// The user's explicit "give it its secrets back" after a kill.
pub async fn restore(db: &SqlitePool, agent_id: i64) -> Result<()> {
    shift(db, agent_id, "suspended", "agent_ids").await
}

async fn shift(db: &SqlitePool, agent_id: i64, from: &str, to: &str) -> Result<()> {
    for r in sqlx::query("SELECT name, agent_ids, suspended FROM secrets").fetch_all(db).await? {
        let mut src: Vec<i64> = serde_json::from_str(r.get(from)).unwrap_or_default();
        let mut dst: Vec<i64> = serde_json::from_str(r.get(to)).unwrap_or_default();
        if src.contains(&agent_id) {
            src.retain(|i| *i != agent_id);
            if !dst.contains(&agent_id) {
                dst.push(agent_id);
            }
            // column names are the two constants above, never user input
            sqlx::query(&format!("UPDATE secrets SET {from} = ?, {to} = ? WHERE name = ?"))
                .bind(json!(src).to_string()).bind(json!(dst).to_string()).bind(r.get::<String, _>("name")).execute(db).await?;
        }
    }
    Ok(())
}

/// Secrets this agent may use, with values.
pub async fn for_agent(db: &SqlitePool, vault: &Vault, agent_id: i64) -> Result<Vec<Bound>> {
    let mut out = vec![];
    for r in sqlx::query("SELECT * FROM secrets WHERE expires_at IS NULL OR expires_at > datetime('now')").fetch_all(db).await? {
        let ids: Vec<i64> = serde_json::from_str(r.get("agent_ids")).unwrap_or_default();
        if !ids.contains(&agent_id) {
            continue;
        }
        let name: String = r.get("name");
        if let Some(value) = vault.get(&secret_key(&name)).await? {
            out.push(Bound { name, value, hosts: serde_json::from_str(r.get("hosts"))?, header: r.get("header") });
        }
    }
    Ok(out)
}

/// Secrets to attach to a request for `host`.
pub fn for_host<'a>(bound: &'a [Bound], host: &str) -> Vec<&'a Bound> {
    bound.iter().filter(|b| fetch::allowed(&b.hosts, host)).collect()
}

/// Replace any secret value in tool output with a placeholder.
pub fn scrub(bound: &[Bound], text: String) -> String {
    bound.iter().fold(text, |t, b| t.replace(&b.value, &format!("[secret:{}]", b.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_inject_scrub_revoke() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = Vault::File(tmp.path().join("keys"));
        let db = crate::store::open(&tmp.path().join("t.db"), &vault.db_key().await.unwrap()).await.unwrap();
        set(&db, &vault, &json!({"name": "gh", "value": "tok-123", "agent_ids": [1],
            "hosts": ["api.github.com"], "header": "Authorization: Bearer {}"})).await.unwrap();
        assert!(!list(&db).await.unwrap()[0].to_string().contains("tok-123"));

        let b = for_agent(&db, &vault, 1).await.unwrap();
        assert_eq!(for_host(&b, "api.github.com")[0].header_pair().unwrap(), ("Authorization".into(), "Bearer tok-123".into()));
        assert!(for_host(&b, "evil.com").is_empty());
        assert!(for_agent(&db, &vault, 2).await.unwrap().is_empty());
        assert_eq!(scrub(&b, "echo tok-123".into()), "echo [secret:gh]");

        revoke(&db, 1).await.unwrap();
        assert!(for_agent(&db, &vault, 1).await.unwrap().is_empty());
        delete(&db, &vault, "gh").await.unwrap();
        assert!(vault.get("secret-gh").await.unwrap().is_none());
    }
}
