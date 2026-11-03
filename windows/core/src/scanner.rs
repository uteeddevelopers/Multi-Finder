//! Walks folders and fills the index. Reads only, never changes anything. Port of `Scanner.swift`.

use std::sync::Mutex;

use anyhow::Result;
use walkdir::WalkDir;

use crate::index::IndexStore;
use crate::paths::{self, PathStyle};
use crate::progress::{Cancel, Throttle};
use crate::record::{self, FileKind, FileRecord};
use crate::rules::ScanRules;
use crate::types::ScanProgressEvent;

/// Scans run one at a time: if overlapping roots were scanned concurrently, the first scan
/// could re-stamp rows with an old generation and the second one's `finish_scan` would then
/// delete them as "gone".
static SCAN_LOCK: Mutex<()> = Mutex::new(());

const BATCH: usize = 1000;

/// Result for one root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootOutcome {
    pub root: String,
    /// entries written (root included)
    pub scanned: u64,
    /// set when the root itself could not be read — its old rows were kept
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    pub roots: Vec<RootOutcome>,
    /// stopped early; nothing was deleted from the index
    pub cancelled: bool,
}

impl ScanOutcome {
    pub fn scanned(&self) -> u64 {
        self.roots.iter().map(|r| r.scanned).sum()
    }
}

pub struct Scanner {
    pub rules: ScanRules,
}

impl Scanner {
    pub fn new(rules: ScanRules) -> Scanner {
        Scanner { rules }
    }

    fn style(&self) -> PathStyle {
        self.rules.style
    }

