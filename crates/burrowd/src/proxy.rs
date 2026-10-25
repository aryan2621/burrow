//! Egress proxy for networked sandboxes: plain HTTP and CONNECT, one listener per agent,
//! guarded by a per-agent token. Hosts are checked against the agent's allowlist, new ones
//! ask the user during a run, DNS is resolved here and private addresses are refused.
//! HTTPS is checked by host only (no TLS interception). Every connection is journaled as `net`.
use crate::agent::State;
use crate::{fetch, journal, policy, store};
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::json;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MAX_HEAD: usize = 16 * 1024;

/// False for loopback, private, link-local, CGNAT, multicast, documentation and other
/// non-routable addresses.
pub fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast()
                || v.is_documentation() || v.is_multicast() || o[0] == 0 || o[0] >= 240
                || (o[0] == 100 && (64..128).contains(&o[1])) || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => public(IpAddr::V4(v4)),
            None => {
                let s = v.segments()[0];
                !(v.is_loopback() || v.is_unspecified() || v.is_multicast() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
                    || s == 0x2001 && v.segments()[1] == 0x0db8)
            }
        },
    }
}

/// Per-agent listener state.
struct Shared {
    agent_id: i64,
    /// Expected `Proxy-Authorization` value.
    auth: String,
    /// (run, host) pairs approved once, so pip's many connections ask once per run.
    approved: Mutex<HashSet<(i64, String)>>,
    /// One approval prompt at a time; parallel connections to a new host wait for its answer.
    asking: tokio::sync::Mutex<()>,
}

/// Start the agent's sandbox with its network and proxy, unless it's already running.
pub async fn ensure(st: &Arc<State>, agent: &store::Agent, scratch: &Path) -> Result<()> {
    if st.sandbox.is_running(agent.id) {
        return Ok(());
    }
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let shared = Arc::new(Shared {
        agent_id: agent.id,
        auth: format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("burrow:{token}"))),
        approved: Default::default(),
        asking: Default::default(),
    });
    let st2 = st.clone();
    st.sandbox.start(agent.id, &agent.folders, scratch, Some(move |ip: IpAddr| async move {
        let l = TcpListener::bind((ip, 0)).await.with_context(|| format!("proxy can't listen on {ip}"))?;
        let port = l.local_addr()?.port();
        Ok((port, token, tokio::spawn(serve(st2, l, shared)).abort_handle()))
    })).await
}

async fn serve(st: Arc<State>, l: TcpListener, shared: Arc<Shared>) {
    while let Ok((c, _)) = l.accept().await {
        let (st, shared) = (st.clone(), shared.clone());
        tokio::spawn(async move {
            let _ = conn(&st, &shared, c).await; // the client sees a status line or a closed socket
        });
    }
}

/// Read up to the end of the request head. Returns the head and any body bytes after it.
async fn read_head(c: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = c.read(&mut chunk).await?;
        if n == 0 {
            bail!("client closed");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Ok((String::from_utf8(buf)?, rest));
        }
        if buf.len() > MAX_HEAD {
            bail!("request head too large");
        }
    }
}

/// A parsed proxy request: where to go and what to send upstream first.
struct Target {
    method: String,
    host: String,
    port: u16,
    /// For plain HTTP: the request rewritten to origin form, minus proxy headers.
    head: Option<String>,
}

fn parse(head: &str) -> Result<(Target, Option<String>)> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target, version) = (first.next().unwrap_or(""), first.next().unwrap_or(""), first.next().unwrap_or("HTTP/1.1"));
    let mut auth = None;
    let mut kept = vec![];
    for l in lines.filter(|l| !l.is_empty()) {
        let (k, v) = l.split_once(':').unwrap_or((l, ""));
        if k.eq_ignore_ascii_case("proxy-authorization") {
            auth = Some(v.trim().to_string());
        } else if !k.to_ascii_lowercase().starts_with("proxy-") {
            kept.push(l);
        }
    }
    let t = if method == "CONNECT" {
        let (h, p) = target.rsplit_once(':').context("CONNECT needs host:port")?;
        Target { method: method.into(), host: h.trim_matches(['[', ']']).to_lowercase(), port: p.parse()?, head: None }
    } else {
        let u = url::Url::parse(target).context("expected an absolute http:// URL")?;
        anyhow::ensure!(u.scheme() == "http", "only http:// is proxied without CONNECT");
        let path = &u[url::Position::BeforePath..];
        let head = format!("{method} {path} {version}\r\n{}\r\n\r\n", kept.join("\r\n"));
        let host = u.host_str().context("URL has no host")?.trim_matches(['[', ']']).to_lowercase();
        Target { method: method.into(), host, port: u.port_or_known_default().unwrap_or(80), head: Some(head) }
    };
    Ok((t, auth))
}

