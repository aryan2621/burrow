//! Connectors: email (IMAP + SMTP), calendar (CalDAV) and GitHub, signed in with an app password
//! or token from the keychain. The user grants each connection to agents. Every side effect
//! returns an inverse so the journal can undo it; email sends wait in the outbox first.
use crate::agent::State;
use crate::tools::Outcome;
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use std::time::Duration;

/// Approved emails wait this long before they go out.
pub const SEND_DELAY: i64 = 60;
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 20_000;

pub struct Conn {
    pub name: String,
    pub kind: String,
    pub config: Value,
    secret: String,
}

impl Conn {
    fn cfg(&self, k: &str) -> Result<&str> {
        self.config[k].as_str().filter(|s| !s.is_empty()).with_context(|| format!("connection {} has no `{k}`", self.name))
    }
}

fn vault_key(name: &str) -> String {
    format!("conn-{name}")
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub async fn list(db: &SqlitePool) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM connections ORDER BY name").fetch_all(db).await?;
    rows.iter().map(|r| Ok(json!({"name": r.get::<String, _>("name"), "kind": r.get::<String, _>("kind"),
        "config": serde_json::from_str::<Value>(r.get("config"))?,
        "agent_ids": serde_json::from_str::<Value>(r.get("agent_ids"))?}))).collect()
}

/// `connections.set`: {name, kind, config, secret?, agent_ids}. A missing secret keeps the stored one.
pub async fn set(db: &SqlitePool, vault: &crate::secrets::Vault, p: &Value) -> Result<()> {
    let name = p["name"].as_str().context("missing `name`")?;
    anyhow::ensure!(!name.is_empty() && name.len() <= 30 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
        "connection names use up to 30 letters, digits, - and _");
    let kind = p["kind"].as_str().context("missing `kind`")?;
    let config = p["config"].as_object().cloned().unwrap_or_default();
    let need: &[&str] = match kind {
        "email" => &["imap_host", "smtp_host", "username"],
        "calendar" => &["url", "username"],
        "github" => &[],
        _ => bail!("kind must be email, calendar or github"),
    };
    for k in need {
        anyhow::ensure!(config.get(*k).and_then(Value::as_str).is_some_and(|s| !s.is_empty()), "{kind} connections need `{k}`");
    }
    if kind == "calendar" {
        let u = url::Url::parse(config["url"].as_str().unwrap_or("")).context("bad calendar URL")?;
        anyhow::ensure!(u.scheme() == "https", "calendar URL must be https");
    }
    let agent_ids: Vec<i64> = serde_json::from_value(p["agent_ids"].clone()).unwrap_or_default();
    match p["secret"].as_str() {
        Some(s) if !s.is_empty() => vault.set(&vault_key(name), s).await?,
        _ => anyhow::ensure!(vault.get(&vault_key(name)).await?.is_some(), "give the app password or token"),
    }
    sqlx::query("INSERT INTO connections (name, kind, config, agent_ids) VALUES (?, ?, ?, ?)
                 ON CONFLICT(name) DO UPDATE SET kind = excluded.kind, config = excluded.config, agent_ids = excluded.agent_ids")
        .bind(name).bind(kind).bind(Value::Object(config).to_string()).bind(json!(agent_ids).to_string()).execute(db).await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, vault: &crate::secrets::Vault, name: &str) -> Result<()> {
    sqlx::query("DELETE FROM connections WHERE name = ?").bind(name).execute(db).await?;
    vault.delete(&vault_key(name)).await
}

/// A connection by name; with `agent_id`, only if that agent was granted it.
pub async fn get(st: &State, name: &str, agent_id: Option<i64>) -> Result<Conn> {
    let r = sqlx::query("SELECT * FROM connections WHERE name = ?").bind(name).fetch_optional(&st.db).await?
        .with_context(|| format!("no connection `{name}`"))?;
    if let Some(a) = agent_id {
        let ids: Vec<i64> = serde_json::from_str(r.get("agent_ids"))?;
        anyhow::ensure!(ids.contains(&a), "this agent may not use connection `{name}`");
    }
    Ok(Conn { name: name.into(), kind: r.get("kind"), config: serde_json::from_str(r.get("config"))?,
        secret: st.vault.get(&vault_key(name)).await?.context("connection secret is missing from the keychain")? })
}

/// (name, kind) of the connections granted to `agent_id`.
pub async fn for_agent(db: &SqlitePool, agent_id: i64) -> Result<Vec<(String, String)>> {
    Ok(list(db).await?.into_iter()
        .filter(|c| c["agent_ids"].as_array().is_some_and(|a| a.contains(&json!(agent_id))))
        .map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["kind"].as_str().unwrap_or("").to_string()))
        .collect())
}

pub const TOOLS: [&str; 12] = ["email_search", "email_read", "email_move", "email_send", "calendar_list", "calendar_create",
    "calendar_update", "calendar_delete", "github_issues", "github_comment", "github_label", "github_close"];

