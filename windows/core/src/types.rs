//! Serde mirrors of `windows/src/api/types.ts` — THE JSON contract with the UI.
//!
//! Field names are camelCase, string enums are spelled exactly as in the TS file,
//! optional fields are omitted when `None`, times are milliseconds since the Unix epoch,
//! sizes are bytes.

use serde::{Deserialize, Serialize};

// ───────────────────────────── files ─────────────────────────────

/// `"file" | "dir" | "link"` — link = symlink / junction (never descended).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Dir,
    Link,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub parent: String,
    /// lowercase, no dot, "" for none
    pub ext: String,
    pub kind: Kind,
    /// files: bytes. dirs: 0 (use dir_info for totals)
    pub size: u64,
    /// ms since epoch
    pub modified: i64,
    /// OneDrive "online-only" placeholder: content not on disk, never hashed/read
    pub cloud: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DirInfo {
    pub path: String,
    /// direct children in the index (files + dirs)
    pub items: u64,
    /// recursive total of file sizes under it
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderSummary {
    pub path: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FolderState {
    Indexed,
    Scanning,
    NotIndexed,
    Missing,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderStatus {
    pub path: String,
    pub state: FolderState,
}

/// What `get_settings` returns. The persisted part is [`crate::settings::Settings`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub folders: Vec<String>,
    /// OS UI locale, e.g. "ko-KR"
    pub locale: String,
    pub data_dir: String,
    pub version: String,
}

/// Filter kinds — same set and extensions as the macOS app, plus Windows installers.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileCategory {
    #[default]
    All,
    Document,
    Image,
    Video,
    Audio,
    Archive,
    Installer,
    Code,
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    /// case-insensitive name contains; "" = none
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub category: FileCategory,
    /// bytes; 0 = any
    #[serde(default)]
    pub min_size: u64,
    /// 0 = any; 1 = today; counted from local midnight
    #[serde(default)]
    pub within_days: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortField {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub field: SortField,
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            field: SortField::Name,
            ascending: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub results: Vec<Entry>,
    /// more than `limit` matches
    pub truncated: bool,
}

// ───────────────────────────── operations ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpFailure {
    pub path: String,
    /// short English reason for logs
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    pub done: u64,
    pub failed: Vec<OpFailure>,
    /// e.g. destination exists (never overwrite), changed since listed, last copy
    pub skipped: Vec<OpFailure>,
    /// operation batch id (0 when nothing happened)
    pub batch: i64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UndoKind {
    Move,
    Trash,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UndoState {
    pub can_undo: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kind: Option<UndoKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub count: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub restored: u64,
    /// item no longer where we put it
    pub missing: u64,
    /// original location is occupied — not overwritten
    pub blocked: u64,
}

// ───────────────────────────── duplicates ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateRequest {
    /// None = whole index; else only groups that include a file under this folder
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scope: Option<String>,
    pub min_size: u64,
    pub hide_within_project: bool,
    pub fold_copy_trees: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileGroup {
    /// content hash
    pub id: String,
    /// size of ONE file
    pub size: u64,
    /// ≥ 2, sorted by path
    pub files: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CopyTree {
    pub id: String,
    pub left: String,
    pub right: String,
    pub groups: Vec<FileGroup>,
    pub file_count: u64,
    pub bytes: u64,
    pub examples: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateReport {
    pub file_groups: Vec<FileGroup>,
    pub copy_trees: Vec<CopyTree>,
    pub raw_group_count: u64,
    pub hidden_within_project: u64,
    pub hidden_by_ignore_list: u64,
    /// keep one per group
    pub reclaimable_bytes: u64,
}

// ───────────────────────────── compare folders ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum DiffStatus {
    Identical,
    Different,
    LeftOnly,
    RightOnly,
    Moved,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffSide {
    pub path: String,
    pub size: u64,
    pub modified: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub id: String,
    /// relative to its root; for moved: the left-side relative path
    pub rel_path: String,
    pub name: String,
    pub status: DiffStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub left: Option<DiffSide>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub right: Option<DiffSide>,
    /// moved: where the same content sits on the other side (relative)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub other_rel_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffCounts {
    pub identical: u64,
    pub different: u64,
    pub left_only: u64,
    pub right_only: u64,
    pub moved: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub left: String,
    pub right: String,
    pub entries: Vec<DiffEntry>,
    pub counts: DiffCounts,
}

// ───────────────────────────── categories ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// what belongs here — given to the model
    #[serde(default)]
    pub hint: String,
    /// the "Other" bucket: always last, cannot be removed
    #[serde(default)]
    pub is_fallback: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Classified {
    pub path: String,
    /// bucket name
    pub bucket: String,
    pub confidence: f64,
    pub summary: String,
    pub manual: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClassifiedList {
    pub results: Vec<Classified>,
    /// files in scope not classified yet
    pub pending: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Missing,
    Downloading,
    Ready,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub state: ModelState,
    pub name: String,
    pub bytes: u64,
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClassifyMode {
    /// only files not classified yet
    #[default]
    New,
    /// re-classify everything except manual choices
    All,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyRequest {
    pub scope: String,
    pub include_subfolders: bool,
    pub mode: ClassifyMode,
}

// ───────────────────────────── app leftovers ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverPath {
    pub path: String,
    pub bytes: u64,
    /// newest modification time inside (ms)
    pub last_modified: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverItem {
    pub id: String,
    pub app: String,
    pub paths: Vec<LeftoverPath>,
    pub bytes: u64,
    /// i18n key (Korean source string)
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverReport {
    pub items: Vec<LeftoverItem>,
    pub scanned_areas: Vec<String>,
}

// ───────────────────────────── launch arguments ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Screen {
    Collection,
    Duplicates,
    Compare,
    Categories,
    Leftovers,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LaunchArgs {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub screen: Option<Screen>,
    pub run: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub search: Option<String>,
}

impl LaunchArgs {
    /// `--screen duplicates|compare|categories|leftovers|collection [--run] [--left P --right P] [--search Q]`
    /// (process arguments; the program name may be included — unknown arguments are ignored).
    pub fn parse<S: AsRef<str>>(args: &[S]) -> LaunchArgs {
        let args: Vec<&str> = args.iter().map(|a| a.as_ref()).collect();
        let value = |flag: &str| -> Option<String> {
            let i = args.iter().position(|a| *a == flag)?;
            args.get(i + 1)
                .filter(|v| !v.starts_with("--"))
                .map(|v| v.to_string())
        };
        let screen = value("--screen").and_then(|s| match s.as_str() {
            "collection" | "all" => Some(Screen::Collection),
            "duplicates" => Some(Screen::Duplicates),
            "compare" => Some(Screen::Compare),
            "categories" => Some(Screen::Categories),
            "leftovers" => Some(Screen::Leftovers),
            _ => None,
        });
        LaunchArgs {
            screen,
            run: args.contains(&"--run"),
            left: value("--left"),
            right: value("--right"),
            search: value("--search"),
        }
    }
}

// ───────────────────────────── events ─────────────────────────────

/// `scan-progress`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressEvent {
    pub root: String,
    pub scanned: u64,
    pub current: String,
}

/// `scan-finished`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanFinishedEvent {
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// `index-changed`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexChangedEvent {
    pub paths: Vec<String>,
}

/// `dup-progress` (phase "size"|"hash"), `compare-progress` ("scan"|"hash"),
/// `classify-progress` ("classify"), `leftover-progress` ("scan")
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub phase: String,
    pub done: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current: Option<String>,
}

impl ProgressEvent {
    pub fn new(phase: &str, done: u64, total: u64, current: Option<String>) -> Self {
        ProgressEvent {
            phase: phase.to_string(),
            done,
            total,
            current,
        }
    }
}

/// `classify-finished`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyFinishedEvent {
    pub classified: u64,
    pub failed: u64,
    pub cancelled: bool,
}

/// `model-progress`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelProgressEvent {
    pub bytes: u64,
    pub total_bytes: u64,
}

/// `model-finished`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelFinishedEvent {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
        k.sort();
        k
    }

    fn sorted(list: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    fn entry() -> Entry {
        Entry {
            path: r"C:\a\b.txt".into(),
            name: "b.txt".into(),
            parent: r"C:\a".into(),
            ext: "txt".into(),
            kind: Kind::File,
            size: 3,
            modified: 1_700_000_000_000,
            cloud: false,
        }
    }

    #[test]
    fn serializes_every_type_with_contract_key_names() {
        let e = serde_json::to_value(entry()).unwrap();
        assert_eq!(
            keys(&e),
            sorted(&["path", "name", "parent", "ext", "kind", "size", "modified", "cloud"])
        );
        assert_eq!(e["kind"], "file");
        assert_eq!(serde_json::to_value(Kind::Dir).unwrap(), "dir");
        assert_eq!(serde_json::to_value(Kind::Link).unwrap(), "link");

        let d = serde_json::to_value(DirInfo {
            path: "p".into(),
            items: 1,
            bytes: 2,
        })
        .unwrap();
        assert_eq!(keys(&d), sorted(&["path", "items", "bytes"]));
        let s = serde_json::to_value(FolderSummary {
            path: "p".into(),
            files: 1,
            bytes: 2,
        })
        .unwrap();
        assert_eq!(keys(&s), sorted(&["path", "files", "bytes"]));
        let st = serde_json::to_value(Stats::default()).unwrap();
        assert_eq!(keys(&st), sorted(&["files", "folders", "bytes"]));

        for (state, text) in [
            (FolderState::Indexed, "indexed"),
            (FolderState::Scanning, "scanning"),
            (FolderState::NotIndexed, "notIndexed"),
            (FolderState::Missing, "missing"),
        ] {
            let v = serde_json::to_value(FolderStatus {
                path: "p".into(),
                state,
            })
            .unwrap();
            assert_eq!(keys(&v), sorted(&["path", "state"]));
            assert_eq!(v["state"], text);
        }

        let set = serde_json::to_value(Settings {
            folders: vec![],
            locale: "ko-KR".into(),
            data_dir: "d".into(),
            version: "1.0.0".into(),
        })
        .unwrap();
        assert_eq!(
            keys(&set),
            sorted(&["folders", "locale", "dataDir", "version"])
        );

        let f = serde_json::to_value(Filter {
            text: "x".into(),
            category: FileCategory::Installer,
            min_size: 1,
            within_days: 7,
        })
        .unwrap();
        assert_eq!(
            keys(&f),
            sorted(&["text", "category", "minSize", "withinDays"])
        );
        assert_eq!(f["category"], "installer");
        let parsed: Filter = serde_json::from_value(
            json!({"text":"","category":"document","minSize":0,"withinDays":0}),
        )
        .unwrap();
        assert_eq!(parsed.category, FileCategory::Document);

        let sort = serde_json::to_value(Sort {
            field: SortField::Modified,
            ascending: false,
        })
        .unwrap();
        assert_eq!(keys(&sort), sorted(&["field", "ascending"]));
        assert_eq!(sort["field"], "modified");
        assert_eq!(serde_json::to_value(SortField::Kind).unwrap(), "kind");

        let sr = serde_json::to_value(SearchResult {
            results: vec![entry()],
            truncated: true,
        })
        .unwrap();
        assert_eq!(keys(&sr), sorted(&["results", "truncated"]));

        let op = serde_json::to_value(OpResult {
            done: 1,
            failed: vec![OpFailure {
                path: "p".into(),
                reason: "r".into(),
            }],
            skipped: vec![],
            batch: 3,
        })
        .unwrap();
        assert_eq!(keys(&op), sorted(&["done", "failed", "skipped", "batch"]));
        assert_eq!(keys(&op["failed"][0]), sorted(&["path", "reason"]));

        let us = serde_json::to_value(UndoState {
            can_undo: false,
            kind: None,
            count: None,
        })
        .unwrap();
        assert_eq!(keys(&us), sorted(&["canUndo"]));
        let us = serde_json::to_value(UndoState {
            can_undo: true,
            kind: Some(UndoKind::Trash),
            count: Some(2),
        })
        .unwrap();
        assert_eq!(keys(&us), sorted(&["canUndo", "kind", "count"]));
        assert_eq!(us["kind"], "trash");
        let ur = serde_json::to_value(UndoResult::default()).unwrap();
        assert_eq!(keys(&ur), sorted(&["restored", "missing", "blocked"]));

        let req: DuplicateRequest = serde_json::from_value(
            json!({"minSize": 1, "hideWithinProject": true, "foldCopyTrees": false}),
        )
        .unwrap();
        assert_eq!(req.scope, None);
        let req = serde_json::to_value(DuplicateRequest {
            scope: Some("s".into()),
            min_size: 1,
            hide_within_project: true,
            fold_copy_trees: true,
        })
        .unwrap();
        assert_eq!(
            keys(&req),
            sorted(&["scope", "minSize", "hideWithinProject", "foldCopyTrees"])
        );

        let g = FileGroup {
            id: "h".into(),
            size: 3,
            files: vec![entry(), entry()],
        };
        assert_eq!(
            keys(&serde_json::to_value(&g).unwrap()),
            sorted(&["id", "size", "files"])
        );
        let t = serde_json::to_value(CopyTree {
            id: "t".into(),
            left: "l".into(),
            right: "r".into(),
            groups: vec![g.clone()],
            file_count: 1,
            bytes: 3,
            examples: vec!["b.txt".into()],
        })
        .unwrap();
        assert_eq!(
            keys(&t),
            sorted(&[
                "id",
                "left",
                "right",
                "groups",
                "fileCount",
                "bytes",
                "examples"
            ])
        );
        let rep = serde_json::to_value(DuplicateReport::default()).unwrap();
        assert_eq!(
            keys(&rep),
            sorted(&[
                "fileGroups",
                "copyTrees",
                "rawGroupCount",
                "hiddenWithinProject",
                "hiddenByIgnoreList",
                "reclaimableBytes"
            ])
        );

        for (status, text) in [
            (DiffStatus::Identical, "identical"),
            (DiffStatus::Different, "different"),
            (DiffStatus::LeftOnly, "leftOnly"),
            (DiffStatus::RightOnly, "rightOnly"),
            (DiffStatus::Moved, "moved"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), text);
        }
        let side = DiffSide {
            path: "p".into(),
            size: 1,
            modified: 2,
        };
        assert_eq!(
            keys(&serde_json::to_value(&side).unwrap()),
            sorted(&["path", "size", "modified"])
        );
        let de = serde_json::to_value(DiffEntry {
            id: "i".into(),
            rel_path: "a".into(),
            name: "a".into(),
            status: DiffStatus::Moved,
            left: Some(side.clone()),
            right: Some(side.clone()),
            other_rel_path: Some("b".into()),
        })
        .unwrap();
        assert_eq!(
            keys(&de),
            sorted(&[
                "id",
                "relPath",
                "name",
                "status",
                "left",
                "right",
                "otherRelPath"
            ])
        );
        let de = serde_json::to_value(DiffEntry {
            id: "i".into(),
            rel_path: "a".into(),
            name: "a".into(),
            status: DiffStatus::LeftOnly,
            left: Some(side),
            right: None,
            other_rel_path: None,
        })
        .unwrap();
        assert_eq!(
            keys(&de),
            sorted(&["id", "relPath", "name", "status", "left"])
        );
        let dr = serde_json::to_value(DiffResult {
            left: "l".into(),
            right: "r".into(),
            entries: vec![],
            counts: DiffCounts::default(),
        })
        .unwrap();
        assert_eq!(keys(&dr), sorted(&["left", "right", "entries", "counts"]));
        assert_eq!(
            keys(&dr["counts"]),
            sorted(&["identical", "different", "leftOnly", "rightOnly", "moved"])
        );

        let b = serde_json::to_value(Bucket {
            id: "x".into(),
            name: "n".into(),
            hint: "h".into(),
            is_fallback: true,
        })
        .unwrap();
        assert_eq!(keys(&b), sorted(&["id", "name", "hint", "isFallback"]));
        let c = Classified {
            path: "p".into(),
            bucket: "b".into(),
            confidence: 0.5,
            summary: "s".into(),
            manual: false,
        };
        assert_eq!(
            keys(&serde_json::to_value(&c).unwrap()),
            sorted(&["path", "bucket", "confidence", "summary", "manual"])
        );
        let cl = serde_json::to_value(ClassifiedList {
            results: vec![c],
            pending: 2,
        })
        .unwrap();
        assert_eq!(keys(&cl), sorted(&["results", "pending"]));

        let ms = serde_json::to_value(ModelStatus {
            state: ModelState::Downloading,
            name: "Qwen3 0.6B (Q8_0)".into(),
            bytes: 1,
            total_bytes: 2,
            error: None,
        })
        .unwrap();
        assert_eq!(keys(&ms), sorted(&["state", "name", "bytes", "totalBytes"]));
        assert_eq!(ms["state"], "downloading");
        for (s, t) in [
            (ModelState::Missing, "missing"),
            (ModelState::Ready, "ready"),
            (ModelState::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), t);
        }
        let cr: ClassifyRequest =
            serde_json::from_value(json!({"scope":"s","includeSubfolders":true,"mode":"all"}))
                .unwrap();
        assert_eq!(cr.mode, ClassifyMode::All);
        assert_eq!(serde_json::to_value(ClassifyMode::New).unwrap(), "new");

        let lp = LeftoverPath {
            path: "p".into(),
            bytes: 1,
            last_modified: 2,
        };
        assert_eq!(
            keys(&serde_json::to_value(&lp).unwrap()),
            sorted(&["path", "bytes", "lastModified"])
        );
        let li = LeftoverItem {
            id: "i".into(),
            app: "Zoom".into(),
            paths: vec![lp],
            bytes: 1,
            reason: "r".into(),
        };
        assert_eq!(
            keys(&serde_json::to_value(&li).unwrap()),
            sorted(&["id", "app", "paths", "bytes", "reason"])
        );
        let lr = serde_json::to_value(LeftoverReport {
            items: vec![li],
            scanned_areas: vec!["%APPDATA%".into()],
        })
        .unwrap();
        assert_eq!(keys(&lr), sorted(&["items", "scannedAreas"]));

        let la = serde_json::to_value(LaunchArgs::default()).unwrap();
        assert_eq!(keys(&la), sorted(&["run"]));
        assert_eq!(
            serde_json::to_value(Screen::Leftovers).unwrap(),
            "leftovers"
        );

        let ev = serde_json::to_value(ScanProgressEvent {
            root: "r".into(),
            scanned: 1,
            current: "c".into(),
        })
        .unwrap();
        assert_eq!(keys(&ev), sorted(&["root", "scanned", "current"]));
        let ev = serde_json::to_value(ScanFinishedEvent {
            root: "r".into(),
            error: None,
        })
        .unwrap();
        assert_eq!(keys(&ev), sorted(&["root"]));
        let ev = serde_json::to_value(IndexChangedEvent { paths: vec![] }).unwrap();
        assert_eq!(keys(&ev), sorted(&["paths"]));
        let ev = serde_json::to_value(ProgressEvent::new("hash", 1, 2, Some("x".into()))).unwrap();
        assert_eq!(keys(&ev), sorted(&["phase", "done", "total", "current"]));
        let ev = serde_json::to_value(ClassifyFinishedEvent::default()).unwrap();
        assert_eq!(keys(&ev), sorted(&["classified", "failed", "cancelled"]));
        let ev = serde_json::to_value(ModelProgressEvent {
            bytes: 1,
            total_bytes: 2,
        })
        .unwrap();
        assert_eq!(keys(&ev), sorted(&["bytes", "totalBytes"]));
        let ev = serde_json::to_value(ModelFinishedEvent {
            ok: false,
            error: Some("e".into()),
        })
        .unwrap();
        assert_eq!(keys(&ev), sorted(&["ok", "error"]));
    }

    #[test]
    fn parses_launch_args() {
        let a = LaunchArgs::parse(&[
            "app.exe", "--screen", "compare", "--left", r"C:\a", "--right", r"D:\b", "--run",
        ]);
        assert_eq!(a.screen, Some(Screen::Compare));
        assert!(a.run);
        assert_eq!(a.left.as_deref(), Some(r"C:\a"));
        assert_eq!(a.right.as_deref(), Some(r"D:\b"));
        assert_eq!(a.search, None);
        let b = LaunchArgs::parse(&["--search", "report", "--screen", "nonsense"]);
        assert_eq!(b.screen, None);
        assert!(!b.run);
        assert_eq!(b.search.as_deref(), Some("report"));
    }
}
