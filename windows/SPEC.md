# Multi-Finder for Windows — build spec

Windows 10/11 (x64) port of Multi-Finder. The macOS app (Swift) in `../Sources` is the behavior
reference; its README (`../README.md`) describes the product. Same five screens:
**All Folders, Duplicates, Compare Folders, Categories, App Leftovers.**

Hard rules carried over from macOS (do not relax):
- Nothing is ever deleted permanently. Removal = Windows **Recycle Bin**, only after the user confirms.
- Every move/trash action is one undo batch; **Ctrl+Z** undoes the whole batch. Undo verifies the item
  is still the one we moved (size + modified time; inode on Linux) and never overwrites anything.
- Never overwrite: if a destination name exists, that item is skipped (reported in `OpResult.skipped`).
- Duplicates: at least one copy of every group always survives (enforced in the backend too).
- File contents never leave the PC. The AI model runs locally.
- UI languages: Korean, English, Japanese, Simplified Chinese, chosen from the Windows UI language;
  anything else → English.

## Stack and layout

Tauri v2 (Rust backend + WebView2 frontend). All paths below are relative to `windows/`.

```
windows/
  Cargo.toml            [workspace] members = ["core", "src-tauri"], resolver = "2"
  core/                 crate `mf-core` (lib) — ALL logic. No Tauri dependency. Unit tests run on Linux.
  src-tauri/            crate `multi-finder` (bin) — Tauri app: commands, events, settings, wiring
  package.json          Vite + React 18 + TypeScript frontend; scripts: dev, build, tauri
  index.html, src/      frontend
  src/api/types.ts      THE JSON CONTRACT (already written — follow it exactly)
  src/api/backend.ts    interface Backend + pick(): tauri impl if window.__TAURI_INTERNALS__, else mock
  src/api/mock.ts       mock backend with realistic fake data (browser dev + screenshots)
  src/api/tauri.ts      real backend (invoke/listen from @tauri-apps/api)
  src/i18n/             generated from ../Packaging/Localizable.xcstrings + extra.json (Windows-only strings)
  tools/                gen-i18n.mjs, smoke.ps1 (Windows CI smoke test), etc.
```

