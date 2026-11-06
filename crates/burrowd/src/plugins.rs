//! Curated plugins: an MCP server plus what it needs. Attaching one is `mcp.add` with the
//! manifest's transport, the domains it must reach, and (for key auth) a secret bound to this agent.
use crate::agent::State;
use crate::{mcp, policy, secrets, store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize, Serialize, Clone)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub transport: String,
    pub command: Option<Vec<String>>,
    pub url: Option<String>,
    /// Domains the agent must be allowed to reach.
    pub hosts: Vec<String>,
    /// none | key | oauth
    pub auth: String,
    pub secret: Option<Needs>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Needs {
    pub name: String,
    pub hint: String,
    pub header: String,
}

pub fn catalog() -> Vec<Plugin> {
    serde_json::from_str(include_str!("../plugins.json")).expect("plugins.json is valid")
}

/// `plugins.attach`: {agent_id, id, value?}. `value` is the key for `auth: key` plugins.
/// Returns Antidote warnings like `mcp.add`.
pub async fn attach(st: &Arc<State>, p: &Value) -> Result<Vec<String>> {
    let agent_id = p["agent_id"].as_i64().context("missing `agent_id`")?;
    let id = p["id"].as_str().context("missing `id`")?;
    let plugin = catalog().into_iter().find(|c| c.id == id).with_context(|| format!("no plugin `{id}`"))?;
    if plugin.auth == "oauth" {
        crate::oauth::signin(st, agent_id, &plugin.id, plugin.url.as_deref().context("oauth plugins are remote")?).await
            .with_context(|| format!("signing in to {}", plugin.name))?;
    }
    let mut agent = store::get_agent(&st.db, agent_id).await?;
    let warnings = mcp::add(&st.db, &json!({"agent_id": agent_id, "name": plugin.id, "transport": plugin.transport,
        "command": plugin.command, "url": plugin.url, "secrets": plugin.secret.iter().map(|s| &s.name).collect::<Vec<_>>()})).await?;

    allow(st, &mut agent, plugin.hosts.clone(), plugin.transport == "stdio").await?;

    if let Some(n) = &plugin.secret {
        let mut ids: Vec<i64> = secrets::list(&st.db).await?.iter().find(|s| s["name"] == n.name)
            .and_then(|s| serde_json::from_value(s["agent_ids"].clone()).ok()).unwrap_or_default();
        if !ids.contains(&agent_id) {
            ids.push(agent_id);
        }
        secrets::set(&st.db, &st.vault, &json!({"name": n.name, "value": p["value"], "hosts": plugin.hosts,
            "agent_ids": ids, "header": n.header})).await.with_context(|| format!("needs {}", n.hint))?;
    }
    Ok(warnings)
}

/// Let the agent reach `hosts` (and the package registries for sandbox-run servers).
async fn allow(st: &State, agent: &mut store::Agent, mut hosts: Vec<String>, stdio: bool) -> Result<()> {
    if stdio {
        hosts.extend(policy::PACKAGE_REGISTRIES.iter().map(|d| d.to_string()));
    }
    for d in hosts {
        if !agent.allowed_domains.contains(&d) {
            agent.allowed_domains.push(d);
        }
    }
    store::update_agent(&st.db, agent).await
}

const REGISTRY: &str = "https://registry.modelcontextprotocol.io/v0/servers";

/// `plugins.search {query}`: community servers from the official MCP Registry. Not reviewed by us.
pub async fn search(st: &State, query: &str) -> Result<Vec<Value>> {
    let v: Value = st.http.get(REGISTRY).query(&[("search", query), ("limit", "20")]).timeout(std::time::Duration::from_secs(15))
        .send().await.context("can't reach the MCP Registry")?.error_for_status()?.json().await?;
    let mut out = vec![];
    for e in v["servers"].as_array().into_iter().flatten() {
        // current entries wrap the server in `server`; older ones are flat
        let s = if e["server"].is_object() { &e["server"] } else { e };
        let title = s["name"].as_str().unwrap_or("");
        let desc = s["description"].as_str().unwrap_or("");
        let short = title.rsplit('/').next().unwrap_or(title).to_string();
        let remote = s["remotes"].as_array().into_iter().flatten()
            .find(|r| matches!(r["type"].as_str(), Some("streamable-http")) && r["url"].as_str().is_some_and(|u| !u.contains('{')));
        let package = s["packages"].as_array().into_iter().flatten().find_map(|p| {
            // we can't pass environment variables to a sandbox server yet
            if p["environmentVariables"].as_array().is_some_and(|v| v.iter().any(|e| e["isRequired"] == true)) {
                return None;
            }
            let id = p["identifier"].as_str()?;
            match p["registryType"].as_str()? {
                "npm" => Some(json!(["npx", "-y", id])),
                "pypi" => Some(json!(["uvx", id])),
                _ => None,
            }
        });
        if let Some(r) = remote {
            out.push(json!({"title": title, "name": short, "description": desc, "transport": "http", "url": r["url"]}));
        } else if let Some(c) = package {
            out.push(json!({"title": title, "name": short, "description": desc, "transport": "stdio", "command": c}));
        }
    }
    Ok(out)
}

/// `plugins.add_community {agent_id, name, transport, command?, url?}`: like `mcp.add`, plus the domains it needs.
pub async fn add_community(st: &Arc<State>, p: &Value) -> Result<Vec<String>> {
    let agent_id = p["agent_id"].as_i64().context("missing `agent_id`")?;
    let mut agent = store::get_agent(&st.db, agent_id).await?;
    let mut p = p.clone();
    // registry names are long and may hold dots; MCP server names are not
    p["name"] = json!(p["name"].as_str().unwrap_or("").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).take(20).collect::<String>());
    let warnings = mcp::add(&st.db, &p).await?;
    let host = p["url"].as_str().and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(String::from));
    allow(st, &mut agent, host.into_iter().collect(), p["transport"] == "stdio").await?;
    Ok(warnings)
}
