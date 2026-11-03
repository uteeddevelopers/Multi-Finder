//! The file index and the operation log (SQLite, WAL). Port of `IndexStore.swift`.
//!
//! Thread-safe: share it as `Arc<IndexStore>`; every call takes the internal connection
//! mutex. Subtrees are queried with descendant *ranges* (`path >= 'P\' AND path < 'P]'`)
//! instead of `LIKE`, so the primary-key index is used and `_`/`%` in names are harmless.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use crate::analyzer::IgnoreList;
use crate::filter::{self, Matcher};
use crate::paths::{self, PathStyle};
use crate::record::{FileKind, FileRecord};
use crate::rules::ScanRules;
use crate::types::{
    DirInfo, Entry, FileCategory, Filter, FolderState, FolderStatus, FolderSummary, SearchResult,
    Sort, SortField, Stats,
};

/// Bump when the meaning of stored rows changes: the `files` cache is rebuilt by rescanning.
const SCHEMA: &str = "win-1";

pub const DB_FILE: &str = "index.sqlite";

const COLUMNS: &str = "path,name,parent,ext,size,mtime,inode,kind,cloud,hash";

const UPSERT: &str = "
INSERT INTO files(path,name,name_key,parent,ext,size,mtime,inode,kind,cloud,hash,hash_mtime,hash_size,seen)
VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,NULL,NULL,?11)
ON CONFLICT(path) DO UPDATE SET
    name=excluded.name, name_key=excluded.name_key, parent=excluded.parent, ext=excluded.ext,
    size=excluded.size, mtime=excluded.mtime, inode=excluded.inode, kind=excluded.kind,
    cloud=excluded.cloud, seen=excluded.seen,
    hash       = CASE WHEN files.hash_mtime = excluded.mtime AND files.hash_size = excluded.size THEN files.hash ELSE NULL END,
    hash_mtime = CASE WHEN files.hash_mtime = excluded.mtime AND files.hash_size = excluded.size THEN files.hash_mtime ELSE NULL END,
    hash_size  = CASE WHEN files.hash_mtime = excluded.mtime AND files.hash_size = excluded.size THEN files.hash_size ELSE NULL END";

/// What an operation-log row did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Trash,
    Move,
    /// a folder created while moving (removed on undo when empty)
    Mkdir,
}

impl OpKind {
    fn as_str(self) -> &'static str {
        match self {
            OpKind::Trash => "trash",
            OpKind::Move => "move",
            OpKind::Mkdir => "mkdir",
        }
    }
    fn parse(s: &str) -> OpKind {
        match s {
            "trash" => OpKind::Trash,
            "mkdir" => OpKind::Mkdir,
            _ => OpKind::Move,
        }
    }
}

/// What the item looked like right after the operation — undo checks it is still the same one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Identity {
    pub kind: Option<FileKind>,
    pub size: u64,
    /// modification time, ns since epoch
    pub mtime: i64,
    /// creation time (ns) when the platform reports it
    pub btime: Option<i64>,
    /// 0 = unknown (Windows)
    pub inode: u64,
}

impl Identity {
    pub fn of(md: &std::fs::Metadata) -> Identity {
        let ft = md.file_type();
        let kind = if ft.is_symlink() {
            FileKind::Link
        } else if ft.is_dir() {
            FileKind::Dir
        } else {
            FileKind::File
        };
        Identity {
            kind: Some(kind),
            size: if kind == FileKind::File { md.len() } else { 0 },
            mtime: md.modified().map(paths::system_time_ns).unwrap_or(0),
            btime: md.created().ok().map(paths::system_time_ns),
            inode: crate::record::inode_of(md),
        }
    }

    pub fn of_path(path: &str) -> Option<Identity> {
        std::fs::symlink_metadata(path)
            .ok()
            .map(|m| Identity::of(&m))
    }

    /// Same item? Kind must match; creation time when both sides know it; then the inode
    /// when both know it (unix — an item edited after the move still counts as the same);
    /// otherwise, for files, size + modification time.
    pub fn matches(&self, now: &Identity) -> bool {
        if self.kind.is_none() {
            return true; // nothing recorded — cannot check
        }
        if self.kind != now.kind {
            return false;
        }
        if let (Some(a), Some(b)) = (self.btime, now.btime) {
            if a != b {
                return false;
            }
        }
        if self.inode != 0 && now.inode != 0 {
            return self.inode == now.inode;
        }
        if self.kind == Some(FileKind::File) {
            return self.size == now.size && self.mtime == now.mtime;
        }
        true
    }
}

/// One row of the operation log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub id: i64,
    /// ms since epoch (logged right after the file-system change)
    pub ts: i64,
    pub kind: OpKind,
    /// where the item was (for mkdir: the created folder)
    pub src: String,
    /// where it went ("" for trash — the Recycle Bin location is looked up on undo)
    pub dst: String,
    pub undone: bool,
    pub batch: Option<i64>,
    pub ident: Identity,
}

pub struct IndexStore {
    conn: Mutex<Connection>,
    generation: AtomicI64,
    style: PathStyle,
    path: PathBuf,
}

fn read_record(row: &Row) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        path: row.get(0)?,
        name: row.get(1)?,
        parent: row.get(2)?,
        ext: row.get(3)?,
        size: row.get::<_, i64>(4)?.max(0) as u64,
        modified: row.get(5)?,
        inode: row.get::<_, i64>(6)? as u64,
        kind: FileKind::from_i64(row.get(7)?),
        cloud: row.get::<_, i64>(8)? != 0,
        hash: row.get(9)?,
    })
}