/// Tools that read outside content (it may carry prompt injection).
pub fn untrusted(tool: &str) -> bool {
    matches!(tool, "email_search" | "email_read" | "calendar_list" | "github_issues")
}

pub fn side_effect(tool: &str) -> bool {
    TOOLS.contains(&tool) && !untrusted(tool)
}

/// Tool specs for the kinds of connection the agent has, each taking a `connection` name.
pub fn specs(conns: &[(String, String)]) -> Vec<Value> {
    let mut out = vec![];
    for kind in ["email", "calendar", "github"] {
        let names: Vec<&str> = conns.iter().filter(|(_, k)| k == kind).map(|(n, _)| n.as_str()).collect();
        if names.is_empty() {
            continue;
        }
        let conn = json!({"type": "string", "enum": names, "description": format!("which {kind} connection")});
        let s = |v: &str| json!({"type": "string", "description": v});
        let f = |name: &str, desc: &str, mut props: Value, req: &[&str]| {
            props["connection"] = conn.clone();
            let mut req: Vec<&str> = req.to_vec();
            req.push("connection");
            json!({"type": "function", "function": {"name": name, "description": desc,
                "parameters": {"type": "object", "properties": props, "required": req}}})
        };
        match kind {
            "email" => out.extend([
                f("email_search", "List recent emails in a folder, newest first, optionally matching words",
                  json!({"folder": s("IMAP folder, default INBOX"), "query": s("words to match"), "limit": {"type": "integer"}}), &[]),
                f("email_read", "Read one email by UID", json!({"folder": s("default INBOX"), "uid": {"type": "integer"}}), &["uid"]),
                f("email_move", "Move an email to another folder (undoable)",
                  json!({"folder": s("default INBOX"), "uid": {"type": "integer"}, "to": s("destination folder")}), &["uid", "to"]),
                f("email_send", "Send an email. It waits 60 s in the outbox so the user can cancel it",
                  json!({"to": {"type": "array", "items": {"type": "string"}}, "cc": {"type": "array", "items": {"type": "string"}},
                         "subject": s("subject"), "body": s("plain-text body"), "in_reply_to": s("Message-ID being answered")}),
                  &["to", "subject", "body"]),
            ]),
            "calendar" => out.extend([
                f("calendar_list", "List events between two times (default: the next 7 days)",
                  json!({"from": s("RFC 3339 time"), "to": s("RFC 3339 time")}), &[]),
                f("calendar_create", "Create an event (undoable)", json!({"summary": s("title"), "start": s("RFC 3339 time"),
                  "end": s("RFC 3339 time"), "location": s("location"), "description": s("notes")}), &["summary", "start", "end"]),
                f("calendar_update", "Change an event's fields (undoable)", json!({"href": s("event href from calendar_list"),
                  "summary": s("title"), "start": s("RFC 3339 time"), "end": s("RFC 3339 time"), "location": s("location"),
                  "description": s("notes")}), &["href"]),
                f("calendar_delete", "Delete an event (undoable)", json!({"href": s("event href from calendar_list")}), &["href"]),
            ]),
            _ => out.extend([
                f("github_issues", "List issues of a repo, or read one issue with its comments when `number` is given",
                  json!({"repo": s("owner/name"), "state": s("open, closed or all"), "number": {"type": "integer"}}), &["repo"]),
                f("github_comment", "Comment on an issue or pull request (undoable: deletes the comment)",
                  json!({"repo": s("owner/name"), "number": {"type": "integer"}, "body": s("markdown")}), &["repo", "number", "body"]),
                f("github_label", "Add labels to an issue or pull request (undoable)",
                  json!({"repo": s("owner/name"), "number": {"type": "integer"}, "labels": {"type": "array", "items": {"type": "string"}}}),
                  &["repo", "number", "labels"]),
                f("github_close", "Close an issue or pull request (undoable: reopens it)",
                  json!({"repo": s("owner/name"), "number": {"type": "integer"}}), &["repo", "number"]),
            ]),
        }
    }
    out
}

/// Run a connector tool for `agent_id`. Errors come back as text for the model.
pub async fn call(st: &State, agent_id: i64, name: &str, args: &Value) -> Outcome {
    let res = async {
        let c = get(st, args["connection"].as_str().context("missing `connection`")?, Some(agent_id)).await?;
        let want = name.split('_').next().unwrap_or("");
        anyhow::ensure!(c.kind == want, "connection `{}` is a {} connection", c.name, c.kind);
        match want {
            "email" => email(st, &c, agent_id, name, args).await,
            "calendar" => calendar(st, &c, name, args).await,
            _ => github(st, &c, name, args).await,
        }
    }.await;
    res.unwrap_or_else(|e| format!("error: {e:#}").into())
}

fn wrap(source: &str, body: &str) -> String {
    format!("<untrusted source=\"{source}\">\n{body}\n</untrusted>")
}

