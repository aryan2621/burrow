//! The built-in llama.cpp runtime. The daemon owns one `llama-server` process with one
//! model loaded and `PARALLEL` slots shared by all agents (through `queue.rs`). An agent
//! that needs a different model waits for in-flight calls, then the server restarts.
use crate::{download, hardware};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, RwLockReadGuard};

pub const PARALLEL: usize = 4;

pub struct Loaded {
    id: String,
    child: Mutex<Child>,
}

pub struct Llama {
    pub url: String,
    /// None: an external server at `url` (`BURROW_LLAMA_URL`), not ours to start.
    port: Option<u16>,
    home: PathBuf,
    loaded: RwLock<Option<Loaded>>,
}

/// `BURROW_LLAMA_SERVER`, then the bundled copy (dev install, next to the daemon, macOS app
/// Resources), then PATH.
pub fn find_server(home: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("BURROW_LLAMA_SERVER") {
        return Some(p.into());
    }
    let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let mut dirs = vec![home.join("llama")];
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(d.join("llama"));
        dirs.push(d.join("../Resources/llama"));
    }
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

impl Llama {
    /// Managed when `external` is empty: picks a free loopback port now, starts on first use.
    pub fn new(home: PathBuf, external: String) -> Result<Self> {
        if !external.is_empty() {
            return Ok(Llama { url: external, port: None, home, loaded: RwLock::new(None) });
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        Ok(Llama { url: format!("http://127.0.0.1:{port}"), port: Some(port), home, loaded: RwLock::new(None) })
    }

    pub fn managed(&self) -> bool {
        self.port.is_some()
    }

    /// Make sure `id` is the loaded model. Hold the guard for the whole model call so a
    /// switch can't kill the server mid-stream.
    pub async fn use_model(&self, http: &reqwest::Client, id: &str) -> Result<Option<RwLockReadGuard<'_, Option<Loaded>>>> {
        let Some(port) = self.port else { return Ok(None) };
        let alive = |l: &Option<Loaded>| l.as_ref().is_some_and(|l| l.id == id && matches!(l.child.lock().unwrap().try_wait(), Ok(None)));
        {
            let r = self.loaded.read().await;
            if alive(&r) {
                return Ok(Some(r));
            }
        }
        let mut w = self.loaded.write().await;
        if !alive(&w) {
            if let Some(old) = w.take() {
                let _ = old.child.into_inner().unwrap().kill().await;
            }
            *w = Some(self.start(http, port, id).await?);
        }
        Ok(Some(w.downgrade()))
    }

    async fn start(&self, http: &reqwest::Client, port: u16, id: &str) -> Result<Loaded> {
        let m = hardware::model(id)?;
        anyhow::ensure!(download::installed(&self.home, &m), "{} isn't downloaded yet; download it in Settings → Models", m.name);
        let bin = find_server(&self.home).context("llama-server not found; it ships with the app (dev: scripts/fetch-llama.sh)")?;
        let log = std::fs::File::create(self.home.join("llama-server.log"))?;
        let total_ctx = (m.ctx * PARALLEL as u64).to_string();
        let mut child = Command::new(&bin)
            .args(["--jinja", "--host", "127.0.0.1", "--port", &port.to_string(), "--parallel", &PARALLEL.to_string(),
                   "-c", &total_ctx, "-ngl", "99", "-ctk", "q8_0", "-ctv", "q8_0", "--alias", id, "-m"])
            .arg(download::dir(&self.home).join(&m.file))
            .stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", bin.display()))?;
        // loading a large model from a cold disk can take a while
        let t0 = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                bail!("llama-server exited ({status}) while loading {}; see {}", m.name, self.home.join("llama-server.log").display());
            }
            if http.get(format!("{}/health", self.url)).send().await.is_ok_and(|r| r.status().is_success()) {
                break;
            }
            if t0.elapsed() > Duration::from_secs(300) {
                bail!("llama-server didn't become ready in 5 minutes");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(Loaded { id: id.to_string(), child: Mutex::new(child) })
    }

    pub async fn stop(&self) {
        if let Some(l) = self.loaded.write().await.take() {
            let _ = l.child.into_inner().unwrap().kill().await;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake llama-server: answers /health and records its `--alias`.
    fn fake_server(dir: &Path) -> PathBuf {
        let p = dir.join("llama-server");
        std::fs::write(&p, r#"#!/usr/bin/env python3
import sys, http.server
a = sys.argv
port, alias = int(a[a.index("--port") + 1]), a[a.index("--alias") + 1]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = alias.encode()
        self.send_response(200); self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[tokio::test]
    async fn starts_switches_and_refuses_missing_models() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        std::env::set_var("BURROW_LLAMA_SERVER", fake_server(&home));
        let l = Llama::new(home.clone(), String::new()).unwrap();
        let http = reqwest::Client::new();
        let err = l.use_model(&http, "qwen3-4b").await.err().unwrap();
        assert!(format!("{err:#}").contains("isn't downloaded"), "{err:#}");

        std::fs::create_dir_all(download::dir(&home)).unwrap();
        for id in ["qwen3-4b", "qwen3-8b"] {
            std::fs::write(download::dir(&home).join(hardware::model(id).unwrap().file), "").unwrap();
        }
        let who = |l: &Llama| { let (http, url) = (http.clone(), l.url.clone());
            async move { http.get(format!("{url}/health")).send().await.unwrap().text().await.unwrap() } };
        drop(l.use_model(&http, "qwen3-4b").await.unwrap());
        assert_eq!(who(&l).await, "qwen3-4b");
        let pid = l.loaded.read().await.as_ref().unwrap().child.lock().unwrap().id();
        drop(l.use_model(&http, "qwen3-4b").await.unwrap());
        assert_eq!(l.loaded.read().await.as_ref().unwrap().child.lock().unwrap().id(), pid, "same model reuses the server");
        drop(l.use_model(&http, "qwen3-8b").await.unwrap());
        assert_eq!(who(&l).await, "qwen3-8b");
        l.stop().await;
        std::env::remove_var("BURROW_LLAMA_SERVER");
    }
}
