//! Rooms: the user and 2–6 agents. A message that @mentions agents wakes them; a user message
//! that mentions nobody wakes the coordinator. An agent's final reply is posted back to the
//! room, so its own @mentions hand work on. Every agent keeps its own sandbox and permissions,
//! and other agents' messages reach it as untrusted text (its side effects then ask first).
use crate::agent::{self, State};
use crate::queue::Priority;
use crate::{notes, tools::Outcome};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use std::sync::Arc;

/// Agent-to-agent hand-offs after one user message; stops loops between agents.
const MAX_HOPS: i64 = 6;
const HISTORY: i64 = 30;

/// Room memory lives in the notes table under a negative owner id, so it never meets an agent's own.
pub fn memory_owner(room_id: i64) -> i64 {
    -room_id
}

pub async fn create(db: &SqlitePool, p: &Value) -> Result<i64> {
    let name = p["name"].as_str().filter(|n| !n.trim().is_empty()).context("missing `name`")?;
    let mut ids: Vec<i64> = serde_json::from_value(p["agent_ids"].clone()).context("missing `agent_ids`")?;
    ids.sort_unstable();
    ids.dedup();
    anyhow::ensure!((2..=6).contains(&ids.len()), "a room has 2–6 agents");
    let coord = p["coordinator_id"].as_i64().context("missing `coordinator_id`")?;
    anyhow::ensure!(ids.contains(&coord), "the coordinator must be a member");
    for id in &ids {
        crate::store::get_agent(db, *id).await?;
    }
    let room = sqlx::query("INSERT INTO rooms (name, coordinator_id) VALUES (?, ?)").bind(name.trim()).bind(coord)
        .execute(db).await?.last_insert_rowid();
    for id in ids {
        sqlx::query("INSERT INTO room_members (room_id, agent_id) VALUES (?, ?)").bind(room).bind(id).execute(db).await?;
    }
    Ok(room)
}

pub async fn list(db: &SqlitePool) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT r.id, r.name, r.coordinator_id, json_group_array(m.agent_id) AS ids
                            FROM rooms r JOIN room_members m ON m.room_id = r.id GROUP BY r.id ORDER BY r.id").fetch_all(db).await?;
    rows.iter().map(|r| Ok(json!({"id": r.get::<i64, _>("id"), "name": r.get::<String, _>("name"),
        "coordinator_id": r.get::<i64, _>("coordinator_id"), "agent_ids": serde_json::from_str::<Value>(r.get("ids"))?}))).collect()
}

pub async fn delete(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM rooms WHERE id = ?").bind(id).execute(db).await?;
    sqlx::query("DELETE FROM notes WHERE agent_id = ?").bind(memory_owner(id)).execute(db).await?;
    Ok(())
}

pub async fn messages(db: &SqlitePool, room_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM room_messages WHERE room_id = ? ORDER BY id").bind(room_id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| json!({"id": r.get::<i64, _>("id"), "agent_id": r.get::<Option<i64>, _>("agent_id"),
        "text": r.get::<String, _>("text"), "created_at": r.get::<String, _>("created_at")})).collect())
}

