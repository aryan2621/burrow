//! One long-lived container per agent. Works with any runtime that speaks the docker-style
//! CLI: Apple `container` (a VM per container, macOS), Podman or Docker (Linux, Windows,
//! macOS fallback). A networked sandbox sits alone on an `--internal` network whose only way
//! out is the daemon's egress proxy (`proxy.rs`).
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::task::AbortHandle;

const MAX_OUT: usize = 16 * 1024;
// shortcut: switch to the GHCR build of sandbox-images/base once M6 publishes it (org name pending);
// until then build it locally and set BURROW_SANDBOX_IMAGE.
const DEFAULT_IMAGE: &str = "python:3.12-slim";

/// Docker/Podman internal networks can't reach the host, so a second container on both the
/// internal and the default network relays port 3128 to the daemon's proxy. Python because
/// the sandbox image already has it.
const FORWARDER: &str = r#"
import socket, sys, threading
target = (sys.argv[1], int(sys.argv[2]))
def pipe(a, b):
    try:
        while (d := a.recv(65536)):
            b.sendall(d)
        b.shutdown(socket.SHUT_WR)
    except OSError:
        a.close(); b.close()
srv = socket.create_server(("0.0.0.0", 3128))
while True:
    c, _ = srv.accept()
    try:
        u = socket.create_connection(target, timeout=10)
        u.settimeout(None)
    except OSError:
        c.close(); continue
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
"#;

struct Running {
    last: Instant,
    /// `KEY=value` pairs for every exec: the proxy settings.
    env: Vec<String>,
    proxy: Option<AbortHandle>,
}

pub struct Sandbox {
    /// Runtime binary, or None when nothing is installed.
    pub bin: Option<String>,
    image: String,
    started: Mutex<HashMap<i64, Running>>,
}

/// What a networked sandbox needs from the proxy: the port it listens on, its token and its task.
pub type Proxy = (u16, String, AbortHandle);

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

/// Proxy settings understood by curl, pip, uv, git, npm and Node's fetch.
fn proxy_env(proxy_url: &str) -> Vec<String> {
    ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "npm_config_proxy", "npm_config_https_proxy"]
        .iter().map(|k| format!("{k}={proxy_url}"))
        .chain(["NO_PROXY=localhost,127.0.0.1".into(), "NODE_USE_ENV_PROXY=1".into()])
        .collect()
}

