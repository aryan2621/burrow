//! Schedules: cron, fixed-interval, file-change and app-event (email, calendar, github) triggers
//! that start background runs.
//! A busy agent keeps one firing queued and drops the rest. A time missed while the daemon
//! was down fires once on the next tick.
use crate::agent::{self, State};
use crate::queue::Priority;
use crate::{connectors, store, tools};
use anyhow::{bail, Context, Result};
use chrono::{Local, TimeZone};
use croner::Cron;
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(5);
const DEBOUNCE: Duration = Duration::from_secs(2);
const MIN_INTERVAL: i64 = 60;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// When a cron/interval schedule fires next after `from` (unix seconds). File triggers have no time.
pub fn next_after(kind: &str, spec: &str, from: i64) -> Result<Option<i64>> {
    match kind {
        "cron" => {
            let cron = Cron::from_str(spec).map_err(|e| anyhow::anyhow!("bad cron expression: {e}"))?;
            let from = Local.timestamp_opt(from, 0).single().context("bad time")?;
            Ok(Some(cron.find_next_occurrence(&from, false).map_err(|e| anyhow::anyhow!("{e}"))?.timestamp()))
        }
        "interval" => {
            let secs: i64 = spec.parse().context("interval must be whole seconds")?;
            anyhow::ensure!(secs >= MIN_INTERVAL, "interval must be at least {MIN_INTERVAL} seconds");
            Ok(Some(from + secs))
        }
        "file" => Ok(None),
        // app events are polled once a minute
        "email" | "calendar" | "github" => Ok(Some(from + MIN_INTERVAL)),
        _ => bail!("kind must be cron, interval, file, email, calendar or github"),
    }
}

/// Check `spec` for `kind`; file paths must exist inside the agent's folders.
async fn validate(db: &SqlitePool, agent_id: i64, kind: &str, spec: &str) -> Result<String> {
    next_after(kind, spec, now())?;
    if matches!(kind, "email" | "calendar" | "github") {
        connectors::validate_trigger(db, agent_id, kind, spec.trim()).await?;
        return Ok(spec.trim().to_string());
    }
    if kind != "file" {
        return Ok(spec.to_string());
    }
    let agent = store::get_agent(db, agent_id).await?;
    let p = tools::scoped(&agent.folders, spec)?;
    anyhow::ensure!(p.exists(), "{} doesn't exist", p.display());
    Ok(p.to_string_lossy().into_owned())
}

