//! MCP servers per agent. Stdio servers run inside the agent's sandbox (`exec -i` carries
//! stdio, installs go through the proxy); HTTP servers are called from the daemon, only on
//! allowlisted hosts, with headers from bound secrets. Tools appear as `mcp__<server>__<tool>`.
use crate::agent::State;
use crate::{fetch, proxy, secrets, store};
use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub type Client = RunningService<RoleClient, ()>;

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 20 && !n.contains("__") && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

pub async fn list(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM mcp_servers WHERE agent_id = ? ORDER BY name").bind(agent_id).fetch_all(db).await?;
    rows.iter().map(|r| Ok(json!({"name": r.get::<String, _>("name"), "transport": r.get::<String, _>("transport"),
        "command": serde_json::from_str::<Value>(r.get("command"))?, "url": r.get::<Option<String>, _>("url"),
        "secrets": serde_json::from_str::<Value>(r.get("secrets"))?}))).collect()
}

/// `mcp.add`: {agent_id, name, transport: stdio|http, command: [argv] | url, secrets: [names]}. Replaces a same-named server.
/// Returns Antidote warnings (medium/low findings) for the UI; critical/high findings refuse the add.
pub async fn add(db: &SqlitePool, p: &Value) -> Result<Vec<String>> {
    let agent_id = p["agent_id"].as_i64().context("missing `agent_id`")?;
    let name = p["name"].as_str().context("missing `name`")?;
    anyhow::ensure!(valid_name(name), "server names use up to 20 letters, digits, - and _ (no `__`)");
    let command: Vec<String> = serde_json::from_value(p["command"].clone()).unwrap_or_default();
    let url = p["url"].as_str();
    match p["transport"].as_str() {
        Some("stdio") => anyhow::ensure!(!command.is_empty(), "stdio servers need `command` (argv list)"),
        Some("http") => {
            let u = url::Url::parse(url.context("http servers need `url`")?).context("bad url")?;
            anyhow::ensure!(matches!(u.scheme(), "http" | "https"), "url must be http(s)");
        }
        _ => bail!("`transport` must be stdio or http"),
    }
    let warnings = match package_ref(&command) {
        Some(r) => antidote_check(&r).await?,
        None => vec!["not scanned: Antidote only checks npx, bunx and uvx servers".into()],
    };
    let secrets: Vec<String> = serde_json::from_value(p["secrets"].clone()).unwrap_or_default();
    sqlx::query("INSERT INTO mcp_servers (agent_id, name, transport, command, url, secrets) VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT(agent_id, name) DO UPDATE SET transport = excluded.transport, command = excluded.command,
                 url = excluded.url, secrets = excluded.secrets")
        .bind(agent_id).bind(name).bind(p["transport"].as_str()).bind(json!(command).to_string()).bind(url)
        .bind(json!(secrets).to_string()).execute(db).await?;
    Ok(warnings)
}

/// `npx -y @x/server@1.2` → `npm:@x/server@1.2`, `uvx mcp-server-git` → `pypi:mcp-server-git`.
fn package_ref(argv: &[String]) -> Option<String> {
    let prog = Path::new(argv.first()?).file_stem()?.to_str()?;
    let (eco, value_flags): (&str, &[&str]) = match prog {
        "npx" | "bunx" => ("npm", &["-p", "--package"]),
        "uvx" => ("pypi", &["--from"]),
        _ => return None,
    };
    let mut args = argv[1..].iter();
    while let Some(a) = args.next() {
        if value_flags.contains(&a.as_str()) {
            return Some(format!("{eco}:{}", args.next()?));
        }
        if !a.starts_with('-') {
            return Some(format!("{eco}:{a}"));
        }
    }
    None
}

/// Look the package up with the `antidote` scanner when it's installed; no scanner means no check.
async fn antidote_check(pkg: &str) -> Result<Vec<String>> {
    if !crate::sandbox::on_path("antidote") {
        return Ok(vec![format!("not scanned: antidote is not installed ({pkg})")]);
    }
    let run = tokio::process::Command::new("antidote").args(["check", pkg, "--format", "json", "--fail-on", "never"]).output();
    let findings: Vec<Value> = match tokio::time::timeout(std::time::Duration::from_secs(20), run).await {
        Ok(Ok(o)) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or_default(),
        _ => return Ok(vec![format!("antidote could not check {pkg}")]),
    };
    let line = |f: &Value| format!("{} {}: {}", f["severity"].as_str().unwrap_or("?"), f["id"].as_str().unwrap_or("?"), f["summary"].as_str().unwrap_or(""));
    let blocking: Vec<String> = findings.iter().filter(|f| matches!(f["severity"].as_str(), Some("critical" | "high"))).map(line).collect();
    if !blocking.is_empty() {
        bail!("Antidote flags {pkg}: {}", blocking.join("; "));
    }
    Ok(findings.iter().map(line).collect())
}

pub async fn remove(db: &SqlitePool, agent_id: i64, name: &str) -> Result<()> {
    sqlx::query("DELETE FROM mcp_servers WHERE agent_id = ? AND name = ?").bind(agent_id).bind(name).execute(db).await?;
    Ok(())
}

/// The agent's MCP servers for one run: connected clients and their tools.
#[derive(Default)]
pub struct Session {
    clients: HashMap<String, Client>,
    /// full tool name → (server, tool, read-only)
    tools: HashMap<String, (String, String, bool)>,
    pub specs: Vec<Value>,
    /// Servers that failed to connect, for the system prompt.
    pub errors: Vec<String>,
    /// Image results (browser screenshots) are saved here, under `screenshots/`.
    scratch: std::path::PathBuf,
}

/// The one-click browser: Playwright's MCP server baked into the sandbox image, its Chromium
/// sent through the agent's egress proxy.
pub const BROWSER: &[&str] = &["sh", "-c",
    r#"node -e 'const u=new URL(process.env.HTTPS_PROXY);require("fs").writeFileSync("/tmp/pw.json",JSON.stringify({browser:{launchOptions:{proxy:{server:u.protocol+"//"+u.host,username:decodeURIComponent(u.username),password:decodeURIComponent(u.password)}}}}))' && exec playwright-mcp --headless --isolated --no-sandbox --browser chromium --config /tmp/pw.json"#];

impl Session {
    /// Connect to every server; one that fails is reported in `errors` and skipped.
    // shortcut: reconnects (and re-runs npx/uvx) every run; keep sessions alive per agent if startup latency hurts.
    pub async fn connect(st: &Arc<State>, agent: &store::Agent, scratch: &Path, bound: &[secrets::Bound]) -> Session {
        let mut s = Session { scratch: scratch.to_path_buf(), ..Default::default() };
        let servers = match list(&st.db, agent.id).await {
            Ok(v) => v,
            Err(e) => { s.errors.push(format!("{e:#}")); return s; }
        };
        for srv in servers {
            let name = srv["name"].as_str().unwrap_or("").to_string();
            let client = match srv["transport"].as_str() {
                Some("stdio") => async {
                    proxy::ensure(st, agent, scratch).await?;
                    let argv: Vec<String> = serde_json::from_value(srv["command"].clone())?;
                    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
                    stdio(st.sandbox.command(agent.id, scratch, true, &argv)?).await
                }.await,
                _ => async {
                    let url = srv["url"].as_str().context("no url")?;
                    let names: Vec<String> = serde_json::from_value(srv["secrets"].clone())?;
                    let host = fetch::host_of(url, &agent.allowed_domains)?;
                    anyhow::ensure!(fetch::allowed(&agent.allowed_domains, &host), "{host} is not in the agent's allowed domains");
                    if let Some(c) = crate::oauth::connect(st, agent.id, &name, url, agent.allowed_domains.clone()).await? {
                        return Ok(c);
                    }
                    let headers = secrets::for_host(bound, &host).into_iter().filter(|b| names.contains(&b.name))
                        .filter_map(|b| b.header_pair()).collect();
                    http(url, headers, agent.allowed_domains.clone()).await
                }.await,
            };
            match client {
                Ok(c) => s.add(name, c).await,
                Err(e) => s.errors.push(format!("{name}: {e:#}")),
            }
        }
        s
    }

    async fn add(&mut self, server: String, c: Client) {
        let tools = match c.list_all_tools().await {
            Ok(t) => t,
            Err(e) => { self.errors.push(format!("{server}: listing tools failed: {e}")); return; }
        };
        for t in tools {
            let mut full = format!("mcp__{server}__{}", t.name).replace(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-', "_");
            full.truncate(64); // OpenAI-style function name limit
            let ro = t.annotations.as_ref().and_then(|a| a.read_only_hint).unwrap_or(false);
            self.specs.push(json!({"type": "function", "function": {"name": full,
                "description": t.description.as_deref().unwrap_or(""), "parameters": *t.input_schema}}));
            self.tools.insert(full, (server.clone(), t.name.to_string(), ro));
        }
        self.clients.insert(server, c);
    }

    pub fn has(&self, server: &str) -> bool {
        self.clients.contains_key(server)
    }

    pub fn read_only(&self, full: &str) -> bool {
        self.tools.get(full).is_some_and(|t| t.2)
    }

    /// Call a tool; its content comes back as text wrapped in `<untrusted>`.
    pub async fn call(&self, full: &str, args: &Value) -> Result<String> {
        let (server, tool, _) = self.tools.get(full).context("unknown MCP tool")?;
        let mut params = CallToolRequestParams::new(tool.clone());
        if let Some(obj) = args.as_object() {
            params = params.with_arguments(obj.clone());
        }
        let res = self.clients[server].call_tool(params).await?;
        let content = serde_json::to_value(&res.content)?;
        let text: Vec<String> = content.as_array().into_iter().flatten().map(|b| match (b["text"].as_str(), b["data"].as_str()) {
            (Some(t), _) => t.to_string(),
            // shortcut: the model gets the path, not the pixels; pass images to vision models once we track which can see
            (None, Some(data)) if b["type"] == "image" => self.save_image(data, b["mimeType"].as_str().unwrap_or(""))
                .map(|p| format!("[screenshot: {}]", p.display())).unwrap_or_else(|e| format!("[image not saved: {e:#}]")),
            _ => format!("[{} content]", b["type"].as_str().unwrap_or("other")),
        }).collect();
        let err = if res.is_error == Some(true) { "error: " } else { "" };
        Ok(format!("<untrusted source=\"mcp:{server}\">\n{err}{}\n</untrusted>", text.join("\n")))
    }
}

impl Session {
    fn save_image(&self, b64: &str, mime: &str) -> Result<std::path::PathBuf> {
        use base64::Engine;
        let ext = if mime.contains("jpeg") { "jpg" } else { "png" };
        let dir = self.scratch.join("screenshots");
        std::fs::create_dir_all(&dir)?;
        let p = dir.join(format!("{}.{ext}", chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f")));
        std::fs::write(&p, base64::engine::general_purpose::STANDARD.decode(b64)?)?;
        Ok(p)
    }
}

async fn stdio(cmd: tokio::process::Command) -> Result<Client> {
    let (t, _) = TokioChildProcess::builder(cmd).stderr(std::process::Stdio::null()).spawn()?;
    Ok(().serve(t).await?)
}

/// HTTP servers: no redirects (they'd carry the secret headers elsewhere) and public DNS only.
async fn http(url: &str, headers: Vec<(String, String)>, allowed: Vec<String>) -> Result<Client> {
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(fetch::PublicOnly(allowed))).build()?;
    let mut map = HashMap::new();
    for (k, v) in headers {
        map.insert(HeaderName::try_from(k)?, HeaderValue::try_from(v)?);
    }
    let cfg = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(map);
    Ok(().serve(StreamableHttpClientTransport::with_client(client, cfg)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Answers initialize, tools/list (one read-only `echo`) and tools/call.
    fn reply(req: &Value) -> Option<Value> {
        let id = req.get("id")?.clone();
        let result = match req["method"].as_str()? {
            "initialize" => json!({"protocolVersion": req["params"]["protocolVersion"], "capabilities": {"tools": {}},
                "serverInfo": {"name": "stub", "version": "0"}}),
            "tools/list" => json!({"tools": [{"name": "echo", "description": "Echo", "inputSchema": {"type": "object"},
                "annotations": {"readOnlyHint": true}}]}),
            "tools/call" => json!({"content": [{"type": "text", "text": req["params"]["arguments"]["text"]}]}),
            _ => json!({}),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    async fn exercise(c: Client) {
        let mut s = Session::default();
        s.add("stub".into(), c).await;
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.specs[0]["function"]["name"], "mcp__stub__echo");
        assert!(s.read_only("mcp__stub__echo"));
        let out = s.call("mcp__stub__echo", &json!({"text": "hi there"})).await.unwrap();
        assert!(out.starts_with("<untrusted source=\"mcp:stub\">") && out.contains("hi there"), "{out}");
    }

    #[tokio::test]
    async fn stdio_server() {
        // the same stub logic in Python, speaking newline-delimited JSON-RPC on stdio
        let py = r#"
import json, sys
for line in sys.stdin:
    r = json.loads(line)
    if "id" not in r: continue
    m = r["method"]
    if m == "initialize": res = {"protocolVersion": r["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "stub", "version": "0"}}
    elif m == "tools/list": res = {"tools": [{"name": "echo", "description": "Echo", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": True}}]}
    elif m == "tools/call": res = {"content": [{"type": "text", "text": r["params"]["arguments"]["text"]}]}
    else: res = {}
    print(json.dumps({"jsonrpc": "2.0", "id": r["id"], "result": res}), flush=True)
"#;
        let mut cmd = tokio::process::Command::new(if cfg!(windows) { "python" } else { "python3" });
        cmd.args(["-c", py]);
        exercise(stdio(cmd).await.unwrap()).await;
    }

    #[tokio::test]
    async fn http_server_gets_secret_header() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://localhost:{}/mcp", l.local_addr().unwrap().port());
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 8192];
                    loop {
                        // one request per read loop; keep-alive connections carry several
                        let (head_end, len) = loop {
                            let r = String::from_utf8_lossy(&req).to_string();
                            if let Some(h) = r.find("\r\n\r\n") {
                                let len = r.to_lowercase().split("content-length:").nth(1)
                                    .and_then(|x| x.lines().next()).and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
                                if req.len() >= h + 4 + len { break (h + 4, len); }
                            }
                            let k = s.read(&mut buf).await.unwrap_or(0);
                            if k == 0 { return; }
                            req.extend_from_slice(&buf[..k]);
                        };
                        let head = String::from_utf8_lossy(&req[..head_end]).to_lowercase();
                        let body: Value = serde_json::from_slice(&req[head_end..head_end + len]).unwrap_or(Value::Null);
                        req.drain(..head_end + len);
                        let resp = if !head.starts_with("post") {
                            "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n".to_string()
                        } else if !head.contains("x-api-key: s3cret") {
                            "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n".to_string()
                        } else if let Some(r) = reply(&body) {
                            let b = r.to_string();
                            format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{b}", b.len())
                        } else {
                            "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\n\r\n".to_string()
                        };
                        if s.write_all(resp.as_bytes()).await.is_err() { return; }
                    }
                });
            }
        });
        let headers = vec![("X-Api-Key".to_string(), "s3cret".to_string())];
        exercise(http(&url, headers, vec!["localhost".into()]).await.unwrap()).await;
        assert!(http(&url, vec![], vec!["localhost".into()]).await.is_err(), "no secret, no session");
    }

    /// Needs a runtime and the base image (for npx):
    /// `BURROW_SANDBOX_IMAGE=<sandbox-images/base build> cargo test -- --ignored sandbox_mcp`.
    #[tokio::test]
    #[ignore]
    async fn sandbox_mcp_stdio() {
        use crate::{policy, sandbox::Sandbox, secrets::Vault};
        let tmp = tempfile::tempdir().unwrap();
        let st = Arc::new(State::new(tmp.path().to_path_buf(), String::new(), Sandbox::detect(), Vault::File(tmp.path().join("keys"))).await.unwrap());
        let mut a = store::Agent { id: 0, name: "m".into(), role_prompt: "".into(), model_ref: "llama:x".into(), folders: vec![],
            allowed_domains: policy::PACKAGE_REGISTRIES.iter().map(|d| d.to_string()).collect(), approval_rules: Default::default(), context_budget: 8_000, limits: Default::default() };
        a.id = store::create_agent(&st.db, &a).await.unwrap();
        add(&st.db, &json!({"agent_id": a.id, "name": "everything", "transport": "stdio",
            "command": ["npx", "-y", "@modelcontextprotocol/server-everything"]})).await.unwrap();
        let s = Session::connect(&st, &a, &st.scratch(a.id).unwrap(), &[]).await;
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        let out = s.call("mcp__everything__echo", &json!({"message": "from the sandbox"})).await.unwrap();
        assert!(out.contains("from the sandbox"), "{out}");
        drop(s);
        st.sandbox.stop_all().await;
    }
}