const OP_COLUMNS: &str =
    "id, ts, kind, src, dst, undone, batch, item_kind, size, mtime, btime, inode";

fn read_op(row: &Row) -> rusqlite::Result<Operation> {
    let item_kind: i64 = row.get(7)?;
    Ok(Operation {
        id: row.get(0)?,
        ts: row.get(1)?,
        kind: OpKind::parse(&row.get::<_, String>(2)?),
        src: row.get(3)?,
        dst: row.get(4)?,
        undone: row.get::<_, i64>(5)? != 0,
        batch: row.get(6)?,
        ident: Identity {
            kind: if item_kind < 0 {
                None
            } else {
                Some(FileKind::from_i64(item_kind))
            },
            size: row.get::<_, i64>(8)?.max(0) as u64,
            mtime: row.get(9)?,
            btime: row.get(10)?,
            inode: row.get::<_, i64>(11)? as u64,
        },
    })
}

impl IndexStore {
    /// `data_dir/index.sqlite`, native path style.
    pub fn open_in(data_dir: &Path) -> Result<IndexStore> {
        Self::open(&data_dir.join(DB_FILE))
    }

    pub fn open(path: &Path) -> Result<IndexStore> {
        Self::open_with_style(path, PathStyle::native())
    }

    /// Like [`open_in`](Self::open_in), but a database file that SQLite cannot read is set
    /// aside (`index.sqlite.corrupt-<ms>`) and a fresh one is created — the index is a cache
    /// that a rescan rebuilds (the undo log of earlier sessions is lost in that case).
    pub fn open_in_or_reset(data_dir: &Path) -> Result<IndexStore> {
        let path = data_dir.join(DB_FILE);
        match Self::open(&path) {
            Ok(s) => Ok(s),
            Err(e) => {
                let text = format!("{e:#}").to_lowercase();
                if !(text.contains("not a database")
                    || text.contains("malformed")
                    || text.contains("corrupt"))
                {
                    return Err(e);
                }
                let stamp = paths::now_ms();
                for suffix in ["", "-wal", "-shm"] {
                    let from = data_dir.join(format!("{DB_FILE}{suffix}"));
                    if from.exists() {
                        let _ = std::fs::rename(
                            &from,
                            data_dir.join(format!("{DB_FILE}{suffix}.corrupt-{stamp}")),
                        );
                    }
                }
                Self::open(&path)
            }
        }
    }

