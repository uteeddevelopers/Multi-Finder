//! Keeps the index current without full rescans: file-system events (ReadDirectoryChangesW
//! on Windows via `notify`), debounced, applied to the index. Port of `Watcher.swift`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;
use notify::{RecursiveMode, Watcher as _};

use crate::index::IndexStore;
use crate::paths;
use crate::progress::{no_progress, Cancel};
use crate::record::{self, FileKind};
use crate::rules::ScanRules;
use crate::scanner::Scanner;

/// Default quiet period before a batch of changes is applied.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(1500);
/// Under a constant stream of events, flush at least this often.
const MAX_DELAY: Duration = Duration::from_secs(10);

/// Apply changed paths to the index: existing paths are (re)recorded, missing ones removed
/// with everything below them, folders that are new to the index are scanned whole (a folder
/// moved or copied in arrives as one event). Returns the paths whose index state changed.
pub fn apply_changes(store: &IndexStore, rules: &ScanRules, changed: &[String]) -> Vec<String> {
    let style = rules.style;
    let gen = store.current_generation();
    let mut upserts = Vec::new();
    let mut new_folders: Vec<String> = Vec::new();
    let mut touched = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in changed {
        let path = paths::normalize(raw);
        if !seen.insert(path.clone()) {
            continue;
        }
        let name = style.file_name(&path).to_string();
        if rules.is_excluded(&path, &name) {
            // moved into an excluded area (e.g. the Recycle Bin): drop it from the index too
            if store.remove(&path).unwrap_or(0) > 0 {
                touched.push(path);
            }
            continue;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(md) => {
                if rules.skip_hidden && record::is_hidden_attrs(record::file_attributes(&md)) {
                    if store.remove(&path).unwrap_or(0) > 0 {
                        touched.push(path);
                    }
                    continue;
                }
                let rec = record::FileRecord::from_metadata(&path, &md, style);
                if rec.kind == FileKind::Dir && !store.is_indexed(&path).unwrap_or(true) {
                    new_folders.push(path.clone());
                }
                upserts.push(rec);
                touched.push(path);
            }
            Err(_) => {
                let _ = store.remove(&path);
                touched.push(path);
            }
        }
    }
    let _ = store.upsert(&upserts, gen);
    // folders inside another new folder are covered by the outer scan
    let tops: Vec<String> = new_folders
        .iter()
        .filter(|f| !new_folders.iter().any(|o| o != *f && style.is_under(f, o)))
        .cloned()
        .collect();
    if !tops.is_empty() {
        let scanner = Scanner::new(rules.clone());
        let _ = scanner.scan(store, &tops, &no_progress, &Cancel::new());
    }
    touched
}

