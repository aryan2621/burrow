//! `http_fetch`: GET only, allowlisted hosts only, re-checked on every redirect.
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use reqwest::Url;
use std::time::Duration;

const MAX_BODY: usize = 100 * 1024;

/// `example.com` in the list allows `example.com` and `api.example.com`.
pub fn allowed(list: &[String], host: &str) -> bool {
    let host = host.trim_end_matches('.').to_lowercase();
    list.iter().any(|d| {
        let d = d.trim_end_matches('.').to_lowercase();
        host == d || host.ends_with(&format!(".{d}"))
    })
}

/// Parse and return the host the user must approve. Local and IP-literal hosts are
/// refused unless allowlisted verbatim.
// shortcut: no DNS-rebinding check (a name resolving to 127.0.0.1); add a resolved-IP check with the egress proxy.
pub fn host_of(url: &str, list: &[String]) -> Result<String> {
    let u = Url::parse(url).context("invalid URL")?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("only http and https URLs are allowed");
    }
    let host = u.host_str().context("URL has no host")?.to_lowercase();
    let local = matches!(u.host(), Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)))
        || host == "localhost" || host.ends_with(".localhost");
    if local && !list.iter().any(|d| d.eq_ignore_ascii_case(&host)) {
        bail!("local and IP addresses are blocked");
    }
    Ok(host)
}

/// Fetch `url`. `list` must already contain the host (allowlisted or approved for this call).
/// `headers` carry injected secrets; redirects may then only stay on `secret_hosts`.
pub async fn get(url: &str, list: Vec<String>, headers: Vec<(String, String)>, secret_hosts: Vec<String>) -> Result<String> {
    let hops = list.clone();
    let carrying = !headers.is_empty();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            let host = a.url().host_str().unwrap_or("").to_string();
            if a.previous().len() >= 5 {
                a.error("too many redirects")
            } else if host_of(a.url().as_str(), &hops).is_err() || !allowed(&hops, &host) {
                a.error(format!("redirect to {host} blocked; fetch it directly to ask for approval"))
            } else if carrying && !allowed(&secret_hosts, &host) {
                // custom auth headers survive cross-host redirects in reqwest; don't hand them on
                a.error(format!("redirect to {host} blocked: the request carries a secret bound to other hosts"))
            } else {
                a.follow()
            }
        }))
        .build()?;
    let host = host_of(url, &list)?;
    anyhow::ensure!(allowed(&list, &host), "{host} is not allowed");
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let res = req.send().await?;
    let status = res.status();
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !(ctype.is_empty() || ctype.starts_with("text/") || ctype.contains("json") || ctype.contains("xml")) {
        return Ok(format!("status {status}; {ctype} body skipped (not text)"));
    }
    let mut body = Vec::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
        if body.len() >= MAX_BODY {
            body.truncate(MAX_BODY);
            break;
        }
    }
    Ok(format!("status {status}\n{}", String::from_utf8_lossy(&body)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn matching() {
        let l = vec!["example.com".to_string()];
        assert!(allowed(&l, "example.com"));
        assert!(allowed(&l, "API.example.com."));
        assert!(!allowed(&l, "badexample.com"));
        assert!(!allowed(&l, "example.com.evil.net"));
        assert!(host_of("http://localhost:11434/api", &l).is_err());
        assert!(host_of("http://127.0.0.1/", &l).is_err());
        assert!(host_of("http://[::1]/", &l).is_err());
        assert!(host_of("file:///etc/passwd", &l).is_err());
        assert!(host_of("http://127.0.0.1/", &["127.0.0.1".into()]).is_ok());
        assert_eq!(host_of("https://Docs.Example.com/x", &l).unwrap(), "docs.example.com");
    }

    /// Serves a redirect to `to` on /go (`@self` = this server via `localhost`), else a page.
    async fn server(to: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let to = if to == "@self" { format!("http://localhost:{}/page", addr.port()) } else { to.to_string() };
        tokio::spawn(async move {
            loop {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let resp = if req.starts_with("GET /go") {
                    format!("HTTP/1.1 302 Found\r\nlocation: {to}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                } else {
                    "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok".into()
                };
                s.write_all(resp.as_bytes()).await.unwrap();
            }
        });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn redirects_are_rechecked() {
        let addr = server("http://evil.example.net/").await;
        let list = vec!["127.0.0.1".to_string()];
        assert!(get(&format!("http://{addr}/page"), list.clone(), vec![], vec![]).await.unwrap().ends_with("ok"));
        let err = get(&format!("http://{addr}/go"), list, vec![], vec![]).await.unwrap_err();
        assert!(format!("{err:#}").contains("evil.example.net"), "{err:#}");
    }

    #[tokio::test]
    async fn secrets_do_not_follow_redirects_off_their_host() {
        let addr = server("@self").await;
        let list = vec!["127.0.0.1".to_string(), "localhost".to_string()];
        let auth = vec![("X-Api-Key".to_string(), "s3cret".to_string())];
        // without a secret the hop is fine (both hosts allowlisted)
        assert!(get(&format!("http://{addr}/go"), list.clone(), vec![], vec![]).await.is_ok());
        let err = get(&format!("http://{addr}/go"), list, auth, vec!["127.0.0.1".into()]).await.unwrap_err();
        assert!(format!("{err:#}").contains("carries a secret"), "{err:#}");
    }
}