Data dir: `%LOCALAPPDATA%\com.taeholim.multifinder` on Windows, `$XDG_DATA_HOME/multi-finder` (or
`~/.local/share/multi-finder`) elsewhere; **env `MF_DATA_DIR` overrides** (tests, smoke runs).
Contents: `settings.json`, `index.sqlite`, `models\`.

## Backend commands (Tauri v2 `invoke`)

JS calls `invoke("name", { camelCaseArgs })`; Rust params are snake_case. All return
`Result<T, String>`. Types are from `src/api/types.ts`.

| command | args | returns | notes |
|---|---|---|---|
| `get_settings` | — | `Settings` | |
| `get_launch_args` | — | `LaunchArgs` | parsed from process args |
| `add_folders` | `paths: string[]` | `Settings` | canonicalize (dunce), dedupe, persist, scan new ones, restart watcher, allow asset scope |
| `remove_folder` | `path` | `Settings` | index rows may stay (cache) |
| `reorder_folders` | `folders: string[]` | `Settings` | |
| `folder_states` | — | `FolderStatus[]` | for "Not indexed"/"Scanning…" placeholders |
| `rescan` | `path?: string` | `void` | background; events `scan-progress`, `scan-finished`. No path = all folders |
| `is_scanning` | — | `boolean` | true also during startup before the first scan begins |
| `list_children` | `path, filter: Filter, sort: Sort` | `Entry[]` | direct children from the index |
| `dir_info` | `paths: string[]` | `DirInfo[]` | batch, for visible folder rows |
| `folder_summaries` | `paths: string[]` | `FolderSummary[]` | sidebar sizes |
| `stats` | — | `Stats` | totals over the user's folders |
| `search` | `text, filter, sort, limit` | `SearchResult` | names under all folders |
| `open_path` | `path` | `void` | default app |
| `reveal_paths` | `paths: string[]` | `void` | Explorer with the item(s) selected |
| `read_text_preview` | `path, maxBytes` | `string` | decoded text (UTF-8/UTF-16/CP949 guess) |
| `move_items` | `paths, destination` | `OpResult` | one batch |
| `move_across` | `paths, fromRoot, toRoot` | `OpResult` | keep relative paths (Compare "move to other side"); created folders are logged so undo removes them if empty |
| `trash_items` | `paths` | `OpResult` | Recycle Bin, one batch |
| `trash_duplicates` | `paths` | `OpResult` | like trash_items but refuses (skips) any file whose group would lose its last unchanged copy |
| `undo` | — | `UndoResult` | undo the newest not-yet-undone batch of THIS app session |
| `undo_state` | — | `UndoState` | |
| `find_duplicates` | `req: DuplicateRequest` | `DuplicateReport` | events `dup-progress`; waits for running scans first |
| `cancel_duplicates` | — | `void` | |
| `ignore_duplicate_group` | `id` | `void` | ignore list (persisted in index) |
| `clear_duplicate_ignores` | — | `void` | |
| `compare_folders` | `left, right` | `DiffResult` | rescans both first; events `compare-progress` |
| `get_buckets` | — | `Bucket[]` | defaults created in the UI language on first call |
| `save_buckets` | `buckets` | `Bucket[]` | fallback bucket forced last |
| `model_status` | — | `ModelStatus` | |
| `download_model` | — | `void` | events `model-progress`, `model-finished`; resumable; SHA-256 checked |
| `cancel_model_download` | — | `void` | |
| `classify` | `req: ClassifyRequest` | `void` | background; events `classify-progress`, `classify-finished` |
| `cancel_classify` | — | `void` | |
| `list_classified` | `scope, includeSubfolders` | `ClassifiedList` | |
| `set_bucket` | `paths, bucket` | `void` | manual = true |
| `scan_leftovers` | — | `LeftoverReport` | events `leftover-progress` |
| `trash_leftovers` | `paths` | `OpResult` | same executor + undo |

Events (payload types in types.ts): `scan-progress`, `scan-finished`, `index-changed`,
`dup-progress`, `compare-progress`, `classify-progress`, `classify-finished`, `model-progress`,
`model-finished`, `leftover-progress`.

## Core behavior to port (Swift file → Rust module)

| Swift (`../Sources/MultiFinderCore/…`) | Rust (`core/src/…`) | notes |
|---|---|---|
| ScanRules.swift | rules.rs | Windows exclusions below; hidden = name starts with "." OR FILE_ATTRIBUTE_HIDDEN/SYSTEM |
| FileRecord.swift | record.rs | `FileRecord` ⇄ `Entry` |
| IndexStore.swift | index.rs | SQLite (rusqlite, `bundled`), WAL, busy_timeout 5000, generations (`seen`), descendant ranges instead of LIKE (separator `\` → upper bound char `]`; on Linux `/` → `0`) |
| Scanner.swift | scanner.rs | serialized scans (global mutex); **if a root cannot be read, do NOT delete its rows** (keep the old index for it) |
| Watcher.swift | watcher.rs | `notify` (ReadDirectoryChangesW) + ~1.5 s debounce → update rows; scan folders that appear whole |
| Hasher.swift | hasher.rs | quick hash (first+last 128 KiB) then full hash; BLAKE3 |
| DuplicateFinder.swift | duplicates.rs | re-check disk state before hashing; skip cloud placeholders |
| DuplicateAnalyzer.swift, ProjectRoots.swift | analyzer.rs | hide within-project copies, fold copy trees, ignore list |
| FolderDiff.swift | diff.rs | unreadable files are never "identical" |
| Executor.swift | executor.rs | ops log (batch ids), move/trash/mkdir, undo per batch with verification |
| TextExtractor.swift | text.rs | txt/md/csv/code (64 KiB head; UTF-8/UTF-16 BOM; else CP949 via encoding_rs when invalid UTF-8), PDF (pdf-extract, wrapped in catch_unwind + size cap), docx/pptx/xlsx/hwpx (zip + quick-xml, ≤400 KB of XML), images: Windows OCR (`Windows.Media.Ocr`, cfg(windows), optional) |
| Classification.swift + ../MultiFinderApp/ClassifierEngine.swift + CategoriesModel.swift | classify.rs, model.rs | buckets, results table, prompt, local LLM (below) |
| LeftoverFinder.swift | leftovers.rs | rewritten for Windows (below) |

Windows scan exclusions (prefix, case-insensitive): `C:\Windows`, `C:\Program Files`,
`C:\Program Files (x86)`, `C:\ProgramData`, `X:\$Recycle.Bin`, `X:\System Volume Information`,
`%USERPROFILE%\AppData`, the data dir. Excluded directory names anywhere: `node_modules`, `.git`,
`.svn`, `.hg`, `__pycache__`, `.venv`, `venv`, `.cache`, `.npm`, `.nvm`, `$RECYCLE.BIN`.
Reparse points (symlinks/junctions) are recorded as `link` and never descended.
Path comparisons on Windows are case-insensitive; store paths exactly as the enumerator returns them
under a canonicalized root.

Cloud placeholders (OneDrive Files On-Demand): `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS (0x400000)`,
`FILE_ATTRIBUTE_RECALL_ON_OPEN (0x40000)`, or `FILE_ATTRIBUTE_OFFLINE (0x1000)` → `cloud = true`:
listed, never read or hashed.

Recycle Bin: `trash` crate (`delete_all`); undo uses `trash::os_limited::list()` +
`restore_all()` (Windows & Linux), matching original path and deletion time (newest match).

## Categories (local AI)

- Model: **Qwen3-0.6B Q8_0** (Apache-2.0), downloaded on first use into `models\`:
  `https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/13ac06624948e4d8589cb4b6f90100837342fcb8/Qwen3-0.6B-Q8_0.gguf`
  size **804753632**, SHA-256 **361cc68159042c36ebff7715dc5a2e4612153e88f3e9c9c234820849d6dc9e1d**.
  Download to `*.part` with HTTP Range resume, verify size + SHA-256, then rename. Use `ureq` (rustls).
- Inference: llama.cpp through the `llama-cpp-2` crate (CPU), behind cargo feature `llm` in mf-core.
  Context 2048, threads = physical cores (cap 8). Load the model once per classify run.
- Prompt: port `ClassifierEngine.Prompt.make` (Korean instructions for Korean UI, English otherwise
  + "write the summary in <language>"). Qwen3 chat format with thinking disabled:
  `<|im_start|>system\n…<|im_end|>\n<|im_start|>user\n…<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n`
- Output is constrained with a GBNF grammar to
  `{"category": <one of the exact bucket names>, "confidence": <0.0–1.0>, "summary": "<≤ 60 chars>"}`.
  Unknown name → closest bucket → fallback. Text given to the model ≤ ~1500 chars.
- Engine is behind a trait so tests run with a fake engine; the real one is tested only when env
  `MF_TEST_MODEL` points to a GGUF file.
- Default buckets (UI language): see CategoriesModel.swift (업무, 학교·공부, 개인, 개발, 디자인·미디어,
  설치·임시, 기타) — the installers hint on Windows mentions exe·msi instead of dmg·pkg.

## App Leftovers (Windows rules — conservative)

Only clear-cut items are reported; anything uncertain is NOT listed at all.
- Areas: direct children of `%APPDATA%` and `%LOCALAPPDATA%` (one level, plus one level under a
  vendor folder when the vendor folder itself matches no installed app but its child is clearly
  an app folder). Not ProgramData, not the registry, not `Packages`.
- Installed apps = registry Uninstall keys (HKLM, HKLM\WOW6432Node, HKCU): DisplayName, Publisher,
  InstallLocation leaf, DisplayIcon/UninstallString exe stem; plus running process names; plus
  folder names under `%LOCALAPPDATA%\Programs` and `Program Files*`. On Linux the provider is a fake
  for tests (trait).
- A folder is reported only if ALL hold: its normalized name matches none of the installed-app tokens
  (normalization: lowercase, drop spaces/punctuation/version numbers; token match both ways, min len 3);
  not on the protected list (Microsoft, Windows, Packages, Temp, Programs, Comms, ConnectedDevicesPlatform,
  D3DSCache, CrashDumps, Publishers, Google, Mozilla, Apple, Adobe, NVIDIA*, Intel, AMD, Realtek,
  pip, npm, npm-cache, Package Cache, … extend generously); newest file inside older than 30 days;
  contains no `.exe`/`.dll` (could be a portable app); size > 0.
- `reason` = one of a few i18n keys (Korean source strings) — add them to `src/i18n/extra.json`.

## Frontend (React 18 + TypeScript + Vite)

- Controls: `@fluentui/react-components` (v9) + `@fluentui/react-icons` → Windows 11 look.
  Theme follows the system (light/dark). Font: "Segoe UI Variable", "Segoe UI", system-ui.
- Layout (mirror the macOS screenshots in `../docs/screenshots` and the staged copies listed in the
  brief): left sidebar (View: All Folders, Duplicates, Compare Folders, Categories · Tidy Up: App Leftovers
  · Folders: user folders with sizes, "Add Folder…"), main pane with a toolbar row (title + actions +
  search box), status bar at the bottom ("44 files · 10 folders · 89.4 MB · Space: preview · Drag: move").
- Big lists use a custom virtualized row list (fixed row height) — the index can hold 30k+ files.
- All Folders: multi-root tree (expand/collapse persisted in localStorage), columns Name / Date Modified /
  Size (folders show "N items · size"), sort by clicking headers, filters (kind, size, date) in a toolbar
  menu, search → flat result list with path subtitles and "N results for 'x'" banner + Clear.
- Selection: click, Ctrl+click, Shift+click, Ctrl+A. Enter/double-click = open (folders toggle).
  Space = preview pane (images/PDF/video via `convertFileSrc`, text via `read_text_preview`).
  Delete = confirm → Recycle Bin. Ctrl+Z = undo. F5 = rescan. Ctrl+F = search. Ctrl+O = add folder.
  Ctrl+Shift+M = move to folder… (folder picker). Ctrl+Shift+R / context menu = Show in Explorer.
  Drag rows onto a folder row to move (no confirm, undoable — like macOS).
- Duplicates / Compare / Categories / Leftovers: same information and actions as the macOS views
  (`../Sources/MultiFinderApp/*View.swift`): scope/size/options in the toolbar, "Find"/"Find Again",
  progress, empty states, quick-select menu, per-group menus, bottom action bar.
  Duplicates must never let the user select every file of a group.
  Categories shows the model download state (download button + progress bar + size) before first use.
- i18n: `tools/gen-i18n.mjs` converts `../Packaging/Localizable.xcstrings` (keys = Korean source text;
  `%lld`, `%@`, `%1$@`… → `{0}`, `{1}`…; plural variations → `{ one, other }`) into
  `src/i18n/{ko,en,ja,zh-Hans}.json`; Windows-only strings live in `src/i18n/extra.json`
  (`{ "<korean key>": { "en": …, "ja": …, "zh-Hans": … } }`). `t(key, ...args)`; missing → key.
  Reuse existing keys wherever the macOS wording fits (Finder → Explorer, Trash → Recycle Bin need
  Windows-specific keys).
- Mock backend must exercise every screen with believable data so screenshots can be taken in a
  plain browser (`npm run dev`, Playwright).

## Testing

- `cargo test -p mf-core` on Linux: index ranges, scanner rules (Windows paths tested with pure
  string logic), duplicates + analyzer, diff, executor (move/trash/undo on Linux trash), text extraction
  fixtures, prompt/grammar building, leftover matching with a fake app provider.
- Frontend: `npm run build` (tsc + vite) clean; Playwright screenshots of every screen (ko + en, light + dark).
- Windows CI (`.github/workflows/windows.yml`): `cargo test` on Windows, `tauri build` (NSIS setup.exe +
  the raw exe), smoke run with `MF_DATA_DIR` + demo folders + screenshot, optional model test.
