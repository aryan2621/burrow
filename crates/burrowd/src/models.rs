use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use crate::secrets::Vault;

/// Split `"ollama:qwen3:8b"` into `("ollama", "qwen3:8b")`.
pub fn parse_ref(model_ref: &str) -> Result<(&str, &str)> {
    match model_ref.split_once(':') {
        Some((b @ ("ollama" | "llama" | "anthropic" | "openai"), m)) if !m.is_empty() => Ok((b, m)),
        _ => bail!("model_ref must be `ollama:<name>`, `llama:<name>`, `anthropic:<name>` or `openai:<name>`"),
    }
}

/// USD per million (input, output) tokens. Local models are free; other cloud models are priced
/// from the `prices` setting ({"openai:gpt-x": [in, out]}), else unknown.
pub async fn price(db: &SqlitePool, model_ref: &str) -> Option<(f64, f64)> {
    if !is_cloud(model_ref) {
        return Some((0.0, 0.0));
    }
    // cached: 2026-10 list prices
    let builtin = match model_ref {
        "anthropic:claude-fable-5-1" | "anthropic:claude-fable-5" => Some((10.0, 50.0)),
        "anthropic:claude-opus-5-5" => Some((4.0, 20.0)),
        "anthropic:claude-opus-5" | "anthropic:claude-opus-4-8" | "anthropic:claude-opus-4-7" | "anthropic:claude-opus-4-6" => Some((5.0, 25.0)),
        "anthropic:claude-sonnet-5-5" | "anthropic:claude-sonnet-5" => Some((2.0, 10.0)),
        "anthropic:claude-sonnet-4-6" => Some((3.0, 15.0)),
        "anthropic:claude-haiku-5-5" => Some((0.10, 0.50)),
        "anthropic:claude-haiku-4-5" => Some((1.0, 5.0)),
        _ => None,
    };
    if builtin.is_some() {
        return builtin;
    }
    let s: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'prices'").fetch_optional(db).await.ok()??;
    let v: Value = serde_json::from_str(&s).ok()?;
    Some((v[model_ref][0].as_f64()?, v[model_ref][1].as_f64()?))
}

/// Cost of one call; cache reads bill at a tenth of the input price.
// shortcut: cache writes (1.25x) count as plain input; track them if cache-heavy agents need exact bills.
pub fn cost((input, output): (f64, f64), usage: &Value) -> f64 {
    let prompt = usage["prompt_tokens"].as_f64().unwrap_or(0.0);
    let cached = usage["cache_read_tokens"].as_f64().unwrap_or(0.0);
    ((prompt - cached) * input + cached * input * 0.1 + usage["completion_tokens"].as_f64().unwrap_or(0.0) * output) / 1e6
}

pub fn is_cloud(model_ref: &str) -> bool {
    model_ref.starts_with("anthropic:") || model_ref.starts_with("openai:")
}

const ANTHROPIC: &str = "https://api.anthropic.com/v1";

/// Cloud provider `(base_url, api_key)`; `None` until a key is set (cloud is off by default).
pub async fn provider(db: &SqlitePool, vault: &Vault, name: &str) -> Result<Option<(String, String)>> {
    let Some(key) = vault.get(&format!("provider-{name}")).await? else { return Ok(None) };
    let base = match name {
        "anthropic" => ANTHROPIC.to_string(),
        _ => sqlx::query_scalar("SELECT value FROM settings WHERE key = 'openai_base_url'").fetch_optional(db).await?
            .unwrap_or_else(|| "https://api.openai.com/v1".into()),
    };
    Ok(Some((base, key)))
}

