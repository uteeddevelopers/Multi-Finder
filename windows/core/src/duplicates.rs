//! Duplicate = identical content. Narrowed in three steps: size → quick hash (both ends) →
//! full hash. Full hashes are stored in the index and reused while the file is unchanged.
//! Port of `DuplicateFinder.swift`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::analyzer::{self, DupGroup, DuplicateOptions, ProjectRoots};
use crate::hasher;
use crate::index::IndexStore;
use crate::progress::{Cancel, Throttle};
use crate::record::{self, FileKind, FileRecord};
use crate::types::{DuplicateReport, DuplicateRequest, ProgressEvent};

/// Raw duplicate groups (biggest waste first). `under`: only groups with at least one file
/// below that folder (other members can be anywhere in the index).
/// Progress: phase "size" once, then "hash" with done/total = size buckets.
pub fn find_groups(
    store: &IndexStore,
    min_size: u64,
    under: Option<&str>,
    progress: &dyn Fn(&ProgressEvent),
    cancel: &Cancel,
) -> Result<Vec<DupGroup>> {
    let style = store.style();
    progress(&ProgressEvent::new("size", 0, 0, None));
    let size_groups = store.size_groups(min_size.max(1), under)?;
    let total = size_groups.len() as u64;
    let mut throttle = Throttle::new(Duration::from_millis(150));
    let mut result = Vec::new();

    for (i, group) in size_groups.iter().enumerate() {
        cancel.check()?;
        if throttle.ready() {
            progress(&ProgressEvent::new(
                "hash",
                i as u64,
                total,
                group.first().map(|f| f.name.clone()),
            ));
        }
        // the index may be stale (unwatched folders, edits while the app was closed):
        // compare with the disk now — gone or resized files drop out, files with only a new
        // mtime lose their stored hash and are hashed again
        let mut current: Vec<FileRecord> = Vec::new();
        for f in group {
            let Some(now) = record::record_for(&f.path, style) else {
                let _ = store.remove(&f.path);
                continue;
            };
            if now.kind != FileKind::File || now.cloud || now.size != f.size {
                let _ = store.upsert(std::slice::from_ref(&now), store.current_generation());
                continue;
            }
            if now.modified != f.modified {
                let _ = store.upsert(std::slice::from_ref(&now), store.current_generation()); // drops the stale hash
                current.push(now);
            } else {
                current.push(f.clone());
            }
        }
        if current.len() < 2 {
            continue;
        }

        // quick hash for every candidate (stored full hashes must land in the same bucket)
        let mut by_quick: HashMap<String, Vec<FileRecord>> = HashMap::new();
        for f in current {
            if let Ok(q) = hasher::quick_hash(Path::new(&f.path), f.size) {
                by_quick.entry(q).or_default().push(f);
            }
        }
        // full hash inside buckets; stored hashes are reused
        let mut by_full: HashMap<String, Vec<FileRecord>> = HashMap::new();
        for (_, files) in by_quick {
            if files.len() < 2 {
                continue;
            }
            for mut f in files {
                if f.hash.is_none() {
                    let h = match hasher::full_hash(Path::new(&f.path), cancel) {
                        Ok(h) => h,
                        Err(e) if crate::progress::is_cancelled(&e) => return Err(e),
                        Err(_) => continue, // unreadable now — never call it a duplicate
                    };
                    let _ = store.set_hash(&f.path, &h, f.modified, f.size);
                    f.hash = Some(h);
                }
                let h = f.hash.clone().unwrap_or_default();
                by_full.entry(h).or_default().push(f);
            }
        }
        for (hash, mut files) in by_full {
            if files.len() < 2 {
                continue;
            }
            if let Some(u) = under {
                if !files.iter().any(|f| style.is_under(&f.path, u)) {
                    continue;
                }
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            result.push(DupGroup {
                hash,
                size: files[0].size,
                files,
            });
        }
    }
    progress(&ProgressEvent::new("hash", total, total, None));
    result.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then_with(|| a.hash.cmp(&b.hash))
    });
    Ok(result)
}