async fn run_ok(cmd: &mut Command, what: &str) -> Result<String> {
    let out = cmd.kill_on_drop(true).output().await?;
    if !out.status.success() {
        bail!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Sandbox {
    /// `BURROW_SANDBOX` overrides; otherwise prefer VM isolation (Apple container), then Podman, Docker.
    pub fn detect() -> Self {
        let bin = std::env::var("BURROW_SANDBOX").ok().or_else(|| {
            ["container", "podman", "docker"].into_iter().find(|b| on_path(b)).map(String::from)
        });
        let image = std::env::var("BURROW_SANDBOX_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.into());
        Sandbox { bin, image, started: Default::default() }
    }

    fn name(agent_id: i64) -> String {
        format!("burrow-agent-{agent_id}")
    }

    fn net(agent_id: i64) -> String {
        format!("burrow-net-{agent_id}")
    }

    fn fwd(agent_id: i64) -> String {
        format!("burrow-fwd-{agent_id}")
    }

    fn bin(&self) -> Result<&str> {
        self.bin.as_deref().context("no sandbox runtime found; install Apple container (macOS), Podman or Docker")
    }

    pub fn is_running(&self, agent_id: i64) -> bool {
        self.started.lock().unwrap().contains_key(&agent_id)
    }

    /// Where the daemon's proxy listens so a Docker/Podman forwarder can reach it via `host-gateway`.
    async fn host_ip(bin: &str) -> Result<IpAddr> {
        if !cfg!(target_os = "linux") {
            return Ok(IpAddr::from([127, 0, 0, 1])); // Docker Desktop / podman machine map host-gateway to host loopback
        }
        // shortcut: rootless Podman has no host address on its bridge; works with Docker and rootful Podman.
        let (net, fmt) = if bin == "podman" { ("podman", "{{(index .Subnets 0).Gateway}}") } else { ("bridge", "{{(index .IPAM.Config 0).Gateway}}") };
        let ip = run_ok(Command::new(bin).args(["network", "inspect", net, "-f", fmt]), "finding the bridge gateway").await?;
        ip.parse().with_context(|| format!("bad gateway address `{ip}`"))
    }

    /// Start the agent's container. Without `proxy` it has no network at all. With it, the
    /// container gets its own internal network and `proxy` is called with the host address
    /// to listen on.
    pub async fn start<F, Fut>(&self, agent_id: i64, folders: &[std::path::PathBuf], scratch: &Path, proxy: Option<F>) -> Result<()>
    where F: FnOnce(IpAddr) -> Fut, Fut: Future<Output = Result<Proxy>> {
        let bin = self.bin()?;
        self.remove(bin, agent_id).await;
        let res = self.start_inner(bin, agent_id, folders, scratch, proxy).await;
        if res.is_err() {
            self.remove(bin, agent_id).await;
        }
        res
    }

    async fn start_inner<F, Fut>(&self, bin: &str, agent_id: i64, folders: &[std::path::PathBuf], scratch: &Path, proxy: Option<F>) -> Result<()>
    where F: FnOnce(IpAddr) -> Fut, Fut: Future<Output = Result<Proxy>> {
        let net = if proxy.is_some() {
            run_ok(Command::new(bin).args(["network", "create", "--internal", &Self::net(agent_id)]), "creating the sandbox network").await?;
            Self::net(agent_id)
        } else {
            "none".into()
        };
        let mut cmd = Command::new(bin);
        cmd.args(["run", "-d", "--name", &Self::name(agent_id), "--network", &net, "--cpus", "2", "--memory", "2048M"]);
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
        tokio::time::timeout(Duration::from_secs(600), run_ok(&mut cmd, "sandbox start"))
            .await
            .context("sandbox start timed out")??;

        let mut running = Running { last: Instant::now(), env: vec![], proxy: None };
        if let Some(proxy) = proxy {
            let url = if bin == "container" {
                // the host only gets the gateway address once a container is up on the network
                let v: serde_json::Value = serde_json::from_str(&run_ok(Command::new(bin).args(["network", "inspect", &net]), "network inspect").await?)?;
                let gw: IpAddr = v[0]["status"]["ipv4Gateway"].as_str().context("network has no gateway")?.parse()?;
                let (port, token, task) = proxy(gw).await?;
                running.proxy = Some(task);
                format!("http://burrow:{token}@{gw}:{port}")
            } else {
                let (port, token, task) = proxy(Self::host_ip(bin).await?).await?;
                running.proxy = Some(task);
                run_ok(Command::new(bin).args(["run", "-d", "--name", &Self::fwd(agent_id), "--add-host", "host.docker.internal:host-gateway",
                    "--cap-drop", "ALL", "--security-opt", "no-new-privileges", &self.image,
                    "python3", "-c", FORWARDER, "host.docker.internal", &port.to_string()]), "starting the proxy forwarder").await?;
                run_ok(Command::new(bin).args(["network", "connect", "--alias", "burrow-proxy", &net, &Self::fwd(agent_id)]), "connecting the forwarder").await?;
                format!("http://burrow:{token}@burrow-proxy:3128")
            };
            running.env = proxy_env(&url);
        }
        self.started.lock().unwrap().insert(agent_id, running);
        Ok(())
    }

    /// `exec` into a running sandbox with the proxy env set; `-i` keeps stdin open (MCP stdio).
    pub fn command(&self, agent_id: i64, scratch: &Path, interactive: bool, argv: &[&str]) -> Result<Command> {
        let mut started = self.started.lock().unwrap();
        let r = started.get_mut(&agent_id).context("sandbox is not running")?;
        r.last = Instant::now();
        let mut cmd = Command::new(self.bin()?);
        cmd.arg("exec");
        if interactive {
            cmd.arg("-i");
        }
        for e in &r.env {
            cmd.args(["-e", e]);
        }
        cmd.args(["-w", &inside(scratch), &Self::name(agent_id)]).args(argv).kill_on_drop(true);
        Ok(cmd)
    }

    /// Run `command` with `sh -c` in the agent's running sandbox.
    pub async fn exec(&self, agent_id: i64, scratch: &Path, command: &str, timeout: Duration) -> Result<String> {
        // `timeout` inside so the process dies in the sandbox too, not just our exec client
        let secs = timeout.as_secs().to_string();
        let mut cmd = self.command(agent_id, scratch, false, &["timeout", "-k", "5", &secs, "sh", "-c", command])?;
        let out = tokio::time::timeout(timeout + Duration::from_secs(10), cmd.output())
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

    /// Remove the container, its forwarder and network (leftovers from a crash included).
    async fn remove(&self, bin: &str, agent_id: i64) {
        if let Some(Running { proxy: Some(h), .. }) = self.started.lock().unwrap().remove(&agent_id) {
            h.abort();
        }
        let _ = Command::new(bin).args(["rm", "-f", &Self::name(agent_id), &Self::fwd(agent_id)]).output().await;
        let _ = Command::new(bin).args(["network", "rm", &Self::net(agent_id)]).output().await;
    }

    pub async fn stop(&self, agent_id: i64) {
        if let (true, Some(bin)) = (self.is_running(agent_id), &self.bin) {
            self.remove(bin, agent_id).await;
        }
    }

    pub async fn stop_idle(&self, idle: Duration) {
        let ids: Vec<i64> = self.started.lock().unwrap().iter()
            .filter(|(_, r)| r.last.elapsed() > idle).map(|(id, _)| *id).collect();
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
        type NoProxy = fn(IpAddr) -> std::future::Ready<Result<Proxy>>;
        sb.start(id, &[ro.clone()], &scratch, None::<NoProxy>).await.unwrap();
        let run = |c: String| { let (scratch, sb) = (scratch.clone(), &sb);
            async move { sb.exec(id, &scratch, &c, Duration::from_secs(5)).await.unwrap() } };

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
