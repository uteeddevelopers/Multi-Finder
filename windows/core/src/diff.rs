//! Compare two folders by content. Port of `FolderDiff.swift`.
//!
//! Only files that need it are hashed (same relative path and size, or a same-size
//! candidate on the other side); hashes are stored in the index. A file that cannot be read
//! is never "identical" and never "moved" — calling it identical would invite deleting the
//! only readable copy.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Result};

use crate::hasher;
use crate::index::IndexStore;
use crate::paths::PathStyle;
use crate::progress::{Cancel, Throttle};
use crate::record::FileRecord;
use crate::scanner::Scanner;
use crate::types::{DiffCounts, DiffEntry, DiffResult, DiffSide, DiffStatus, ProgressEvent};

/// `compare_folders`: rescan both folders (anything may have changed), then compare.
/// Progress: phase "scan" (0/2 → 2/2), then "hash".
pub fn compare_folders(
    store: &IndexStore,
    scanner: &Scanner,
    left: &str,
    right: &str,
    progress: &dyn Fn(&ProgressEvent),
    cancel: &Cancel,
) -> Result<DiffResult> {
    check_disjoint(store.style(), left, right)?;
    progress(&ProgressEvent::new("scan", 0, 2, Some(left.to_string())));
    let out = scanner.scan(
        store,
        &[left.to_string(), right.to_string()],
        &|_| {},
        cancel,
    )?;
    if out.cancelled {
        return Err(crate::progress::Cancelled.into());
    }
    if let Some(e) = out
        .roots
        .iter()
        .find_map(|r| r.error.as_ref().map(|e| (r.root.clone(), e.clone())))
    {
        bail!("cannot read {}: {}", e.0, e.1);
    }
    progress(&ProgressEvent::new("scan", 2, 2, None));
    diff_indexed(store, left, right, progress, cancel)
}

fn check_disjoint(style: PathStyle, left: &str, right: &str) -> Result<()> {
    if style.is_same_or_under(left, right) || style.is_same_or_under(right, left) {
        bail!("the two folders overlap");
    }
    Ok(())
}

struct Hashes<'a> {
    store: &'a IndexStore,
    cache: HashMap<String, Option<String>>,
    cancel: &'a Cancel,
}

impl Hashes<'_> {
    /// Content hash; None when unreadable.
    fn get(&mut self, f: &FileRecord) -> Result<Option<String>> {
        if let Some(h) = &f.hash {
            return Ok(Some(h.clone()));
        }
        if let Some(h) = self.cache.get(&f.path) {
            return Ok(h.clone());
        }
        let h = match hasher::full_hash(Path::new(&f.path), self.cancel) {
            Ok(h) => {
                let _ = self.store.set_hash(&f.path, &h, f.modified, f.size);
                Some(h)
            }
            Err(e) if crate::progress::is_cancelled(&e) => return Err(e),
            Err(_) => None,
        };
        self.cache.insert(f.path.clone(), h.clone());
        Ok(h)
    }
}

fn side(f: &FileRecord) -> DiffSide {
    DiffSide {
        path: f.path.clone(),
        size: f.size,
        modified: f.modified,
    }
}

