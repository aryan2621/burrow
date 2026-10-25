use crate::agent::{self, State};
use crate::queue::Priority;
use crate::{backup, connectors, download, notes, hardware, journal, mcp, models, policy, schedule, search, secrets, service, store};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Where clients connect: a Unix socket in `home`, or a per-user named pipe on Windows.
pub fn endpoint(home: &Path) -> PathBuf {
    if cfg!(windows) {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        PathBuf::from(format!(r"\\.\pipe\burrowd-{user}"))
    } else {
        home.join("burrowd.sock")
    }
}

#[cfg(unix)]
pub async fn serve(home: &Path, st: Arc<State>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let sock = endpoint(home);
    if tokio::net::UnixStream::connect(&sock).await.is_ok() {
        anyhow::bail!("burrowd is already running ({})", sock.display());
    }
    let _ = std::fs::remove_file(&sock); // stale socket from a crash
    let listener = tokio::net::UnixListener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    eprintln!("burrowd listening on {}", sock.display());
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle(stream, st.clone()));
    }
}

#[cfg(windows)]
pub async fn serve(home: &Path, st: Arc<State>) -> Result<()> {
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::{
        Security::{Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES},
    };
    // Owner-only DACL: other local users can't open the pipe at all. The descriptor lives
    // as long as the server loop (the whole process), so it's never freed.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)".encode_utf16().chain([0]).collect();
    let mut sd = std::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error()).context("pipe security descriptor");
    }
    let mut sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let name = endpoint(home);
    let mut create = |first: bool| -> std::io::Result<NamedPipeServer> {
        let mut o = ServerOptions::new();
        o.first_pipe_instance(first).reject_remote_clients(true);
        unsafe { o.create_with_security_attributes_raw(&name, &mut sa as *mut _ as *mut _) }
    };
    let mut server = create(true).context("burrowd is already running")?;
    eprintln!("burrowd listening on {}", name.display());
    loop {
        server.connect().await?;
        let client = std::mem::replace(&mut server, create(false)?);
        tokio::spawn(handle(client, st.clone()));
    }
}

/// Newline-delimited JSON-RPC 2.0. A client gets notifications after `events.subscribe`.
async fn handle<S: AsyncRead + AsyncWrite + Send + 'static>(stream: S, st: Arc<State>) {
    let (r, mut w) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(v) = rx.recv().await {
            if w.write_all(format!("{v}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let mut forward: Option<tokio::task::JoinHandle<()>> = None;
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                let id = req["id"].clone();
                let method = req["method"].as_str().unwrap_or("");
                let res = if method == "events.subscribe" {
                    if forward.is_none() {
                        let mut events = st.events.subscribe();
                        let ev_tx = tx.clone();
                        forward = Some(tokio::spawn(async move {
                            // a lagging client just misses some deltas; the DB still has the full history
                            loop {
                                match events.recv().await {
                                    Ok(v) => if ev_tx.send(v).is_err() { break },
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                    Err(_) => break,
                                }
                            }
                        }));
                    }
                    Ok(json!(true))
                } else {
                    dispatch(&st, method, &req["params"]).await
                };
                match res {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err(e) => json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32000, "message": format!("{e:#}")}}),
                }
            }
            Err(e) => json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
        };
        if tx.send(resp).is_err() {
            break;
        }
    }
    if let Some(f) = forward {
        f.abort();
    }
    drop(tx);
    let _ = writer.await;
}

fn domains(v: &Value) -> Result<Vec<String>> {
    v.as_array().map(|a| a.iter().map(|d| {
        let d = d.as_str().context("domain must be a string")?.trim().to_lowercase();
        anyhow::ensure!(!d.is_empty() && !d.contains(['/', ':', ' ']), "bad domain `{d}` (use e.g. example.com)");
        Ok(d)
    }).collect()).unwrap_or(Ok(vec![]))
}

fn budget(v: &Value) -> Result<Option<i64>> {
    match v {
        Value::Null => Ok(None),
        v => {
            let b = v.as_i64().context("`context_budget` must be an integer")?;
            anyhow::ensure!((1_000..=1_000_000).contains(&b), "context_budget must be 1000–1000000 tokens");
            Ok(Some(b))
        }
    }
}

/// Fields given in `v` override `cur`.
fn limits(v: &Value, cur: store::Limits) -> Result<store::Limits> {
    let mut m = serde_json::to_value(cur)?;
    for (k, x) in v.as_object().into_iter().flatten() {
        m[k] = x.clone();
    }
    let l: store::Limits = serde_json::from_value(m).context("bad limits")?;
    anyhow::ensure!((1..=200).contains(&l.max_steps), "max_steps must be 1–200");
    anyhow::ensure!((10..=24 * 3600).contains(&l.max_run_secs), "max_run_secs must be 10–86400");
    anyhow::ensure!((1_000..=10_000_000).contains(&l.max_run_tokens), "max_run_tokens must be 1000–10000000");
    Ok(l)
}