fn clip(mut s: String, n: usize) -> String {
    if s.len() > n {
        s.truncate(s.floor_char_boundary(n));
        s.push_str("\n[truncated]");
    }
    s
}

/// Undo a connector side effect from its inverse.
pub async fn undo(st: &State, inv: &Value) -> Result<()> {
    let op = inv["op"].as_str().unwrap_or("");
    if op == "cancel_send" {
        let n = sqlx::query("UPDATE outbox SET status = 'cancelled' WHERE id = ? AND status = 'pending'")
            .bind(inv["outbox_id"].as_i64()).execute(&st.db).await?.rows_affected();
        anyhow::ensure!(n == 1, "too late: the email was already sent");
        return Ok(());
    }
    let c = get(st, inv["connection"].as_str().context("bad inverse")?, None).await?;
    let s = |k: &str| inv[k].as_str().unwrap_or("").to_string();
    match op {
        "move_back" => {
            let mut imap = imap(&c).await?;
            imap.select(s("folder")).await?;
            let uids = imap.uid_search(format!("HEADER Message-ID {}", quote(&s("message_id")))).await?;
            let uid = uids.into_iter().max().context("the email is no longer in that folder")?;
            imap.uid_mv(uid.to_string(), s("to")).await?;
            let _ = imap.logout().await;
        }
        "cal_delete" => {
            let res = cal_req(st, &c, "DELETE", &s("href"))?.send().await?;
            anyhow::ensure!(res.status().is_success() || res.status() == 404, "calendar server returned {}", res.status());
        }
        "cal_put" => {
            let res = cal_req(st, &c, "PUT", &s("href"))?.header("Content-Type", "text/calendar; charset=utf-8").body(s("ics")).send().await?;
            anyhow::ensure!(res.status().is_success(), "calendar server returned {}", res.status());
        }
        "gh_delete_comment" => {
            gh(st, &c, reqwest::Method::DELETE, &format!("/repos/{}/issues/comments/{}", s("repo"), inv["comment_id"]), None).await?;
        }
        "gh_remove_labels" => {
            for l in inv["labels"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                let path = format!("/repos/{}/issues/{}/labels/{}", s("repo"), inv["number"], urlencode(l));
                gh(st, &c, reqwest::Method::DELETE, &path, None).await?;
            }
        }
        "gh_reopen" => {
            gh(st, &c, reqwest::Method::PATCH, &format!("/repos/{}/issues/{}", s("repo"), inv["number"]), Some(json!({"state": "open"}))).await?;
        }
        _ => bail!("unknown inverse `{op}`"),
    }
    Ok(())
}

// ---- email ----

type Imap = async_imap::Session<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

async fn imap(c: &Conn) -> Result<Imap> {
    use rustls_platform_verifier::ConfigVerifierExt;
    let host = c.cfg("imap_host")?;
    let port = c.config["imap_port"].as_u64().unwrap_or(993) as u16;
    let tcp = tokio::time::timeout(TIMEOUT, tokio::net::TcpStream::connect((host, port))).await.context("IMAP connect timed out")??;
    let tls = tokio_rustls::rustls::ClientConfig::with_platform_verifier()?;
    let server = tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_string())?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(tls)).connect(server, tcp).await?;
    let mut client = async_imap::Client::new(tls);
    client.read_response().await.context("no IMAP greeting")?;
    client.login(c.cfg("username")?, &c.secret).await.map_err(|(e, _)| e).context("IMAP login failed")
}

/// An IMAP quoted string.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn mailbox(args: &Value, k: &str) -> String {
    args[k].as_str().filter(|s| !s.is_empty()).unwrap_or("INBOX").to_string()
}