/// (id, name) of the room's members.
async fn members(db: &SqlitePool, room_id: i64) -> Result<Vec<(i64, String)>> {
    Ok(sqlx::query("SELECT a.id, a.name FROM room_members m JOIN agents a ON a.id = m.agent_id WHERE m.room_id = ? ORDER BY a.id")
        .bind(room_id).fetch_all(db).await?.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// `@Name` matches a member's name with spaces removed, ignoring case.
fn mentioned(text: &str, members: &[(i64, String)]) -> Vec<i64> {
    let lower = text.to_lowercase();
    members.iter().filter(|(_, n)| {
        let tag = format!("@{}", n.to_lowercase().replace(' ', ""));
        lower.match_indices(&tag).any(|(i, _)| !lower[i + tag.len()..].starts_with(|c: char| c.is_alphanumeric()))
    }).map(|(id, _)| *id).collect()
}

/// Add a message and wake whoever it's for.
pub async fn post(st: &Arc<State>, room_id: i64, author: Option<i64>, text: &str) -> Result<()> {
    anyhow::ensure!(!text.trim().is_empty(), "empty message");
    let coord: i64 = sqlx::query_scalar("SELECT coordinator_id FROM rooms WHERE id = ?").bind(room_id).fetch_optional(&st.db).await?
        .context("no such room")?;
    let id = sqlx::query("INSERT INTO room_messages (room_id, agent_id, text) VALUES (?, ?, ?)")
        .bind(room_id).bind(author).bind(text).execute(&st.db).await?.last_insert_rowid();
    st.emit("room.message", json!({"room_id": room_id, "id": id, "agent_id": author, "text": text}));
    let members = members(&st.db, room_id).await?;
    // the coordinator's agent may have been deleted since
    let coord = if members.iter().any(|m| m.0 == coord) { coord } else { members.first().context("the room has no agents left")?.0 };
    let mut targets: Vec<i64> = mentioned(text, &members).into_iter().filter(|t| Some(*t) != author).collect();
    if targets.is_empty() && author.is_none() {
        targets.push(coord);
    }
    if author.is_some() && !targets.is_empty() {
        let hops: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM room_messages WHERE room_id = ? AND agent_id IS NOT NULL
            AND id > COALESCE((SELECT MAX(id) FROM room_messages WHERE room_id = ? AND agent_id IS NULL), 0)")
            .bind(room_id).bind(room_id).fetch_one(&st.db).await?;
        if hops > MAX_HOPS {
            st.emit("room.stopped", json!({"room_id": room_id, "reason": format!("{MAX_HOPS} hand-offs without you; reply to continue")}));
            return Ok(());
        }
    }
    for t in targets {
        wake(st, room_id, t, &members, coord).await?;
    }
    Ok(())
}

async fn wake(st: &Arc<State>, room_id: i64, agent_id: i64, members: &[(i64, String)], coord: i64) -> Result<()> {
    let room: String = sqlx::query_scalar("SELECT name FROM rooms WHERE id = ?").bind(room_id).fetch_one(&st.db).await?;
    let name = |id: Option<i64>| match id {
        None => "User".to_string(),
        Some(id) => members.iter().find(|m| m.0 == id).map(|m| m.1.clone()).unwrap_or(format!("agent {id}")),
    };
    let mut recent = sqlx::query("SELECT agent_id, text FROM room_messages WHERE room_id = ? ORDER BY id DESC LIMIT ?")
        .bind(room_id).bind(HISTORY).fetch_all(&st.db).await?;
    recent.reverse();
    let log: Vec<String> = recent.iter().map(|r| {
        let (who, text) = (r.get::<Option<i64>, _>(0), r.get::<String, _>(1));
        match who {
            None => format!("User: {text}"),
            Some(id) if id == agent_id => format!("You: {text}"),
            // another agent may relay injected web content; keep it data
            Some(id) => format!("<untrusted source=\"agent:{}\">\n{}: {text}\n</untrusted>", name(Some(id)), name(Some(id))),
        }
    }).collect();
    let roster: Vec<String> = members.iter().map(|(id, n)| format!("@{}{}", n.replace(' ', ""), if *id == coord { " (coordinator)" } else { "" })).collect();
    let prompt = format!("[Room \"{room}\"] Members: {}. You are {}. Your final reply is posted to the room. Mention @Name only to hand \
        a member new work, never just to answer or thank them; with nothing left to hand off, mention nobody. Use the task board (tasks_*) and room memory (room_notes_*) to coordinate.\n\nTask board:\n{}\n\nRecent messages:\n{}",
        roster.join(", "), name(Some(agent_id)), board(&st.db, room_id).await?, log.join("\n"));
    let trigger = format!("room:{room_id}");
    let busy = st.running.lock().unwrap().contains_key(&agent_id);
    if busy {
        // shortcut: one queued firing per agent (shared with schedules); later wake-ups while it's busy are dropped
        st.queued_fire.lock().unwrap().entry(agent_id).or_insert((trigger, prompt));
    } else {
        agent::spawn(st.clone(), agent_id, prompt, Priority::Interactive, trigger)?;
    }
    Ok(())
}

/// After a room run: post the agent's last reply to the room.
pub async fn on_done(st: &Arc<State>, room_id: i64, agent_id: i64, error: Option<String>) {
    let text = match error {
        Some(e) => format!("(couldn't finish: {e})"),
        None => {
            let msgs = crate::store::messages(&st.db, agent_id).await.unwrap_or_default();
            msgs.iter().rev().find(|m| m["role"] == "assistant" && m.get("tool_calls").is_none())
                .and_then(|m| m["content"].as_str()).map(|s| s.rsplit("</think>").next().unwrap_or("").trim().to_string()).unwrap_or_default()
        }
    };
    if !text.is_empty() {
        if let Err(e) = post(st, room_id, Some(agent_id), &text).await {
            eprintln!("room {room_id}: {e:#}");
        }
    }
}

async fn board(db: &SqlitePool, room_id: i64) -> Result<String> {
    let rows = sqlx::query("SELECT t.id, t.title, t.status, a.name FROM tasks t LEFT JOIN agents a ON a.id = t.assignee_id WHERE t.room_id = ? ORDER BY t.id")
        .bind(room_id).fetch_all(db).await?;
    if rows.is_empty() {
        return Ok("(empty)".into());
    }
    Ok(rows.iter().map(|r| format!("#{} [{}] {}{}", r.get::<i64, _>(0), r.get::<String, _>(2), r.get::<String, _>(1),
        r.get::<Option<String>, _>(3).map(|n| format!(" → {n}")).unwrap_or_default())).collect::<Vec<_>>().join("\n"))
}

pub async fn tasks(db: &SqlitePool, room_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM tasks WHERE room_id = ? ORDER BY id").bind(room_id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| json!({"id": r.get::<i64, _>("id"), "title": r.get::<String, _>("title"),
        "assignee_id": r.get::<Option<i64>, _>("assignee_id"), "status": r.get::<String, _>("status")})).collect())
}

