//! Teams by delegation: an agent whose `delegate` rule isn't `deny` can hand a task to another
//! agent. The specialist runs as its own run (own sandbox, permissions and approvals) and its
//! final reply comes back as the tool result. Specialists can't delegate further.
use crate::agent::{self, State};
use crate::queue::Priority;
use crate::{hardware, models, policy, store};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_DELEGATIONS: i64 = 6;

pub fn spec(others: &[String]) -> Value {
    json!({"type": "function", "function": {"name": "delegate",
        "description": format!("Hand a task to a specialist agent and get its final answer back. Give it everything it needs: \
            it can't see this chat. Available: {}", others.join(", ")),
        "parameters": {"type": "object", "properties": {"agent": {"type": "string"}, "task": {"type": "string"}}, "required": ["agent", "task"]}}})
}

pub async fn others(st: &State, lead: i64) -> Result<Vec<String>> {
    Ok(store::agents(&st.db).await?.into_iter().filter(|a| a.id != lead).map(|a| a.name).collect())
}

/// `delegate {agent, task}` from `lead` during `run_id`.
pub async fn delegate(st: &Arc<State>, lead: i64, run_id: i64, args: &Value) -> Result<String> {
    let name = args["agent"].as_str().context("missing `agent`")?;
    let task = args["task"].as_str().filter(|t| !t.trim().is_empty()).context("missing `task`")?;
    let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM actions WHERE run_id = ? AND tool = 'delegate'")
        .bind(run_id).fetch_one(&st.db).await?;
    if used >= MAX_DELEGATIONS {
        bail!("at most {MAX_DELEGATIONS} delegations per run; answer with what you have");
    }
    let target = store::agents(&st.db).await?.into_iter().find(|a| a.id != lead && a.name.eq_ignore_ascii_case(name))
        .with_context(|| format!("no agent named `{name}`"))?;
    if st.busy().contains(&target.id) {
        bail!("{} is busy; try another agent or answer without it", target.name);
    }
    let mut ev = st.events.subscribe();
    agent::spawn(st.clone(), target.id, task.to_string(), Priority::Interactive, "delegate".into())?;
    let wait = Duration::from_secs(target.limits.max_run_secs as u64 + 60);
    let error = tokio::time::timeout(wait, async {
        loop {
            match ev.recv().await {
                Ok(v) if v["method"] == "chat.done" && v["params"]["agent_id"] == target.id => break v["params"]["error"].clone(),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break json!("daemon is shutting down"),
                _ => {}
            }
        }
    }).await.context("the specialist took too long")?;
    if let Some(e) = error.as_str() {
        bail!("{} failed: {e}", target.name);
    }
    let reply = store::messages(&st.db, target.id).await?.into_iter().rev()
        .find(|m| m["role"] == "assistant" && m["content"].as_str().is_some_and(|c| !c.trim().is_empty()))
        .and_then(|m| m["content"].as_str().map(String::from))
        .unwrap_or_else(|| "(no answer)".into());
    // the specialist may have read outside content: treat its answer as untrusted data
    Ok(format!("<untrusted source=\"agent {}\">\n{reply}\n</untrusted>", target.name))
}

/// Cloud models always; a catalog model needs 9B+ (smaller ones lost the thread in team tests).
/// Models not in the catalog (your own server) are allowed: we can't tell their size.
pub fn capable(model_ref: &str) -> bool {
    if models::is_cloud(model_ref) {
        return true;
    }
    let Ok((_, id)) = models::parse_ref(model_ref) else { return false };
    match hardware::model(id) {
        Ok(m) => m.name.split_whitespace().filter_map(|w| w.strip_suffix('B')?.parse::<f64>().ok()).any(|b| b >= 9.0),
        Err(_) => true,
    }
}

fn need_capable(model_ref: &str) -> Result<()> {
    anyhow::ensure!(capable(model_ref), "team mode needs a larger model: use a cloud model or a built-in one of 9B or more");
    Ok(())
}

/// `team.propose {lead_id, model_ref, problem}` → {members: [{agent: existing name | null, name, role, why}]}.
pub async fn propose(st: &State, p: &Value) -> Result<Value> {
    let lead = p["lead_id"].as_i64().context("missing `lead_id`")?;
    let model_ref = p["model_ref"].as_str().context("missing `model_ref`")?;
    let problem = p["problem"].as_str().filter(|t| !t.trim().is_empty()).context("missing `problem`")?;
    need_capable(model_ref)?;
    let existing: Vec<String> = store::agents(&st.db).await?.into_iter().filter(|a| a.id != lead)
        .map(|a| format!("- {}: {}", a.name, a.role_prompt.chars().take(200).collect::<String>())).collect();
    let prompt = format!("Propose a small team (2 to 5 specialists) to solve the problem below. Prefer these existing agents when they fit:\n{}\n\n\
        For a new specialist set \"agent\" to null and write a one-paragraph role prompt. Keep each specialist limited to what it needs. \
        Reply with JSON only: {{\"members\":[{{\"agent\":\"existing name or null\",\"name\":\"name\",\"role\":\"role prompt for a new agent, else empty\",\"why\":\"one line\"}}]}}\n\n\
        Problem:\n{problem}", if existing.is_empty() { "(none yet)".into() } else { existing.join("\n") });
    let r = agent::call_model(st, lead, 0, Priority::Interactive, model_ref, &[json!({"role": "user", "content": prompt})], &json!([]), |_| {}).await?;
    let text = r["content"].as_str().unwrap_or("");
    let text = text.rsplit("</think>").next().unwrap_or("");
    let (a, b) = (text.find('{'), text.rfind('}'));
    let v: Value = match (a, b) {
        (Some(a), Some(b)) if a < b => serde_json::from_str(&text[a..=b]).context("the model's team proposal wasn't valid JSON; try again")?,
        _ => bail!("the model didn't propose a team; try again"),
    };
    anyhow::ensure!(v["members"].as_array().is_some_and(|m| !m.is_empty()), "the model proposed an empty team; try again");
    Ok(v)
}

/// `team.start {lead_id, problem}`: switch delegation on for the lead (Ask, unless already allowed) and hand it the problem.
pub async fn start(st: &Arc<State>, p: &Value) -> Result<()> {
    let lead = p["lead_id"].as_i64().context("missing `lead_id`")?;
    let problem = p["problem"].as_str().filter(|t| !t.trim().is_empty()).context("missing `problem`")?;
    let mut a = store::get_agent(&st.db, lead).await?;
    need_capable(&a.model_ref)?;
    if a.approval_rules.get("delegate").map(String::as_str).unwrap_or(policy::default_rule("delegate")) == "deny" {
        a.approval_rules.insert("delegate".into(), "ask".into());
        store::update_agent(&st.db, &a).await?;
    }
    let team: Vec<String> = others(st, lead).await?;
    let text = format!("Problem: {problem}\n\nSplit the work and use the delegate tool to hand each part to the right specialist ({}). \
        Give each one everything it needs, then combine their answers into one result.", team.join(", "));
    agent::spawn(st.clone(), lead, text, Priority::Interactive, "chat".into())
}
