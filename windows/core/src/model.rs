//! The local model file: status and a resumable, verified download.
//!
//! Downloads go to `<file>.part` with an HTTP `Range` resume; when complete the size and
//! SHA-256 are checked and the file is renamed into place. A wrong checksum deletes the
//! partial file (it cannot be trusted for resuming either).

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::progress::{Cancel, Cancelled, Throttle};
use crate::types::{ModelProgressEvent, ModelState, ModelStatus};

/// Which file to fetch and how to check it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub name: String,
    pub file_name: String,
    pub url: String,
    pub size: u64,
    /// lowercase hex
    pub sha256: String,
}

impl ModelSpec {
    /// Qwen3-0.6B Q8_0 (Apache-2.0), pinned to a revision.
    pub fn qwen3_0_6b() -> ModelSpec {
        ModelSpec {
            name: "Qwen3 0.6B (Q8_0)".into(),
            file_name: "Qwen3-0.6B-Q8_0.gguf".into(),
            url: "https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/13ac06624948e4d8589cb4b6f90100837342fcb8/Qwen3-0.6B-Q8_0.gguf".into(),
            size: 804_753_632,
            sha256: "361cc68159042c36ebff7715dc5a2e4612153e88f3e9c9c234820849d6dc9e1d".into(),
        }
    }
}

#[derive(Default)]
struct State {
    downloading: bool,
    error: Option<String>,
}

/// Status + download of the model in `models_dir`. Share one instance (`Arc`) per app.
pub struct ModelManager {
    dir: PathBuf,
    spec: ModelSpec,
    state: Mutex<State>,
    cancel: Cancel,
    /// use HTTP(S)_PROXY / NO_PROXY from the environment
    env_proxy: bool,
}

impl ModelManager {
    pub fn new(models_dir: &Path) -> ModelManager {
        Self::with_spec(models_dir, ModelSpec::qwen3_0_6b())
    }

    pub fn with_spec(models_dir: &Path, spec: ModelSpec) -> ModelManager {
        ModelManager {
            dir: models_dir.to_path_buf(),
            spec,
            state: Mutex::new(State::default()),
            cancel: Cancel::new(),
            env_proxy: true,
        }
    }

    /// Ignore proxy environment variables (tests against a local server).
    pub fn without_env_proxy(mut self) -> ModelManager {
        self.env_proxy = false;
        self
    }

