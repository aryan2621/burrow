mod agent;
mod compact;
mod download;
mod fetch;
mod hardware;
mod journal;
mod llama;
mod mcp;
mod models;
mod notes;
mod policy;
mod proxy;
mod queue;
mod rpc;
mod sandbox;
mod search;
mod secrets;
mod service;
mod store;
mod tools;

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let home = match std::env::var_os("BURROW_HOME") {
        Some(h) => h.into(),
        None => dirs::home_dir().context("no home dir")?.join(".burrow"),
    };
    std::fs::create_dir_all(&home)?;
    match std::env::args().nth(1).as_deref() {
        None | Some("run") => {}
        Some("install") => {
            service::install(&home)?;
            println!("burrowd will start at login and is running now");
            return Ok(());
        }
        Some("uninstall") => {
            service::uninstall()?;
            println!("burrowd service removed");
            return Ok(());
        }
        Some("status") => {
            println!("service installed: {}", service::installed()?);
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown command `{other}`; use run, install, uninstall or status"),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))?;
    }

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
    let sandbox = sandbox::Sandbox::detect();
    eprintln!("sandbox runtime: {}", sandbox.bin.as_deref().unwrap_or("none (shell tool disabled)"));
    let vault = secrets::Vault::detect(&home).await;
    let st = Arc::new(agent::State::new(home.clone(), env("BURROW_OLLAMA_URL", "http://127.0.0.1:11434"),
        env("BURROW_LLAMA_URL", ""), sandbox, vault).await?);

    let idle = st.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            idle.sandbox.stop_idle(Duration::from_secs(600)).await;
        }
    });
    let res = tokio::select! {
        r = rpc::serve(&home, st.clone()) => r,
        _ = shutdown_signal() => Ok(()),
    };
    st.sandbox.stop_all().await;
    st.llama.stop().await;
    res
}

/// Ctrl-C, or SIGTERM from launchd/systemd stopping the service.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
