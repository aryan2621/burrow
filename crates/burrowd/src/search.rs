//! `web_search`: the Brave Search API (key in the keychain) or a SearXNG instance.
//! With neither configured the tool isn't offered.
use crate::secrets::Vault;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::time::Duration;

const BRAVE_KEY: &str = "search-brave";
const HITS: usize = 8;

pub enum Backend {
    Brave(String),
    Searx(String),
}

/// SearXNG wins when both are set: it's the fully local option.
pub async fn backend(db: &SqlitePool, vault: &Vault) -> Result<Option<Backend>> {
    let url: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'searxng_url'").fetch_optional(db).await?;
    if let Some(u) = url {
        return Ok(Some(Backend::Searx(u)));
    }
    Ok(vault.get(BRAVE_KEY).await?.map(Backend::Brave))
}

pub fn name(b: &Option<Backend>) -> Value {
    match b {
        Some(Backend::Brave(_)) => json!("brave"),
        Some(Backend::Searx(_)) => json!("searxng"),
        None => Value::Null,
    }
}

/// `search.configure`: a string sets a backend, null clears it, a missing key leaves it alone.
pub async fn configure(db: &SqlitePool, vault: &Vault, p: &Value) -> Result<()> {
    match &p["searxng_url"] {
        Value::String(u) => {
            let parsed = url::Url::parse(u).context("bad SearXNG URL")?;
            anyhow::ensure!(matches!(parsed.scheme(), "http" | "https"), "SearXNG URL must be http(s)");
            sqlx::query("INSERT INTO settings (key, value) VALUES ('searxng_url', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(u.trim_end_matches('/')).execute(db).await?;
        }
        Value::Null if p.get("searxng_url").is_some() => {
            sqlx::query("DELETE FROM settings WHERE key = 'searxng_url'").execute(db).await?;
        }
        _ => {}
    }
    match &p["brave_key"] {
        Value::String(k) => vault.set(BRAVE_KEY, k).await?,
        Value::Null if p.get("brave_key").is_some() => vault.delete(BRAVE_KEY).await?,
        _ => {}
    }
    Ok(())
}

/// Results as a short list, wrapped in `<untrusted>` since titles and snippets are web content.
pub async fn search(http: &reqwest::Client, b: &Backend, query: &str) -> Result<String> {
    let (req, list_ptr, snippet) = match b {
        Backend::Brave(key) => (
            http.get("https://api.search.brave.com/res/v1/web/search").header("X-Subscription-Token", key)
                .query(&[("q", query), ("count", &HITS.to_string())]),
            "/web/results", "description",
        ),
        Backend::Searx(base) => (http.get(format!("{base}/search")).query(&[("q", query), ("format", "json")]), "/results", "content"),
    };
    let v: Value = req.timeout(Duration::from_secs(15)).send().await?.error_for_status()?.json().await?;
    let hits = v.pointer(list_ptr).and_then(Value::as_array).cloned().unwrap_or_default();
    let lines: Vec<String> = hits.iter().take(HITS).map(|h| {
        let s = |k: &str| h[k].as_str().unwrap_or("").to_string();
        format!("- {}\n  {}\n  {}", s("title"), s("url"), s(snippet))
    }).collect();
    let body = if lines.is_empty() { "no results".into() } else { lines.join("\n") };
    Ok(format!("<untrusted source=\"web_search\">\n{body}\n</untrusted>"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn searxng_results_are_wrapped() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).await.unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("GET /search?q=rust+lang&format=json"));
            let body = json!({"results": [{"title": "Rust", "url": "https://rust-lang.org", "content": "A language"}]}).to_string();
            s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let out = search(&reqwest::Client::new(), &Backend::Searx(base), "rust lang").await.unwrap();
        assert!(out.starts_with("<untrusted") && out.contains("https://rust-lang.org") && out.contains("A language"), "{out}");
    }
}