/// A running watcher. Dropping it (or calling [`Watcher::stop`]) stops watching.
pub struct Watcher {
    watcher: Option<notify::RecommendedWatcher>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

enum Msg {
    Paths(Vec<String>),
    Rescan,
}

impl Watcher {
    /// Watch `roots` recursively. Changes are applied to `store` after `debounce` of quiet,
    /// then `on_change(paths)` is called (emit `index-changed`). When the OS reports lost
    /// events, the affected roots are rescanned and reported as changed.
    pub fn start(
        store: Arc<IndexStore>,
        rules: ScanRules,
        roots: Vec<String>,
        debounce: Duration,
        on_change: impl Fn(Vec<String>) + Send + 'static,
    ) -> Result<Watcher> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(ev) => {
                    if ev.need_rescan() {
                        let _ = tx.send(Msg::Rescan);
                    }
                    let list: Vec<String> = ev
                        .paths
                        .iter()
                        .filter_map(|p| p.to_str().map(str::to_string))
                        .collect();
                    if !list.is_empty() {
                        let _ = tx.send(Msg::Paths(list));
                    }
                }
                Err(_) => {
                    let _ = tx.send(Msg::Rescan);
                }
            })?;
        for r in &roots {
            // a root that vanished is simply not watched (it shows as "missing")
            let _ = watcher.watch(Path::new(r), RecursiveMode::Recursive);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let worker = std::thread::Builder::new()
            .name("mf-watcher".into())
            .spawn(move || {
                while !stop2.load(Ordering::SeqCst) {
                    // wait for the first event
                    let first = match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(m) => m,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let mut pending = BTreeSet::new();
                    let mut rescan = false;
                    let mut absorb = |m: Msg| match m {
                        Msg::Paths(p) => pending.extend(p),
                        Msg::Rescan => rescan = true,
                    };
                    absorb(first);
                    let started = Instant::now();
                    // then until it has been quiet for `debounce`
                    loop {
                        match rx.recv_timeout(debounce) {
                            Ok(m) => {
                                absorb(m);
                                if started.elapsed() >= MAX_DELAY {
                                    break;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    let mut changed: Vec<String> = Vec::new();
                    if rescan {
                        let scanner = Scanner::new(rules.clone());
                        let _ = scanner.scan(&store, &roots, &no_progress, &Cancel::new());
                        changed.extend(roots.iter().cloned());
                    }
                    if !pending.is_empty() {
                        let list: Vec<String> = pending.into_iter().collect();
                        changed.extend(apply_changes(&store, &rules, &list));
                    }
                    if !changed.is_empty() {
                        on_change(changed);
                    }
                }
            })?;
        Ok(Watcher {
            watcher: Some(watcher),
            stop,
            worker: Some(worker),
        })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.watcher.take(); // drops the OS watch and the event sender
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::PathStyle;

    #[test]
    fn updater_scans_folders_moved_in_and_drops_deleted() {
        let base = tempfile::Builder::new()
            .prefix("mf-watch")
            .tempdir()
            .unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let store = IndexStore::open(&base.path().join("db/index.sqlite")).unwrap();
        let rules = ScanRules::open(PathStyle::native());
        let root_s = root.to_str().unwrap().to_string();
        Scanner::new(rules.clone())
            .scan(&store, &[root_s.clone()], &no_progress, &Cancel::new())
            .unwrap();

        // a folder moved in arrives as one event — its contents must be indexed too
        let outside = base.path().join("outside/pack");
        std::fs::create_dir_all(outside.join("inner")).unwrap();
        std::fs::write(outside.join("inner/f.txt"), "x").unwrap();
        let dest = root.join("pack");
        std::fs::rename(&outside, &dest).unwrap();
        let changed = apply_changes(&store, &rules, &[dest.to_str().unwrap().to_string()]);
        assert_eq!(changed.len(), 1);
        let inner = dest.join("inner/f.txt");
        assert!(store.record(inner.to_str().unwrap()).unwrap().is_some());

        // deleted → removed with its subtree
        std::fs::remove_dir_all(&dest).unwrap();
        apply_changes(&store, &rules, &[dest.to_str().unwrap().to_string()]);
        assert!(store.record(inner.to_str().unwrap()).unwrap().is_none());
        assert!(store.record(dest.to_str().unwrap()).unwrap().is_none());
    }

    #[test]
    fn live_watcher_reports_new_files() {
        let base = tempfile::Builder::new()
            .prefix("mf-watch-live")
            .tempdir()
            .unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(IndexStore::open(&base.path().join("db/index.sqlite")).unwrap());
        let rules = ScanRules::open(PathStyle::native());
        let root_s = root.to_str().unwrap().to_string();
        Scanner::new(rules.clone())
            .scan(&store, &[root_s.clone()], &no_progress, &Cancel::new())
            .unwrap();
        let (tx, rx) = mpsc::channel();
        let w = Watcher::start(
            store.clone(),
            rules,
            vec![root_s],
            Duration::from_millis(200),
            move |p| {
                let _ = tx.send(p);
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let f = root.join("new.txt");
        std::fs::write(&f, "hello").unwrap();
        let got = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("change reported");
        assert!(got.iter().any(|p| p.ends_with("new.txt")), "{got:?}");
        assert_eq!(store.record(f.to_str().unwrap()).unwrap().unwrap().size, 5);
        w.stop();
    }
}
