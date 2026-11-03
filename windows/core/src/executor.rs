//! The only code that changes files. Port of `Executor.swift`. Rules:
//! 1. nothing is deleted — items go to the Recycle Bin;
//! 2. nothing is overwritten;
//! 3. protected paths are refused;
//! 4. every change is logged; one user action = one batch = one undo.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::fsops;
use crate::index::{Identity, IndexStore, OpKind, Operation};
use crate::paths::PathStyle;
use crate::record::{self, FileKind};
use crate::rules::ProtectedPaths;
use crate::types::{OpFailure, OpResult, UndoKind, UndoResult, UndoState};

/// Files that may be thrown away when removing a folder we created ourselves.
const JUNK_FILES: &[&str] = &[".DS_Store", "Thumbs.db"];

/// Undo result plus what the caller should do next.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoOutcome {
    pub result: UndoResult,
    /// folders that came back from the Recycle Bin: rescan them (their contents were not indexed)
    pub rescan: Vec<String>,
    /// original paths that were restored
    pub restored_paths: Vec<String>,
    /// details for logs (restore errors etc.)
    pub errors: Vec<String>,
}

enum Outcome {
    Done(#[allow(dead_code)] String),
    Skipped(String),
    Failed(String),
}

pub struct Executor {
    store: Arc<IndexStore>,
    protected: ProtectedPaths,
    style: PathStyle,
    /// one action at a time (batch ids, undo)
    lock: Mutex<()>,
}

impl Executor {
    pub fn new(store: Arc<IndexStore>, protected: ProtectedPaths) -> Executor {
        let style = store.style();
        Executor {
            store,
            protected,
            style,
            lock: Mutex::new(()),
        }
    }

