//! Approval rules: which tool calls run on their own, wait for the user, or are refused.
use crate::agent::State;
use crate::{fetch, store};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sqlx::Row;
use tokio::sync::oneshot;

pub const TOOLS: [&str; 6] = ["list_dir", "read_file", "write_file", "delete_file", "shell", "http_fetch"];

#[derive(PartialEq, Debug)]
pub enum Rule {
    Allow,
    Ask,
    Deny,
}

/// Writes are undoable, so they run without asking; deletes, shell and new domains ask.
pub fn default_rule(tool: &str) -> &'static str {
    match tool {
        "list_dir" | "read_file" | "write_file" => "allow",
        _ => "ask",
    }
}

pub fn validate(rules: &std::collections::BTreeMap<String, String>) -> Result<()> {
    for (tool, r) in rules {
        if !TOOLS.contains(&tool.as_str()) || !matches!(r.as_str(), "allow" | "ask" | "deny") {
            bail!("bad approval rule {tool}={r}");
        }
    }
    Ok(())
}

/// For `http_fetch` the rule covers hosts outside the allowlist; allowlisted hosts just run
/// (unless the rule is `deny`).
pub fn rule(agent: &store::Agent, tool: &str, args: &Value) -> Rule {
    let r = agent.approval_rules.get(tool).map(String::as_str).unwrap_or(default_rule(tool));
    if tool == "http_fetch" && r != "deny" {
        let url = args["url"].as_str().unwrap_or("");
        // a bad URL fails inside the tool with a clear message; nothing to approve
        match fetch::host_of(url, &agent.allowed_domains) {
            Ok(h) if !fetch::allowed(&agent.allowed_domains, &h) => {}
            _ => return Rule::Allow,
        }
    }
    match r {
        "allow" => Rule::Allow,
        "deny" => Rule::Deny,
        _ => Rule::Ask,
    }
}

/// Ask the user and wait. A dropped request (kill, shutdown) counts as a denial.
pub async fn ask(st: &State, agent_id: i64, run_id: i64, tool: &str, args: &Value) -> Result<bool> {
    let id = sqlx::query("INSERT INTO approvals (agent_id, run_id, tool, args) VALUES (?, ?, ?, ?)")
        .bind(agent_id).bind(run_id).bind(tool).bind(args.to_string())
        .execute(&st.db).await?.last_insert_rowid();
    let (tx, rx) = oneshot::channel();
    st.pending.lock().unwrap().insert(id, (agent_id, tx));
    st.emit("approval.requested", json!({"approval_id": id, "agent_id": agent_id, "tool": tool, "args": args}));
    Ok(rx.await.unwrap_or(false))
}

pub async fn pending(st: &State) -> Result<Vec<Value>> {
    let ids: Vec<i64> = st.pending.lock().unwrap().keys().copied().collect();
    let mut out = vec![];
    for id in ids {
        let r = sqlx::query("SELECT * FROM approvals WHERE id = ?").bind(id).fetch_one(&st.db).await?;
        out.push(json!({"approval_id": id, "agent_id": r.get::<i64, _>("agent_id"), "tool": r.get::<String, _>("tool"),
            "args": serde_json::from_str::<Value>(r.get("args"))?}));
    }
    Ok(out)
}

/// `remember` turns an approval into a standing rule: the host joins the allowlist for
/// `http_fetch`, otherwise the tool becomes `allow`.
pub async fn decide(st: &State, approval_id: i64, allow: bool, remember: bool) -> Result<()> {
    let (agent_id, tx) = st.pending.lock().unwrap().remove(&approval_id).context("no such pending approval")?;
    let r = sqlx::query("SELECT tool, args FROM approvals WHERE id = ?").bind(approval_id).fetch_one(&st.db).await?;
    sqlx::query("UPDATE approvals SET decision = ?, decided_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(if allow { "approved" } else { "denied" }).bind(approval_id).execute(&st.db).await?;
    if allow && remember {
        let mut a = store::get_agent(&st.db, agent_id).await?;
        let tool: String = r.get("tool");
        let args: Value = serde_json::from_str(r.get("args"))?;
        if tool == "http_fetch" {
            let host = fetch::host_of(args["url"].as_str().unwrap_or(""), &a.allowed_domains)?;
            a.allowed_domains.push(host);
        } else {
            a.approval_rules.insert(tool, "allow".into());
        }
        store::update_permissions(&st.db, &a).await?;
    }
    let _ = tx.send(allow); // receiver gone = run was killed
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let mut a = store::Agent { id: 1, name: "a".into(), role_prompt: "".into(), model_ref: "ollama:x".into(),
            folders: vec![], allowed_domains: vec!["example.com".into()], approval_rules: Default::default(), context_budget: 8_000 };
        assert_eq!(rule(&a, "write_file", &json!({})), Rule::Allow);
        assert_eq!(rule(&a, "shell", &json!({})), Rule::Ask);
        assert_eq!(rule(&a, "http_fetch", &json!({"url": "https://docs.example.com"})), Rule::Allow);
        assert_eq!(rule(&a, "http_fetch", &json!({"url": "https://other.org"})), Rule::Ask);
        a.approval_rules.insert("http_fetch".into(), "deny".into());
        assert_eq!(rule(&a, "http_fetch", &json!({"url": "https://docs.example.com"})), Rule::Deny);
        a.approval_rules.insert("shell".into(), "allow".into());
        assert_eq!(rule(&a, "shell", &json!({})), Rule::Allow);
    }
}
