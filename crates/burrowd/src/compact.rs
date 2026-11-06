//! Keeps a long conversation inside the agent's context budget: when the history the model
//! would see passes 75% of the budget, the oldest turns are folded into one summary.
use anyhow::Result;
use serde_json::{json, Value};

const TRIGGER: f64 = 0.75;
/// After compaction the kept turns should use at most this share of the budget.
const KEEP: f64 = 0.4;
const CLIP: usize = 2_000;

/// Rough token count: ~4 characters per token across JSON.
pub fn tokens(msgs: &[Value]) -> usize {
    msgs.iter().map(|m| m.to_string().len()).sum::<usize>() / 4
}

/// Old tool results are the bulkiest part of a history; past the last few they're cut down.
pub fn clip_old_tool_output(msgs: &mut [Value]) {
    let tool_idx: Vec<usize> = msgs.iter().enumerate().filter(|(_, m)| m["role"] == "tool").map(|(i, _)| i).collect();
    for &i in tool_idx.iter().rev().skip(4) {
        if let Some(c) = msgs[i]["content"].as_str() {
            if c.len() > CLIP {
                let cut = c.floor_char_boundary(CLIP);
                msgs[i]["content"] = json!(format!("{}…[clipped {} chars]", &c[..cut], c.len() - cut));
            }
        }
    }
}

/// Index where the kept part starts, or None if nothing can be folded. The kept part
/// always starts at a user message so a tool call is never split from its results.
pub fn cut_point(msgs: &[(i64, Value)], budget: usize) -> Option<usize> {
    let all: Vec<Value> = msgs.iter().map(|(_, m)| m.clone()).collect();
    if (tokens(&all) as f64) < budget as f64 * TRIGGER {
        return None;
    }
    let keep = (budget as f64 * KEEP) as usize;
    let mut best = None;
    for i in (1..msgs.len()).rev() {
        if msgs[i].1["role"] != "user" {
            continue;
        }
        if tokens(&all[i..]) > keep && best.is_some() {
            break;
        }
        best = Some(i); // the earliest user turn that still fits, or the last one if none fit
    }
    best
}

pub fn summary_prompt(previous: Option<&str>, folded: &[(i64, Value)]) -> Vec<Value> {
    let mut text = String::new();
    if let Some(p) = previous {
        text.push_str(&format!("Earlier summary:\n{p}\n\n"));
    }
    for (_, m) in folded.iter().filter(|(_, m)| m["role"] != "summary") {
        let body = m["content"].as_str().unwrap_or("");
        let body = &body[..body.floor_char_boundary(1_000)];
        let calls = m["tool_calls"].as_array().map(|c| format!(" [tool calls: {}]", Value::Array(c.clone()))).unwrap_or_default();
        text.push_str(&format!("{}: {body}{calls}\n", m["role"].as_str().unwrap_or("?")));
    }
    vec![
        json!({"role": "system", "content": "You compress conversations. Write a concise summary that keeps facts, \
            decisions, file paths, results and open tasks. No preamble."}),
        json!({"role": "user", "content": text}),
    ]
}

/// The newest summary, stored as a `summary` message. It goes into the system prompt.
pub fn latest_summary(msgs: &[(i64, Value)]) -> Option<String> {
    msgs.iter().rev().find(|(_, m)| m["role"] == "summary").and_then(|(_, m)| m["content"].as_str().map(String::from))
}

/// Messages for the model: summaries removed (they live in the system prompt), old tool output clipped.
pub fn model_view(msgs: &[(i64, Value)]) -> Vec<Value> {
    let mut v: Vec<Value> = msgs.iter().filter(|(_, m)| m["role"] != "summary").map(|(_, m)| m.clone()).collect();
    clip_old_tool_output(&mut v);
    v
}

pub fn summary_message(text: &str) -> Value {
    json!({"role": "summary", "content": text})
}

/// Fold if needed; returns true if it compacted. `summarize` runs the model.
pub async fn maybe<F, Fut>(db: &sqlx::SqlitePool, agent_id: i64, budget: usize, summarize: F) -> Result<bool>
where
    F: FnOnce(Vec<Value>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let msgs = crate::store::live_messages(db, agent_id).await?;
    let Some(cut) = cut_point(&msgs, budget) else { return Ok(false) };
    let folded = &msgs[..cut];
    let text = summarize(summary_prompt(latest_summary(&msgs).as_deref(), folded)).await?;
    crate::store::compact_upto(db, agent_id, folded.last().unwrap().0).await?;
    crate::store::add_message(db, agent_id, &summary_message(&text)).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(i: i64, role: &str, len: usize) -> (i64, Value) {
        (i, json!({"role": role, "content": "x".repeat(len)}))
    }

    #[test]
    fn cuts_at_a_user_turn_within_budget() {
        let msgs: Vec<_> = (0..20).map(|i| turn(i, if i % 2 == 0 { "user" } else { "assistant" }, 400)).collect();
        assert_eq!(cut_point(&msgs, 100_000), None, "under budget");
        let cut = cut_point(&msgs, 2_000).unwrap();
        assert_eq!(msgs[cut].1["role"], "user");
        let kept: Vec<Value> = msgs[cut..].iter().map(|(_, m)| m.clone()).collect();
        assert!(tokens(&kept) <= 800, "kept {} tokens", tokens(&kept));
    }

    #[test]
    fn clips_only_older_tool_output() {
        let mut v: Vec<Value> = (0..6).map(|_| json!({"role": "tool", "content": "y".repeat(5_000)})).collect();
        clip_old_tool_output(&mut v);
        assert!(v[0]["content"].as_str().unwrap().contains("clipped"));
        assert!(v[1]["content"].as_str().unwrap().contains("clipped"));
        assert_eq!(v[5]["content"].as_str().unwrap().len(), 5_000);
    }

    #[tokio::test]
    async fn folds_history_into_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::store::open(&tmp.path().join("t.db"), &"ab".repeat(32)).await.unwrap();
        let a = crate::store::Agent { id: 0, name: "a".into(), role_prompt: "".into(), model_ref: "llama:x".into(),
            folders: vec![], allowed_domains: vec![], approval_rules: Default::default(), context_budget: 1_000, limits: Default::default() };
        let id = crate::store::create_agent(&db, &a).await.unwrap();
        for i in 0..20 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            crate::store::add_message(&db, id, &json!({"role": role, "content": format!("turn {i} {}", "z".repeat(300))})).await.unwrap();
        }
        let did = maybe(&db, id, 1_000, |p| async move {
            assert!(p[1]["content"].as_str().unwrap().contains("turn 0"));
            Ok("SUMMARY".to_string())
        }).await.unwrap();
        assert!(did);
        let live = crate::store::live_messages(&db, id).await.unwrap();
        assert_eq!(latest_summary(&live).as_deref(), Some("SUMMARY"));
        assert!(tokens(&model_view(&live)) < 750);
        assert_eq!(crate::store::messages(&db, id).await.unwrap().len(), 21, "UI keeps everything");
    }
}