async fn conn(st: &Arc<State>, sh: &Shared, mut c: TcpStream) -> Result<()> {
    let (head, rest) = read_head(&mut c).await?;
    let (t, auth) = match parse(&head) {
        Ok(x) => x,
        Err(e) => {
            c.write_all(format!("HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n{e}").as_bytes()).await?;
            return Ok(());
        }
    };
    if auth.as_deref() != Some(&sh.auth) {
        c.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nproxy-authenticate: Basic realm=\"burrow\"\r\ncontent-length: 0\r\n\r\n").await?;
        return Ok(());
    }
    let run_id: i64 = sqlx::query_scalar("SELECT id FROM runs WHERE agent_id = ? AND status = 'running' ORDER BY id DESC LIMIT 1")
        .bind(sh.agent_id).fetch_optional(&st.db).await?.unwrap_or(0);
    let (output, decision, addr) = match check(st, sh, run_id, &t.host, t.port).await {
        Ok((addr, decision)) => ("connected".to_string(), decision, Some(addr)),
        Err(e) => (format!("blocked: {e:#}"), "denied", None),
    };
    journal::record(&st.db, journal::Entry { agent_id: sh.agent_id, run_id, tool: "net",
        input: &json!({"method": t.method, "host": t.host, "port": t.port}), output: &output, inverse: None, decision }).await?;
    let Some(addr) = addr else {
        c.write_all(format!("HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\r\n{output}", output.len()).as_bytes()).await?;
        return Ok(());
    };
    let mut up = match TcpStream::connect(addr).await {
        Ok(u) => u,
        Err(e) => {
            c.write_all(format!("HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n{e}").as_bytes()).await?;
            return Ok(());
        }
    };
    match &t.head {
        Some(h) => up.write_all(h.as_bytes()).await?,
        None => c.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?,
    }
    up.write_all(&rest).await?;
    tokio::io::copy_bidirectional(&mut c, &mut up).await?;
    Ok(())
}