/// UI preferences only; settings with their own validation (search) have their own RPCs.
fn setting_key(k: &str) -> Result<&str> {
    anyhow::ensure!(matches!(k, "onboarded"), "unknown setting `{k}`");
    Ok(k)
}

fn rules(v: &Value) -> Result<std::collections::BTreeMap<String, String>> {
    let r = if v.is_null() { Default::default() } else { serde_json::from_value(v.clone()).context("bad approval_rules")? };
    policy::validate(&r)?;
    Ok(r)
}

async fn dispatch(st: &Arc<State>, method: &str, p: &Value) -> Result<Value> {
    let s = |k: &str| p[k].as_str().with_context(|| format!("missing string `{k}`"));
    let int = |k: &str| p[k].as_i64().with_context(|| format!("missing integer `{k}`"));
    let id = || int("agent_id");
    Ok(match method {
        "status" => json!({"sandbox": st.sandbox.bin, "service_installed": service::installed().unwrap_or(false),
            "keystore": match st.vault { secrets::Vault::Keychain => "keychain", secrets::Vault::File(_) => "file" },
            "search": search::name(&search::backend(&st.db, &st.vault).await?)}),
        "search.configure" => {
            search::configure(&st.db, &st.vault, p).await?;
            json!(true)
        }
        "domains.presets" => json!({"package_registries": policy::PACKAGE_REGISTRIES}),
        "models.list" => {
            let mut m = models::list(&st.http, &st.ollama_url, &st.llama, &st.home).await;
            m.extend(models::cloud(&st.http, &st.db, &st.vault).await);
            json!(m)
        }
        "providers.list" => models::providers(&st.db, &st.vault).await?,
        "providers.set" => {
            models::configure(&st.db, &st.vault, p).await?;
            json!(true)
        }
        "notes.list" => json!(notes::list(&st.db, id()?, p["query"].as_str().unwrap_or("")).await?),
        "notes.update" => {
            notes::update(&st.db, id()?, int("note_id")?, s("text")?).await?;
            json!(true)
        }
        "notes.delete" => {
            notes::delete(&st.db, int("note_id")?).await?;
            json!(true)
        }
        "connections.list" => json!(connectors::list(&st.db).await?),
        "connections.set" => {
            connectors::set(&st.db, &st.vault, p).await?;
            json!(true)
        }
        "connections.delete" => {
            connectors::delete(&st.db, &st.vault, s("name")?).await?;
            json!(true)
        }
        "backup.export" => {
            backup::export(st, Path::new(s("path")?), s("passphrase")?).await?;
            json!(true)
        }
        "backup.import" => {
            backup::import(st, Path::new(s("path")?), s("passphrase")?).await?;
            json!(true)
        }
        "data.wipe" => {
            anyhow::ensure!(p["confirm"] == "wipe", "pass confirm: \"wipe\"");
            backup::wipe(st).await?;
            json!(true)
        }
        "hardware.detect" => hardware::detect(),
        "models.catalog" => {
            let busy: Vec<String> = st.downloads.ids();
            json!(hardware::catalog().into_iter().map(|m| {
                let mut v = json!(m);
                v["installed"] = json!(download::installed(&st.home, &m));
                v["downloading"] = json!(busy.contains(&m.id));
                v
            }).collect::<Vec<_>>())
        }
        "models.download" => {
            download::start(st, s("id")?)?;
            json!(true)
        }
        "models.cancel" => {
            download::cancel(st, s("id")?)?;
            json!(true)
        }
        "models.pull" => {
            models::pull(&st.http, &st.ollama_url, s("name")?).await?;
            json!(true)
        }
        "agents.list" => json!(store::agents(&st.db).await?),
        "agents.create" => {
            models::parse_ref(s("model_ref")?)?;
            let folders = p["folders"].as_array().context("missing `folders`")?.iter()
                .map(|f| {
                    let f = PathBuf::from(f.as_str().context("folder must be a string")?);
                    anyhow::ensure!(f.is_absolute() && f.is_dir(), "{} is not an existing absolute directory", f.display());
                    Ok(f.canonicalize()?)
                })
                .collect::<Result<Vec<_>>>()?;
            anyhow::ensure!(!folders.is_empty(), "give the agent at least one folder");
            let a = store::Agent { id: 0, name: s("name")?.into(), role_prompt: s("role_prompt")?.into(),
                model_ref: s("model_ref")?.into(), folders,
                allowed_domains: domains(&p["allowed_domains"])?, approval_rules: rules(&p["approval_rules"])?,
                context_budget: budget(&p["context_budget"])?.unwrap_or(8_000), limits: limits(&p["limits"], Default::default())? };
            json!(store::create_agent(&st.db, &a).await?)
        }
        "agents.update" => {
            let mut a = store::get_agent(&st.db, id()?).await?;
            a.allowed_domains = domains(&p["allowed_domains"])?;
            a.approval_rules = rules(&p["approval_rules"])?;
            if let Some(b) = budget(&p["context_budget"])? {
                a.context_budget = b;
            }
            a.limits = limits(&p["limits"], a.limits)?;
            if let Some(m) = p["model_ref"].as_str() {
                models::parse_ref(m)?;
                a.model_ref = m.into();
            }
            store::update_permissions(&st.db, &a).await?;
            json!(true)
        }
        "agents.delete" => {
            agent::kill(st, id()?).await?;
            store::delete_agent(&st.db, id()?).await?;
            let _ = std::fs::remove_dir_all(st.home.join("scratch").join(id()?.to_string()));
            json!(true)
        }
        "agents.kill" => {
            agent::kill(st, id()?).await?;
            json!(true)
        }
        "chat.history" => json!(store::messages(&st.db, id()?).await?),
        "chat.send" => {
            store::get_agent(&st.db, id()?).await?;
            agent::spawn(st.clone(), id()?, s("text")?.into(), Priority::Interactive, "chat".into())?;
            json!(true)
        }
        "chat.busy" => json!(st.busy()),
        "actions.list" => json!(journal::list(&st.db, id()?).await?),
        "actions.undo" => {
            journal::undo(&st.db, &st.snapshots(), Some(st), int("action_id")?, p["force"].as_bool().unwrap_or(false)).await?;
            json!(true)
        }
        "runs.undo" => json!(journal::undo_run(&st.db, &st.snapshots(), Some(st), int("run_id")?, p["force"].as_bool().unwrap_or(false)).await?),
        "journal.verify" => json!(journal::verify(&st.db).await?),
        "model_calls.list" => json!(store::model_calls(&st.db, id()?).await?),
        "secrets.set" => {
            secrets::set(&st.db, &st.vault, p).await?;
            json!(true)
        }
        "secrets.list" => json!(secrets::list(&st.db).await?),
        "secrets.delete" => {
            secrets::delete(&st.db, &st.vault, s("name")?).await?;
            json!(true)
        }
        "mcp.list" => json!(mcp::list(&st.db, id()?).await?),
        "mcp.add" => {
            store::get_agent(&st.db, id()?).await?;
            json!({"warnings": mcp::add(&st.db, p).await?})
        }
        "mcp.add_browser" => {
            store::get_agent(&st.db, id()?).await?;
            mcp::add(&st.db, &json!({"agent_id": id()?, "name": "browser", "transport": "stdio", "command": mcp::BROWSER})).await?;
            json!(true)
        }
        "files.image" => {
            // only the agent's scratch folder, where screenshots land
            let p = Path::new(s("path")?).canonicalize()?;
            anyhow::ensure!(p.starts_with(st.scratch(id()?)?) && matches!(p.extension().and_then(|e| e.to_str()), Some("png" | "jpg")), "not a screenshot");
            use base64::Engine;
            let mime = if p.extension().is_some_and(|e| e == "jpg") { "image/jpeg" } else { "image/png" };
            json!(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(std::fs::read(&p)?)))
        }
        "mcp.remove" => {
            mcp::remove(&st.db, id()?, s("name")?).await?;
            json!(true)
        }
        "settings.get" => {
            let v: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?").bind(setting_key(s("key")?)?)
                .fetch_optional(&st.db).await?;
            json!(v)
        }
        "settings.set" => {
            sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(setting_key(s("key")?)?).bind(s("value")?).execute(&st.db).await?;
            json!(true)
        }
        "service.install" => {
            let home = st.home.clone();
            tokio::task::spawn_blocking(move || service::register(&home)).await??;
            // hand over after this reply: exit, then the service starts once the socket is free
            st.handover.store(true, std::sync::atomic::Ordering::SeqCst);
            let st2 = st.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                st2.shutdown.notify_one();
            });
            json!(true)
        }
        "service.uninstall" => {
            // after the reply: if this daemon is the service, uninstalling stops it (the app restarts one)
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let _ = tokio::task::spawn_blocking(service::uninstall).await;
            });
            json!(true)
        }
        "runs.list" => json!(store::runs(&st.db, id()?).await?),
        "schedules.list" => json!(schedule::list(&st.db, id()?).await?),
        "schedules.add" => json!(schedule::add(&st.db, p).await?),
        "schedules.update" => {
            schedule::update(&st.db, p).await?;
            json!(true)
        }
        "schedules.delete" => {
            schedule::delete(&st.db, int("id")?).await?;
            json!(true)
        }
        "approvals.pending" => json!(policy::pending(st).await?),
        "approvals.decide" => {
            let allow = p["allow"].as_bool().context("missing bool `allow`")?;
            policy::decide(st, int("approval_id")?, allow, p["remember"].as_bool().unwrap_or(false)).await?;
            json!(true)
        }
        _ => anyhow::bail!("unknown method `{method}`"),
    })
}