/// "uid | date | from | subject" lines for `uids`, newest first.
async fn headers(imap: &mut Imap, uids: &[u32]) -> Result<Vec<String>> {
    if uids.is_empty() {
        return Ok(vec![]);
    }
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let mut rows: Vec<(u32, String)> = vec![];
    {
        let mut fetches = imap.uid_fetch(set, "(UID RFC822.HEADER)").await?;
        while let Some(f) = fetches.next().await {
            let f = f?;
            let (Some(uid), Some(h)) = (f.uid, f.header()) else { continue };
            let m = mail_parser::MessageParser::default().parse_headers(h).context("bad email headers")?;
            let from = m.from().and_then(|a| a.first()).map(|a| a.address().unwrap_or("").to_string()).unwrap_or_default();
            let date = m.date().map(|d| d.to_rfc3339()).unwrap_or_default();
            rows.push((uid, format!("{uid} | {date} | {from} | {}", m.subject().unwrap_or(""))));
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(rows.into_iter().map(|r| r.1).collect())
}

async fn email(st: &State, c: &Conn, agent_id: i64, name: &str, args: &Value) -> Result<Outcome> {
    if name == "email_send" {
        return queue_send(st, c, agent_id, args).await;
    }
    let mut imap = imap(c).await?;
    let folder = mailbox(args, "folder");
    imap.select(&folder).await.with_context(|| format!("no folder {folder}"))?;
    let uid = args["uid"].as_u64().map(|u| u as u32);
    let out = match name {
        "email_search" => {
            let q = match args["query"].as_str().filter(|q| !q.trim().is_empty()) {
                Some(q) => format!("TEXT {}", quote(q)),
                None => "ALL".into(),
            };
            let mut uids: Vec<u32> = imap.uid_search(q).await?.into_iter().collect();
            uids.sort_unstable_by(|a, b| b.cmp(a));
            uids.truncate(args["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize);
            let lines = headers(&mut imap, &uids).await?;
            wrap(&format!("email:{}", c.name), &if lines.is_empty() { "no emails".into() } else { lines.join("\n") }).into()
        }
        "email_read" => {
            let uid = uid.context("missing `uid`")?;
            let raw = {
                let mut fetches = imap.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?;
                let mut raw = None;
                while let Some(f) = fetches.next().await {
                    if let Some(b) = f?.body() { raw = Some(b.to_vec()); }
                }
                raw.with_context(|| format!("no email with UID {uid}"))?
            };
            let m = mail_parser::MessageParser::default().parse(&raw).context("bad email")?;
            let addrs = |a: Option<&mail_parser::Address>| a.map(|a| a.iter().filter_map(|x| x.address()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let body = m.body_text(0).map(|b| b.to_string()).unwrap_or_default();
            wrap(&format!("email:{}", c.name), &clip(format!("From: {}\nTo: {}\nDate: {}\nSubject: {}\nMessage-ID: {}\n\n{body}",
                addrs(m.from()), addrs(m.to()), m.date().map(|d| d.to_rfc3339()).unwrap_or_default(),
                m.subject().unwrap_or(""), m.message_id().unwrap_or("")), MAX_BODY)).into()
        }
        _ => {
            // email_move: find the Message-ID first so undo can locate it in the new folder
            let uid = uid.context("missing `uid`")?;
            let to = args["to"].as_str().filter(|s| !s.is_empty()).context("missing `to`")?;
            let mid = {
                let mut fetches = imap.uid_fetch(uid.to_string(), "(UID RFC822.HEADER)").await?;
                let mut mid = None;
                while let Some(f) = fetches.next().await {
                    if let Some(h) = f?.header() {
                        mid = mail_parser::MessageParser::default().parse_headers(h).and_then(|m| m.message_id().map(String::from));
                    }
                }
                mid
            };
            imap.uid_mv(uid.to_string(), to).await.with_context(|| format!("moving to {to}"))?;
            Outcome { output: format!("moved email {uid} from {folder} to {to}"),
                inverse: mid.map(|m| json!({"connector": "email", "op": "move_back", "connection": c.name, "folder": to, "to": folder, "message_id": m})) }
        }
    };
    let _ = imap.logout().await;
    Ok(out)
}

/// Approved sends wait in the outbox for `SEND_DELAY` seconds; undo cancels them.
async fn queue_send(st: &State, c: &Conn, agent_id: i64, args: &Value) -> Result<Outcome> {
    let list = |k: &str| -> Result<Vec<String>> {
        let v: Vec<String> = serde_json::from_value(args[k].clone()).unwrap_or_default();
        for a in &v {
            a.parse::<lettre::message::Mailbox>().with_context(|| format!("bad address `{a}`"))?;
        }
        Ok(v)
    };
    let (to, cc) = (list("to")?, list("cc")?);
    anyhow::ensure!(!to.is_empty(), "give at least one `to` address");
    let msg = json!({"to": to, "cc": cc, "subject": args["subject"].as_str().unwrap_or(""),
        "body": args["body"].as_str().unwrap_or(""), "in_reply_to": args["in_reply_to"].as_str()});
    build(c, &msg)?; // fail now, not after the delay
    let id = sqlx::query("INSERT INTO outbox (connection, agent_id, message, send_at) VALUES (?, ?, ?, ?)")
        .bind(&c.name).bind(agent_id).bind(msg.to_string()).bind(now() + SEND_DELAY).execute(&st.db).await?.last_insert_rowid();
    Ok(Outcome { output: format!("queued: sends to {} in {SEND_DELAY} s unless the user cancels it", to.join(", ")),
        inverse: Some(json!({"connector": "email", "op": "cancel_send", "outbox_id": id})) })
}

fn build(c: &Conn, m: &Value) -> Result<lettre::Message> {
    let from = c.config["address"].as_str().filter(|s| !s.is_empty()).unwrap_or(c.cfg("username")?);
    let mut b = lettre::Message::builder().from(from.parse().context("bad sender address")?)
        .subject(m["subject"].as_str().unwrap_or(""));
    for k in ["to", "cc"] {
        for a in m[k].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let mb = a.parse()?;
            b = if k == "to" { b.to(mb) } else { b.cc(mb) };
        }
    }
    if let Some(r) = m["in_reply_to"].as_str() {
        b = b.in_reply_to(r.into()).references(r.into());
    }
    Ok(b.body(m["body"].as_str().unwrap_or("").to_string())?)
}

async fn smtp_send(c: &Conn, msg: lettre::Message) -> Result<()> {
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
    let host = c.cfg("smtp_host")?;
    let port = c.config["smtp_port"].as_u64().unwrap_or(465) as u16;
    // 465 is TLS from the start; anything else (587) upgrades with STARTTLS
    let t = if port == 465 { AsyncSmtpTransport::<Tokio1Executor>::relay(host)? } else { AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)? };
    let t = t.port(port).timeout(Some(TIMEOUT))
        .credentials(lettre::transport::smtp::authentication::Credentials::new(c.cfg("username")?.into(), c.secret.clone())).build();
    t.send(msg).await?;
    Ok(())
}

/// Send outbox emails whose delay is over. Runs for the daemon's lifetime.
pub async fn outbox(st: Arc<State>) {
    // a crash mid-send leaves `sending`; we can't tell if it went out, so say so rather than resend
    let _ = sqlx::query("UPDATE outbox SET status = 'failed', error = 'daemon stopped while sending; check your Sent folder' WHERE status = 'sending'")
        .execute(&st.db).await;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let Ok(due) = sqlx::query("SELECT id, connection, agent_id, message FROM outbox WHERE status = 'pending' AND send_at <= ?")
            .bind(now()).fetch_all(&st.db).await else { continue };
        for r in due {
            let id: i64 = r.get("id");
            // claim it; a cancel that got there first wins
            let claimed = sqlx::query("UPDATE outbox SET status = 'sending' WHERE id = ? AND status = 'pending'").bind(id).execute(&st.db).await;
            if !claimed.is_ok_and(|c| c.rows_affected() == 1) {
                continue;
            }
            let res = async {
                let c = get(&st, r.get("connection"), None).await?;
                let m: Value = serde_json::from_str(r.get("message"))?;
                smtp_send(&c, build(&c, &m)?).await
            }.await;
            let (status, err) = match &res {
                Ok(()) => ("sent", None),
                Err(e) => ("failed", Some(format!("{e:#}"))),
            };
            let _ = sqlx::query("UPDATE outbox SET status = ?, error = ? WHERE id = ?").bind(status).bind(&err).bind(id).execute(&st.db).await;
            st.emit("outbox.done", json!({"agent_id": r.get::<i64, _>("agent_id"), "outbox_id": id, "status": status, "error": err}));
        }
    }
}

// ---- calendar (CalDAV) ----

/// An authenticated request to `href`, which must lie under the connection's calendar URL.
fn cal_req(st: &State, c: &Conn, method: &str, href: &str) -> Result<reqwest::RequestBuilder> {
    let base = url::Url::parse(c.cfg("url")?)?;
    let u = base.join(href)?;
    anyhow::ensure!(u.origin() == base.origin() && u.path().starts_with(base.path()), "{href} is outside this calendar");
    Ok(st.http.request(reqwest::Method::from_bytes(method.as_bytes())?, u).basic_auth(c.cfg("username")?, Some(&c.secret)).timeout(TIMEOUT))
}

fn ical_time(s: &str) -> Result<String> {
    let t = chrono::DateTime::parse_from_rfc3339(s).with_context(|| format!("`{s}` is not an RFC 3339 time"))?;
    Ok(t.with_timezone(&chrono::Utc).format("%Y%m%dT%H%M%SZ").to_string())
}

fn ical_text(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

/// Contents of every element named `local` (any namespace prefix), entity-decoded.
fn elements(xml: &str, local: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let tag_end = rest.find(['>', ' ', '/']).unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        if tag.rsplit(':').next() != Some(local) || tag.starts_with('/') {
            continue;
        }
        let Some(open_end) = rest.find('>') else { break };
        if rest[..open_end].ends_with('/') {
            continue;
        }
        let close = format!("</{tag}>");
        let body = &rest[open_end + 1..];
        let Some(j) = body.find(&close) else { break };
        out.push(body[..j].replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
            .replace("&#13;", "\r").replace("&amp;", "&"));
        rest = &body[j + close.len()..];
    }
    out
}

/// The VEVENT's properties as (name, value), unfolded; parameters dropped from the name.
fn props(ics: &str) -> Vec<(String, String)> {
    let unfolded = ics.replace("\r\n ", "").replace("\n ", "").replace("\r\n\t", "").replace("\n\t", "");
    let mut out = vec![];
    let mut in_event = false;
    for line in unfolded.lines() {
        match line {
            "BEGIN:VEVENT" => in_event = true,
            "END:VEVENT" => break,
            _ if in_event => if let Some((k, v)) = line.split_once(':') {
                out.push((k.split(';').next().unwrap_or(k).to_uppercase(), v.to_string()));
            },
            _ => {}
        }
    }
    out
}

/// `ics` with the first VEVENT's `name` properties replaced by `line`.
fn replace_prop(ics: &str, name: &str, line: &str) -> String {
    let mut out: Vec<String> = vec![];
    let (mut in_event, mut skipping, mut done) = (false, false, false);
    for l in ics.lines() {
        if skipping && (l.starts_with(' ') || l.starts_with('\t')) {
            continue; // folded continuation of the removed line
        }
        skipping = false;
        if l == "BEGIN:VEVENT" { in_event = true; }
        let key = l.split([':', ';']).next().unwrap_or("").to_uppercase();
        if in_event && !done && key == name {
            skipping = true;
            continue;
        }
        if in_event && !done && l == "END:VEVENT" {
            out.push(line.to_string());
            done = true;
        }
        out.push(l.to_string());
    }
    out.join("\r\n") + "\r\n"
}

async fn calendar(st: &State, c: &Conn, name: &str, args: &Value) -> Result<Outcome> {
    let s = |k: &str| args[k].as_str().filter(|s| !s.is_empty());
    let fields = |ics: String| -> Result<String> {
        let mut ics = ics;
        for (k, prop) in [("summary", "SUMMARY"), ("location", "LOCATION"), ("description", "DESCRIPTION")] {
            if let Some(v) = s(k) { ics = replace_prop(&ics, prop, &format!("{prop}:{}", ical_text(v))); }
        }
        for (k, prop) in [("start", "DTSTART"), ("end", "DTEND")] {
            if let Some(v) = s(k) { ics = replace_prop(&ics, prop, &format!("{prop}:{}", ical_time(v)?)); }
        }
        Ok(ics)
    };
    let inverse = |op: &str, href: &str, ics: Option<&str>| Some(json!({"connector": "calendar", "op": op, "connection": c.name, "href": href, "ics": ics}));
    Ok(match name {
        "calendar_list" => {
            let from = match s("from") { Some(f) => ical_time(f)?, None => ical_time(&chrono::Utc::now().to_rfc3339())? };
            let to = match s("to") { Some(t) => ical_time(t)?, None => ical_time(&(chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339())? };
            let body = format!(r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">
    <c:time-range start="{from}" end="{to}"/>
  </c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#);
            let res = cal_req(st, c, "REPORT", "")?.header("Depth", "1").header("Content-Type", "application/xml; charset=utf-8").body(body).send().await?;
            anyhow::ensure!(res.status().is_success(), "calendar server returned {}", res.status());
            let xml = res.text().await?;
            let mut lines: Vec<(String, String)> = elements(&xml, "response").iter().filter_map(|r| {
                let href = elements(r, "href").into_iter().next()?;
                let ics = elements(r, "calendar-data").into_iter().next()?;
                let p = props(&ics);
                let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|x| x.1.clone()).unwrap_or_default();
                Some((get("DTSTART"), format!("{href} | {} – {} | {} | {}", get("DTSTART"), get("DTEND"), get("SUMMARY"), get("LOCATION"))))
            }).collect();
            lines.sort();
            let body = if lines.is_empty() { "no events".into() } else { lines.into_iter().map(|l| l.1).collect::<Vec<_>>().join("\n") };
            wrap(&format!("calendar:{}", c.name), &body).into()
        }
        "calendar_create" => {
            let mut b = [0u8; 16];
            getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("{e}"))?;
            let uid: String = b.iter().map(|x| format!("{x:02x}")).collect();
            let ics = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Burrow//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
                ical_time(&chrono::Utc::now().to_rfc3339())?);
            anyhow::ensure!(s("summary").is_some() && s("start").is_some() && s("end").is_some(), "give summary, start and end");
            let ics = fields(ics)?;
            let href = format!("{uid}.ics");
            let res = cal_req(st, c, "PUT", &href)?.header("If-None-Match", "*").header("Content-Type", "text/calendar; charset=utf-8").body(ics).send().await?;
            anyhow::ensure!(res.status().is_success(), "calendar server returned {}", res.status());
            let abs = url::Url::parse(c.cfg("url")?)?.join(&href)?.path().to_string();
            Outcome { output: format!("created event {abs}"), inverse: inverse("cal_delete", &abs, None) }
        }
        _ => {
            // update or delete: keep the old event so undo can put it back
            let href = s("href").context("missing `href`")?;
            let res = cal_req(st, c, "GET", href)?.send().await?;
            anyhow::ensure!(res.status().is_success(), "calendar server returned {} for {href}", res.status());
            let etag = res.headers().get("etag").and_then(|e| e.to_str().ok()).map(String::from);
            let old = res.text().await?;
            let mut req = if name == "calendar_delete" {
                cal_req(st, c, "DELETE", href)?
            } else {
                cal_req(st, c, "PUT", href)?.header("Content-Type", "text/calendar; charset=utf-8").body(fields(old.clone())?)
            };
            if let Some(e) = etag {
                req = req.header("If-Match", e); // fail rather than clobber a change made meanwhile
            }
            let res = req.send().await?;
            anyhow::ensure!(res.status().is_success(), "calendar server returned {}", res.status());
            let verb = if name == "calendar_delete" { "deleted" } else { "updated" };
            Outcome { output: format!("{verb} event {href}"), inverse: inverse("cal_put", href, Some(&old)) }
        }
    })
}

// ---- GitHub ----

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

async fn gh(st: &State, c: &Conn, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
    let mut req = st.http.request(method, format!("https://api.github.com{path}")).bearer_auth(&c.secret)
        .header("User-Agent", "burrow").header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28").timeout(TIMEOUT);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await?;
    let status = res.status();
    let text = res.text().await?;
    anyhow::ensure!(status.is_success(), "GitHub returned {status}: {}", clip(text, 500));
    Ok(if text.is_empty() { Value::Null } else { serde_json::from_str(&text)? })
}

/// `owner/name`, and in the connection's repo list if it has one.
fn repo<'a>(c: &Conn, args: &'a Value) -> Result<&'a str> {
    let r = args["repo"].as_str().context("missing `repo`")?;
    anyhow::ensure!(r.split('/').count() == 2 && r.chars().all(|ch| ch.is_ascii_alphanumeric() || "-_./".contains(ch)) && !r.contains(".."),
        "repo must be owner/name");
    if let Some(list) = c.config["repos"].as_array().filter(|l| !l.is_empty()) {
        anyhow::ensure!(list.iter().any(|x| x.as_str().is_some_and(|x| x.eq_ignore_ascii_case(r))), "{r} is not in this connection's repos");
    }
    Ok(r)
}