pub async fn add(db: &SqlitePool, p: &Value) -> Result<i64> {
    let agent_id = p["agent_id"].as_i64().context("missing `agent_id`")?;
    let kind = p["kind"].as_str().context("missing `kind`")?;
    // a routine brings its own prompt
    let routine = p["routine_id"].as_i64();
    let prompt = match routine {
        Some(r) => {
            let (owner, prompt) = crate::routines::start(db, r).await?;
            anyhow::ensure!(owner == agent_id, "that routine belongs to another agent");
            prompt
        }
        None => p["prompt"].as_str().filter(|s| !s.trim().is_empty()).context("missing `prompt`")?.to_string(),
    };
    let spec = validate(db, agent_id, kind, p["spec"].as_str().context("missing `spec`")?).await?;
    Ok(sqlx::query("INSERT INTO schedules (agent_id, kind, spec, prompt, next_run_at, routine_id) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(agent_id).bind(kind).bind(&spec).bind(prompt).bind(next_after(kind, &spec, now())?).bind(routine)
        .execute(db).await?.last_insert_rowid())
}

pub async fn update(db: &SqlitePool, p: &Value) -> Result<()> {
    let id = p["id"].as_i64().context("missing `id`")?;
    let r = sqlx::query("SELECT * FROM schedules WHERE id = ?").bind(id).fetch_optional(db).await?.context("no such schedule")?;
    let kind: String = r.get("kind");
    let spec = match p["spec"].as_str() {
        Some(s) => validate(db, r.get("agent_id"), &kind, s).await?,
        None => r.get("spec"),
    };
    let prompt = p["prompt"].as_str().map(String::from).unwrap_or_else(|| r.get("prompt"));
    let enabled = p["enabled"].as_bool().unwrap_or(r.get::<i64, _>("enabled") != 0);
    sqlx::query("UPDATE schedules SET spec = ?, prompt = ?, enabled = ?, next_run_at = ? WHERE id = ?")
        .bind(&spec).bind(prompt).bind(enabled).bind(next_after(&kind, &spec, now())?).bind(id).execute(db).await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM schedules WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}

pub async fn list(db: &SqlitePool, agent_id: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM schedules WHERE agent_id = ? ORDER BY id").bind(agent_id).fetch_all(db).await?;
    Ok(rows.iter().map(|r| json!({"id": r.get::<i64, _>("id"), "agent_id": agent_id, "kind": r.get::<String, _>("kind"),
        "spec": r.get::<String, _>("spec"), "prompt": r.get::<String, _>("prompt"), "enabled": r.get::<i64, _>("enabled") != 0,
        "next_run_at": r.get::<Option<i64>, _>("next_run_at"), "last_run_at": r.get::<Option<i64>, _>("last_run_at"),
        "routine_id": r.get::<Option<i64>, _>("routine_id")})).collect())
}

/// Start a background run, or queue it if the agent is busy (one queued firing per agent).
pub fn fire(st: &Arc<State>, schedule_id: i64, agent_id: i64, prompt: String) {
    let trigger = format!("schedule:{schedule_id}");
    let queued = {
        let running = st.running.lock().unwrap();
        if running.contains_key(&agent_id) {
            let mut q = st.queued_fire.lock().unwrap();
            if q.contains_key(&agent_id) {
                st.emit("schedule.skipped", json!({"schedule_id": schedule_id, "agent_id": agent_id, "reason": "agent busy, one firing already queued"}));
                return;
            }
            q.insert(agent_id, (trigger.clone(), prompt.clone()));
            true
        } else {
            false
        }
    };
    if !queued {
        if let Err(e) = agent::spawn(st.clone(), agent_id, prompt, Priority::Background, trigger) {
            st.emit("schedule.skipped", json!({"schedule_id": schedule_id, "agent_id": agent_id, "reason": format!("{e:#}")}));
            return;
        }
    }
    st.emit("schedule.fired", json!({"schedule_id": schedule_id, "agent_id": agent_id, "queued": queued}));
}

/// Fire cron/interval schedules that are due and move them to their next time.
async fn tick(st: &Arc<State>) -> Result<()> {
    let t = now();
    let due = sqlx::query("SELECT * FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?")
        .bind(t).fetch_all(&st.db).await?;
    for r in due {
        let (id, kind, spec): (i64, String, String) = (r.get("id"), r.get("kind"), r.get("spec"));
        // computed from now, not from the missed time: a long sleep fires once, not once per missed slot
        if matches!(kind.as_str(), "email" | "calendar" | "github") {
            sqlx::query("UPDATE schedules SET next_run_at = ? WHERE id = ?").bind(next_after(&kind, &spec, t)?).bind(id).execute(&st.db).await?;
            // shortcut: polls run one after another inside the tick; move them to their own task if many connections slow it
            match connectors::poll(st, &kind, &spec, r.get::<Option<String>, _>("cursor").as_deref()).await {
                Ok((cursor, new)) => {
                    sqlx::query("UPDATE schedules SET cursor = ? WHERE id = ?").bind(&cursor).bind(id).execute(&st.db).await?;
                    if let Some(new) = new {
                        sqlx::query("UPDATE schedules SET last_run_at = ? WHERE id = ?").bind(t).bind(id).execute(&st.db).await?;
                        fire(st, id, r.get("agent_id"), format!("{}\n\n{new}", r.get::<String, _>("prompt")));
                    }
                }
                Err(e) => st.emit("schedule.skipped", json!({"schedule_id": id, "agent_id": r.get::<i64, _>("agent_id"), "reason": format!("{e:#}")})),
            }
            continue;
        }
        sqlx::query("UPDATE schedules SET next_run_at = ?, last_run_at = ? WHERE id = ?")
            .bind(next_after(&kind, &spec, t)?).bind(t).bind(id).execute(&st.db).await?;
        fire(st, id, r.get("agent_id"), r.get("prompt"));
    }
    Ok(())
}

/// Changed paths per file schedule, waiting out the debounce.
#[derive(Default)]
struct Changes(Mutex<HashMap<i64, (Instant, HashSet<PathBuf>)>>);

/// The scheduler: ticks for timed schedules and keeps one watcher per enabled file schedule.
pub async fn run(st: Arc<State>) {
    let changes = Arc::new(Changes::default());
    let mut watchers: HashMap<i64, (String, notify::RecommendedWatcher)> = HashMap::new();
    loop {
        // due schedules and file changes keep waiting and fire once power is back
        let low = tokio::task::spawn_blocking(low_battery).await.unwrap_or(false);
        if !low {
            if let Err(e) = tick(&st).await {
                eprintln!("scheduler: {e:#}");
            }
        }
        if let Err(e) = sync_watchers(&st, &changes, &mut watchers).await {
            eprintln!("scheduler: {e:#}");
        }
        if !low {
            fire_file_changes(&st, &changes).await;
        }
        tokio::time::sleep(TICK).await;
    }
}

/// Discharging below 20%: background runs wait. Desktops and unreadable batteries count as fine.
fn low_battery() -> bool {
    let Ok(m) = starship_battery::Manager::new() else { return false };
    let Ok(bs) = m.batteries() else { return false };
    bs.flatten().any(|b| b.state() == starship_battery::State::Discharging && b.state_of_charge().value < 0.2)
}

async fn sync_watchers(st: &Arc<State>, changes: &Arc<Changes>, watchers: &mut HashMap<i64, (String, notify::RecommendedWatcher)>) -> Result<()> {
    let rows = sqlx::query("SELECT id, agent_id, spec FROM schedules WHERE kind = 'file' AND enabled = 1").fetch_all(&st.db).await?;
    let want: HashMap<i64, (i64, String)> = rows.iter().map(|r| (r.get("id"), (r.get("agent_id"), r.get("spec")))).collect();
    watchers.retain(|id, (path, _)| want.get(id).is_some_and(|(_, p)| p == path));
    for (id, (agent_id, path)) in want {
        if watchers.contains_key(&id) {
            continue;
        }
        let (st2, changes2) = (st.clone(), changes.clone());
        let mut w = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            let Ok(ev) = ev else { return };
            if ev.kind.is_access() {
                return;
            }
            // the agent's own writes during its run would otherwise retrigger it forever
            if st2.running.lock().unwrap().contains_key(&agent_id) {
                return;
            }
            let mut c = changes2.0.lock().unwrap();
            let e = c.entry(id).or_insert_with(|| (Instant::now(), HashSet::new()));
            e.0 = Instant::now();
            e.1.extend(ev.paths);
        })?;
        w.watch(std::path::Path::new(&path), RecursiveMode::Recursive)?;
        watchers.insert(id, (path, w));
    }
    Ok(())
}

