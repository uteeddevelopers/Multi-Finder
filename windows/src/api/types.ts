// Multi-Finder for Windows — backend ⇄ UI contract.
// THE SINGLE SOURCE OF TRUTH for every JSON shape that crosses the Tauri bridge.
// Rust mirrors these with #[serde(rename_all = "camelCase")]. Optional fields (`?`) are
// omitted or null in JSON. Times are milliseconds since the Unix epoch (number). Sizes are bytes.
// Paths are absolute, native separators (Windows: `C:\Users\me\Downloads`, never `\\?\`),
// with the on-disk letter case (roots are canonicalized when added).

// ───────────────────────────── files ─────────────────────────────

export type Kind = "file" | "dir" | "link"; // link = symlink / junction (never descended)

export interface Entry {
  path: string;
  name: string;
  parent: string;
  ext: string; // lowercase, no dot, "" for none
  kind: Kind;
  size: number; // files: bytes. dirs: 0 (use dirInfo for totals)
  modified: number; // ms
  cloud: boolean; // OneDrive "online-only" placeholder: content not on disk, never hashed/read
}

export interface DirInfo {
  path: string;
  items: number; // direct children in the index (files + dirs)
  bytes: number; // recursive total of file sizes under it
}

export interface FolderSummary {
  path: string;
  files: number; // recursive file count
  bytes: number; // recursive bytes
}

export interface Stats {
  files: number;
  folders: number;
  bytes: number;
}

export type FolderState = "indexed" | "scanning" | "notIndexed" | "missing";

export interface FolderStatus {
  path: string;
  state: FolderState;
}

export interface Settings {
  folders: string[]; // the user's folders, in sidebar order
  locale: string; // OS UI locale, e.g. "ko-KR", "en-US", "ja-JP", "zh-CN"
  dataDir: string; // where index/settings/models live
  version: string; // app version, e.g. "1.0.0"
}

// Filter kinds — same set and extensions as the macOS app (FileFilter.swift), plus Windows installers.
export type FileCategory =
  | "all" | "document" | "image" | "video" | "audio" | "archive" | "installer" | "code" | "other";

export interface Filter {
  text: string; // case-insensitive name contains; "" = none
  category: FileCategory;
  minSize: number; // bytes; 0 = any (UI offers 0, 1MB, 10MB, 100MB, 1GB)
  withinDays: number; // 0 = any (UI offers 0, 1 = today, 7, 30, 365); counted from local midnight
}
// Directory entries always pass category/size/date (so the tree stays navigable); only `text` filters dirs.

export type SortField = "name" | "modified" | "size" | "kind";
export interface Sort {
  field: SortField;
  ascending: boolean;
} // folders always listed before files

export interface SearchResult {
  results: Entry[];
  truncated: boolean; // more than `limit` matches
}

// ───────────────────────────── operations (move / trash / undo) ─────────────────────────────

export interface OpFailure {
  path: string;
  reason: string; // short English reason for logs; UI shows a localized generic line + this detail
}

export interface OpResult {
  done: number;
  failed: OpFailure[];
  skipped: OpFailure[]; // e.g. destination exists (never overwrite), file changed since listed, last copy
  batch: number; // operation batch id (0 when nothing happened)
}

export interface UndoState {
  canUndo: boolean;
  kind?: "move" | "trash"; // what the next Ctrl+Z will undo
  count?: number; // how many items in that batch
}

export interface UndoResult {
  restored: number;
  missing: number; // item no longer where we put it (moved/deleted by someone else)
  blocked: number; // original location is occupied — not overwritten
}

// ───────────────────────────── duplicates ─────────────────────────────

export interface DuplicateRequest {
  scope?: string; // undefined = whole index; else only groups that include a file under this folder
  minSize: number; // bytes (≥ 1)
  hideWithinProject: boolean; // hide copies inside the same project (git repo etc.)
  foldCopyTrees: boolean; // fold whole-folder copies into one row
}

export interface FileGroup {
  id: string; // content hash
  size: number; // size of ONE file
  files: Entry[]; // ≥ 2, sorted by path
}

export interface CopyTree {
  id: string;
  left: string; // folder A
  right: string; // folder B
  groups: FileGroup[]; // the identical files shared by A and B
  fileCount: number; // number of identical file pairs
  bytes: number; // bytes reclaimable by removing one side
  examples: string[]; // up to 3 file names
}