async fn github(st: &State, c: &Conn, name: &str, args: &Value) -> Result<Outcome> {
    use reqwest::Method;
    let r = repo(c, args)?;
    let num = args["number"].as_u64();
    let inverse = |op: &str, extra: Value| {
        let mut v = json!({"connector": "github", "op": op, "connection": c.name, "repo": r, "number": num});
        v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        Some(v)
    };
    Ok(match name {
        "github_issues" => {
            let body = match num {
                Some(n) => {
                    let i = gh(st, c, Method::GET, &format!("/repos/{r}/issues/{n}"), None).await?;
                    let cs = gh(st, c, Method::GET, &format!("/repos/{r}/issues/{n}/comments?per_page=30"), None).await?;
                    let comments: Vec<String> = cs.as_array().into_iter().flatten().map(|x| format!("--- {} ({}):\n{}",
                        x["user"]["login"].as_str().unwrap_or("?"), x["created_at"].as_str().unwrap_or(""), x["body"].as_str().unwrap_or(""))).collect();
                    format!("#{n} [{}] {}\nby {}\n\n{}\n\n{}", i["state"].as_str().unwrap_or(""), i["title"].as_str().unwrap_or(""),
                        i["user"]["login"].as_str().unwrap_or("?"), i["body"].as_str().unwrap_or(""), comments.join("\n"))
                }
                None => {
                    let state = args["state"].as_str().filter(|s| matches!(*s, "open" | "closed" | "all")).unwrap_or("open");
                    let list = gh(st, c, Method::GET, &format!("/repos/{r}/issues?state={state}&per_page=30"), None).await?;
                    let lines: Vec<String> = list.as_array().into_iter().flatten().map(|i| format!("#{} [{}{}] {} (by {}) {}",
                        i["number"], i["state"].as_str().unwrap_or(""), if i.get("pull_request").is_some() { ", PR" } else { "" },
                        i["title"].as_str().unwrap_or(""), i["user"]["login"].as_str().unwrap_or("?"),
                        i["labels"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str()).collect::<Vec<_>>().join(","))).collect();
                    if lines.is_empty() { "no issues".into() } else { lines.join("\n") }
                }
            };
            wrap(&format!("github:{r}"), &clip(body, MAX_BODY)).into()
        }
        "github_comment" => {
            let n = num.context("missing `number`")?;
            let v = gh(st, c, Method::POST, &format!("/repos/{r}/issues/{n}/comments"), Some(json!({"body": args["body"].as_str().unwrap_or("")}))).await?;
            Outcome { output: format!("commented on {r}#{n}: {}", v["html_url"].as_str().unwrap_or("")),
                inverse: inverse("gh_delete_comment", json!({"comment_id": v["id"]})) }
        }
        "github_label" => {
            let n = num.context("missing `number`")?;
            let before = gh(st, c, Method::GET, &format!("/repos/{r}/issues/{n}/labels"), None).await?;
            let had: Vec<&str> = before.as_array().into_iter().flatten().filter_map(|l| l["name"].as_str()).collect();
            let want: Vec<String> = serde_json::from_value(args["labels"].clone()).unwrap_or_default();
            let added: Vec<&String> = want.iter().filter(|l| !had.contains(&l.as_str())).collect();
            gh(st, c, Method::POST, &format!("/repos/{r}/issues/{n}/labels"), Some(json!({"labels": want}))).await?;
            Outcome { output: format!("labeled {r}#{n}: {}", want.join(", ")), inverse: inverse("gh_remove_labels", json!({"labels": added})) }
        }
        _ => {
            let n = num.context("missing `number`")?;
            gh(st, c, Method::PATCH, &format!("/repos/{r}/issues/{n}"), Some(json!({"state": "closed"}))).await?;
            Outcome { output: format!("closed {r}#{n}"), inverse: inverse("gh_reopen", json!({})) }
        }
    })
}