    pub fn store(&self) -> &Arc<IndexStore> {
        &self.store
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ───────────── move ─────────────

    /// Move `paths` into `destination` (names kept). One batch.
    pub fn move_items(&self, paths: &[String], destination: &str) -> Result<OpResult> {
        let _g = self.guard();
        let batch = self.store.next_batch_id()?;
        let mut res = OpResult::default();
        let dest = self.style.trim(destination);
        for p in drop_nested(paths, self.style) {
            self.record_outcome(&mut res, &p, self.move_one(&p, &dest, batch, false));
        }
        self.finish_batch(&mut res, batch)?;
        Ok(res)
    }

    /// Compare's "move to the other side": each path keeps its location relative to
    /// `from_root` under `to_root`; missing folders are created (and logged, so undo removes
    /// them again when empty). One batch.
    pub fn move_across(
        &self,
        paths: &[String],
        from_root: &str,
        to_root: &str,
    ) -> Result<OpResult> {
        let _g = self.guard();
        let batch = self.store.next_batch_id()?;
        let mut res = OpResult::default();
        let from = self.style.trim(from_root);
        let to = self.style.trim(to_root);
        for p in drop_nested(paths, self.style) {
            let Some(rel) = self.style.relative(&p, &from) else {
                res.failed.push(OpFailure {
                    path: p.clone(),
                    reason: "not inside the source folder".into(),
                });
                continue;
            };
            let rel_parent = self.style.parent(rel).unwrap_or("");
            let dest_dir = if rel_parent.is_empty() {
                to.clone()
            } else {
                self.style.join(&to, rel_parent)
            };
            self.record_outcome(&mut res, &p, self.move_one(&p, &dest_dir, batch, true));
        }
        self.finish_batch(&mut res, batch)?;
        Ok(res)
    }

    fn record_outcome(&self, res: &mut OpResult, path: &str, o: Outcome) {
        match o {
            Outcome::Done(_) => res.done += 1,
            Outcome::Skipped(r) => res.skipped.push(OpFailure {
                path: path.to_string(),
                reason: r,
            }),
            Outcome::Failed(r) => res.failed.push(OpFailure {
                path: path.to_string(),
                reason: r,
            }),
        }
    }

    /// Set the batch id; when nothing was done, undo the folders this batch created.
    fn finish_batch(&self, res: &mut OpResult, batch: i64) -> Result<()> {
        if res.done > 0 {
            res.batch = batch;
            return Ok(());
        }
        res.batch = 0;
        let ops = self.store.last_undoable_batch(0)?;
        for op in ops
            .iter()
            .filter(|o| o.batch == Some(batch) && o.kind == OpKind::Mkdir)
        {
            self.undo_mkdir(op);
            self.store.mark_undone(op.id)?;
        }
        Ok(())
    }

    fn move_one(&self, src: &str, dest_dir: &str, batch: i64, create_dirs: bool) -> Outcome {
        let style = self.style;
        if std::fs::symlink_metadata(src).is_err() {
            return Outcome::Failed("not found".into());
        }
        if self.protected.is_protected(src) {
            return Outcome::Failed("protected location".into());
        }
        if style.is_same_or_under(dest_dir, src) {
            return Outcome::Failed("cannot move a folder into itself".into());
        }
        if self.protected.is_in_protected_area(dest_dir) {
            return Outcome::Failed("destination is a protected location".into());
        }
        if create_dirs && std::fs::symlink_metadata(dest_dir).is_err() {
            // log the topmost folder we create: undo removes it if everything below is empty
            let mut top = dest_dir.to_string();
            while let Some(parent) = style.parent(&top) {
                if std::fs::symlink_metadata(parent).is_ok() {
                    break;
                }
                top = parent.to_string();
            }
            if let Err(e) = std::fs::create_dir_all(dest_dir) {
                return Outcome::Failed(format!("cannot create folder: {e}"));
            }
            let ident = Identity::of_path(&top).unwrap_or_default();
            if let Err(e) = self
                .store
                .log_op(OpKind::Mkdir, &top, "", Some(batch), &ident)
            {
                return Outcome::Failed(format!("cannot record operation: {e}"));
            }
        }
        match std::fs::metadata(dest_dir) {
            Ok(m) if m.is_dir() => {}
            _ => return Outcome::Failed("destination is not a folder".into()),
        }
        let name = style.file_name(src);
        let dest = style.join(dest_dir, name);
        if std::fs::symlink_metadata(&dest).is_ok() {
            return Outcome::Skipped("destination exists".into());
        }
        if let Err(e) = fsops::move_path(Path::new(src), Path::new(&dest)) {
            return if e.kind() == std::io::ErrorKind::AlreadyExists {
                Outcome::Skipped("destination exists".into())
            } else {
                Outcome::Failed(e.to_string())
            };
        }
        let ident = Identity::of_path(&dest).unwrap_or_default();
        if let Err(e) = self
            .store
            .log_op(OpKind::Move, src, &dest, Some(batch), &ident)
        {
            // an unlogged move could not be undone: put it back and report failure
            let _ = fsops::move_path(Path::new(&dest), Path::new(src));
            return Outcome::Failed(format!("cannot record operation: {e}"));
        }
        let _ = self.store.rename(src, &dest);
        Outcome::Done(dest)
    }

    // ───────────── trash ─────────────

    /// Move `paths` to the Recycle Bin. One batch.
    pub fn trash_items(&self, paths: &[String]) -> Result<OpResult> {
        let _g = self.guard();
        let batch = self.store.next_batch_id()?;
        let mut res = OpResult::default();
        let list = drop_nested(paths, self.style);
        self.trash_batch(&list, batch, &mut res);
        res.batch = if res.done > 0 { batch } else { 0 };
        Ok(res)
    }

    /// Like `trash_items`, but at least one unchanged copy of every content group survives:
    /// a file is skipped when no other unchanged copy of its hash would remain, or when it
    /// changed since it was hashed (then "identical" is no longer certain).
    pub fn trash_duplicates(&self, paths: &[String]) -> Result<OpResult> {
        let _g = self.guard();
        let batch = self.store.next_batch_id()?;
        let mut res = OpResult::default();
        let style = self.style;
        let requested = drop_nested(paths, style);
        let req_keys: HashSet<String> = requested.iter().map(|p| style.key(p)).collect();
        let mut by_hash: HashMap<String, Vec<record::FileRecord>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for p in &requested {
            match self.store.record(p)? {
                Some(r) if r.kind == FileKind::File && r.hash.is_some() => {
                    let h = r.hash.clone().unwrap_or_default();
                    if !by_hash.contains_key(&h) {
                        order.push(h.clone());
                    }
                    by_hash.entry(h).or_default().push(r);
                }
                Some(_) | None => res.skipped.push(OpFailure {
                    path: p.clone(),
                    reason: "not a verified duplicate (search again)".into(),
                }),
            }
        }
        let mut allowed: Vec<String> = Vec::new();
        for h in order {
            let chosen = &by_hash[&h];
            let keepers = self
                .store
                .files_with_hash(&h)?
                .into_iter()
                .filter(|f| {
                    !req_keys.contains(&style.key(&f.path)) && record::is_unchanged(f, style)
                })
                .count();
            let mut trashing: Vec<&record::FileRecord> = chosen.iter().collect();
            if keepers == 0 {
                // every copy was chosen: keep the first unchanged one; if none is unchanged, touch nothing
                match chosen.iter().find(|f| record::is_unchanged(f, style)) {
                    Some(keep) => {
                        res.skipped.push(OpFailure {
                            path: keep.path.clone(),
                            reason: "last copy of this content is kept".into(),
                        });
                        trashing.retain(|f| f.path != keep.path);
                    }
                    None => {
                        for f in chosen {
                            res.skipped.push(OpFailure {
                                path: f.path.clone(),
                                reason: "changed since it was listed".into(),
                            });
                        }
                        continue;
                    }
                }
            }
            for f in trashing {
                if record::is_unchanged(f, style) {
                    allowed.push(f.path.clone());
                } else {
                    res.skipped.push(OpFailure {
                        path: f.path.clone(),
                        reason: "changed since it was listed".into(),
                    });
                }
            }
        }
        self.trash_batch(&allowed, batch, &mut res);
        res.batch = if res.done > 0 { batch } else { 0 };
        Ok(res)
    }

    fn trash_batch(&self, paths: &[String], batch: i64, res: &mut OpResult) {
        let mut valid: Vec<(String, Identity)> = Vec::new();
        for p in paths {
            let Ok(md) = std::fs::symlink_metadata(p) else {
                res.failed.push(OpFailure {
                    path: p.clone(),
                    reason: "not found".into(),
                });
                continue;
            };
            if self.protected.is_protected(p) {
                res.failed.push(OpFailure {
                    path: p.clone(),
                    reason: "protected location".into(),
                });
                continue;
            }
            if !fsops::has_recycle_bin(p) {
                res.failed.push(OpFailure {
                    path: p.clone(),
                    reason: "this drive has no Recycle Bin (network or removable drive)".into(),
                });
                continue;
            }
            valid.push((p.clone(), Identity::of(&md)));
        }
        for chunk in valid.chunks(64) {
            let list: Vec<&str> = chunk.iter().map(|(p, _)| p.as_str()).collect();
            let bulk = catch_unwind(AssertUnwindSafe(|| trash::delete_all(&list)));
            let bulk_ok = matches!(bulk, Ok(Ok(())));
            for (p, ident) in chunk {
                let mut gone = std::fs::symlink_metadata(p).is_err();
                let mut error = None;
                if !gone && !bulk_ok {
                    match catch_unwind(AssertUnwindSafe(|| trash::delete(p))) {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => error = Some(e.to_string()),
                        Err(_) => error = Some("Recycle Bin call failed".into()),
                    }
                    gone = std::fs::symlink_metadata(p).is_err();
                }
                if gone {
                    // logged after the fact; the Recycle Bin location is found again on undo
                    let _ = self.store.log_op(OpKind::Trash, p, "", Some(batch), ident);
                    let _ = self.store.remove(p);
                    res.done += 1;
                } else {
                    res.failed.push(OpFailure {
                        path: p.clone(),
                        reason: error.unwrap_or_else(|| "could not move to the Recycle Bin".into()),
                    });
                }
            }
        }
    }

    // ───────────── undo ─────────────

    /// What the next undo would do (only batches after `session_start`).
    pub fn undo_state(&self, session_start: i64) -> Result<UndoState> {
        let ops = self.store.last_undoable_batch(session_start)?;
        let count = ops.iter().filter(|o| o.kind != OpKind::Mkdir).count() as u64;
        if count == 0 {
            return Ok(UndoState {
                can_undo: false,
                kind: None,
                count: None,
            });
        }
        let kind = if ops.iter().any(|o| o.kind == OpKind::Trash) {
            UndoKind::Trash
        } else {
            UndoKind::Move
        };
        Ok(UndoState {
            can_undo: true,
            kind: Some(kind),
            count: Some(count),
        })
    }

    /// Undo the newest batch done after `session_start` (the max op id when the app started).
    /// Items are verified first and nothing is overwritten. A batch that was already put back
    /// outside the app is skipped silently and the one before it is undone instead.
    pub fn undo(&self, session_start: i64) -> Result<UndoOutcome> {
        let _g = self.guard();
        let mut trash_list: Option<std::result::Result<Vec<TrashEntry>, String>> = None;
        loop {
            let ops = self.store.last_undoable_batch(session_start)?;
            if ops.is_empty() {
                return Ok(UndoOutcome::default());
            }
            let mut out = UndoOutcome::default();
            for op in &ops {
                match op.kind {
                    OpKind::Mkdir => {
                        self.undo_mkdir(op);
                        self.store.mark_undone(op.id)?;
                    }
                    OpKind::Move => self.undo_move(op, &mut out)?,
                    OpKind::Trash => match trash_list.get_or_insert_with(list_trash) {
                        Ok(items) => self.undo_trash(op, items, &mut out)?,
                        Err(e) => {
                            // could not read the Recycle Bin: keep the record for another try
                            out.result.blocked += 1;
                            out.errors.push(e.clone());
                        }
                    },
                }
            }
            let r = &out.result;
            if r.restored + r.missing + r.blocked > 0 {
                return Ok(out);
            }
        }
    }

    fn exists(p: &str) -> bool {
        std::fs::symlink_metadata(p).is_ok()
    }

    fn undo_move(&self, op: &Operation, out: &mut UndoOutcome) -> Result<()> {
        let now = Identity::of_path(&op.dst);
        let same = now.as_ref().is_some_and(|n| op.ident.matches(n));
        let source_exists = Self::exists(&op.src);
        if !same {
            // gone or replaced by something else: this record is done
            self.store.mark_undone(op.id)?;
            if !source_exists {
                out.result.missing += 1;
            }
            return Ok(());
        }
        if source_exists {
            out.result.blocked += 1; // keep the record: free the place and undo again
            return Ok(());
        }
        if let Some(parent) = self.style.parent(&op.src) {
            let _ = std::fs::create_dir_all(parent);
        }
        match fsops::move_path(Path::new(&op.dst), Path::new(&op.src)) {
            Ok(()) => {
                self.store.mark_undone(op.id)?;
                let _ = self.store.rename(&op.dst, &op.src);
                out.result.restored += 1;
                out.restored_paths.push(op.src.clone());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => out.result.blocked += 1,
            Err(e) => {
                out.result.blocked += 1;
                out.errors.push(format!("{}: {e}", op.src));
            }
        }
        Ok(())
    }

    fn undo_trash(
        &self,
        op: &Operation,
        items: &[TrashEntry],
        out: &mut UndoOutcome,
    ) -> Result<()> {
        let source_exists = Self::exists(&op.src);
        if !CAN_RESTORE_FROM_TRASH {
            // macOS: the Trash cannot be read here — report it and move on
            self.store.mark_undone(op.id)?;
            out.result.blocked += 1;
            out.errors.push(format!(
                "{}: restoring from the Trash is not supported on this platform",
                op.src
            ));
            return Ok(());
        }
        let Some(entry) = find_trashed(items, op, self.style) else {
            // not in the Recycle Bin any more (emptied, or put back by the user)
            self.store.mark_undone(op.id)?;
            if !source_exists {
                out.result.missing += 1;
            }
            return Ok(());
        };
        if source_exists {
            out.result.blocked += 1;
            return Ok(());
        }
        if let Some(parent) = self.style.parent(&op.src) {
            let _ = std::fs::create_dir_all(parent);
        }
        match restore(entry) {
            Ok(()) if Self::exists(&op.src) => {
                self.store.mark_undone(op.id)?;
                if let Some(rec) = record::record_for(&op.src, self.style) {
                    let _ = self
                        .store
                        .upsert(std::slice::from_ref(&rec), self.store.current_generation());
                    if rec.kind == FileKind::Dir {
                        out.rescan.push(op.src.clone());
                    }
                }
                out.result.restored += 1;
                out.restored_paths.push(op.src.clone());
            }
            Ok(()) => {
                out.result.blocked += 1;
                out.errors.push(format!(
                    "{}: restore reported success but the item is not back",
                    op.src
                ));
            }
            Err(e) => {
                out.result.blocked += 1;
                out.errors.push(format!("{}: {e}", op.src));
            }
        }
        Ok(())
    }

    /// Remove a folder created by a move — only the one we made, only if everything in it
    /// is empty folders (or junk like Thumbs.db).
    fn undo_mkdir(&self, op: &Operation) {
        let Some(now) = Identity::of_path(&op.src) else {
            return;
        };
        if op.ident.kind.is_some() && !op.ident.matches(&now) {
            return;
        }
        remove_if_empty_tree(Path::new(&op.src));
    }
}

/// Drop paths that lie inside another listed path (the outer item carries them along) and
/// duplicates, keeping the original order.
fn drop_nested(paths: &[String], style: PathStyle) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        let p = style.trim(p);
        if out.iter().any(|o| style.eq(o, &p)) {
            continue;
        }
        if paths.iter().any(|o| {
            let o = style.trim(o);
            !style.eq(&o, &p) && style.is_under(&p, &o)
        }) {
            continue;
        }
        out.push(p);
    }
    out
}

/// Remove `dir` when it contains nothing but empty folders and junk files.
pub(crate) fn remove_if_empty_tree(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut junk = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { return false };
        if ft.is_dir() && !ft.is_symlink() {
            if !remove_if_empty_tree(&e.path()) {
                return false;
            }
        } else if JUNK_FILES.iter().any(|j| j.eq_ignore_ascii_case(&name)) {
            junk.push(e.path());
        } else {
            return false;
        }
    }
    for j in junk {
        let _ = std::fs::remove_file(j);
    }
    std::fs::remove_dir(dir).is_ok()
}