/// Decide whether the agent may reach `host:port` and pick a public address for it.
async fn check(st: &Arc<State>, sh: &Shared, run_id: i64, host: &str, port: u16) -> Result<(SocketAddr, &'static str)> {
    anyhow::ensure!(port == 80 || port == 443, "only ports 80 and 443 are allowed");
    let agent = store::get_agent(&st.db, sh.agent_id).await?;
    let rule = agent.approval_rules.get("http_fetch").map(String::as_str).unwrap_or(policy::default_rule("http_fetch"));
    anyhow::ensure!(rule != "deny", "web access is denied for this agent");
    let verbatim = agent.allowed_domains.iter().any(|d| d.eq_ignore_ascii_case(host));
    let literal = host.parse::<IpAddr>().is_ok() || host == "localhost" || host.ends_with(".localhost");
    anyhow::ensure!(!literal || verbatim, "local and IP addresses are blocked");

    let key = (run_id, host.to_string());
    let decision = if fetch::allowed(&agent.allowed_domains, host) || rule == "allow" {
        "auto"
    } else if sh.approved.lock().unwrap().contains(&key) {
        "approved"
    } else {
        anyhow::ensure!(run_id != 0, "{host} is not on the allowlist and no run is active to ask the user");
        // shortcut: the shell command keeps its 60 s timeout while this waits; a slow answer fails that command
        // (the approval still counts for the rest of the run). Pause the timeout during approvals if it bites.
        let _one = sh.asking.lock().await;
        // another connection may have asked while this one waited
        let fresh = store::get_agent(&st.db, sh.agent_id).await?;
        if !sh.approved.lock().unwrap().contains(&key) && !fetch::allowed(&fresh.allowed_domains, host) {
            anyhow::ensure!(policy::ask(st, sh.agent_id, run_id, "net", &json!({"host": host})).await?, "{host} denied by user");
        }
        sh.approved.lock().unwrap().insert(key);
        "approved"
    };
    let addrs = tokio::net::lookup_host((host, port)).await.with_context(|| format!("can't resolve {host}"))?;
    let addr = addrs.into_iter().find(|a| verbatim || public(a.ip())).with_context(|| format!("{host} resolves to a private address"))?;
    Ok((addr, decision))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ips() {
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0",
                   "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1", "224.0.0.1"] {
            assert!(!public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn parsing() {
        let (t, auth) = parse("CONNECT pypi.org:443 HTTP/1.1\r\nHost: pypi.org:443\r\nProxy-Authorization: Basic abc\r\n\r\n").unwrap();
        assert_eq!((t.host.as_str(), t.port, auth.as_deref()), ("pypi.org", 443, Some("Basic abc")));
        let (t, _) = parse("GET http://Example.com/a?b=1 HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\n\r\n").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("example.com", 80));
        assert_eq!(t.head.unwrap(), "GET /a?b=1 HTTP/1.1\r\nHost: example.com\r\n\r\n");
        assert!(parse("GET /relative HTTP/1.1\r\n\r\n").is_err());
    }

    /// Full path through a real listener: auth, allowlist, private-IP block, approval.
    #[tokio::test]
    async fn allows_asks_and_blocks() {
        use crate::{sandbox::Sandbox, secrets::Vault};
        let tmp = tempfile::tempdir().unwrap();
        let st = Arc::new(State::new(tmp.path().to_path_buf(), String::new(), String::new(), Sandbox::detect(), Vault::File(tmp.path().join("keys"))).await.unwrap());
        let a = store::Agent { id: 0, name: "t".into(), role_prompt: "".into(), model_ref: "ollama:x".into(), folders: vec![],
            allowed_domains: vec!["localhost".into()], approval_rules: Default::default(), context_budget: 8_000, limits: Default::default() };
        let id = store::create_agent(&st.db, &a).await.unwrap();
        // an "upstream" on loopback, reachable only because `localhost` is allowlisted verbatim
        let up = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_port = up.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = up.accept().await {
                let mut b = [0u8; 1024];
                let n = s.read(&mut b).await.unwrap();
                let first = String::from_utf8_lossy(&b[..n]).lines().next().unwrap_or("").to_string();
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{first}", first.len()).as_bytes()).await;
            }
        });
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = l.local_addr().unwrap();
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("burrow:tok"));
        let sh = Arc::new(Shared { agent_id: id, auth: auth.clone(), approved: Default::default(), asking: Default::default() });
        tokio::spawn(serve(st.clone(), l, sh.clone()));
        let send = |req: String| async move {
            let mut c = TcpStream::connect(proxy).await.unwrap();
            c.write_all(req.as_bytes()).await.unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).await.unwrap();
            out
        };
        let get = |host: &str, auth: &str| format!("GET http://{host}/x HTTP/1.1\r\nHost: {host}\r\nProxy-Authorization: {auth}\r\nConnection: close\r\n\r\n");

        assert!(send(get("localhost", "Basic bad")).await.starts_with("HTTP/1.1 407"));
        // port check comes first: the test upstream isn't on 80, so exercise check() for allowlisted hosts directly
        assert!(send(get(&format!("localhost:{up_port}"), &auth)).await.contains("only ports 80 and 443"));
        let (addr, d) = check(&st, &sh, 0, "localhost", 80).await.unwrap();
        assert!(addr.ip().is_loopback() && d == "auto");
        assert!(format!("{:#}", check(&st, &sh, 0, "127.0.0.1", 80).await.unwrap_err()).contains("blocked"));
        assert!(format!("{:#}", check(&st, &sh, 0, "example.org", 443).await.unwrap_err()).contains("no run is active"));

        // a new host during a run asks; approving once covers the run
        let run_id = store::start_run(&st.db, id, "chat").await.unwrap();
        let mut ev = st.events.subscribe();
        let (st2, sh2) = (st.clone(), sh.clone());
        let asking = tokio::spawn(async move { check(&st2, &sh2, run_id, "localhost.example", 443).await });
        let approval = loop {
            let v = ev.recv().await.unwrap();
            if v["method"] == "approval.requested" { break v["params"]["approval_id"].as_i64().unwrap(); }
        };
        policy::decide(&st, approval, true, false).await.unwrap();
        // resolution fails for the made-up name, but only after approval
        assert!(format!("{:#}", asking.await.unwrap().unwrap_err()).contains("resolve"));
        assert!(sh.approved.lock().unwrap().contains(&(run_id, "localhost.example".into())));
    }

    /// Needs a runtime: `cargo test -- --ignored sandbox_network`.
    #[tokio::test]
    #[ignore]
    async fn sandbox_network() {
        use crate::{sandbox::Sandbox, secrets::Vault};
        use std::time::Duration;
        let tmp = tempfile::tempdir().unwrap();
        let st = Arc::new(State::new(tmp.path().to_path_buf(), String::new(), String::new(), Sandbox::detect(), Vault::File(tmp.path().join("keys"))).await.unwrap());
        let mk = |domains: &[&str]| store::Agent { id: 0, name: "n".into(), role_prompt: "".into(), model_ref: "ollama:x".into(),
            folders: vec![], allowed_domains: domains.iter().map(|d| d.to_string()).collect(), approval_rules: Default::default(), context_budget: 8_000, limits: Default::default() };
        let mut open = mk(&policy::PACKAGE_REGISTRIES);
        open.allowed_domains.push("example.com".into());
        let mut shut = mk(&[]);
        open.id = store::create_agent(&st.db, &open).await.unwrap();
        shut.id = store::create_agent(&st.db, &shut).await.unwrap();
        let sh = |a: store::Agent, cmd: &'static str| { let st = st.clone(); async move {
            let scratch = st.scratch(a.id).unwrap();
            ensure(&st, &a, &scratch).await.unwrap();
            st.sandbox.exec(a.id, &scratch, cmd, Duration::from_secs(120)).await.unwrap()
        }};
        let pip = "pip install --no-cache-dir --target /tmp/p requests -q && echo OK";
        let out = sh(open.clone(), pip).await;
        assert!(out.contains("OK"), "{out}");
        assert!(sh(open.clone(), "python -c \"import urllib.request as u;print(u.urlopen('https://example.com',timeout=10).status)\"").await.contains("200"));
        // not allowlisted and no run to ask in: blocked by the proxy
        let out = sh(open.clone(), "python -c \"import urllib.request as u;u.urlopen('https://wikipedia.org',timeout=10)\"").await;
        assert!(out.contains("403"), "{out}");
        let out = sh(shut.clone(), pip).await;
        assert!(!out.contains("OK"), "{out}");
        // agent B can't ride agent A's proxy without A's token
        let a_proxy = sh(open.clone(), "echo $HTTPS_PROXY").await;
        let a_addr = a_proxy.split('@').nth(1).unwrap().lines().next().unwrap().to_string();
        let cmd: &'static str = Box::leak(format!("python -c \"import urllib.request as u;o=u.build_opener(u.ProxyHandler({{'https':'http://{a_addr}'}}));o.open('https://example.com',timeout=10)\"").into_boxed_str());
        let out = sh(shut.clone(), cmd).await;
        assert!(!out.starts_with("exit: 0"), "{out}");
        st.sandbox.stop_all().await;
        let blocked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM actions WHERE tool = 'net' AND decision = 'denied'").fetch_one(&st.db).await.unwrap();
        assert!(blocked >= 2, "blocked connections are journaled");
    }
}