    /// `style` lets tests store Windows-style paths on Linux.
    pub fn open_with_style(path: &Path, style: PathStyle) -> Result<IndexStore> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("opening index {}", path.display()))?;
        // another window or process may be writing — wait instead of failing (a lost op-log row breaks undo)
        conn.busy_timeout(Duration::from_millis(5000))?;
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);")?;
        let schema: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if schema.as_deref() != Some(SCHEMA) {
            conn.execute_batch("DROP TABLE IF EXISTS files;")?;
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES('schema', ?1)",
                [SCHEMA],
            )?;
        }
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files(
                path TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                name_key TEXT NOT NULL,
                parent TEXT NOT NULL,
                ext TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                inode INTEGER NOT NULL,
                kind INTEGER NOT NULL,
                cloud INTEGER NOT NULL,
                hash TEXT,
                hash_mtime INTEGER,
                hash_size INTEGER,
                seen INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS files_size ON files(size) WHERE kind = 0;
            CREATE INDEX IF NOT EXISTS files_hash ON files(hash);
            CREATE INDEX IF NOT EXISTS files_parent ON files(parent);
            CREATE TABLE IF NOT EXISTS ops(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                kind TEXT NOT NULL,
                src TEXT NOT NULL,
                dst TEXT NOT NULL,
                undone INTEGER NOT NULL DEFAULT 0,
                batch INTEGER,
                item_kind INTEGER NOT NULL DEFAULT -1,
                size INTEGER NOT NULL DEFAULT 0,
                mtime INTEGER NOT NULL DEFAULT 0,
                btime INTEGER,
                inode INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS ops_batch ON ops(batch);
            CREATE TABLE IF NOT EXISTS ignores(kind TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(kind, value));
            CREATE TABLE IF NOT EXISTS buckets(
                id TEXT PRIMARY KEY, name TEXT NOT NULL, hint TEXT NOT NULL,
                is_fallback INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS classes(
                path TEXT PRIMARY KEY, bucket TEXT NOT NULL, confidence REAL NOT NULL, summary TEXT NOT NULL,
                mtime INTEGER NOT NULL, size INTEGER NOT NULL, manual INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX IF NOT EXISTS classes_bucket ON classes(bucket);
            ",
        )?;
        let generation: i64 =
            conn.query_row("SELECT COALESCE(MAX(seen), 0) FROM files", [], |r| r.get(0))?;
        Ok(IndexStore {
            conn: Mutex::new(conn),
            generation: AtomicI64::new(generation),
            style,
            path: path.to_path_buf(),
        })
    }

    pub fn style(&self) -> PathStyle {
        self.style
    }

    /// Location of the database file.
    pub fn db_path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn range(&self, path: &str) -> (String, String) {
        self.style.descendant_range(path)
    }

    // ───────────── scan generations ─────────────

    pub fn current_generation(&self) -> i64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Start a full scan: pass the result to [`upsert`](Self::upsert), then call
    /// [`finish_scan`](Self::finish_scan) to drop what was not seen.
    pub fn begin_scan(&self) -> i64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Delete rows under `roots` not seen in generation `gen`. Rows under `keep` (folders
    /// that could not be read this time) are kept as they were.
    pub fn finish_scan(&self, roots: &[String], gen: i64, keep: &[String]) -> Result<()> {
        let mut c = self.conn();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut mark = tx.prepare(
                "UPDATE files SET seen = ?1 WHERE path = ?2 OR (path >= ?3 AND path < ?4)",
            )?;
            for k in keep {
                let (lo, hi) = self.range(k);
                mark.execute(params![gen, k, lo, hi])?;
            }
            let mut del = tx.prepare(
                "DELETE FROM files WHERE seen < ?1 AND (path = ?2 OR (path >= ?3 AND path < ?4))",
            )?;
            for root in roots {
                let (lo, hi) = self.range(root);
                del.execute(params![gen, root, lo, hi])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ───────────── writes ─────────────

    /// Insert or update rows. A file whose size and mtime did not change keeps its hash.
    pub fn upsert(&self, records: &[FileRecord], gen: i64) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut c = self.conn();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut st = tx.prepare_cached(UPSERT)?;
            for r in records {
                st.execute(params![
                    r.path,
                    r.name,
                    paths::fold(&r.name),
                    r.parent,
                    r.ext,
                    r.size as i64,
                    r.modified,
                    r.inode as i64,
                    r.kind.as_i64(),
                    r.cloud as i64,
                    gen
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Store a content hash valid for this mtime/size.
    pub fn set_hash(&self, path: &str, hash: &str, mtime: i64, size: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE files SET hash = ?1, hash_mtime = ?2, hash_size = ?3 WHERE path = ?4",
            params![hash, mtime, size as i64, path],
        )?;
        Ok(())
    }

    /// Remove a path and everything below it (index rows and classifications). Returns rows deleted.
    pub fn remove(&self, path: &str) -> Result<usize> {
        let (lo, hi) = self.range(path);
        let c = self.conn();
        let n = c.execute(
            "DELETE FROM files WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
            params![path, lo, hi],
        )?;
        c.execute(
            "DELETE FROM classes WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
            params![path, lo, hi],
        )?;
        Ok(n)
    }

    /// Reflect a move: rewrite the path prefix of the item and its children.
    pub fn rename(&self, old: &str, new: &str) -> Result<()> {
        let style = self.style;
        let mut c = self.conn();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            // rows left at the new place are stale (the place had to be empty for the move)
            let (tlo, thi) = self.range(new);
            tx.execute(
                "DELETE FROM files WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
                params![new, tlo, thi],
            )?;
            let name = style.file_name(new).to_string();
            let parent = style.parent(new).unwrap_or("").to_string();
            let kind: Option<i64> = tx
                .query_row("SELECT kind FROM files WHERE path = ?1", [old], |r| {
                    r.get(0)
                })
                .optional()?;
            let ext = if kind == Some(0) {
                crate::record::extension_of(&name)
            } else {
                String::new()
            };
            tx.execute(
                "UPDATE files SET path = ?1, name = ?2, name_key = ?3, parent = ?4, ext = ?5 WHERE path = ?6",
                params![new, name, paths::fold(&name), parent, ext, old],
            )?;
            // SQLite substr counts characters, not bytes — cut by character count
            let cut = old.chars().count() as i64 + 1;
            let (lo, hi) = self.range(old);
            tx.execute(
                "UPDATE files SET path = ?1 || substr(path, ?2), parent = ?1 || substr(parent, ?2)
                 WHERE path >= ?3 AND path < ?4",
                params![new, cut, lo, hi],
            )?;
            tx.execute(
                "UPDATE OR REPLACE classes SET path = ?1 || substr(path, ?2)
                 WHERE path = ?3 OR (path >= ?4 AND path < ?5)",
                params![new, cut, old, lo, hi],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Clean rows that the rules exclude but that came in through an older version or the
    /// watcher (Recycle Bin, node_modules, hidden folders …). Returns rows deleted.
    pub fn remove_excluded(&self, rules: &ScanRules) -> Result<usize> {
        let all: Vec<(String, String)> = {
            let c = self.conn();
            let mut st = c.prepare("SELECT path, name FROM files ORDER BY path")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut doomed: Vec<String> = Vec::new();
        for (p, name) in all {
            if doomed.last().is_some_and(|d| self.style.is_under(&p, d)) {
                continue;
            }
            if rules.is_excluded(&p, &name) {
                doomed.push(p);
            }
        }
        let mut removed = 0;
        for d in doomed {
            removed += self.remove(&d)?;
        }
        Ok(removed)
    }

    // ───────────── reads ─────────────

    pub fn record(&self, path: &str) -> Result<Option<FileRecord>> {
        let c = self.conn();
        Ok(c.query_row(
            &format!("SELECT {COLUMNS} FROM files WHERE path = ?1"),
            [path],
            read_record,
        )
        .optional()?)
    }

    pub fn is_indexed(&self, path: &str) -> Result<bool> {
        let c = self.conn();
        Ok(
            c.query_row("SELECT 1 FROM files WHERE path = ?1", [path], |_| Ok(()))
                .optional()?
                .is_some(),
        )
    }

    /// Direct children (folders first, then name).
    pub fn children(&self, parent: &str) -> Result<Vec<FileRecord>> {
        let c = self.conn();
        let mut st = c.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM files WHERE parent = ?1 ORDER BY kind = 1 DESC, name_key"
        ))?;
        let rows = st.query_map([parent], read_record)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Direct children for the tree view, filtered and sorted (folders first).
    pub fn list_children(&self, parent: &str, filter: &Filter, sort: Sort) -> Result<Vec<Entry>> {
        let m = Matcher::new(filter);
        let mut rows: Vec<FileRecord> = self
            .children(parent)?
            .into_iter()
            .filter(|r| m.matches(r))
            .collect();
        self.sort_with_dir_sizes(&mut rows, sort)?;
        Ok(rows.iter().map(FileRecord::to_entry).collect())
    }

    fn sort_with_dir_sizes(&self, rows: &mut [FileRecord], sort: Sort) -> Result<()> {
        let mut dir_bytes = std::collections::HashMap::new();
        if sort.field == SortField::Size {
            for r in rows.iter().filter(|r| r.kind == FileKind::Dir) {
                dir_bytes.insert(r.path.clone(), self.bytes_under(&r.path)?);
            }
        }
        filter::sort_records(rows, sort, &|p| dir_bytes.get(p).copied());
        Ok(())
    }

    /// Every regular file under `prefix` (recursive), by path.
    pub fn files_under(&self, prefix: &str) -> Result<Vec<FileRecord>> {
        let (lo, hi) = self.range(prefix);
        let c = self.conn();
        let mut st = c.prepare(&format!(
            "SELECT {COLUMNS} FROM files WHERE kind = 0 AND path >= ?1 AND path < ?2 ORDER BY path"
        ))?;
        let rows = st.query_map(params![lo, hi], read_record)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Regular files with this content hash.
    pub fn files_with_hash(&self, hash: &str) -> Result<Vec<FileRecord>> {
        let c = self.conn();
        let mut st = c.prepare(&format!(
            "SELECT {COLUMNS} FROM files WHERE kind = 0 AND hash = ?1 ORDER BY path"
        ))?;
        let rows = st.query_map([hash], read_record)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn bytes_under(&self, path: &str) -> Result<u64> {
        let (lo, hi) = self.range(path);
        let c = self.conn();
        let b: i64 = c.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM files WHERE kind = 0 AND path >= ?1 AND path < ?2",
            params![lo, hi],
            |r| r.get(0),
        )?;
        Ok(b.max(0) as u64)
    }

    /// Recursive file count and bytes under a folder.
    pub fn summary(&self, path: &str) -> Result<(u64, u64)> {
        let (lo, hi) = self.range(path);
        let c = self.conn();
        let (n, b): (i64, i64) = c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE kind = 0 AND path >= ?1 AND path < ?2",
            params![lo, hi],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((n.max(0) as u64, b.max(0) as u64))
    }

    /// `dir_info` command: direct child count + recursive bytes for each folder.
    pub fn dir_info(&self, paths: &[String]) -> Result<Vec<DirInfo>> {
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let items: i64 = {
                let c = self.conn();
                c.query_row("SELECT COUNT(*) FROM files WHERE parent = ?1", [p], |r| {
                    r.get(0)
                })?
            };
            out.push(DirInfo {
                path: p.clone(),
                items: items.max(0) as u64,
                bytes: self.bytes_under(p)?,
            });
        }
        Ok(out)
    }

    /// `folder_summaries` command.
    pub fn folder_summaries(&self, paths: &[String]) -> Result<Vec<FolderSummary>> {
        paths
            .iter()
            .map(|p| {
                let (files, bytes) = self.summary(p)?;
                Ok(FolderSummary {
                    path: p.clone(),
                    files,
                    bytes,
                })
            })
            .collect()
    }

    /// Drop roots that lie inside another root (so nothing is counted twice).
    fn outer_roots(&self, roots: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in roots {
            if roots
                .iter()
                .any(|o| !self.style.eq(o, r) && self.style.is_under(r, o))
            {
                continue;
            }
            if !out.iter().any(|o| self.style.eq(o, r)) {
                out.push(r.clone());
            }
        }
        out
    }

    /// Totals under the user's folders (descendants; nested folders counted once).
    pub fn stats(&self, roots: &[String]) -> Result<Stats> {
        let mut s = Stats::default();
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT COALESCE(SUM(kind = 0), 0), COALESCE(SUM(kind = 1), 0),
                    COALESCE(SUM(CASE WHEN kind = 0 THEN size END), 0)
             FROM files WHERE path >= ?1 AND path < ?2",
        )?;
        for r in self.outer_roots(roots) {
            let (lo, hi) = self.range(&r);
            let (f, d, b): (i64, i64, i64) =
                st.query_row(params![lo, hi], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            s.files += f.max(0) as u64;
            s.folders += d.max(0) as u64;
            s.bytes += b.max(0) as u64;
        }
        Ok(s)
    }

    /// Whole-index totals plus how many files carry a hash (diagnostics/tests).
    pub fn totals(&self) -> Result<(u64, u64, u64, u64)> {
        let c = self.conn();
        let t: (i64, i64, i64, i64) = c.query_row(
            "SELECT COALESCE(SUM(kind = 0), 0), COALESCE(SUM(kind = 1), 0),
                    COALESCE(SUM(CASE WHEN kind = 0 THEN size END), 0), COALESCE(SUM(hash IS NOT NULL), 0)
             FROM files",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        Ok((t.0 as u64, t.1 as u64, t.2 as u64, t.3 as u64))
    }

    /// Names containing `text` (case-insensitive, NFC) anywhere under `roots`, filtered by
    /// kind/size/date, folders first, at most `limit` (`truncated` when there were more).
    pub fn search(
        &self,
        text: &str,
        roots: &[String],
        filter: &Filter,
        sort: Sort,
        limit: usize,
    ) -> Result<SearchResult> {
        let needle = paths::fold(text.trim());
        let roots = self.outer_roots(roots);
        if needle.is_empty() || roots.is_empty() || limit == 0 {
            return Ok(SearchResult::default());
        }
        let mut sql = format!("SELECT {COLUMNS} FROM files WHERE instr(name_key, ?) > 0 AND (");
        let mut args: Vec<rusqlite::types::Value> = vec![needle.into()];
        for (i, r) in roots.iter().enumerate() {
            if i > 0 {
                sql.push_str(" OR ");
            }
            sql.push_str("(path >= ? AND path < ?)");
            let (lo, hi) = self.range(r);
            args.push(lo.into());
            args.push(hi.into());
        }
        sql.push(')');
        // kind / size / date in SQL so LIMIT counts only real matches (folders always pass)
        let quoted = |exts: &[&str]| {
            exts.iter()
                .map(|e| format!("'{}'", e.replace('\'', "''")))
                .collect::<Vec<_>>()
        };
        match filter.category {
            FileCategory::All => {}
            FileCategory::Other => {
                sql.push_str(&format!(
                    " AND (kind != 0 OR ext NOT IN ({}))",
                    quoted(&filter::all_known_extensions()).join(",")
                ));
            }
            c => {
                sql.push_str(&format!(
                    " AND (kind = 1 OR (kind = 0 AND ext IN ({})))",
                    quoted(filter::extensions(c)).join(",")
                ));
            }
        }
        if filter.min_size > 0 {
            sql.push_str(" AND (kind = 1 OR size >= ?)");
            args.push((filter.min_size as i64).into());
        }
        if let Some(cut) = paths::window_start_ms(filter.within_days) {
            sql.push_str(" AND (kind = 1 OR mtime >= ?)");
            args.push(cut.into());
        }
        let dir = if sort.ascending { "ASC" } else { "DESC" };
        let order = match sort.field {
            SortField::Name => format!("name_key {dir}"),
            SortField::Modified => format!("mtime {dir}, name_key"),
            SortField::Size => format!("size {dir}, name_key"),
            SortField::Kind => format!("ext {dir}, name_key {dir}"),
        };
        sql.push_str(&format!(" ORDER BY kind = 1 DESC, {order} LIMIT ?"));
        args.push(((limit + 1) as i64).into());
        let mut rows: Vec<FileRecord> = {
            let c = self.conn();
            let mut st = c.prepare(&sql)?;
            let it = st.query_map(rusqlite::params_from_iter(args.iter()), read_record)?;
            it.collect::<rusqlite::Result<_>>()?
        };
        let truncated = rows.len() > limit;
        rows.truncate(limit);
        self.sort_with_dir_sizes(&mut rows, sort)?;
        Ok(SearchResult {
            results: rows.iter().map(FileRecord::to_entry).collect(),
            truncated,
        })
    }

    /// `folder_states` command. `scanning` = roots with a scan in progress.
    pub fn folder_states(
        &self,
        folders: &[String],
        scanning: &[String],
    ) -> Result<Vec<FolderStatus>> {
        let mut out = Vec::with_capacity(folders.len());
        for f in folders {
            let state = if std::fs::metadata(f).is_err() {
                FolderState::Missing
            } else if scanning.iter().any(|s| self.style.eq(s, f)) {
                FolderState::Scanning
            } else if self.is_indexed(f)? {
                FolderState::Indexed
            } else {
                FolderState::NotIndexed
            };
            out.push(FolderStatus {
                path: f.clone(),
                state,
            });
        }
        Ok(out)
    }

    /// Size buckets with ≥ 2 regular, local (not cloud) files of at least `min_size` bytes —
    /// the first duplicate filter. Hard links (same inode) count once. With `under`, only
    /// buckets with at least one file below that folder (other members may be anywhere).
    pub fn size_groups(&self, min_size: u64, under: Option<&str>) -> Result<Vec<Vec<FileRecord>>> {
        let mut sql = format!(
            "SELECT {COLUMNS} FROM files WHERE kind = 0 AND cloud = 0 AND size >= ?1
               AND size IN (SELECT size FROM files WHERE kind = 0 AND cloud = 0 AND size >= ?1
                            GROUP BY size HAVING COUNT(*) > 1)"
        );
        let min = min_size.max(1) as i64;
        let mut args: Vec<rusqlite::types::Value> = vec![min.into()];
        if let Some(u) = under {
            sql.push_str(" AND size IN (SELECT size FROM files WHERE kind = 0 AND cloud = 0 AND path >= ?2 AND path < ?3)");
            let (lo, hi) = self.range(u);
            args.push(lo.into());
            args.push(hi.into());
        }
        sql.push_str(" ORDER BY size DESC, path");
        let c = self.conn();
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(args.iter()), read_record)?;
        let mut groups: Vec<Vec<FileRecord>> = Vec::new();
        let mut current: Vec<FileRecord> = Vec::new();
        let mut inodes = std::collections::HashSet::new();
        for r in rows {
            let r = r?;
            if current.last().is_some_and(|l| l.size != r.size) {
                if current.len() > 1 {
                    groups.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
                inodes.clear();
            }
            if r.inode != 0 && !inodes.insert(r.inode) {
                continue;
            }
            current.push(r);
        }
        if current.len() > 1 {
            groups.push(current);
        }
        Ok(groups)
    }

    // ───────────── ignore list ─────────────

    pub fn ignore_list(&self) -> Result<IgnoreList> {
        let c = self.conn();
        let mut st = c.prepare("SELECT kind, value FROM ignores")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut list = IgnoreList::default();
        for row in rows {
            let (k, v) = row?;
            if k == "hash" {
                list.hashes.insert(v);
            } else {
                list.folders.push(v);
            }
        }
        Ok(list)
    }

    pub fn add_ignore_hashes(&self, hashes: &[String]) -> Result<()> {
        let c = self.conn();
        for h in hashes {
            c.execute(
                "INSERT OR IGNORE INTO ignores(kind, value) VALUES('hash', ?1)",
                [h],
            )?;
        }
        Ok(())
    }

    pub fn add_ignore_folder(&self, folder: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO ignores(kind, value) VALUES('folder', ?1)",
            [folder],
        )?;
        Ok(())
    }

    pub fn clear_ignores(&self) -> Result<()> {
        self.conn().execute("DELETE FROM ignores", [])?;
        Ok(())
    }

    // ───────────── operation log ─────────────

    /// Record one change. Returns the row id.
    pub fn log_op(
        &self,
        kind: OpKind,
        src: &str,
        dst: &str,
        batch: Option<i64>,
        ident: &Identity,
    ) -> Result<i64> {
        let c = self.conn();
        c.execute(
            "INSERT INTO ops(ts, kind, src, dst, batch, item_kind, size, mtime, btime, inode)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                paths::now_ms(),
                kind.as_str(),
                src,
                dst,
                batch,
                ident.kind.map(|k| k.as_i64()).unwrap_or(-1),
                ident.size as i64,
                ident.mtime,
                ident.btime,
                ident.inode as i64
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// A fresh batch number (one per user action).
    pub fn next_batch_id(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COALESCE(MAX(batch), 0) + 1 FROM ops", [], |r| {
                r.get(0)
            })?)
    }

    /// The newest operation id — remember it at app start so undo only touches this session.
    pub fn max_op_id(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COALESCE(MAX(id), 0) FROM ops", [], |r| r.get(0))?)
    }

    /// The newest batch that is not undone yet (ops newest first). Ops without a batch
    /// number come one at a time.
    pub fn last_undoable_batch(&self, after: i64) -> Result<Vec<Operation>> {
        let c = self.conn();
        let last: Option<Operation> = c
            .query_row(
                &format!("SELECT {OP_COLUMNS} FROM ops WHERE undone = 0 AND id > ?1 ORDER BY id DESC LIMIT 1"),
                [after],
                read_op,
            )
            .optional()?;
        let Some(last) = last else { return Ok(vec![]) };
        let Some(batch) = last.batch else {
            return Ok(vec![last]);
        };
        let mut st = c.prepare(&format!(
            "SELECT {OP_COLUMNS} FROM ops WHERE undone = 0 AND id > ?1 AND batch = ?2 ORDER BY id DESC"
        ))?;
        let rows = st.query_map(params![after, batch], read_op)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_undone(&self, id: i64) -> Result<()> {
        let n = self
            .conn()
            .execute("UPDATE ops SET undone = 1 WHERE id = ?1", [id])?;
        anyhow::ensure!(n == 1, "operation #{id} could not be updated");
        Ok(())
    }

    pub fn op_history(&self, limit: usize) -> Result<Vec<Operation>> {
        let c = self.conn();
        let mut st = c.prepare(&format!(
            "SELECT {OP_COLUMNS} FROM ops ORDER BY id DESC LIMIT ?1"
        ))?;
        let rows = st.query_map([limit as i64], read_op)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(p: &str, style: PathStyle, kind: FileKind, size: u64) -> FileRecord {
        FileRecord {
            path: p.to_string(),
            name: style.file_name(p).to_string(),
            parent: style.parent(p).unwrap_or("").to_string(),
            ext: crate::record::extension_of(style.file_name(p)),
            size,
            modified: 1_000,
            inode: 0,
            kind,
            cloud: false,
            hash: None,
        }
    }

    fn store(style: PathStyle) -> (tempfile::TempDir, IndexStore) {
        let dir = tempfile::Builder::new()
            .prefix("mf-index")
            .tempdir()
            .unwrap();
        let s = IndexStore::open_with_style(&dir.path().join("db").join("index.sqlite"), style)
            .unwrap();
        (dir, s)
    }

    #[test]
    fn descendant_range_only_covers_children_unix() {
        let (_d, s) = store(PathStyle::UNIX);
        let g = s.begin_scan();
        let recs: Vec<_> = [
            "/t/b/c", "/t/b c", "/t/bc", "/t/b0", "/t/b-x", "/t/b/d/e", "/t/b.x",
        ]
        .iter()
        .map(|p| mk(p, PathStyle::UNIX, FileKind::File, 1))
        .collect();
        s.upsert(&recs, g).unwrap();
        let under: std::collections::HashSet<_> = s
            .files_under("/t/b")
            .unwrap()
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(
            under,
            ["/t/b/c", "/t/b/d/e"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
        assert_eq!(s.summary("/t/b").unwrap().0, 2);
        s.remove("/t/b").unwrap();
        assert_eq!(s.files_under("/t").unwrap().len(), 5);
    }

    #[test]
    fn descendant_range_only_covers_children_windows_style() {
        let w = PathStyle::WINDOWS;
        let (_d, s) = store(w);
        let g = s.begin_scan();
        let recs: Vec<_> = [
            r"C:\t\b\c",
            r"C:\t\b c",
            r"C:\t\bc",
            r"C:\t\b]",
            r"C:\t\b[x",
            r"C:\t\b\d\e",
            r"C:\t\b.x",
        ]
        .iter()
        .map(|p| mk(p, w, FileKind::File, 1))
        .collect();
        s.upsert(&recs, g).unwrap();
        let under: std::collections::HashSet<_> = s
            .files_under(r"C:\t\b")
            .unwrap()
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(
            under,
            [r"C:\t\b\c", r"C:\t\b\d\e"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
        s.remove(r"C:\t\b").unwrap();
        assert_eq!(s.files_under(r"C:\t").unwrap().len(), 5);
        // drive root
        s.upsert(
            &[
                mk(r"D:\x.txt", w, FileKind::File, 3),
                mk(r"D:\sub\y.txt", w, FileKind::File, 4),
            ],
            g,
        )
        .unwrap();
        assert_eq!(s.summary(r"D:\").unwrap(), (2, 7));
    }

    #[test]
    fn upsert_keeps_hash_only_while_unchanged_and_rename_moves_children() {
        let u = PathStyle::UNIX;
        let (_d, s) = store(u);
        let g = s.begin_scan();
        let mut a = mk("/r/사진/a.jpg", u, FileKind::File, 10);
        s.upsert(
            &[
                mk("/r/사진", u, FileKind::Dir, 0),
                a.clone(),
                mk("/r/사진/2024", u, FileKind::Dir, 0),
                mk("/r/사진/2024/b.jpg", u, FileKind::File, 5),
            ],
            g,
        )
        .unwrap();
        s.set_hash(&a.path, "h1", a.modified, a.size).unwrap();
        s.upsert(&[a.clone()], g).unwrap();
        assert_eq!(
            s.record(&a.path).unwrap().unwrap().hash.as_deref(),
            Some("h1")
        );
        a.modified += 1;
        s.upsert(&[a.clone()], g).unwrap();
        assert_eq!(s.record(&a.path).unwrap().unwrap().hash, None);

        s.rename("/r/사진", "/r/보관/사진").unwrap();
        assert!(s.record("/r/보관/사진/a.jpg").unwrap().is_some());
        assert!(s.record("/r/보관/사진/2024/b.jpg").unwrap().is_some());
        assert_eq!(
            s.record("/r/보관/사진/2024/b.jpg").unwrap().unwrap().parent,
            "/r/보관/사진/2024"
        );
        assert!(s.record("/r/사진/a.jpg").unwrap().is_none());
        let names: Vec<_> = s
            .children("/r/보관/사진")
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, ["2024", "a.jpg"]);
    }

    #[test]
    fn finish_scan_keeps_unreadable_subtrees() {
        let u = PathStyle::UNIX;
        let (_d, s) = store(u);
        let g1 = s.begin_scan();
        s.upsert(
            &[
                mk("/r/a.txt", u, FileKind::File, 1),
                mk("/r/locked/b.txt", u, FileKind::File, 1),
            ],
            g1,
        )
        .unwrap();
        let g2 = s.begin_scan();
        s.finish_scan(&["/r".to_string()], g2, &["/r/locked".to_string()])
            .unwrap();
        assert!(s.record("/r/a.txt").unwrap().is_none());
        assert!(s.record("/r/locked/b.txt").unwrap().is_some());
    }

    #[test]
    fn search_is_case_insensitive_nfc_and_literal() {
        use unicode_normalization::UnicodeNormalization;
        let u = PathStyle::UNIX;
        let (_d, s) = store(u);
        let g = s.begin_scan();
        let nfd_name: String = "한글보고서.pdf".nfd().collect();
        s.upsert(
            &[
                mk("/r/a/b/c/report-2024.pdf", u, FileKind::File, 1),
                mk("/r/a/notes.txt", u, FileKind::File, 1),
                mk("/r/Report folder", u, FileKind::Dir, 0),
                mk(&format!("/r/{nfd_name}"), u, FileKind::File, 1),
                mk("/elsewhere/report.txt", u, FileKind::File, 1),
            ],
            g,
        )
        .unwrap();
        let roots = vec!["/r".to_string()];
        let res = s
            .search("REPORT", &roots, &Filter::default(), Sort::default(), 100)
            .unwrap();
        let names: Vec<_> = res.results.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Report folder", "report-2024.pdf"]);
        assert!(!res.truncated);
        assert!(s
            .search("100%", &roots, &Filter::default(), Sort::default(), 10)
            .unwrap()
            .results
            .is_empty());
        let hangul = s
            .search("한글", &roots, &Filter::default(), Sort::default(), 10)
            .unwrap();
        assert_eq!(hangul.results.len(), 1);
        let limited = s
            .search("r", &roots, &Filter::default(), Sort::default(), 1)
            .unwrap();
        assert!(limited.truncated);
        assert_eq!(limited.results.len(), 1);
        let docs_only = Filter {
            category: FileCategory::Document,
            ..Default::default()
        };
        let r = s
            .search("o", &roots, &docs_only, Sort::default(), 100)
            .unwrap();
        assert!(r
            .results
            .iter()
            .all(|e| e.kind == crate::types::Kind::Dir || e.ext == "pdf" || e.ext == "txt"));
    }

    #[test]
    fn list_children_dir_info_stats_and_states() {
        let u = PathStyle::UNIX;
        let (d, s) = store(u);
        let g = s.begin_scan();
        s.upsert(
            &[
                mk("/r", u, FileKind::Dir, 0),
                mk("/r/sub", u, FileKind::Dir, 0),
                mk("/r/sub/x.bin", u, FileKind::File, 100),
                mk("/r/b.txt", u, FileKind::File, 5),
                mk("/r/a.png", u, FileKind::File, 7),
            ],
            g,
        )
        .unwrap();
        let all = s
            .list_children("/r", &Filter::default(), Sort::default())
            .unwrap();
        let names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.png", "b.txt"]);
        let images = Filter {
            category: FileCategory::Image,
            ..Default::default()
        };
        let names: Vec<_> = s
            .list_children("/r", &images, Sort::default())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["sub", "a.png"]);
        let info = s
            .dir_info(&["/r".to_string(), "/r/sub".to_string()])
            .unwrap();
        assert_eq!((info[0].items, info[0].bytes), (3, 112));
        assert_eq!((info[1].items, info[1].bytes), (1, 100));
        let stats = s.stats(&["/r".to_string(), "/r/sub".to_string()]).unwrap();
        assert_eq!(
            stats,
            Stats {
                files: 3,
                folders: 1,
                bytes: 112
            }
        );
        let real = d.path().to_str().unwrap().to_string();
        let states = s
            .folder_states(
                &["/r".to_string(), format!("{real}/nope"), real.clone()],
                &[real.clone()],
            )
            .unwrap();
        // "/r" does not exist on disk → missing
        assert_eq!(states[0].state, FolderState::Missing);
        assert_eq!(states[1].state, FolderState::Missing);
        assert_eq!(states[2].state, FolderState::Scanning);
    }

    #[test]
    fn size_groups_skip_cloud_and_hard_links() {
        let u = PathStyle::UNIX;
        let (_d, s) = store(u);
        let g = s.begin_scan();
        let mut c = mk("/r/c.bin", u, FileKind::File, 10);
        c.cloud = true;
        let mut l1 = mk("/r/l1", u, FileKind::File, 20);
        l1.inode = 7;
        let mut l2 = mk("/r/l2", u, FileKind::File, 20);
        l2.inode = 7;
        s.upsert(
            &[
                mk("/r/a.bin", u, FileKind::File, 10),
                mk("/r/b.bin", u, FileKind::File, 10),
                c,
                l1,
                l2,
            ],
            g,
        )
        .unwrap();
        let groups = s.size_groups(1, None).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
        assert!(s.size_groups(1, Some("/elsewhere")).unwrap().is_empty());
    }

    #[test]
    fn remove_excluded_purges_rows_that_came_in_before() {
        let u = PathStyle::UNIX;
        let (_d, s) = store(u);
        let rules = ScanRules::unix("/Users/me", "");
        let g = s.begin_scan();
        s.upsert(
            &[
                mk("/Users/me/.Trash/a.mp4", u, FileKind::File, 10),
                mk("/Users/me/Downloads/a.mp4", u, FileKind::File, 10),
                mk("/Users/me/p/node_modules", u, FileKind::Dir, 0),
                mk("/Users/me/p/node_modules/b.mp4", u, FileKind::File, 10),
            ],
            g,
        )
        .unwrap();
        assert_eq!(s.remove_excluded(&rules).unwrap(), 3);
        assert!(s.record("/Users/me/Downloads/a.mp4").unwrap().is_some());
        assert!(s.record("/Users/me/.Trash/a.mp4").unwrap().is_none());
        // Windows rules on Windows-style rows
        let w = PathStyle::WINDOWS;
        let (_d2, sw) = store(w);
        let dirs = crate::rules::WindowsDirs {
            system_root: r"C:\Windows".into(),
            program_files: vec![r"C:\Program Files".into()],
            program_data: r"C:\ProgramData".into(),
            user_profile: r"C:\Users\me".into(),
            appdata: r"C:\Users\me\AppData\Roaming".into(),
            local_appdata: r"C:\Users\me\AppData\Local".into(),
            onedrive: vec![],
        };
        let wr = ScanRules::windows(&dirs, "");
        let g = sw.begin_scan();
        sw.upsert(
            &[
                mk(r"C:\Users\me\Downloads\x.zip", w, FileKind::File, 1),
                mk(r"C:\Users\me\Downloads\$RECYCLE.BIN", w, FileKind::Dir, 0),
                mk(r"D:\$Recycle.Bin\S-1\$R1.txt", w, FileKind::File, 1),
                mk(
                    r"C:\Users\me\AppData\Local\Temp\t.tmp",
                    w,
                    FileKind::File,
                    1,
                ),
            ],
            g,
        )
        .unwrap();
        assert_eq!(sw.remove_excluded(&wr).unwrap(), 3);
        assert!(sw.record(r"C:\Users\me\Downloads\x.zip").unwrap().is_some());
    }

    #[test]
    fn corrupt_database_is_set_aside() {
        let dir = tempfile::Builder::new()
            .prefix("mf-index")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join(DB_FILE), vec![0x42u8; 4096]).unwrap();
        assert!(IndexStore::open_in(dir.path()).is_err());
        let s = IndexStore::open_in_or_reset(dir.path()).unwrap();
        assert_eq!(s.max_op_id().unwrap(), 0);
        let moved = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(moved);
    }

    #[test]
    fn op_log_batches() {
        let (_d, s) = store(PathStyle::UNIX);
        let start = s.max_op_id().unwrap();
        let b = s.next_batch_id().unwrap();
        let id1 = s
            .log_op(OpKind::Move, "/a", "/b/a", Some(b), &Identity::default())
            .unwrap();
        let id2 = s
            .log_op(OpKind::Move, "/c", "/b/c", Some(b), &Identity::default())
            .unwrap();
        assert_eq!(s.next_batch_id().unwrap(), b + 1);
        let ops = s.last_undoable_batch(start).unwrap();
        assert_eq!(ops.iter().map(|o| o.id).collect::<Vec<_>>(), vec![id2, id1]);
        s.mark_undone(id1).unwrap();
        s.mark_undone(id2).unwrap();
        assert!(s.last_undoable_batch(start).unwrap().is_empty());
        assert!(s.mark_undone(9999).is_err());
    }
}