// ───────────── Recycle Bin listing / restore ─────────────

/// `trash::os_limited` (list/restore) exists on Windows and freedesktop systems only.
const CAN_RESTORE_FROM_TRASH: bool = cfg!(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
));

#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
type TrashEntry = trash::TrashItem;

#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
#[derive(Clone, Debug)]
pub struct TrashEntry;

#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn list_trash() -> std::result::Result<Vec<TrashEntry>, String> {
    match catch_unwind(trash::os_limited::list) {
        Ok(Ok(items)) => Ok(items),
        Ok(Err(e)) => Err(format!("cannot list the Recycle Bin: {e}")),
        Err(_) => Err("cannot list the Recycle Bin".into()),
    }
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
fn list_trash() -> std::result::Result<Vec<TrashEntry>, String> {
    Ok(Vec::new())
}

/// Seconds of slack between the Recycle Bin's deletion time and our log time.
const TRASH_TIME_SLACK_S: i64 = 5;

/// The Recycle Bin entry for a trash op: same original path, deleted no later than we logged
/// it, newest such deletion, and — when its size/time can be read — the same item.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn find_trashed<'a>(
    items: &'a [TrashEntry],
    op: &Operation,
    style: PathStyle,
) -> Option<&'a TrashEntry> {
    let alt = canonical_alias(&op.src, style);
    let mut cands: Vec<&TrashEntry> = items
        .iter()
        .filter(|it| {
            let orig = it.original_path();
            let Some(o) = orig.to_str() else { return false };
            let o = crate::paths::strip_verbatim(o);
            style.eq(&o, &op.src) || alt.as_deref().is_some_and(|a| style.eq(&o, a))
        })
        .filter(|it| it.time_deleted < 0 || it.time_deleted <= op.ts / 1000 + TRASH_TIME_SLACK_S)
        .collect();
    cands.sort_by_key(|c| std::cmp::Reverse(c.time_deleted));
    cands
        .into_iter()
        .find(|it| trashed_identity_matches(it, op))
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
fn find_trashed<'a>(
    _items: &'a [TrashEntry],
    _op: &Operation,
    _style: PathStyle,
) -> Option<&'a TrashEntry> {
    None
}

