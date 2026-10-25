use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use crate::journal;
use std::path::{Path, PathBuf};

const MAX_READ: u64 = 256 * 1024;

/// Resolve `p` (relative paths are taken from the first folder) and fail unless it
/// lies inside one of `folders` after following symlinks.
pub fn scoped(folders: &[PathBuf], p: &str) -> Result<PathBuf> {
    let first = folders.first().context("agent has no folders")?;
    let p = first.join(p); // absolute `p` replaces `first`
    let resolved = match p.canonicalize() {
        Ok(r) => r,
        Err(_) => {
            // a dangling symlink would let a write land outside the folder
            if p.symlink_metadata().is_ok() {
                bail!("refusing dangling symlink");
            }
            let name = p.file_name().context("bad path")?;
            p.parent().context("bad path")?.canonicalize()?.join(name)
        }
    };
    let ok = folders
        .iter()
        .filter_map(|f| f.canonicalize().ok())
        .any(|f| resolved.starts_with(f));
    if !ok {
        bail!("path is outside the agent's folders");
    }
    Ok(resolved)
}

/// Built-in tools; `web_search` only when a search backend is configured.
pub fn specs(search: bool) -> Vec<Value> {
    let f = |name: &str, desc: &str, props: Value, req: &[&str]| {
        json!({"type": "function", "function": {"name": name, "description": desc,
            "parameters": {"type": "object", "properties": props, "required": req}}})
    };
    let path = json!({"type": "string", "description": "file or directory path"});
    let mut out = vec![
        f("list_dir", "List entries of a directory", json!({"path": path}), &["path"]),
        f("read_file", "Read a UTF-8 text file", json!({"path": path}), &["path"]),
        f("write_file", "Create or overwrite a text file",
          json!({"path": path, "content": {"type": "string"}}), &["path", "content"]),
        f("delete_file", "Delete a file", json!({"path": path}), &["path"]),
        f("shell", "Run a shell command in your isolated Linux sandbox (has Python). Your folders \
          are read-only there; your scratch folder is writable and is the working directory. Network goes \
          through a proxy: allowed domains work (pip, npm, git, curl), new domains need the user's approval",
          json!({"command": {"type": "string"}}), &["command"]),
        f("http_fetch", "GET a web page or API (http/https). New domains need the user's approval",
          json!({"url": {"type": "string"}}), &["url"]),
        f("notes_add", "Save a note to your long-term notes", json!({"text": {"type": "string"}}), &["text"]),
        f("notes_search", "Search your notes by keywords", json!({"query": {"type": "string"}}), &["query"]),
    ];
    if search {
        out.push(f("web_search", "Search the web; returns titles, URLs and snippets", json!({"query": {"type": "string"}}), &["query"]));
    }
    out
}

/// What a tool returns: text for the model, plus how to undo it if that's possible.
pub struct Outcome {
    pub output: String,
    pub inverse: Option<Value>,
}

impl From<String> for Outcome {
    fn from(output: String) -> Self {
        Outcome { output, inverse: None }
    }
}

/// Run a file tool. Errors come back as text so the model can see and recover from them.
pub fn run(folders: &[PathBuf], snapshots: &Path, name: &str, args: &Value) -> Outcome {
    call(folders, snapshots, name, args).unwrap_or_else(|e| format!("error: {e:#}").into())
}

fn call(folders: &[PathBuf], snapshots: &Path, name: &str, args: &Value) -> Result<Outcome> {
    let arg = |k: &str| args[k].as_str().with_context(|| format!("missing `{k}`"));
    let path = scoped(folders, arg("path")?)?;
    match name {
        "list_dir" => {
            let mut out = vec![];
            for e in std::fs::read_dir(&path)? {
                let e = e?;
                let slash = if e.file_type()?.is_dir() { "/" } else { "" };
                out.push(format!("{}{slash}", e.file_name().to_string_lossy()));
            }
            out.sort();
            Ok(out.join("\n").into())
        }
        "read_file" => {
            if std::fs::metadata(&path)?.len() > MAX_READ {
                bail!("file larger than {MAX_READ} bytes");
            }
            Ok(std::fs::read_to_string(&path)?.into())
        }
        "write_file" => {
            let content = arg("content")?;
            if path.is_dir() {
                bail!("{} is a directory", path.display());
            }
            let inverse = journal::snapshot(snapshots, &path, Some(content.as_bytes()))?;
            std::fs::write(&path, content)?;
            let output = format!("wrote {} bytes to {}", content.len(), path.display());
            Ok(Outcome { output, inverse: Some(inverse) })
        }
        "delete_file" => {
            if !path.is_file() {
                bail!("{} is not a file", path.display());
            }
            let inverse = journal::snapshot(snapshots, &path, None)?;
            std::fs::remove_file(&path)?;
            Ok(Outcome { output: format!("deleted {}", path.display()), inverse: Some(inverse) })
        }
        _ => bail!("unknown tool `{name}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoping() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let ok = root.path().join("ok");
        std::fs::create_dir(&ok).unwrap();
        std::fs::write(outside.path().join("secret"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), ok.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("nope"), ok.join("dangling")).unwrap();
        let folders = vec![ok.clone()];

        assert!(scoped(&folders, "new.txt").is_ok());
        assert!(scoped(&folders, ok.join("new.txt").to_str().unwrap()).is_ok());
        assert!(scoped(&folders, "../escape.txt").is_err());
        assert!(scoped(&folders, "link").is_err());
        assert!(scoped(&folders, "dangling").is_err());
        assert!(scoped(&folders, outside.path().join("secret").to_str().unwrap()).is_err());
        assert!(scoped(&folders, "/etc/passwd").is_err());
        assert!(scoped(&folders, "..").is_err());
    }
}