/// `providers.set`: a string sets, null clears, a missing field is left alone.
pub async fn configure(db: &SqlitePool, vault: &Vault, p: &Value) -> Result<()> {
    let name = p["provider"].as_str().unwrap_or("");
    anyhow::ensure!(matches!(name, "anthropic" | "openai"), "provider must be `anthropic` or `openai`");
    match &p["api_key"] {
        Value::String(k) => vault.set(&format!("provider-{name}"), k.trim()).await?,
        Value::Null if p.get("api_key").is_some() => vault.delete(&format!("provider-{name}")).await?,
        _ => {}
    }
    match &p["base_url"] {
        Value::String(u) if name == "openai" => {
            let parsed = url::Url::parse(u).context("bad base URL")?;
            anyhow::ensure!(matches!(parsed.scheme(), "http" | "https"), "base URL must be http(s)");
            sqlx::query("INSERT INTO settings (key, value) VALUES ('openai_base_url', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(u.trim_end_matches('/')).execute(db).await?;
        }
        Value::Null if p.get("base_url").is_some() => {
            sqlx::query("DELETE FROM settings WHERE key = 'openai_base_url'").execute(db).await?;
        }
        _ => {}
    }
    Ok(())
}

pub async fn providers(db: &SqlitePool, vault: &Vault) -> Result<Value> {
    let mut out = serde_json::Map::new();
    for name in ["anthropic", "openai"] {
        out.insert(name.into(), match provider(db, vault, name).await? {
            Some((base, _)) => json!({"configured": true, "base_url": base}),
            None => json!({"configured": false}),
        });
    }
    Ok(out.into())
}

fn anthropic_headers(req: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
    req.header("x-api-key", key).header("anthropic-version", "2026-10-21")
}

/// Models of each configured cloud provider, as model refs. Unreachable providers are skipped.
pub async fn cloud(http: &reqwest::Client, db: &SqlitePool, vault: &Vault) -> Vec<String> {
    let mut out = vec![];
    for name in ["anthropic", "openai"] {
        let Ok(Some((base, key))) = provider(db, vault, name).await else { continue };
        let req = http.get(format!("{base}/models")).timeout(std::time::Duration::from_secs(10));
        let req = if name == "anthropic" { anthropic_headers(req, &key) } else { req.bearer_auth(&key) };
        let Ok(res) = req.send().await else { continue };
        let Ok(v) = res.json::<Value>().await else { continue };
        for m in v["data"].as_array().into_iter().flatten() {
            out.extend(m["id"].as_str().map(|n| format!("{name}:{n}")));
        }
    }
    out
}

/// Ollama models plus downloaded built-in models (or, for an external llama-server, what it
/// has loaded). Unreachable backends are skipped.
pub async fn list(http: &reqwest::Client, ollama: &str, llama: &crate::llama::Llama, home: &std::path::Path) -> Vec<String> {
    let get = |url: String| async move {
        let v: Value = http.get(url).send().await.ok()?.json().await.ok()?;
        Some(v)
    };
    let mut out = vec![];
    if let Some(v) = get(format!("{ollama}/api/tags")).await {
        for m in v["models"].as_array().into_iter().flatten() {
            out.extend(m["name"].as_str().map(|n| format!("ollama:{n}")));
        }
    }
    if llama.managed() {
        let installed = crate::hardware::catalog().into_iter().filter(|m| crate::download::installed(home, m));
        out.extend(installed.map(|m| format!("llama:{}", m.id)));
    } else if let Some(v) = get(format!("{}/v1/models", llama.url)).await {
        for m in v["data"].as_array().into_iter().flatten() {
            out.extend(m["id"].as_str().map(|n| format!("llama:{n}")));
        }
    }
    out
}

pub async fn pull(http: &reqwest::Client, ollama: &str, name: &str) -> Result<()> {
    let v: Value = http
        .post(format!("{ollama}/api/pull"))
        .json(&json!({"model": name, "stream": false}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    match v["error"].as_str() {
        Some(e) => bail!("{e}"),
        None => Ok(()),
    }
}

/// One streamed `<base>/chat/completions` call (`base` ends in `/v1`). Calls `on_delta` for
/// each content chunk and returns the full assistant message (tool calls included) and the
/// server's token usage (`Null` if it doesn't report it). `key` marks a cloud provider.
pub async fn chat(
    http: &reqwest::Client,
    base: &str,
    key: Option<&str>,
    model: &str,
    messages: &[Value],
    tools: &Value,
    mut on_delta: impl FnMut(&str),
) -> Result<(Value, Value)> {
    // Claude's raw content blocks ride along on assistant messages; other servers don't know them
    let messages: Vec<Value> = messages.iter().map(|m| {
        let mut m = m.clone();
        if let Some(o) = m.as_object_mut() { o.remove("_anthropic"); }
        m
    }).collect();
    let mut req = http.post(format!("{base}/chat/completions"));
    if let Some(k) = key {
        req = req.bearer_auth(k);
    }
    let res = req
        .json(&{
            let mut body = json!({"model": model, "messages": messages, "stream": true,
                                  "stream_options": {"include_usage": true}});
            if key.is_none() {
                // llama-server: reuse the KV cache of the slot holding the most similar prompt
                body["cache_prompt"] = json!(true);
            }
            if tools.as_array().is_some_and(|t| !t.is_empty()) {
                body["tools"] = tools.clone();
                // small models batch dependent calls (read then write) and write placeholders
                body["parallel_tool_calls"] = json!(false);
            }
            body
        })
        .send()
        .await?;
    if !res.status().is_success() {
        bail!("model server returned {}: {}", res.status(), res.text().await?);
    }
    let mut content = String::new();
    let mut calls: Vec<Value> = vec![];
    let mut usage = Value::Null;
    let mut buf = Vec::new();
    let mut body = res.bytes_stream();
    'outer: while let Some(chunk) = body.next().await {
        buf.extend_from_slice(&chunk?);
        while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=nl).collect();
            let line = std::str::from_utf8(&line)?.trim();
            let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                break 'outer;
            }
            let v: Value = serde_json::from_str(data).context("bad stream chunk")?;
            if v["usage"].is_object() {
                usage = v["usage"].clone();
            }
            let delta = &v["choices"][0]["delta"];
            if let Some(t) = delta["content"].as_str() {
                content.push_str(t);
                on_delta(t);
            }
            // tool calls arrive in pieces keyed by index; arguments are concatenated
            for tc in delta["tool_calls"].as_array().into_iter().flatten() {
                let i = tc["index"].as_u64().unwrap_or(calls.len() as u64) as usize;
                if i >= calls.len() {
                    calls.resize(i + 1, json!({"type": "function", "function": {"name": "", "arguments": ""}}));
                }
                let c = &mut calls[i];
                if let Some(id) = tc["id"].as_str() {
                    c["id"] = json!(id);
                }
                for k in ["name", "arguments"] {
                    if let Some(s) = tc["function"][k].as_str() {
                        let cur = c["function"][k].as_str().unwrap_or("").to_owned();
                        c["function"][k] = json!(cur + s);
                    }
                }
            }
        }
    }
    for (i, c) in calls.iter_mut().enumerate() {
        if c.get("id").is_none() {
            c["id"] = json!(format!("call_{i}"));
        }
    }
    let mut msg = json!({"role": "assistant", "content": content});
    if !calls.is_empty() {
        msg["tool_calls"] = json!(calls);
    }
    Ok((msg, usage))
}

