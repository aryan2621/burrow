//! Sign-in for remote MCP servers (MCP's OAuth flow via rmcp). The browser does the login; a
//! one-shot listener on 127.0.0.1 catches the redirect. Tokens live in the vault, not the database.
use crate::agent::State;
use crate::fetch;
use crate::mcp::Client;
use anyhow::{bail, Context, Result};
use rmcp::transport::auth::{AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore, OAuthState, StoredCredentials};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn key(agent_id: i64, server: &str) -> String {
    format!("oauth-{agent_id}-{server}")
}

fn internal(e: impl std::fmt::Display) -> AuthError {
    AuthError::InternalError(e.to_string())
}

struct VaultStore {
    st: Arc<State>,
    key: String,
}

#[async_trait::async_trait]
impl CredentialStore for VaultStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self.st.vault.get(&self.key).await.map_err(internal)? {
            Some(s) => Ok(Some(serde_json::from_str(&s).map_err(internal)?)),
            None => Ok(None),
        }
    }
    async fn save(&self, c: StoredCredentials) -> Result<(), AuthError> {
        self.st.vault.set(&self.key, &serde_json::to_string(&c).map_err(internal)?).await.map_err(internal)
    }
    async fn clear(&self) -> Result<(), AuthError> {
        self.st.vault.delete(&self.key).await.map_err(internal)
    }
}

fn open_browser(url: &str) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url).spawn().context("can't open the browser")?;
    Ok(())
}

/// Log in to `url` in the browser and keep the tokens for this agent's server `name`.
pub async fn signin(st: &Arc<State>, agent_id: i64, name: &str, url: &str) -> Result<()> {
    let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = l.local_addr()?.port();
    let mut state = OAuthState::new(url, None).await?;
    if let OAuthState::Unauthorized(m) = &mut state {
        m.set_credential_store(VaultStore { st: st.clone(), key: key(agent_id, name) });
    }
    state.start_authorization(AuthorizationRequest::new(format!("http://127.0.0.1:{port}/callback")).with_client_name("Burrow")).await?;
    open_browser(&state.get_authorization_url().await?)?;

    let (mut c, _) = tokio::time::timeout(Duration::from_secs(300), l.accept()).await.context("sign-in timed out")??;
    let mut buf = vec![0u8; 8192];
    let n = c.read(&mut buf).await?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let target = head.lines().next().and_then(|l| l.split(' ').nth(1)).context("bad redirect")?.to_string();
    let result = state.handle_callback_url(&format!("http://127.0.0.1:{port}{target}")).await;
    let msg = if result.is_ok() { "Signed in. You can close this tab and return to Burrow." } else { "Sign-in failed. Return to Burrow for details." };
    let _ = c.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{msg}", msg.len()).as_bytes()).await;
    result?;
    Ok(())
}

/// A client signed in with stored tokens, or `None` when this server has none (use secret headers instead).
pub async fn connect(st: &Arc<State>, agent_id: i64, name: &str, url: &str, allowed: Vec<String>) -> Result<Option<Client>> {
    let k = key(agent_id, name);
    if st.vault.get(&k).await?.is_none() {
        return Ok(None);
    }
    let mut manager = AuthorizationManager::new(url).await?;
    manager.set_credential_store(VaultStore { st: st.clone(), key: k });
    if !manager.initialize_from_store().await? {
        bail!("signed out: sign in again from the plugin list");
    }
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(fetch::PublicOnly(allowed))).build()?;
    let t = StreamableHttpClientTransport::with_client(AuthClient::new(http, manager), StreamableHttpClientTransportConfig::with_uri(url));
    Ok(Some(().serve(t).await?))
}

/// Forget the tokens (agent or server removed).
pub async fn forget(st: &State, agent_id: i64, name: &str) {
    let _ = st.vault.delete(&key(agent_id, name)).await;
}
