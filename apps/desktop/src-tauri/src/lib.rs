use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::Emitter;
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
        .setup(|app| {
            tauri::async_runtime::spawn(forward_events(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .run(tauri::generate_context!())
        .expect("error while running burrow");
}
