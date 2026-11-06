//! Model downloads: resumable (HTTP Range into a `.part` file), checked against the
//! catalog's pinned sha256 before the file takes its real name.
use crate::agent::State;
use crate::hardware::{self, Model};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::task::AbortHandle;

#[derive(Default)]
pub struct Downloads(Mutex<HashMap<String, AbortHandle>>);

impl Downloads {
    pub fn ids(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
}

pub fn dir(home: &Path) -> PathBuf {
    home.join("models")
}

pub fn installed(home: &Path, m: &Model) -> bool {
    dir(home).join(&m.file).is_file()
}

/// Start downloading in the background; progress and the result arrive as events.
pub fn start(st: &Arc<State>, id: &str) -> Result<()> {
    let m = hardware::model(id)?;
    let mut running = st.downloads.0.lock().unwrap();
    if running.contains_key(id) {
        bail!("{id} is already downloading");
    }
    let st2 = st.clone();
    let h = tokio::spawn(async move {
        let url = format!("https://huggingface.co/{}/resolve/main/{}", m.repo, m.file);
        let mut last = 0u64;
        // the model is useless without the runtime, so a download leaves both in place
        let res: Result<()> = async {
            crate::llama::ensure_server(&st2.http, &st2.home).await?;
            fetch(&st2.http, &url, &dir(&st2.home), &m.file, m.size, &m.sha256, |done, total| {
                // about one event per percent
                if done - last >= total / 100 || done == total {
                    last = done;
                    st2.emit("models.progress", json!({"id": m.id, "done": done, "total": total}));
                }
            }).await?;
            Ok(())
        }.await;
        st2.downloads.0.lock().unwrap().remove(&m.id);
        match res {
            Ok(_) => st2.emit("models.downloaded", json!({"id": m.id})),
            Err(e) => st2.emit("models.download_failed", json!({"id": m.id, "error": format!("{e:#}")})),
        }
    });
    running.insert(id.to_string(), h.abort_handle());
    Ok(())
}

/// Stop a download; the `.part` file stays so the next start resumes.
pub fn cancel(st: &State, id: &str) -> Result<()> {
    st.downloads.0.lock().unwrap().remove(id).context("not downloading")?.abort();
    st.emit("models.download_failed", json!({"id": id, "error": "cancelled"}));
    Ok(())
}

fn hash_file(path: &Path, h: &mut Sha256) -> Result<u64> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            return Ok(n);
        }
        h.update(&buf[..k]);
        n += k as u64;
    }
}

/// A download that gets no bytes for this long fails (the `.part` file stays, so Download resumes it).
const STALL: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn fetch(http: &reqwest::Client, url: &str, dir: &Path, file: &str, size: u64, sha256: &str,
                   mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    let dest = dir.join(file);
    if dest.is_file() {
        return Ok(dest);
    }
    std::fs::create_dir_all(dir)?;
    let part = dir.join(format!("{file}.part"));
    // a resumed part can be gigabytes; hash it off the async threads
    let p2 = part.clone();
    let (mut h, mut done) = tokio::task::spawn_blocking(move || -> Result<(Sha256, u64)> {
        let mut h = Sha256::new();
        let n = if p2.is_file() { hash_file(&p2, &mut h)? } else { 0 };
        Ok((h, n))
    }).await??;
    if done > size {
        done = 0;
        h = Sha256::new();
    }
    if done < size {
        let mut req = http.get(url);
        if done > 0 {
            req = req.header("Range", format!("bytes={done}-"));
        }
        let res = tokio::time::timeout(STALL, req.send()).await.context("the server didn't answer in 60 s")??.error_for_status()?;
        let resumed = res.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !resumed {
            // server ignored the range: start over
            done = 0;
            h = Sha256::new();
        }
        let mut out = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part)?;
        let mut body = res.bytes_stream();
        while let Some(chunk) = tokio::time::timeout(STALL, body.next()).await.context("download stalled for 60 s; press Download to resume")? {
            let chunk = chunk?;
            out.write_all(&chunk)?;
            h.update(&chunk);
            done += chunk.len() as u64;
            progress(done, size);
        }
        out.sync_all()?;
    }
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if done != size || got != sha256 {
        std::fs::remove_file(&part)?;
        bail!("download of {file} is corrupt (got {done} bytes, sha256 {got}); try again");
    }
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves `body`, honouring `Range: bytes=N-`. Returns the URL and the ranges it saw.
    async fn server(body: &'static [u8]) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/m.gguf", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(vec![]));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let mut b = [0u8; 4096];
                let n = s.read(&mut b).await.unwrap();
                let req = String::from_utf8_lossy(&b[..n]).to_lowercase();
                let from = req.split("range: bytes=").nth(1).and_then(|r| r.split('-').next()).and_then(|x| x.parse::<usize>().ok());
                seen2.lock().unwrap().push(from.map(|f| f.to_string()).unwrap_or_default());
                let (status, slice) = match from {
                    Some(f) => ("206 Partial Content", &body[f..]),
                    None => ("200 OK", body),
                };
                let head = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", slice.len());
                s.write_all(head.as_bytes()).await.unwrap();
                s.write_all(slice).await.unwrap();
            }
        });
        (url, seen)
    }

    fn sha(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    #[tokio::test]
    async fn resumes_and_verifies() {
        const BODY: &[u8] = b"pretend this is a few gigabytes of GGUF weights";
        let (url, seen) = server(BODY).await;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("m.gguf.part"), &BODY[..10]).unwrap();
        let mut last = 0;
        let p = fetch(&reqwest::Client::new(), &url, tmp.path(), "m.gguf", BODY.len() as u64, &sha(BODY), |d, _| last = d).await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), BODY);
        assert_eq!(*seen.lock().unwrap(), ["10"], "resumed from the part file");
        assert_eq!(last, BODY.len() as u64);
        assert!(!tmp.path().join("m.gguf.part").exists());
    }

    #[tokio::test]
    async fn checksum_mismatch_is_refused() {
        const BODY: &[u8] = b"tampered weights";
        let (url, _) = server(BODY).await;
        let tmp = tempfile::tempdir().unwrap();
        let err = fetch(&reqwest::Client::new(), &url, tmp.path(), "m.gguf", BODY.len() as u64, &"0".repeat(64), |_, _| {}).await.unwrap_err();
        assert!(format!("{err:#}").contains("corrupt"));
        assert!(!tmp.path().join("m.gguf").exists() && !tmp.path().join("m.gguf.part").exists());
    }
}
