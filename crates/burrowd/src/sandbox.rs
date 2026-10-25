//! One long-lived, network-less container per agent. Works with any runtime that speaks the
//! docker-style CLI: Apple `container` (a VM per container, macOS), Podman or Docker
//! (Linux, Windows, macOS fallback).
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::process::Command;

const MAX_OUT: usize = 16 * 1024;

pub struct Sandbox {
    /// Runtime binary, or None when nothing is installed.
    pub bin: Option<String>,
    image: String,
    started: Mutex<HashMap<i64, Instant>>,
}

fn on_path(bin: &str) -> bool {
    let exe = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(&exe).is_file()))
        .unwrap_or(false)
}

/// Where a host folder appears inside the Linux sandbox. Same path on Unix so the
/// model sees one set of paths; `C:\x\y` → `/mnt/c/x/y` on Windows.
pub fn inside(host: &Path) -> String {
    let s = host.to_string_lossy().replace('\\', "/");
    let s = s.trim_start_matches("//?/");
    match s.split_once(":/") {
        Some((drive, rest)) if cfg!(windows) => format!("/mnt/{}/{rest}", drive.to_lowercase()),
        _ => s.to_string(),
    }
}

impl Sandbox {
    /// `BURROW_SANDBOX` overrides; otherwise prefer VM isolation (Apple container), then Podman, Docker.
    pub fn detect() -> Self {
        let bin = std::env::var("BURROW_SANDBOX").ok().or_else(|| {
            ["container", "podman", "docker"].into_iter().find(|b| on_path(b)).map(String::from)
        });
        let image = std::env::var("BURROW_SANDBOX_IMAGE").unwrap_or_else(|_| "python:3.12-slim".into());
        Sandbox { bin, image, started: Default::default() }
    }

    fn name(agent_id: i64) -> String {
        format!("burrow-agent-{agent_id}")
    }

    async fn start(&self, bin: &str, agent_id: i64, folders: &[std::path::PathBuf], scratch: &Path) -> Result<()> {
        let _ = Command::new(bin).args(["rm", "-f", &Self::name(agent_id)]).output().await;
        let mut cmd = Command::new(bin);
        cmd.args(["run", "-d", "--name", &Self::name(agent_id), "--network", "none", "--cpus", "2", "--memory", "2048M"]);
        if bin != "container" {
            // shared-kernel runtimes: drop what a VM would have isolated anyway
            cmd.args(["--cap-drop", "ALL", "--security-opt", "no-new-privileges"]);
        }
        for f in folders {
            cmd.args(["-v", &format!("{}:{}:ro", f.display(), inside(f))]);
        }
        cmd.args(["-v", &format!("{}:{}", scratch.display(), inside(scratch))]);
        cmd.args([&self.image, "sleep", "infinity"]);
        // first start may pull the image
        let out = tokio::time::timeout(Duration::from_secs(600), cmd.kill_on_drop(true).output())
            .await
            .context("sandbox start timed out")??;
        if !out.status.success() {
            bail!("sandbox start failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    /// Run `command` with `sh -c` in the agent's sandbox, starting it if needed.
    pub async fn exec(&self, agent_id: i64, folders: &[std::path::PathBuf], scratch: &Path, command: &str, timeout: Duration) -> Result<String> {
        let bin = self.bin.as_deref()
            .context("no sandbox runtime found; install Apple container (macOS), Podman or Docker")?;
        let fresh = !self.started.lock().unwrap().contains_key(&agent_id);
        if fresh {
            self.start(bin, agent_id, folders, scratch).await?;
        }
        self.started.lock().unwrap().insert(agent_id, Instant::now());
        // `timeout` inside so the process dies in the sandbox too, not just our exec client
        let secs = timeout.as_secs().to_string();
        let out = tokio::time::timeout(
            timeout + Duration::from_secs(10),
            Command::new(bin)
                .args(["exec", "-w", &inside(scratch), &Self::name(agent_id), "timeout", "-k", "5", &secs, "sh", "-c", command])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("sandbox exec hung")??;
        let clip = |b: &[u8]| {
            let s = String::from_utf8_lossy(b);
            if s.len() > MAX_OUT { format!("{}…[truncated]", &s[..s.floor_char_boundary(MAX_OUT)]) } else { s.into_owned() }
        };
        let code = match out.status.code() {
            Some(124) => "timed out".to_string(),
            Some(c) => c.to_string(),
            None => "killed".into(),
        };
        Ok(format!("exit: {code}\nstdout:\n{}\nstderr:\n{}", clip(&out.stdout), clip(&out.stderr)))
    }

    pub async fn stop(&self, agent_id: i64) {
        let was = self.started.lock().unwrap().remove(&agent_id).is_some();
        if let (true, Some(bin)) = (was, &self.bin) {
            let _ = Command::new(bin).args(["rm", "-f", &Self::name(agent_id)]).output().await;
        }
    }

    pub async fn stop_idle(&self, idle: Duration) {
        let ids: Vec<i64> = self.started.lock().unwrap().iter()
            .filter(|(_, t)| t.elapsed() > idle).map(|(id, _)| *id).collect();
        for id in ids {
            self.stop(id).await;
        }
    }

    pub async fn stop_all(&self) {
        let ids: Vec<i64> = self.started.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.stop(id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a runtime: `cargo test -- --ignored sandbox`.
    #[tokio::test]
    #[ignore]
    async fn sandbox_isolation() {
        let sb = Sandbox::detect();
        assert!(sb.bin.is_some(), "no runtime installed");
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (ro, scratch) = (root.join("ro"), root.join("scratch"));
        std::fs::create_dir_all(&ro).unwrap();
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(ro.join("f"), "hi").unwrap();
        let id = 900_000 + std::process::id() as i64;
        let run = |c: String| { let (ro, scratch, sb) = (ro.clone(), scratch.clone(), &sb);
            async move { sb.exec(id, &[ro], &scratch, &c, Duration::from_secs(5)).await.unwrap() } };

        assert!(run(format!("cat {}/f", ro.display())).await.contains("hi"));
        assert!(!run(format!("touch {}/x", ro.display())).await.starts_with("exit: 0"));
        assert!(run("touch made && pwd".into()).await.starts_with("exit: 0"));
        assert!(scratch.join("made").exists());
        let net = run("python -c \"import urllib.request;urllib.request.urlopen('http://1.1.1.1',timeout=3)\"".into()).await;
        assert!(!net.starts_with("exit: 0"), "{net}");
        assert!(run("sleep 999".into()).await.starts_with("exit: timed out"));
        sb.stop_all().await;
    }

    #[test]
    fn inside_paths() {
        assert_eq!(inside(Path::new("/Users/me/notes")), "/Users/me/notes");
    }
}
