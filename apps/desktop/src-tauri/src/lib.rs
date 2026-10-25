use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

trait Conn: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Conn for T {}

/// Must match `burrowd`'s `rpc::endpoint`.
fn endpoint() -> PathBuf {
    if cfg!(windows) {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        return PathBuf::from(format!(r"\\.\pipe\burrowd-{user}"));
    }
    match std::env::var_os("BURROW_HOME") {
        Some(h) => PathBuf::from(h),
        None => dirs::home_dir().expect("no home dir").join(".burrow"),
    }
    .join("burrowd.sock")
}

async fn open() -> std::io::Result<Box<dyn Conn>> {
    #[cfg(unix)]
    return Ok(Box::new(tokio::net::UnixStream::connect(endpoint()).await?));
    #[cfg(windows)]
    return Ok(Box::new(tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint())?));
}

/// Start burrowd detached from the app so it keeps running after the app quits.
/// shortcut: looks next to the app binary or on PATH; replace with launchd/systemd/service install (phase 3).
fn spawn_daemon() {
    let exe = if cfg!(windows) { "burrowd.exe" } else { "burrowd" };
    let bin = std::env::var_os("BURROW_DAEMON").map(PathBuf::from).unwrap_or_else(|| {
        let sibling = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join(exe)));
        sibling.filter(|p| p.exists()).unwrap_or_else(|| exe.into())
    });
    let mut cmd = std::process::Command::new(&bin);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x8 | 0x200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    if let Err(e) = cmd.spawn() {
        eprintln!("could not start {}: {e}", bin.display());
    }
}

async fn connect() -> Result<Box<dyn Conn>, String> {
    if let Ok(s) = open().await {
        return Ok(s);
    }
    spawn_daemon();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(s) = open().await {
            return Ok(s);
        }
    }
    Err("burrowd is not running and could not be started".into())
}

/// One request per connection; notifications that arrive first are skipped.
#[tauri::command]
async fn rpc(method: String, params: Value) -> Result<Value, String> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (r, mut w) = tokio::io::split(connect().await?);
    let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    w.write_all(format!("{req}\n").as_bytes()).await.map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if v["id"] != json!(id) {
            continue;
        }
        return match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or("rpc error").into()),
            None => Ok(v["result"].clone()),
        };
    }
    Err("burrowd closed the connection".into())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

async fn agent_name(id: &Value) -> String {
    let agents = rpc("agents.list".into(), json!({})).await.unwrap_or_default();
    agents.as_array().into_iter().flatten().find(|a| &a["id"] == id)
        .and_then(|a| a["name"].as_str().map(String::from)).unwrap_or_else(|| "An agent".into())
}

/// Desktop notifications for things that need the user while the window may be hidden.
async fn notify(app: &AppHandle, v: &Value) {
    let p = &v["params"];
    let (title, body) = match v["method"].as_str() {
        Some("approval.requested") => {
            let what = if p["tool"] == "net" { format!("connect to {}", p["args"]["host"].as_str().unwrap_or("a site")) } else { format!("run {}", p["tool"].as_str().unwrap_or("a tool")) };
            (format!("{} needs your approval", agent_name(&p["agent_id"]).await), format!("It wants to {what}."))
        }
        // interactive runs finish in front of the user; scheduled ones are worth a heads-up
        Some("chat.done") if p["trigger"].as_str().is_some_and(|t| t != "chat") => {
            let name = agent_name(&p["agent_id"]).await;
            match p["error"].as_str() {
                Some(e) => (format!("{name}'s scheduled run failed"), e.chars().take(120).collect()),
                None => (format!("{name} finished a scheduled run"), "Open Burrow to see what it did.".into()),
            }
        }
        Some("models.downloaded") => ("Model ready".into(), format!("{} finished downloading.", p["id"].as_str().unwrap_or("The model"))),
        Some("models.download_failed") if p["error"] != "cancelled" =>
            ("Model download failed".into(), p["error"].as_str().unwrap_or("").chars().take(120).collect()),
        _ => return,
    };
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Keep one connection open and re-emit daemon notifications as `burrow` events.
async fn forward_events(app: tauri::AppHandle) {
    loop {
        if let Ok(mut s) = connect().await {
            let sub = json!({"jsonrpc": "2.0", "id": 0, "method": "events.subscribe"});
            if s.write_all(format!("{sub}\n").as_bytes()).await.is_err() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    notify(&app, &v).await;
                    let _ = app.emit("burrow", v);
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open Burrow", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Burrow (agents keep running)", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("bundle icon").clone())
                .tooltip("Burrow")
                .menu(&Menu::with_items(app, &[&open, &quit])?)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            tauri::async_runtime::spawn(forward_events(app.handle().clone()));
            Ok(())
        })
        // closing the window only hides it: the tray stays and notifications keep coming
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .build(tauri::generate_context!())
        .expect("error while building burrow")
        .run(|_app, _e| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _e {
                show_main(_app); // dock click
            }
        });
}