/// The trash crate records the canonical parent; our log has the index path.
fn canonical_alias(src: &str, style: PathStyle) -> Option<String> {
    let parent = style.parent(src)?;
    let name = style.file_name(src);
    let c = dunce::canonicalize(parent).ok()?;
    Some(style.join(&crate::paths::strip_verbatim(c.to_str()?), name))
}

/// Where the trashed item physically is, when known (Windows: the `$R…` file the id points
/// to; freedesktop: `Trash/files/<name>` next to the `info` file).
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn trashed_location(it: &TrashEntry) -> Option<std::path::PathBuf> {
    let id = Path::new(&it.id);
    if cfg!(windows) {
        return Some(id.to_path_buf());
    }
    let stem = id.file_stem()?;
    let info_dir = id.parent()?;
    if info_dir.file_name()? != "info" || id.extension()? != "trashinfo" {
        return None;
    }
    Some(info_dir.parent()?.join("files").join(stem))
}

#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn trashed_identity_matches(it: &TrashEntry, op: &Operation) -> bool {
    match trashed_location(it).and_then(|p| std::fs::symlink_metadata(p).ok()) {
        Some(md) => op.ident.matches(&Identity::of(&md)),
        None => match catch_unwind(AssertUnwindSafe(|| trash::os_limited::metadata(it))) {
            // the shell can at least tell a file's size
            Ok(Ok(m)) => match (m.size.size(), op.ident.kind) {
                (Some(bytes), Some(FileKind::File)) => bytes == op.ident.size,
                _ => true,
            },
            _ => true, // cannot check — path + deletion time already match
        },
    }
}

