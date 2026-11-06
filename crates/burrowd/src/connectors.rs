//! Connectors: email (IMAP + SMTP), signed in with an app password from the keychain.
//! The user grants each connection to agents. Every side effect
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
    anyhow::ensure!(kind == "email", "kind must be email");
    for k in ["imap_host", "smtp_host", "username"] {
        anyhow::ensure!(config.get(k).and_then(Value::as_str).is_some_and(|s| !s.is_empty()), "email connections need `{k}`");
    }
    let agent_ids: Vec<i64> = serde_json::from_value(p["agent_ids"].clone()).unwrap_or_default();
    let given = p["secret"].as_str().filter(|s| !s.is_empty());
    let secret = match given {
        Some(s) => s.to_string(),
        None => vault.get(&vault_key(name)).await?.context("give the app password or token")?,
    };
    // sign in once now, so a wrong host or password shows up here and not at the first scheduled run
    let c = Conn { name: name.into(), kind: kind.into(), config: Value::Object(config.clone()), secret };
    let _ = imap(&c).await.context("couldn't sign in to IMAP")?.logout().await;
    if let Some(s) = given {
        vault.set(&vault_key(name), s).await?;
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

pub const TOOLS: [&str; 5] = ["email_search", "email_read", "email_move", "email_draft", "email_send"];

/// Tools that read outside content (it may carry prompt injection).
pub fn untrusted(tool: &str) -> bool {
    matches!(tool, "email_search" | "email_read")
}

pub fn side_effect(tool: &str) -> bool {
    TOOLS.contains(&tool) && !untrusted(tool)
}

/// Tool specs for the email connections the agent has, each taking a `connection` name.
pub fn specs(conns: &[(String, String)]) -> Vec<Value> {
    let names: Vec<&str> = conns.iter().filter(|(_, k)| k == "email").map(|(n, _)| n.as_str()).collect();
    if names.is_empty() {
        return vec![];
    }
    let conn = json!({"type": "string", "enum": names, "description": "which email connection"});
    let s = |v: &str| json!({"type": "string", "description": v});
    let f = |name: &str, desc: &str, mut props: Value, req: &[&str]| {
        props["connection"] = conn.clone();
        let mut req: Vec<&str> = req.to_vec();
        req.push("connection");
        json!({"type": "function", "function": {"name": name, "description": desc,
            "parameters": {"type": "object", "properties": props, "required": req}}})
    };
    let message = json!({"to": {"type": "array", "items": {"type": "string"}}, "cc": {"type": "array", "items": {"type": "string"}},
        "subject": s("subject"), "body": s("plain-text body"), "in_reply_to": s("Message-ID being answered")});
    vec![
        f("email_search", "List recent emails in a folder, newest first, optionally matching words",
          json!({"folder": s("IMAP folder, default INBOX"), "query": s("words that must appear in the email; leave out to list every email"), "limit": {"type": "integer"}}), &[]),
        f("email_read", "Read one email by UID", json!({"folder": s("default INBOX"), "uid": {"type": "integer"}}), &["uid"]),
        f("email_move", "Move an email to another folder (undoable)",
          json!({"folder": s("default INBOX"), "uid": {"type": "integer"}, "to": s("destination folder")}), &["uid", "to"]),
        f("email_draft", "Save an email as a draft in the user's Drafts folder (undoable). Nothing is sent: the user reviews and sends it",
          message.clone(), &["to", "subject", "body"]),
        f("email_send", "Send an email. It waits 60 s in the outbox so the user can cancel it", message, &["to", "subject", "body"]),
    ]
}

/// Run a connector tool for `agent_id`. Errors come back as text for the model.
pub async fn call(st: &State, agent_id: i64, name: &str, args: &Value) -> Outcome {
    let res = async {
        let c = get(st, args["connection"].as_str().context("missing `connection`")?, Some(agent_id)).await?;
        anyhow::ensure!(c.kind == "email", "connection `{}` is a {} connection", c.name, c.kind);
        email(st, &c, agent_id, name, args).await
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
        "delete_draft" => {
            let mut imap = imap(&c).await?;
            imap.select(s("folder")).await?;
            let uids = imap.uid_search(format!("HEADER Message-ID {}", quote(&s("message_id")))).await?;
            anyhow::ensure!(!uids.is_empty(), "the draft is no longer there (sent or deleted)");
            let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            imap.uid_store(&set, "+FLAGS (\\Deleted)").await?.collect::<Vec<_>>().await;
            imap.uid_expunge(&set).await?.collect::<Vec<_>>().await;
            let _ = imap.logout().await;
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
    if name == "email_draft" {
        return save_draft(c, args).await;
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
            let _ = imap.create(to).await; // fails harmlessly when it exists
            imap.uid_mv(uid.to_string(), to).await.with_context(|| format!("moving to {to}"))?;
            Outcome { output: format!("moved email {uid} from {folder} to {to}"),
                inverse: mid.map(|m| json!({"connector": "email", "op": "move_back", "connection": c.name, "folder": to, "to": folder, "message_id": m})) }
        }
    };
    let _ = imap.logout().await;
    Ok(out)
}

/// Append the message to the Drafts folder; undo deletes it by its Message-ID.
async fn save_draft(c: &Conn, args: &Value) -> Result<Outcome> {
    let mid = format!("<{}.draft@burrow>", chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default());
    let msg = json!({"to": args["to"], "cc": args["cc"], "subject": args["subject"].as_str().unwrap_or(""),
        "body": args["body"].as_str().unwrap_or(""), "in_reply_to": args["in_reply_to"].as_str(), "message_id": mid});
    let raw = build(c, &msg)?.formatted();
    let mut imap = imap(c).await?;
    let mut folder = None;
    {
        let mut names = imap.list(Some(""), Some("*")).await?;
        while let Some(n) = names.next().await {
            let n = n?;
            if n.attributes().iter().any(|a| matches!(a, async_imap::types::NameAttribute::Drafts)) {
                folder = Some(n.name().to_string());
            }
        }
    }
    let folder = match folder {
        Some(f) => f,
        None => {
            let _ = imap.create("Drafts").await; // fails harmlessly when it exists
            "Drafts".to_string()
        }
    };
    imap.append(&folder, Some("(\\Draft \\Seen)"), None, raw).await.with_context(|| format!("saving to {folder}"))?;
    let _ = imap.logout().await;
    Ok(Outcome { output: format!("saved draft \"{}\" in {folder}; the user sends it from their mail app", msg["subject"].as_str().unwrap_or("")),
        inverse: Some(json!({"connector": "email", "op": "delete_draft", "connection": c.name, "folder": folder, "message_id": mid})) })
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
    if let Some(id) = m["message_id"].as_str() {
        b = b.message_id(Some(id.into()));
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

// ---- event triggers ----

/// Poll one event trigger. Returns the new cursor and, if something new arrived, text for the
/// run's prompt. The first poll (no cursor) only records where we are.
pub async fn poll(st: &State, kind: &str, spec: &str, cursor: Option<&str>) -> Result<(String, Option<String>)> {
    let c = get(st, spec.trim(), None).await?;
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
        _ => bail!("unknown trigger kind `{kind}`"),
    }
}

/// Check an event trigger's spec when the schedule is saved.
pub async fn validate_trigger(db: &SqlitePool, agent_id: i64, kind: &str, spec: &str) -> Result<()> {
    let conn = spec.trim();
    let granted = for_agent(db, agent_id).await?;
    anyhow::ensure!(granted.iter().any(|(n, k)| n == conn && k == kind), "`{conn}` is not a {kind} connection this agent may use");
    Ok(())
}