    pub fn spec(&self) -> &ModelSpec {
        &self.spec
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.spec.file_name)
    }

    fn part_path(&self) -> PathBuf {
        self.dir.join(format!("{}.part", self.spec.file_name))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The model file when it is present with the right size (verified when downloaded).
    pub fn ready_path(&self) -> Option<PathBuf> {
        let p = self.path();
        match std::fs::metadata(&p) {
            Ok(m) if m.len() == self.spec.size => Some(p),
            _ => None,
        }
    }

    /// `model_status`.
    pub fn status(&self) -> ModelStatus {
        let st = self.state();
        let part = std::fs::metadata(self.part_path())
            .map(|m| m.len())
            .unwrap_or(0);
        let (state, bytes, error) = if let Some(p) = self.ready_path() {
            (
                ModelState::Ready,
                std::fs::metadata(p)
                    .map(|m| m.len())
                    .unwrap_or(self.spec.size),
                None,
            )
        } else if st.downloading {
            (ModelState::Downloading, part, None)
        } else if let Some(e) = &st.error {
            (ModelState::Error, part, Some(e.clone()))
        } else {
            (ModelState::Missing, part, None)
        };
        ModelStatus {
            state,
            name: self.spec.name.clone(),
            bytes,
            total_bytes: self.spec.size,
            error,
        }
    }

    /// `cancel_model_download` — the partial file is kept for resuming.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// `download_model` (blocking; run it on a worker thread). Resumes a previous `.part`,
    /// verifies size + SHA-256, renames into place. `progress` fires about 4×/s.
    pub fn download(&self, progress: &dyn Fn(&ModelProgressEvent)) -> Result<PathBuf> {
        {
            let mut st = self.state();
            if st.downloading {
                bail!("a download is already running");
            }
            st.downloading = true;
            st.error = None;
        }
        self.cancel.reset();
        let result = self.download_inner(progress);
        let mut st = self.state();
        st.downloading = false;
        match &result {
            Err(e) if crate::progress::is_cancelled(e) => st.error = None,
            Err(e) => st.error = Some(format!("{e:#}")),
            Ok(_) => st.error = None,
        }
        result
    }

    fn agent(&self) -> ureq::Agent {
        let mut b = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)));
        if !self.env_proxy {
            b = b.proxy(None);
        }
        b.build().into()
    }

    fn download_inner(&self, progress: &dyn Fn(&ModelProgressEvent)) -> Result<PathBuf> {
        if let Some(p) = self.ready_path() {
            return Ok(p);
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let part = self.part_path();
        let total = self.spec.size;
        let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have > total {
            std::fs::remove_file(&part)?;
            have = 0;
        }
        if have < total {
            let agent = self.agent();
            let mut req = agent.get(&self.spec.url);
            if have > 0 {
                req = req.header("Range", &format!("bytes={have}-"));
            }
            let mut resp = req
                .call()
                .map_err(|e| anyhow::anyhow!("download failed: {e}"))?;
            let status = resp.status().as_u16();
            let mut file = match status {
                206 if have > 0 => OpenOptions::new().append(true).open(&part)?,
                200 => {
                    // server ignored the range (or a fresh start): begin again
                    have = 0;
                    File::create(&part)?
                }
                416 => OpenOptions::new().append(true).open(&part)?, // nothing left to fetch
                s => bail!("download failed: HTTP {s}"),
            };
            if status != 416 {
                let mut reader = resp.body_mut().as_reader();
                let mut buf = vec![0u8; 256 * 1024];
                let mut throttle = Throttle::new(Duration::from_millis(250));
                progress(&ModelProgressEvent {
                    bytes: have,
                    total_bytes: total,
                });
                loop {
                    if self.cancel.is_cancelled() {
                        file.flush()?;
                        return Err(Cancelled.into());
                    }
                    let n = reader.read(&mut buf).context("download interrupted")?;
                    if n == 0 {
                        break;
                    }
                    if have + n as u64 > total {
                        bail!("download is larger than expected");
                    }
                    file.write_all(&buf[..n])?;
                    have += n as u64;
                    if throttle.ready() {
                        progress(&ModelProgressEvent {
                            bytes: have,
                            total_bytes: total,
                        });
                    }
                }
                file.sync_all()?;
            }
        }
        let len = std::fs::metadata(&part)?.len();
        if len != total {
            bail!("download incomplete ({len} of {total} bytes) — try again to resume");
        }
        progress(&ModelProgressEvent {
            bytes: len,
            total_bytes: total,
        });
        let digest = sha256_file(&part, &self.cancel)?;
        if digest != self.spec.sha256.to_lowercase() {
            let _ = std::fs::remove_file(&part);
            bail!("checksum mismatch — the download was corrupted, please try again");
        }
        let dest = self.path();
        std::fs::rename(&part, &dest)?;
        Ok(dest)
    }
}

