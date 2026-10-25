use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};

/// Split `"ollama:qwen3:8b"` into `("ollama", "qwen3:8b")`.
pub fn parse_ref(model_ref: &str) -> Result<(&str, &str)> {
    match model_ref.split_once(':') {
        Some((b @ ("ollama" | "llama"), m)) if !m.is_empty() => Ok((b, m)),
        _ => bail!("model_ref must be `ollama:<name>` or `llama:<name>`"),
    }
}

/// Ollama models plus whatever llama-server has loaded. Unreachable backends are skipped.
pub async fn list(http: &reqwest::Client, ollama: &str, llama: &str) -> Vec<String> {
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
    if let Some(v) = get(format!("{llama}/v1/models")).await {
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

/// One streamed `/v1/chat/completions` call. Calls `on_delta` for each content
/// chunk and returns the full assistant message (tool calls included) and the
/// server's token usage (`Null` if it doesn't report it).
pub async fn chat(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    messages: &[Value],
    tools: &Value,
    mut on_delta: impl FnMut(&str),
) -> Result<(Value, Value)> {
    let res = http
        .post(format!("{base}/v1/chat/completions"))
        .json(&{
            let mut body = json!({"model": model, "messages": messages, "stream": true,
                                  "stream_options": {"include_usage": true}});
            if tools.as_array().is_some_and(|t| !t.is_empty()) {
                body["tools"] = tools.clone();
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

/// Start `llama-server` if `BURROW_LLAMA_SERVER` and `BURROW_LLAMA_MODEL` are set.
/// shortcut: configured by env vars; move to the settings UI with model downloads (v0.1).
pub fn spawn_llama_server(port: u16) -> Result<Option<tokio::process::Child>> {
    let (Ok(bin), Ok(gguf)) = (std::env::var("BURROW_LLAMA_SERVER"), std::env::var("BURROW_LLAMA_MODEL")) else {
        return Ok(None);
    };
    let child = tokio::process::Command::new(bin)
        .args(["--jinja", "-m", &gguf, "--port", &port.to_string(), "--host", "127.0.0.1"])
        .kill_on_drop(true)
        .spawn()
        .context("starting llama-server")?;
    Ok(Some(child))
}