/// `find_duplicates`: raw groups + analysis (ignore list, project hiding, copy-tree folding).
pub fn find_duplicates(
    store: &IndexStore,
    req: &DuplicateRequest,
    projects: &mut ProjectRoots,
    progress: &dyn Fn(&ProgressEvent),
    cancel: &Cancel,
) -> Result<DuplicateReport> {
    let groups = find_groups(
        store,
        req.min_size.max(1),
        req.scope.as_deref(),
        progress,
        cancel,
    )?;
    let ignore = store.ignore_list()?;
    let options = DuplicateOptions {
        hide_within_project: req.hide_within_project,
        fold_copy_trees: req.fold_copy_trees,
        ..DuplicateOptions::default()
    };
    let analysis = analyzer::analyze(&groups, &options, &ignore, projects, store.style());
    Ok(analyzer::to_report(&analysis, groups.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::PathStyle;
    use crate::progress::no_progress;
    use crate::rules::ScanRules;
    use crate::scanner::Scanner;

    fn setup() -> (tempfile::TempDir, String, IndexStore) {
        let base = tempfile::Builder::new().prefix("mf-dup").tempdir().unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let store = IndexStore::open(&base.path().join("db/index.sqlite")).unwrap();
        (base, root.to_str().unwrap().to_string(), store)
    }

    fn write(root: &str, rel: &str, content: &[u8]) -> String {
        let p = Path::new(root).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p.to_str().unwrap().to_string()
    }

    fn scan(store: &IndexStore, root: &str) {
        Scanner::new(ScanRules::open(PathStyle::native()))
            .scan(store, &[root.to_string()], &no_progress, &Cancel::new())
            .unwrap();
    }

    #[test]
    fn scan_and_duplicates() {
        let (_b, root, store) = setup();
        write(&root, "one.txt", b"same content here");
        write(&root, "deep/two.txt", b"same content here");
        write(&root, "three.txt", b"different content");
        write(&root, "four.txt", b"");
        scan(&store, &root);
        assert_eq!(store.totals().unwrap().0, 4);
        let groups = find_groups(&store, 1, None, &no_progress, &Cancel::new()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files.len(), 2);
        assert_eq!(groups[0].wasted(), 17);
        // hashes are stored: a second run gives the same result without rehashing
        assert_eq!(store.totals().unwrap().3, 2);
        assert_eq!(
            find_groups(&store, 1, None, &no_progress, &Cancel::new())
                .unwrap()
                .len(),
            1
        );
        let deep = format!("{root}{}deep", std::path::MAIN_SEPARATOR);
        assert_eq!(
            find_groups(&store, 1, Some(&deep), &no_progress, &Cancel::new())
                .unwrap()
                .len(),
            1
        );
        let nowhere = format!("{root}{}nowhere", std::path::MAIN_SEPARATOR);
        assert_eq!(
            find_groups(&store, 1, Some(&nowhere), &no_progress, &Cancel::new())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn rechecks_files_changed_since_scan() {
        let (_b, root, store) = setup();
        let mut data = vec![0x41u8; 600_000];
        let one = write(&root, "one.bin", &data);
        write(&root, "two.bin", &data);
        scan(&store, &root);
        assert_eq!(
            find_groups(&store, 1, None, &no_progress, &Cancel::new())
                .unwrap()
                .len(),
            1
        );
        data[300_000] = 0x42; // outside both 128 KiB samples
        std::fs::write(&one, &data).unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(120);
        filetime::set_file_mtime(&one, filetime::FileTime::from_system_time(later)).unwrap();
        assert_eq!(
            find_groups(&store, 1, None, &no_progress, &Cancel::new())
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_file(&one).unwrap();
        assert_eq!(
            find_groups(&store, 1, None, &no_progress, &Cancel::new())
                .unwrap()
                .len(),
            0
        );
        assert!(store.record(&one).unwrap().is_none());
    }

    #[test]
    fn report_and_cancel() {
        let (b, root, store) = setup();
        for d in ["L", "R"] {
            for (n, c) in [("a.txt", "aaaa"), ("b.txt", "bbbb"), ("c.txt", "cccc")] {
                write(&root, &format!("{d}/{n}"), c.as_bytes());
            }
        }
        scan(&store, &root);
        let req = DuplicateRequest {
            scope: None,
            min_size: 1,
            hide_within_project: true,
            fold_copy_trees: true,
        };
        let mut projects =
            ProjectRoots::new(vec![b.path().to_str().unwrap().to_string()], store.style());
        let report =
            find_duplicates(&store, &req, &mut projects, &no_progress, &Cancel::new()).unwrap();
        assert_eq!(report.raw_group_count, 3);
        assert_eq!(report.copy_trees.len(), 1);
        assert_eq!(report.copy_trees[0].file_count, 3);
        assert!(report.file_groups.is_empty());
        assert_eq!(report.reclaimable_bytes, 12);
        let c = Cancel::new();
        c.cancel();
        let err = find_duplicates(&store, &req, &mut projects, &no_progress, &c).unwrap_err();
        assert!(crate::progress::is_cancelled(&err));
    }
}