/// OpenAI-format history → Anthropic `(system, messages)`. Assistant turns we got from Claude
/// replay their raw blocks (thinking included); tool results merge into one user turn.
fn to_anthropic(messages: &[Value]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut out: Vec<Value> = vec![];
    for m in messages {
        let (role, blocks) = match m["role"].as_str().unwrap_or("") {
            "system" => {
                system.push_str(m["content"].as_str().unwrap_or(""));
                continue;
            }
            "assistant" => ("assistant", match m.get("_anthropic") {
                Some(Value::Array(b)) => b.clone(),
                _ => {
                    let text = m["content"].as_str().unwrap_or("");
                    let mut b: Vec<Value> = if text.is_empty() { vec![] } else { vec![json!({"type": "text", "text": text})] };
                    b.extend(m["tool_calls"].as_array().into_iter().flatten().map(|c| json!({"type": "tool_use",
                        "id": c["id"], "name": c["function"]["name"],
                        "input": serde_json::from_str::<Value>(c["function"]["arguments"].as_str().unwrap_or("{}")).ok()
                            .filter(Value::is_object).unwrap_or(json!({}))})));
                    b
                }
            }),
            "tool" => ("user", vec![json!({"type": "tool_result", "tool_use_id": m["tool_call_id"],
                "content": m["content"].as_str().unwrap_or("")})]),
            _ => ("user", vec![json!({"type": "text", "text": m["content"].as_str().unwrap_or("")})]),
        };
        if blocks.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last["role"] == role => last["content"].as_array_mut().unwrap().extend(blocks),
            _ => out.push(json!({"role": role, "content": blocks})),
        }
    }
    (system, out)
}