/// Add (no `id`) or change a task; used by the UI and the `tasks_*` tools.
pub async fn upsert_task(db: &SqlitePool, room_id: i64, p: &Value) -> Result<i64> {
    let status = p["status"].as_str();
    anyhow::ensure!(status.is_none_or(|s| matches!(s, "todo" | "doing" | "done")), "status is todo, doing or done");
    let assignee = match &p["assignee"] {
        Value::String(n) => {
            let m = members(db, room_id).await?;
            let n = n.trim_start_matches('@').to_lowercase().replace(' ', "");
            Some(m.iter().find(|(_, x)| x.to_lowercase().replace(' ', "") == n).map(|m| m.0).with_context(|| format!("no member @{n}"))?)
        }
        v => v.as_i64(),
    };
    match p["id"].as_i64() {
        None => {
            let title = p["title"].as_str().filter(|t| !t.trim().is_empty()).context("missing `title`")?;
            Ok(sqlx::query("INSERT INTO tasks (room_id, title, assignee_id, status) VALUES (?, ?, ?, ?)")
                .bind(room_id).bind(title).bind(assignee).bind(status.unwrap_or("todo")).execute(db).await?.last_insert_rowid())
        }
        Some(id) => {
            let n = sqlx::query("UPDATE tasks SET title = COALESCE(?, title), status = COALESCE(?, status),
                                 assignee_id = CASE WHEN ? THEN ? ELSE assignee_id END WHERE id = ? AND room_id = ?")
                .bind(p["title"].as_str()).bind(status).bind(p.get("assignee").is_some()).bind(assignee).bind(id).bind(room_id)
                .execute(db).await?.rows_affected();
            anyhow::ensure!(n == 1, "no task #{id} in this room");
            Ok(id)
        }
    }
}

pub const TOOLS: [&str; 5] = ["tasks_list", "task_add", "task_update", "room_notes_add", "room_notes_search"];

pub fn specs() -> Vec<Value> {
    let f = |name: &str, desc: &str, props: Value, req: &[&str]| json!({"type": "function", "function": {"name": name,
        "description": desc, "parameters": {"type": "object", "properties": props, "required": req}}});
    let s = |d: &str| json!({"type": "string", "description": d});
    vec![
        f("tasks_list", "Show the room's task board", json!({}), &[]),
        f("task_add", "Add a task to the room's board", json!({"title": s("what to do"), "assignee": s("@Name of a member")}), &["title"]),
        f("task_update", "Change a task's status, title or assignee",
          json!({"id": {"type": "integer"}, "status": s("todo, doing or done"), "title": s("new title"), "assignee": s("@Name")}), &["id"]),
        f("room_notes_add", "Save a note to the room's shared memory", json!({"text": s("the note")}), &["text"]),
        f("room_notes_search", "Search the room's shared memory", json!({"query": s("keywords")}), &["query"]),
    ]
}

pub async fn call(st: &State, room_id: i64, name: &str, args: &Value) -> Outcome {
    let db = &st.db;
    let res: Result<Outcome> = async {
        Ok(match name {
            "tasks_list" => board(db, room_id).await?.into(),
            "task_add" | "task_update" => {
                let id = upsert_task(db, room_id, args).await?;
                st.emit("room.tasks", json!({"room_id": room_id}));
                format!("task #{id} saved").into()
            }
            "room_notes_add" => {
                let (output, inverse) = notes::add(db, memory_owner(room_id), args["text"].as_str().unwrap_or("")).await?;
                Outcome { output, inverse: Some(inverse) }
            }
            _ => notes::search(db, memory_owner(room_id), args["query"].as_str().unwrap_or("")).await?.into(),
        })
    }.await;
    res.unwrap_or_else(|e| format!("error: {e:#}").into())
}