async fn fire_file_changes(st: &Arc<State>, changes: &Changes) {
    let ready: Vec<(i64, HashSet<PathBuf>)> = {
        let mut c = changes.0.lock().unwrap();
        let ids: Vec<i64> = c.iter().filter(|(_, (t, _))| t.elapsed() >= DEBOUNCE).map(|(id, _)| *id).collect();
        ids.into_iter().filter_map(|id| c.remove(&id).map(|(_, p)| (id, p))).collect()
    };
    for (id, paths) in ready {
        let Ok(Some(r)) = sqlx::query("SELECT agent_id, prompt FROM schedules WHERE id = ? AND enabled = 1").bind(id).fetch_optional(&st.db).await else { continue };
        let mut shown: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        shown.sort();
        let prompt = format!("{}\n\nChanged files:\n{}", r.get::<String, _>("prompt"), shown.join("\n"));
        let _ = sqlx::query("UPDATE schedules SET last_run_at = ? WHERE id = ?").bind(now()).bind(id).execute(&st.db).await;
        fire(st, id, r.get("agent_id"), prompt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sandbox::Sandbox, secrets::Vault};

    async fn state(tmp: &std::path::Path, model_url: String) -> (Arc<State>, i64, PathBuf) {
        let st = Arc::new(State::new(tmp.to_path_buf(), model_url, String::new(), Sandbox::detect(), Vault::File(tmp.join("keys"))).await.unwrap());
        let work = tmp.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let a = store::Agent { id: 0, name: "s".into(), role_prompt: "".into(), model_ref: "ollama:x".into(), folders: vec![work.canonicalize().unwrap()],
            allowed_domains: vec![], approval_rules: Default::default(), context_budget: 8_000, limits: store::Limits { review: false, ..Default::default() } };
        let id = store::create_agent(&st.db, &a).await.unwrap();
        (st, id, work.canonicalize().unwrap())
    }

    #[test]
    fn next_times() {
        let t = Local.with_ymd_and_hms(2026, 10, 25, 9, 0, 30).unwrap().timestamp();
        let next = next_after("cron", "*/15 * * * *", t).unwrap().unwrap();
        assert_eq!(Local.timestamp_opt(next, 0).unwrap().format("%H:%M").to_string(), "09:15");
        assert_eq!(next_after("interval", "120", t).unwrap(), Some(t + 120));
        assert!(next_after("interval", "5", t).is_err());
        assert!(next_after("cron", "not a cron", t).is_err());
        assert!(next_after("hourly", "", t).is_err());
    }

    #[tokio::test]
    async fn validation() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, id, work) = state(tmp.path(), String::new()).await;
        assert!(add(&st.db, &json!({"agent_id": id, "kind": "file", "spec": "/etc", "prompt": "x"})).await.is_err());
        assert!(add(&st.db, &json!({"agent_id": id, "kind": "file", "spec": work.join("nope").to_str(), "prompt": "x"})).await.is_err());
        assert!(add(&st.db, &json!({"agent_id": id, "kind": "file", "spec": work.to_str(), "prompt": "x"})).await.is_ok());
        assert!(add(&st.db, &json!({"agent_id": id, "kind": "interval", "spec": "60", "prompt": " "})).await.is_err());
    }

    #[tokio::test]
    async fn due_schedule_fires_and_busy_agent_queues_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, id, _) = state(tmp.path(), "http://127.0.0.1:1".into()).await; // runs fail fast: no model server
        let sid = add(&st.db, &json!({"agent_id": id, "kind": "interval", "spec": "60", "prompt": "check in"})).await.unwrap();
        sqlx::query("UPDATE schedules SET next_run_at = 0").execute(&st.db).await.unwrap(); // missed while down
        let mut ev = st.events.subscribe();
        tick(&st).await.unwrap();
        let fired = ev.recv().await.unwrap();
        assert_eq!((fired["method"].as_str(), fired["params"]["queued"].as_bool()), (Some("schedule.fired"), Some(false)));
        let next: i64 = sqlx::query_scalar("SELECT next_run_at FROM schedules WHERE id = ?").bind(sid).fetch_one(&st.db).await.unwrap();
        assert!(next > now(), "moved to the future, fired once");
        while ev.recv().await.unwrap()["method"] != "chat.done" {}
        let trigger: String = sqlx::query_scalar("SELECT trigger FROM runs ORDER BY id DESC LIMIT 1").fetch_one(&st.db).await.unwrap();
        assert_eq!(trigger, format!("schedule:{sid}"));

        // pretend the agent is mid-run: the first extra firing queues, the second is dropped
        let blocker = tokio::spawn(std::future::pending::<()>());
        st.running.lock().unwrap().insert(id, blocker.abort_handle());
        fire(&st, sid, id, "a".into());
        fire(&st, sid, id, "b".into());
        let mut seen = vec![];
        while seen.len() < 2 {
            let v = ev.recv().await.unwrap();
            if v["method"].as_str().is_some_and(|m| m.starts_with("schedule.")) { seen.push(v["method"].as_str().unwrap().to_string()); }
        }
        assert_eq!(seen, ["schedule.fired", "schedule.skipped"]);
        assert_eq!(st.queued_fire.lock().unwrap().get(&id).map(|(_, p)| p.as_str()), Some("a"));
        blocker.abort();
    }

    #[tokio::test]
    async fn file_change_fires_with_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, id, work) = state(tmp.path(), "http://127.0.0.1:1".into()).await;
        add(&st.db, &json!({"agent_id": id, "kind": "file", "spec": work.to_str(), "prompt": "summarise new files"})).await.unwrap();
        let changes = Arc::new(Changes::default());
        let mut watchers = HashMap::new();
        sync_watchers(&st, &changes, &mut watchers).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await; // FSEvents needs a moment to arm
        std::fs::write(work.join("inbox.txt"), "new").unwrap();
        let t0 = Instant::now();
        while changes.0.lock().unwrap().is_empty() {
            assert!(t0.elapsed() < Duration::from_secs(10), "no file event");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(DEBOUNCE).await;
        let mut ev = st.events.subscribe();
        fire_file_changes(&st, &changes).await;
        let v = ev.recv().await.unwrap();
        assert_eq!(v["method"], "schedule.fired");
        while ev.recv().await.unwrap()["method"] != "chat.done" {}
        let first: String = sqlx::query_scalar("SELECT body FROM messages WHERE agent_id = ? ORDER BY id LIMIT 1").bind(id)
            .fetch_one(&st.db).await.unwrap();
        assert!(first.contains("summarise new files") && first.contains("inbox.txt"), "{first}");
        assert!(changes.0.lock().unwrap().is_empty());
    }
}