/// Compare what the index knows about the two folders (no rescan).
pub fn diff_indexed(
    store: &IndexStore,
    left: &str,
    right: &str,
    progress: &dyn Fn(&ProgressEvent),
    cancel: &Cancel,
) -> Result<DiffResult> {
    let style = store.style();
    check_disjoint(style, left, right)?;
    // relative-path key (case-insensitive on Windows) → (relative path, record)
    let index_side = |root: &str| -> Result<BTreeMap<String, (String, FileRecord)>> {
        let mut m = BTreeMap::new();
        for f in store.files_under(root)? {
            if f.cloud {
                continue; // online-only: content unknown, never compared
            }
            if let Some(rel) = style.relative(&f.path, root) {
                let rel = rel.to_string();
                m.insert(style.key(&rel), (rel, f));
            }
        }
        Ok(m)
    };
    let l = index_side(left)?;
    let r = index_side(right)?;
    let keys: Vec<&String> = {
        let mut k: Vec<&String> = l.keys().chain(r.keys()).collect();
        k.sort();
        k.dedup();
        k
    };
    let total = keys.len() as u64;
    let mut hashes = Hashes {
        store,
        cache: HashMap::new(),
        cancel,
    };
    let mut throttle = Throttle::new(Duration::from_millis(150));
    let mut entries = Vec::new();
    let mut only_l: Vec<&(String, FileRecord)> = Vec::new();
    let mut only_r: Vec<&(String, FileRecord)> = Vec::new();

    for (i, key) in keys.iter().enumerate() {
        cancel.check()?;
        if throttle.ready() {
            progress(&ProgressEvent::new("hash", i as u64, total, None));
        }
        match (l.get(*key), r.get(*key)) {
            (Some(a), Some(b)) => {
                let same = a.1.size == b.1.size && {
                    let ha = hashes.get(&a.1)?;
                    ha.is_some() && ha == hashes.get(&b.1)?
                };
                let status = if same {
                    DiffStatus::Identical
                } else {
                    DiffStatus::Different
                };
                entries.push(DiffEntry {
                    id: format!("{}:{}", if same { "i" } else { "d" }, a.0),
                    rel_path: a.0.clone(),
                    name: style.file_name(&a.0).to_string(),
                    status,
                    left: Some(side(&a.1)),
                    right: Some(side(&b.1)),
                    other_rel_path: None,
                });
            }
            (Some(a), None) => only_l.push(a),
            (None, Some(b)) => only_r.push(b),
            (None, None) => {}
        }
    }

    // one-sided files: same size → same hash on the other side means moved/renamed
    let mut r_by_size: HashMap<u64, Vec<&(String, FileRecord)>> = HashMap::new();
    for b in &only_r {
        r_by_size.entry(b.1.size).or_default().push(b);
    }
    let mut claimed: HashSet<String> = HashSet::new();
    for a in only_l {
        cancel.check()?;
        let mut found: Option<&(String, FileRecord)> = None;
        if a.1.size > 0 {
            if let Some(cands) = r_by_size.get(&a.1.size) {
                if let Some(ha) = hashes.get(&a.1)? {
                    for c in cands {
                        if claimed.contains(&c.1.path) {
                            continue;
                        }
                        if hashes.get(&c.1)?.as_deref() == Some(ha.as_str()) {
                            found = Some(c);
                            break;
                        }
                    }
                }
            }
        }
        match found {
            Some(c) => {
                claimed.insert(c.1.path.clone());
                entries.push(DiffEntry {
                    id: format!("m:{}|{}", a.0, c.0),
                    rel_path: a.0.clone(),
                    name: style.file_name(&a.0).to_string(),
                    status: DiffStatus::Moved,
                    left: Some(side(&a.1)),
                    right: Some(side(&c.1)),
                    other_rel_path: Some(c.0.clone()),
                });
            }
            None => entries.push(DiffEntry {
                id: format!("l:{}", a.0),
                rel_path: a.0.clone(),
                name: style.file_name(&a.0).to_string(),
                status: DiffStatus::LeftOnly,
                left: Some(side(&a.1)),
                right: None,
                other_rel_path: None,
            }),
        }
    }
    for b in only_r {
        if claimed.contains(&b.1.path) {
            continue;
        }
        entries.push(DiffEntry {
            id: format!("r:{}", b.0),
            rel_path: b.0.clone(),
            name: style.file_name(&b.0).to_string(),
            status: DiffStatus::RightOnly,
            left: None,
            right: Some(side(&b.1)),
            other_rel_path: None,
        });
    }
    progress(&ProgressEvent::new("hash", total, total, None));
    entries.sort_by(|a, b| {
        style
            .key(&a.rel_path)
            .cmp(&style.key(&b.rel_path))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut counts = DiffCounts::default();
    for e in &entries {
        match e.status {
            DiffStatus::Identical => counts.identical += 1,
            DiffStatus::Different => counts.different += 1,
            DiffStatus::LeftOnly => counts.left_only += 1,
            DiffStatus::RightOnly => counts.right_only += 1,
            DiffStatus::Moved => counts.moved += 1,
        }
    }
    Ok(DiffResult {
        left: left.to_string(),
        right: right.to_string(),
        entries,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::no_progress;
    use crate::rules::ScanRules;

    fn setup() -> (tempfile::TempDir, String, IndexStore) {
        let base = tempfile::Builder::new()
            .prefix("mf-diff")
            .tempdir()
            .unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let store = IndexStore::open(&base.path().join("db/index.sqlite")).unwrap();
        (base, root.to_str().unwrap().to_string(), store)
    }

    fn write(root: &str, rel: &str, content: &str) -> String {
        let p = Path::new(root).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p.to_str().unwrap().to_string()
    }

    fn j(a: &str, b: &str) -> String {
        Path::new(a).join(b).to_str().unwrap().to_string()
    }

    #[test]
    fn statuses_and_moved_detection() {
        let (_b, root, store) = setup();
        write(&root, "L/same.txt", "same");
        write(&root, "R/same.txt", "same");
        write(&root, "L/diff.txt", "left");
        write(&root, "R/diff.txt", "right!");
        write(&root, "L/sz.txt", "abcd");
        write(&root, "R/sz.txt", "abce");
        write(&root, "L/photos/a.jpg", "photo-bytes");
        write(&root, "R/archive/a-renamed.jpg", "photo-bytes");
        write(&root, "L/only-left.txt", "l");
        write(&root, "R/only-right.txt", "r");
        let scanner = Scanner::new(ScanRules::open(store.style()));
        let r = compare_folders(
            &store,
            &scanner,
            &j(&root, "L"),
            &j(&root, "R"),
            &no_progress,
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(
            r.counts,
            DiffCounts {
                identical: 1,
                different: 2,
                left_only: 1,
                right_only: 1,
                moved: 1
            },
            "{:#?}",
            r.entries
        );
        let moved = r
            .entries
            .iter()
            .find(|e| e.status == DiffStatus::Moved)
            .unwrap();
        assert_eq!(moved.rel_path, j("photos", "a.jpg"));
        assert_eq!(
            moved.other_rel_path.as_deref(),
            Some(j("archive", "a-renamed.jpg").as_str())
        );
        assert!(moved.left.is_some() && moved.right.is_some());
        let ids: HashSet<_> = r.entries.iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids.len(), r.entries.len());
        assert!(compare_folders(
            &store,
            &scanner,
            &root,
            &j(&root, "L"),
            &no_progress,
            &Cancel::new()
        )
        .is_err());
    }

    #[test]
    fn tolerates_files_deleted_after_scan() {
        let (_b, root, store) = setup();
        write(&root, "L/x.txt", "same");
        write(&root, "R/x.txt", "same");
        let gone = write(&root, "L/gone.txt", "abc");
        write(&root, "R/other.txt", "abc");
        Scanner::new(ScanRules::open(store.style()))
            .scan(&store, &[root.clone()], &no_progress, &Cancel::new())
            .unwrap();
        std::fs::remove_file(gone).unwrap();
        let r = diff_indexed(
            &store,
            &j(&root, "L"),
            &j(&root, "R"),
            &no_progress,
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(r.counts.identical, 1);
        assert_eq!(r.counts.moved, 0);
        assert_eq!(r.counts.left_only, 1);
        assert_eq!(r.counts.right_only, 1);
    }
}