    /// Scan `roots` into `store`. Afterwards rows under each readable root that were not seen
    /// are removed. A root that cannot be read (missing, permission, offline) keeps its rows,
    /// and so do sub-folders that could not be read. `progress` fires about every 1000
    /// entries (and at the end of each root).
    pub fn scan(
        &self,
        store: &IndexStore,
        roots: &[String],
        progress: &dyn Fn(&ScanProgressEvent),
        cancel: &Cancel,
    ) -> Result<ScanOutcome> {
        let _guard = SCAN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let style = self.style();
        let gen = store.begin_scan();
        let mut outcome = ScanOutcome::default();
        let mut finished_roots: Vec<String> = Vec::new();
        let mut keep: Vec<String> = Vec::new();

        for raw in roots {
            let root = style.trim(&paths::storage_form(&paths::strip_verbatim(raw)));
            let mut count = 0u64;
            let mut throttle = Throttle::new(std::time::Duration::from_millis(250));

            // excluded roots: nothing is recorded and old rows go away (as in the macOS app)
            if self.rules.is_excluded(&root, style.file_name(&root)) {
                finished_roots.push(root.clone());
                outcome.roots.push(RootOutcome {
                    root,
                    scanned: 0,
                    error: None,
                });
                continue;
            }
            let md = match std::fs::metadata(&root) {
                Ok(m) => m,
                Err(e) => {
                    outcome.roots.push(RootOutcome {
                        root,
                        scanned: 0,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            };
            // the root is recorded as a folder even when it is reached through a link
            let mut root_rec = FileRecord::from_metadata(&root, &md, style);
            if md.is_dir() {
                root_rec.kind = FileKind::Dir;
                root_rec.size = 0;
                root_rec.ext.clear();
            }
            if md.is_dir() {
                if let Err(e) = std::fs::read_dir(&root) {
                    outcome.roots.push(RootOutcome {
                        root,
                        scanned: 0,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            }
            store.upsert(std::slice::from_ref(&root_rec), gen)?;
            count += 1;
            if !md.is_dir() {
                finished_roots.push(root.clone());
                outcome.roots.push(RootOutcome {
                    root,
                    scanned: count,
                    error: None,
                });
                continue;
            }

            let mut batch: Vec<FileRecord> = Vec::with_capacity(BATCH);
            let rules = &self.rules;
            let walker = WalkDir::new(&root)
                .min_depth(1)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    let (Some(path), Some(name)) = (e.path().to_str(), e.file_name().to_str())
                    else {
                        return false; // not valid Unicode — cannot be stored or handed to the UI
                    };
                    if rules.is_excluded(path, name) {
                        return false;
                    }
                    // Windows HIDDEN/SYSTEM attributes (free there: the walk already has them)
                    if cfg!(windows) && rules.skip_hidden {
                        if let Ok(m) = e.metadata() {
                            if record::is_hidden_attrs(record::file_attributes(&m)) {
                                return false;
                            }
                        }
                    }
                    true
                });
            for item in walker {
                if cancel.is_cancelled() {
                    break;
                }
                let entry = match item {
                    Ok(e) => e,
                    Err(err) => {
                        // a folder we could not list keeps its old rows
                        if let Some(p) = err.path().and_then(|p| p.to_str()) {
                            keep.push(p.to_string());
                        }
                        continue;
                    }
                };
                let Some(path) = entry.path().to_str() else {
                    continue;
                };
                let Ok(m) = entry.metadata() else {
                    keep.push(path.to_string());
                    continue;
                };
                let rec = FileRecord::from_metadata(path, &m, style);
                batch.push(rec);
                count += 1;
                if batch.len() >= BATCH {
                    store.upsert(&batch, gen)?;
                    batch.clear();
                    if throttle.ready() {
                        progress(&ScanProgressEvent {
                            root: root.clone(),
                            scanned: count,
                            current: path.to_string(),
                        });
                    }
                }
            }
            store.upsert(&batch, gen)?;
            progress(&ScanProgressEvent {
                root: root.clone(),
                scanned: count,
                current: String::new(),
            });
            if cancel.is_cancelled() {
                outcome.roots.push(RootOutcome {
                    root,
                    scanned: count,
                    error: None,
                });
                outcome.cancelled = true;
                break;
            }
            finished_roots.push(root.clone());
            outcome.roots.push(RootOutcome {
                root,
                scanned: count,
                error: None,
            });
        }

        if !outcome.cancelled {
            store.finish_scan(&finished_roots, gen, &keep)?;
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::no_progress;

    pub(crate) fn setup() -> (tempfile::TempDir, String, IndexStore) {
        let base = tempfile::Builder::new()
            .prefix("mf-scan")
            .tempdir()
            .unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let store = IndexStore::open(&base.path().join("db").join("index.sqlite")).unwrap();
        (base, root.to_str().unwrap().to_string(), store)
    }

    fn write(root: &str, rel: &str, content: &str) -> String {
        let p = std::path::Path::new(root).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p.to_str().unwrap().to_string()
    }

    fn open_rules() -> ScanRules {
        ScanRules::open(PathStyle::native())
    }

    #[test]
    fn scan_records_everything_and_drops_deleted_files() {
        let (_b, root, store) = setup();
        let gone = write(&root, "gone.txt", "x");
        write(&root, "stay.txt", "y");
        write(&root, "deep/two.txt", "z");
        let s = Scanner::new(open_rules());
        let out = s
            .scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        assert_eq!(out.scanned(), 5); // root, gone, stay, deep, two
        assert!(store.record(&gone).unwrap().is_some());
        assert_eq!(store.record(&root).unwrap().unwrap().kind, FileKind::Dir);
        std::fs::remove_file(&gone).unwrap();
        s.scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        assert!(store.record(&gone).unwrap().is_none());
        assert!(store.record(&format!("{root}/stay.txt")).unwrap().is_some());
    }

    #[test]
    fn unreadable_root_keeps_its_rows() {
        let (_b, root, store) = setup();
        let f = write(&root, "a.txt", "x");
        let s = Scanner::new(open_rules());
        s.scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        // the folder disappears (unplugged drive): rows stay as a cache
        let moved = format!("{root}-away");
        std::fs::rename(&root, &moved).unwrap();
        let out = s
            .scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        assert!(out.roots[0].error.is_some());
        assert!(store.record(&f).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_subfolder_keeps_its_rows() {
        use std::os::unix::fs::PermissionsExt;
        let (_b, root, store) = setup();
        let inner = write(&root, "locked/inner.txt", "x");
        let s = Scanner::new(open_rules());
        s.scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        let locked = format!("{root}/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(&locked).is_ok(); // root ignores permissions
        s.scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            store.record(&inner).unwrap().is_some(),
            "readable={readable}"
        );
    }

    #[test]
    fn rules_exclude_names_and_hidden() {
        let (_b, root, store) = setup();
        write(&root, "app/node_modules/x/y.js", "1");
        write(&root, ".hidden/a.txt", "1");
        write(&root, "keep.txt", "1");
        let rules = ScanRules::new::<&str, &str>(true, &[], &["node_modules"], PathStyle::native());
        let s = Scanner::new(rules);
        s.scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        assert!(store.record(&format!("{root}/keep.txt")).unwrap().is_some());
        assert!(store.record(&format!("{root}/app")).unwrap().is_some());
        assert!(store
            .record(&format!("{root}/app/node_modules"))
            .unwrap()
            .is_none());
        assert!(store
            .record(&format!("{root}/.hidden/a.txt"))
            .unwrap()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_recorded_not_followed() {
        let (_b, root, store) = setup();
        write(&root, "real/a.txt", "1");
        std::os::unix::fs::symlink(format!("{root}/real"), format!("{root}/link")).unwrap();
        Scanner::new(open_rules())
            .scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        assert_eq!(
            store.record(&format!("{root}/link")).unwrap().unwrap().kind,
            FileKind::Link
        );
        assert!(store
            .record(&format!("{root}/link/a.txt"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn cancelled_scan_deletes_nothing() {
        let (_b, root, store) = setup();
        let f = write(&root, "a.txt", "x");
        let s = Scanner::new(open_rules());
        s.scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        std::fs::remove_file(&f).unwrap();
        let c = Cancel::new();
        c.cancel();
        let out = s.scan(&store, &[root.clone()], &no_progress, &c).unwrap();
        assert!(out.cancelled);
        assert!(store.record(&f).unwrap().is_some());
    }

    #[test]
    fn korean_names_roundtrip() {
        use unicode_normalization::UnicodeNormalization;
        let (_b, root, store) = setup();
        let nfd: String = "한글폴더".nfd().collect();
        write(&root, &format!("{nfd}/자료.pdf"), "x");
        Scanner::new(open_rules())
            .scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        // stored exactly as on disk (NTFS/ext4 are normalization-sensitive) …
        let dir = format!("{root}/{}", paths::storage_form(&nfd));
        assert_eq!(store.children(&dir).unwrap().len(), 1);
        assert_eq!(store.summary(&dir).unwrap().0, 1);
        // … and found by the composed spelling the user types
        let found = store
            .search(
                "한글",
                &[root.clone()],
                &crate::types::Filter::default(),
                crate::types::Sort::default(),
                10,
            )
            .unwrap();
        assert_eq!(found.results.len(), 1);
    }
}