#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn restore(entry: &TrashEntry) -> Result<()> {
    let item = entry.clone();
    match catch_unwind(AssertUnwindSafe(move || {
        trash::os_limited::restore_all([item])
    })) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow::anyhow!("restore failed: {e}")),
        Err(_) => Err(anyhow::anyhow!("restore failed")),
    }
}

/// macOS: putting items back from the Trash is not supported here — reported as blocked.
#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
fn restore(_entry: &TrashEntry) -> Result<()> {
    anyhow::bail!("restoring from the Trash is not supported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{no_progress, Cancel};
    use crate::rules::ScanRules;
    use crate::scanner::Scanner;

    struct Env {
        _base: tempfile::TempDir,
        root: String,
        store: Arc<IndexStore>,
        exec: Executor,
    }

    fn env() -> Env {
        let base = tempfile::Builder::new()
            .prefix("mf-exec")
            .tempdir()
            .unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(IndexStore::open(&base.path().join("db/index.sqlite")).unwrap());
        let exec = Executor::new(store.clone(), ProtectedPaths::none(store.style()));
        Env {
            _base: base,
            root: root.to_str().unwrap().to_string(),
            store,
            exec,
        }
    }

    impl Env {
        fn p(&self, rel: &str) -> String {
            Path::new(&self.root)
                .join(rel)
                .to_str()
                .unwrap()
                .to_string()
        }
        fn write(&self, rel: &str, content: &str) -> String {
            let p = self.p(rel);
            std::fs::create_dir_all(Path::new(&p).parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
            p
        }
        fn scan(&self) {
            Scanner::new(ScanRules::open(self.store.style()))
                .scan(
                    &self.store,
                    &[self.root.clone()],
                    &no_progress,
                    &Cancel::new(),
                )
                .unwrap();
        }
        fn exists(&self, p: &str) -> bool {
            Path::new(p).exists()
        }
    }

    #[test]
    fn move_and_undo() {
        let e = env();
        let file = e.write("move-me.txt", "payload");
        let dest = e.p("target");
        std::fs::create_dir_all(&dest).unwrap();
        e.scan();
        let start = e.store.max_op_id().unwrap();
        let r = e.exec.move_items(&[file.clone()], &dest).unwrap();
        assert_eq!(r.done, 1);
        assert!(r.batch > 0);
        let moved = e.p("target/move-me.txt");
        assert!(e.exists(&moved) && !e.exists(&file));
        assert!(e.store.record(&moved).unwrap().is_some());
        assert!(e.store.record(&file).unwrap().is_none());
        assert_eq!(
            e.exec.undo_state(start).unwrap(),
            UndoState {
                can_undo: true,
                kind: Some(UndoKind::Move),
                count: Some(1)
            }
        );

        // never overwrite
        let again = e.write("move-me.txt", "another");
        let r2 = e.exec.move_items(&[again.clone()], &dest).unwrap();
        assert_eq!(r2.done, 0);
        assert_eq!(r2.skipped.len(), 1);
        assert_eq!(r2.batch, 0);
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "payload");
        std::fs::remove_file(&again).unwrap();

        let u = e.exec.undo(start).unwrap();
        assert_eq!(
            u.result,
            UndoResult {
                restored: 1,
                missing: 0,
                blocked: 0
            }
        );
        assert!(e.exists(&file) && !e.exists(&moved));
        assert!(e.store.record(&file).unwrap().is_some());
        assert!(e.store.record(&moved).unwrap().is_none());
        assert_eq!(e.exec.undo(start).unwrap().result, UndoResult::default());
        assert!(!e.exec.undo_state(start).unwrap().can_undo);
    }

    #[test]
    fn undo_is_blocked_when_the_original_place_is_taken() {
        let e = env();
        let a = e.write("a.txt", "1");
        let dest = e.p("into");
        std::fs::create_dir_all(&dest).unwrap();
        let r = e.exec.move_items(&[a.clone()], &dest).unwrap();
        assert_eq!(r.done, 1);
        std::fs::write(&a, "squatter").unwrap();
        let u = e.exec.undo(0).unwrap();
        assert_eq!(u.result.blocked, 1);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "squatter");
        // free the place and try again: the record was kept
        std::fs::remove_file(&a).unwrap();
        assert_eq!(e.exec.undo(0).unwrap().result.restored, 1);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "1");
    }

    #[test]
    fn undo_skips_operations_already_restored_outside_the_app() {
        let e = env();
        let a = e.write("a.txt", "1");
        let b = e.write("b.txt", "2");
        let dest = e.p("into");
        std::fs::create_dir_all(&dest).unwrap();
        e.scan();
        let moved_a = e.p("into/a.txt");
        let moved_b = e.p("into/b.txt");
        e.exec.move_items(&[a.clone()], &dest).unwrap();
        e.exec.move_items(&[b.clone()], &dest).unwrap();
        // the user put b back by hand
        std::fs::rename(&moved_b, &b).unwrap();
        let u = e.exec.undo(0).unwrap();
        assert_eq!(u.restored_paths, vec![a.clone()]);
        assert!(e.exists(&a) && !e.exists(&moved_a));
        assert_eq!(e.exec.undo(0).unwrap().result, UndoResult::default());
    }

    #[test]
    fn moving_korean_folder_keeps_child_paths() {
        let e = env();
        e.write("사진/a.jpg", "1");
        e.write("사진/2024/b.jpg", "2");
        let dest = e.p("보관");
        std::fs::create_dir_all(&dest).unwrap();
        e.scan();
        let r = e.exec.move_items(&[e.p("사진")], &dest).unwrap();
        assert_eq!(r.done, 1);
        let moved = e.p("보관/사진");
        let sep = std::path::MAIN_SEPARATOR;
        assert!(e
            .store
            .record(&format!("{moved}{sep}a.jpg"))
            .unwrap()
            .is_some());
        assert!(e
            .store
            .record(&format!("{moved}{sep}2024{sep}b.jpg"))
            .unwrap()
            .is_some());
        let names: Vec<_> = e
            .store
            .children(&format!("{moved}{sep}2024"))
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, ["b.jpg"]);
        assert_eq!(e.store.files_under(&moved).unwrap().len(), 2);
        assert_eq!(e.exec.undo(0).unwrap().result.restored, 1);
        let back = e.p("사진");
        assert!(e
            .store
            .record(&format!("{back}{sep}2024{sep}b.jpg"))
            .unwrap()
            .is_some());
        assert_eq!(e.store.files_under(&back).unwrap().len(), 2);
        assert_eq!(e.store.files_under(&moved).unwrap().len(), 0);
    }

    #[test]
    fn batch_undo_restores_all_and_removes_created_folders() {
        let e = env();
        let files: Vec<String> = ["a.txt", "b.txt", "c.txt"]
            .iter()
            .map(|n| e.write(&format!("L/{n}"), n))
            .collect();
        e.scan();
        let from = e.p("L");
        let to = e.p("R");
        std::fs::create_dir_all(&to).unwrap();
        // keep relative paths: L/a.txt → R/a.txt; L/new/deeper/x → R/new/deeper/x
        let deep = e.write("L/new/deeper/d.txt", "d");
        let mut all = files.clone();
        all.push(deep.clone());
        let r = e.exec.move_across(&all, &from, &to).unwrap();
        assert_eq!(r.done, 4, "{r:?}");
        assert!(e.exists(&e.p("R/new/deeper/d.txt")));
        assert!(e.exists(&e.p("R/c.txt")));
        assert_eq!(e.exec.undo_state(0).unwrap().count, Some(4));
        let u = e.exec.undo(0).unwrap();
        assert_eq!(u.result.restored, 4);
        for f in &all {
            assert!(e.exists(f), "{f}");
        }
        assert!(
            !e.exists(&e.p("R/new")),
            "created folders are removed again"
        );
        assert!(e.exists(&to));
        assert_eq!(e.exec.undo(0).unwrap().result, UndoResult::default());
    }

    #[test]
    fn undo_does_not_move_a_different_item_that_took_the_place() {
        let e = env();
        let a = e.write("a.txt", "original");
        let dest = e.p("into");
        std::fs::create_dir_all(&dest).unwrap();
        e.exec.move_items(&[a.clone()], &dest).unwrap();
        let moved = e.p("into/a.txt");
        std::fs::remove_file(&moved).unwrap();
        std::fs::write(&moved, "someone else").unwrap();
        let u = e.exec.undo(0).unwrap();
        assert_eq!(u.result.restored, 0);
        assert_eq!(u.result.missing, 1);
        assert!(e.exists(&moved) && !e.exists(&a));
    }

    #[test]
    fn session_scoped_undo_leaves_older_operations() {
        let e = env();
        let a = e.write("a.txt", "1");
        let b = e.write("b.txt", "2");
        let dest = e.p("into");
        std::fs::create_dir_all(&dest).unwrap();
        e.exec.move_items(&[a.clone()], &dest).unwrap();
        let start = e.store.max_op_id().unwrap();
        e.exec.move_items(&[b.clone()], &dest).unwrap();
        assert_eq!(e.exec.undo(start).unwrap().result.restored, 1);
        assert!(e.exists(&b));
        assert_eq!(e.exec.undo(start).unwrap().result, UndoResult::default());
        assert!(!e.exists(&a));
    }

    #[test]
    fn protected_and_self_moves_are_refused() {
        let e = env();
        let f = e.write("p.txt", "x");
        let protected = ProtectedPaths::new(&[e.root.clone()], &[], e.store.style());
        let exec = Executor::new(e.store.clone(), protected);
        let r = exec.trash_items(&[f.clone()]).unwrap();
        assert_eq!(r.failed.len(), 1);
        assert!(e.exists(&f));
        let dir = e.p("d");
        std::fs::create_dir_all(Path::new(&dir).join("inner")).unwrap();
        let r = e.exec.move_items(&[dir.clone()], &e.p("d/inner")).unwrap();
        assert_eq!(r.failed.len(), 1);
        let r = e.exec.move_items(&[e.p("nope.txt")], &dir).unwrap();
        assert_eq!(r.failed[0].reason, "not found");
    }

    fn purge_test_items(root: &str) {
        #[cfg(target_os = "linux")]
        if let Ok(items) = trash::os_limited::list() {
            let mine: Vec<_> = items
                .into_iter()
                .filter(|i| i.original_parent.to_str().is_some_and(|p| p.contains(root)))
                .collect();
            let _ = trash::os_limited::purge_all(mine);
        }
        let _ = root;
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trash_and_undo_via_recycle_bin() {
        let e = env();
        let f = e.write("bin/t.txt", "trash me");
        let d = e.write("bin/folder/inner.txt", "inner");
        let folder = e.p("bin/folder");
        e.scan();
        let start = e.store.max_op_id().unwrap();
        let r = e.exec.trash_items(&[f.clone(), folder.clone()]).unwrap();
        assert_eq!(r.done, 2, "{r:?}");
        assert!(!e.exists(&f) && !e.exists(&folder));
        assert!(e.store.record(&f).unwrap().is_none());
        assert!(e.store.record(&d).unwrap().is_none());
        assert_eq!(
            e.exec.undo_state(start).unwrap().kind,
            Some(UndoKind::Trash)
        );
        let u = e.exec.undo(start).unwrap();
        assert_eq!(
            u.result,
            UndoResult {
                restored: 2,
                missing: 0,
                blocked: 0
            },
            "{:?}",
            u.errors
        );
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "trash me");
        assert!(e.exists(&d));
        assert_eq!(u.rescan, vec![folder.clone()]);
        assert!(e.store.record(&f).unwrap().is_some());

        // trashed, then the original place is taken → blocked, never overwritten
        e.exec.trash_items(&[f.clone()]).unwrap();
        std::fs::write(&f, "new file").unwrap();
        let u = e.exec.undo(start).unwrap();
        assert_eq!(u.result.blocked, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "new file");
        // free the place → restored
        std::fs::remove_file(&f).unwrap();
        assert_eq!(e.exec.undo(start).unwrap().result.restored, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "trash me");
        purge_test_items(&e.root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trash_duplicates_keeps_one_unchanged_copy() {
        let e = env();
        let a = e.write("d1/a.txt", "same content");
        let b = e.write("d2/a.txt", "same content");
        let c = e.write("d3/a.txt", "same content");
        let x = e.write("x/solo.txt", "unique");
        e.scan();
        crate::duplicates::find_groups(&e.store, 1, None, &no_progress, &Cancel::new()).unwrap();
        // everything chosen → one survives
        let r = e
            .exec
            .trash_duplicates(&[a.clone(), b.clone(), c.clone(), x.clone()])
            .unwrap();
        assert_eq!(r.done, 2, "{r:?}");
        let survivors = [&a, &b, &c].iter().filter(|p| e.exists(p)).count();
        assert_eq!(survivors, 1);
        assert!(e.exists(&x), "a file without a verified hash is skipped");
        assert!(r.skipped.iter().any(|s| s.reason.contains("last copy")));
        purge_test_items(&e.root);

        // a copy that changed after hashing is not trashed and does not count as keeper
        let e2 = env();
        let a = e2.write("d1/a.txt", "same content");
        let b = e2.write("d2/a.txt", "same content");
        e2.scan();
        crate::duplicates::find_groups(&e2.store, 1, None, &no_progress, &Cancel::new()).unwrap();
        std::fs::write(&b, "same contenT").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        filetime::set_file_mtime(&b, filetime::FileTime::from_system_time(later)).unwrap();
        let r = e2.exec.trash_duplicates(&[a.clone()]).unwrap();
        assert_eq!(r.done, 0, "the only unchanged copy stays");
        assert!(e2.exists(&a));
        purge_test_items(&e2.root);
    }

    #[test]
    fn nested_selections_are_handled_once() {
        let style = PathStyle::UNIX;
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            drop_nested(&v(&["/a/x", "/a", "/b", "/b/", "/ab"]), style),
            v(&["/a", "/b", "/ab"])
        );
        let e = env();
        e.write("d/inner.txt", "1");
        let dest = e.p("to");
        std::fs::create_dir_all(&dest).unwrap();
        let r = e
            .exec
            .move_items(&[e.p("d/inner.txt"), e.p("d")], &dest)
            .unwrap();
        assert_eq!((r.done, r.failed.len()), (1, 0), "{r:?}");
        assert!(Path::new(&e.p("to/d/inner.txt")).exists());
    }

    #[test]
    fn remove_if_empty_tree_spares_content() {
        let d = tempfile::Builder::new()
            .prefix("mf-empty")
            .tempdir()
            .unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("b/c")).unwrap();
        std::fs::write(a.join("b/Thumbs.db"), "x").unwrap();
        assert!(remove_if_empty_tree(&a));
        assert!(!a.exists());
        let k = d.path().join("k");
        std::fs::create_dir_all(k.join("b")).unwrap();
        std::fs::write(k.join("b/real.txt"), "x").unwrap();
        assert!(!remove_if_empty_tree(&k));
        assert!(k.join("b/real.txt").exists());
    }
}