// ---- event triggers ----

/// Poll one event trigger. Returns the new cursor and, if something new arrived, text for the
/// run's prompt. The first poll (no cursor) only records where we are.
pub async fn poll(st: &State, kind: &str, spec: &str, cursor: Option<&str>) -> Result<(String, Option<String>)> {
    let (conn, rest) = spec.split_once(' ').unwrap_or((spec, ""));
    let c = get(st, conn, None).await?;
    match kind {
        "email" => {
            let mut imap = imap(&c).await?;
            imap.select("INBOX").await?;
            let last: u32 = cursor.and_then(|x| x.parse().ok()).unwrap_or(0);
            let mut uids: Vec<u32> = imap.uid_search(format!("UID {}:*", last + 1)).await?.into_iter().filter(|u| *u > last).collect();
            uids.sort_unstable();
            let max = uids.last().copied().unwrap_or(last);
            let new = if cursor.is_some() && !uids.is_empty() {
                uids = uids.into_iter().rev().take(20).collect();
                Some(format!("New email in {}:\n{}", c.name, wrap(&format!("email:{}", c.name), &headers(&mut imap, &uids).await?.join("\n"))))
            } else {
                None
            };
            let _ = imap.logout().await;
            Ok((max.to_string(), new))
        }
        "calendar" => {
            // events starting in the next 15 minutes, each announced once
            let soon = chrono::Utc::now() + chrono::Duration::minutes(15);
            let out = calendar(st, &c, "calendar_list", &json!({"to": soon.to_rfc3339()})).await?.output;
            let mut seen: Vec<String> = cursor.and_then(|x| serde_json::from_str(x).ok()).unwrap_or_default();
            let lines: Vec<String> = out.lines().filter(|l| l.contains(" | ") && !seen.contains(&l.to_string())).map(String::from).collect();
            seen.extend(lines.iter().cloned());
            let keep = seen.len().saturating_sub(100);
            seen.drain(..keep);
            let new = (!lines.is_empty()).then(|| format!("Starting soon in {}:\n{}", c.name, wrap(&format!("calendar:{}", c.name), &lines.join("\n"))));
            Ok((json!(seen).to_string(), new))
        }
        _ => {
            let args = json!({"repo": rest.trim()});
            let r = repo(&c, &args)?;
            let list = gh(st, &c, reqwest::Method::GET, &format!("/repos/{r}/issues?state=open&sort=created&direction=desc&per_page=20"), None).await?;
            let issues: Vec<&Value> = list.as_array().into_iter().flatten().filter(|i| i.get("pull_request").is_none()).collect();
            let last: u64 = cursor.and_then(|x| x.parse().ok()).unwrap_or(0);
            let max = issues.iter().filter_map(|i| i["number"].as_u64()).max().unwrap_or(0).max(last);
            let fresh: Vec<String> = issues.iter().filter(|i| i["number"].as_u64().unwrap_or(0) > last)
                .map(|i| format!("#{} {} (by {})\n{}", i["number"], i["title"].as_str().unwrap_or(""), i["user"]["login"].as_str().unwrap_or("?"),
                    clip(i["body"].as_str().unwrap_or("").to_string(), 2_000))).collect();
            let new = (cursor.is_some() && !fresh.is_empty()).then(|| format!("New issues in {r}:\n{}", wrap(&format!("github:{r}"), &fresh.join("\n\n"))));
            Ok((max.to_string(), new))
        }
    }
}

/// Check an event trigger's spec when the schedule is saved.
pub async fn validate_trigger(db: &SqlitePool, agent_id: i64, kind: &str, spec: &str) -> Result<()> {
    let (conn, rest) = spec.split_once(' ').unwrap_or((spec, ""));
    let granted = for_agent(db, agent_id).await?;
    anyhow::ensure!(granted.iter().any(|(n, k)| n == conn && k == kind), "`{conn}` is not a {kind} connection this agent may use");
    if kind == "github" {
        anyhow::ensure!(rest.trim().split('/').count() == 2, "github triggers take `<connection> owner/repo`");
    }
    Ok(())
}