export interface DuplicateReport {
  fileGroups: FileGroup[];
  copyTrees: CopyTree[];
  rawGroupCount: number;
  hiddenWithinProject: number;
  hiddenByIgnoreList: number;
  reclaimableBytes: number; // keep one per group
}

// ───────────────────────────── compare folders ─────────────────────────────

export type DiffStatus = "identical" | "different" | "leftOnly" | "rightOnly" | "moved";

export interface DiffSide {
  path: string;
  size: number;
  modified: number;
}

export interface DiffEntry {
  id: string; // stable within one result
  relPath: string; // path relative to its root ("photos\\IMG_1.jpg"); for moved: the left-side relative path
  name: string;
  status: DiffStatus;
  left?: DiffSide;
  right?: DiffSide;
  otherRelPath?: string; // moved: where the same content sits on the other side (relative)
}

export interface DiffCounts {
  identical: number;
  different: number;
  leftOnly: number;
  rightOnly: number;
  moved: number;
}

export interface DiffResult {
  left: string;
  right: string;
  entries: DiffEntry[];
  counts: DiffCounts;
}

// ───────────────────────────── categories (on-device AI) ─────────────────────────────

export interface Bucket {
  id: string;
  name: string;
  hint: string; // what belongs here — given to the model
  isFallback: boolean; // the "Other" bucket: always last, cannot be removed
}

export interface Classified {
  path: string;
  bucket: string; // bucket name
  confidence: number; // 0..1
  summary: string; // one short line in the UI language
  manual: boolean; // user moved it by hand → never overwritten by re-classify
}

export interface ClassifiedList {
  results: Classified[];
  pending: number; // files in scope not classified yet
}

export type ModelState = "missing" | "downloading" | "ready" | "error";

export interface ModelStatus {
  state: ModelState;
  name: string; // "Qwen3 0.6B (Q8_0)"
  bytes: number; // downloaded so far / size on disk
  totalBytes: number; // 804753632
  error?: string;
}

export interface ClassifyRequest {
  scope: string; // one of the user's folders
  includeSubfolders: boolean;
  mode: "new" | "all"; // all = re-classify everything except manual choices
}

// ───────────────────────────── app leftovers (Windows) ─────────────────────────────

export interface LeftoverPath {
  path: string;
  bytes: number;
  lastModified: number; // newest modification time inside (ms)
}

export interface LeftoverItem {
  id: string;
  app: string; // best display name, e.g. "Zoom"
  paths: LeftoverPath[];
  bytes: number;
  reason: string; // i18n key (Korean source string) explaining why it is considered left over
}

export interface LeftoverReport {
  items: LeftoverItem[]; // only clear-cut leftovers; anything uncertain is not listed
  scannedAreas: string[]; // e.g. %APPDATA%, %LOCALAPPDATA%
}

// ───────────────────────────── launch arguments (dev / smoke tests) ─────────────────────────────

export type Screen = "collection" | "duplicates" | "compare" | "categories" | "leftovers";

export interface LaunchArgs {
  screen?: Screen; // --screen
  run: boolean; // --run : start the screen's main action
  left?: string; // --left
  right?: string; // --right
  search?: string; // --search
}

// ───────────────────────────── events (backend → UI, via Tauri `listen`) ─────────────────────────────

export interface ScanProgressEvent { root: string; scanned: number; current: string } // "scan-progress"
export interface ScanFinishedEvent { root: string; error?: string } // "scan-finished"
export interface IndexChangedEvent { paths: string[] } // "index-changed" (file watcher; UI drops caches)
export interface ProgressEvent { phase: string; done: number; total: number; current?: string }
// "dup-progress" (phase "size"|"hash"), "compare-progress" (phase "scan"|"hash"),
// "classify-progress" (phase "classify"), "leftover-progress" (phase "scan")
export interface ClassifyFinishedEvent { classified: number; failed: number; cancelled: boolean } // "classify-finished"
export interface ModelProgressEvent { bytes: number; totalBytes: number } // "model-progress"
export interface ModelFinishedEvent { ok: boolean; error?: string } // "model-finished"