/// SHA-256 (lowercase hex) of a file.
pub fn sha256_file(path: &Path, cancel: &Cancel) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        cancel.check()?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Tiny HTTP server for a fixed blob with `Range: bytes=N-` support.
    struct Server {
        url: String,
        requests: Arc<AtomicUsize>,
        ranges: Arc<Mutex<Vec<Option<String>>>>,
        _thread: std::thread::JoinHandle<()>,
    }

    fn serve(data: Vec<u8>, honor_range: bool) -> Server {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let requests = Arc::new(AtomicUsize::new(0));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let (rq, rg) = (requests.clone(), ranges.clone());
        let thread = std::thread::spawn(move || {
            for req in server.incoming_requests() {
                rq.fetch_add(1, Ordering::SeqCst);
                let range = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .map(|h| h.value.as_str().to_string());
                rg.lock().unwrap().push(range.clone());
                let start = range
                    .as_deref()
                    .and_then(|r| r.strip_prefix("bytes="))
                    .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok())
                    .filter(|_| honor_range);
                let resp = match start {
                    Some(s) if s >= data.len() => {
                        tiny_http::Response::from_data(Vec::new()).with_status_code(416)
                    }
                    Some(s) => {
                        tiny_http::Response::from_data(data[s..].to_vec()).with_status_code(206)
                    }
                    None => tiny_http::Response::from_data(data.clone()).with_status_code(200),
                };
                let _ = req.respond(resp);
            }
        });
        Server {
            url: format!("http://127.0.0.1:{port}/model.gguf"),
            requests,
            ranges,
            _thread: thread,
        }
    }

    fn blob() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn spec(url: &str, data: &[u8]) -> ModelSpec {
        let mut h = Sha256::new();
        h.update(data);
        ModelSpec {
            name: "Test".into(),
            file_name: "test.gguf".into(),
            url: url.into(),
            size: data.len() as u64,
            sha256: h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        }
    }

    #[test]
    fn downloads_and_verifies() {
        let data = blob();
        let srv = serve(data.clone(), true);
        let dir = tempfile::Builder::new()
            .prefix("mf-model")
            .tempdir()
            .unwrap();
        let m = ModelManager::with_spec(dir.path(), spec(&srv.url, &data)).without_env_proxy();
        assert_eq!(m.status().state, ModelState::Missing);
        let seen = Mutex::new(Vec::new());
        let p = m.download(&|e| seen.lock().unwrap().push(e.bytes)).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), data);
        assert_eq!(m.status().state, ModelState::Ready);
        assert_eq!(m.status().bytes, data.len() as u64);
        assert_eq!(*seen.lock().unwrap().last().unwrap(), data.len() as u64);
        // already there: no request
        let before = srv.requests.load(Ordering::SeqCst);
        m.download(&|_| {}).unwrap();
        assert_eq!(srv.requests.load(Ordering::SeqCst), before);
    }

    #[test]
    fn resumes_a_partial_download_with_range() {
        let data = blob();
        let srv = serve(data.clone(), true);
        let dir = tempfile::Builder::new()
            .prefix("mf-model")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("test.gguf.part"), &data[..123_456]).unwrap();
        let m = ModelManager::with_spec(dir.path(), spec(&srv.url, &data)).without_env_proxy();
        assert_eq!(m.status().bytes, 123_456);
        m.download(&|_| {}).unwrap();
        assert_eq!(std::fs::read(m.path()).unwrap(), data);
        assert_eq!(
            srv.ranges.lock().unwrap().as_slice(),
            &[Some("bytes=123456-".to_string())]
        );
        assert!(!dir.path().join("test.gguf.part").exists());
    }

    #[test]
    fn server_without_range_support_restarts_cleanly() {
        let data = blob();
        let srv = serve(data.clone(), false);
        let dir = tempfile::Builder::new()
            .prefix("mf-model")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("test.gguf.part"), &data[..1000]).unwrap();
        let m = ModelManager::with_spec(dir.path(), spec(&srv.url, &data)).without_env_proxy();
        m.download(&|_| {}).unwrap();
        assert_eq!(std::fs::read(m.path()).unwrap(), data);
    }

    #[test]
    fn complete_part_is_verified_without_downloading() {
        let data = blob();
        let srv = serve(data.clone(), true);
        let dir = tempfile::Builder::new()
            .prefix("mf-model")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("test.gguf.part"), &data).unwrap();
        let m = ModelManager::with_spec(dir.path(), spec(&srv.url, &data)).without_env_proxy();
        m.download(&|_| {}).unwrap();
        assert_eq!(srv.requests.load(Ordering::SeqCst), 0);
        assert_eq!(m.status().state, ModelState::Ready);
    }

    #[test]
    fn wrong_checksum_is_an_error_and_drops_the_part() {
        let data = blob();
        let srv = serve(data.clone(), true);
        let dir = tempfile::Builder::new()
            .prefix("mf-model")
            .tempdir()
            .unwrap();
        let mut s = spec(&srv.url, &data);
        s.sha256 = "00".repeat(32);
        let m = ModelManager::with_spec(dir.path(), s).without_env_proxy();
        let e = m.download(&|_| {}).unwrap_err();
        assert!(e.to_string().contains("checksum"), "{e}");
        assert!(!dir.path().join("test.gguf.part").exists());
        assert!(!m.path().exists());
        let st = m.status();
        assert_eq!(st.state, ModelState::Error);
        assert!(st.error.unwrap().contains("checksum"));
    }

    #[test]
    fn cancel_keeps_the_part_for_resuming() {
        let data = blob();
        let srv = serve(data.clone(), true);
        let dir = tempfile::Builder::new()
            .prefix("mf-model")
            .tempdir()
            .unwrap();
        let m = ModelManager::with_spec(dir.path(), spec(&srv.url, &data)).without_env_proxy();
        // cancel from inside the first progress callback
        let e = m.download(&|_| m.cancel()).unwrap_err();
        assert!(crate::progress::is_cancelled(&e));
        assert_eq!(m.status().state, ModelState::Missing);
        assert!(!m.path().exists());
        m.download(&|_| {}).unwrap();
        assert_eq!(std::fs::read(m.path()).unwrap(), data);
    }

    #[test]
    fn real_spec_is_pinned() {
        let s = ModelSpec::qwen3_0_6b();
        assert_eq!(s.size, 804_753_632);
        assert_eq!(s.sha256.len(), 64);
        assert!(s
            .url
            .contains("/resolve/13ac06624948e4d8589cb4b6f90100837342fcb8/"));
    }
}