/// One streamed Messages API call. Same contract as [`chat`]: an OpenAI-format assistant message
/// (plus `_anthropic`, the raw blocks to replay) and usage as prompt/completion tokens.
pub async fn anthropic(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    messages: &[Value],
    tools: &Value,
    mut on_delta: impl FnMut(&str),
) -> Result<(Value, Value)> {
    let (system, msgs) = to_anthropic(messages);
    let mut body = json!({"model": model, "max_tokens": 64000, "stream": true, "system": system, "messages": msgs,
        "fallbacks": "default", "cache_control": {"type": "ephemeral"},
        // compaction drops old turns, which would invalidate replayed thinking: drop those blocks instead of failing
        "thinking": {"type": "adaptive", "block_binding": {"prefix_mismatch_behavior": "drop_block"}},
        "output_config": {"effort": "high"}});
    if let Some(t) = tools.as_array().filter(|t| !t.is_empty()) {
        body["tools"] = t.iter().map(|t| json!({"name": t["function"]["name"], "description": t["function"]["description"],
            "input_schema": t["function"]["parameters"]})).collect();
    }
    let res = anthropic_headers(http.post(format!("{ANTHROPIC}/messages")), key)
        .header("anthropic-beta", "server-side-fallback-2026-07-01,thinking-binding-controls-2026-08-01")
        .json(&body).send().await?;
    if !res.status().is_success() {
        bail!("Anthropic API returned {}: {}", res.status(), res.text().await?);
    }
    let mut blocks: Vec<Value> = vec![];
    let mut partial: Vec<String> = vec![]; // tool input JSON per block index
    let (mut input_tokens, mut cache_read, mut output_tokens, mut stop) = (0i64, 0i64, 0i64, String::new());
    let mut buf = Vec::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk?);
        while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=nl).collect();
            let Some(data) = std::str::from_utf8(&line)?.trim().strip_prefix("data:").map(str::trim) else { continue };
            let v: Value = serde_json::from_str(data).context("bad stream event")?;
            let i = v["index"].as_u64().unwrap_or(0) as usize;
            match v["type"].as_str().unwrap_or("") {
                "message_start" => {
                    let u = &v["message"]["usage"];
                    input_tokens = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
                        .iter().filter_map(|k| u[k].as_i64()).sum();
                    cache_read = u["cache_read_input_tokens"].as_i64().unwrap_or(0);
                }
                "content_block_start" => {
                    if blocks.len() <= i {
                        blocks.resize(i + 1, Value::Null);
                        partial.resize(i + 1, String::new());
                    }
                    blocks[i] = v["content_block"].clone();
                }
                "content_block_delta" => {
                    let d = &v["delta"];
                    let Some(b) = blocks.get_mut(i) else { continue };
                    let mut append = |k: &str, s: &str| b[k] = json!(format!("{}{s}", b[k].as_str().unwrap_or("")));
                    match d["type"].as_str().unwrap_or("") {
                        "text_delta" => {
                            let t = d["text"].as_str().unwrap_or("");
                            append("text", t);
                            on_delta(t);
                        }
                        "thinking_delta" => append("thinking", d["thinking"].as_str().unwrap_or("")),
                        "signature_delta" => b["signature"] = d["signature"].clone(),
                        "input_json_delta" => partial[i].push_str(d["partial_json"].as_str().unwrap_or("")),
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    if blocks.get(i).is_some_and(|b| b["type"] == "tool_use") && !partial[i].is_empty() {
                        blocks[i]["input"] = serde_json::from_str(&partial[i])
                            .unwrap_or_else(|e| json!({"_parse_error": e.to_string()}));
                    }
                }
                "message_delta" => {
                    output_tokens = v["usage"]["output_tokens"].as_i64().unwrap_or(output_tokens);
                    stop = v["delta"]["stop_reason"].as_str().unwrap_or("").into();
                }
                "error" => bail!("Anthropic API error: {}", v["error"]["message"].as_str().unwrap_or(data)),
                _ => {}
            }
        }
    }
    if stop == "refusal" {
        bail!("Claude declined this request");
    }
    blocks.retain(|b| !b.is_null());
    let text: String = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    let calls: Vec<Value> = blocks.iter().filter(|b| b["type"] == "tool_use").map(|b| json!({"id": b["id"], "type": "function",
        "function": {"name": b["name"], "arguments": b["input"].to_string()}})).collect();
    let mut msg = json!({"role": "assistant", "content": text, "_anthropic": blocks});
    if !calls.is_empty() {
        msg["tool_calls"] = json!(calls);
    }
    Ok((msg, json!({"prompt_tokens": input_tokens, "cache_read_tokens": cache_read, "completion_tokens": output_tokens})))
}
