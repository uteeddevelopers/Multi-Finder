// The single owner of UI state (port of the macOS AppModel). Screens read it through useApp() and
// call its methods; every mutation ends in changed(), which re-renders subscribers once per tick.
import type { Backend } from "../api/backend";
import type {
  DirInfo,
  Entry,
  Filter,
  FolderState,
  FolderSummary,
  LaunchArgs,
  OpFailure,
  OpResult,
  Screen,
  Settings,
  Sort,
  SortField,
  Stats,
  UndoState,
} from "../api/types";
import { t, type MsgKey } from "../i18n";
import { emptySel, type Sel } from "../util/selection";
import { basename, dirname, isInside, isUnderOrSame, rootOf, samePath } from "../util/paths";
import { DuplicatesModel } from "./duplicates";
import { CompareModel } from "./compare";
import { CategoriesModel } from "./categories";
import { LeftoversModel } from "./leftovers";

export type TrashKind = "items" | "duplicates" | "leftovers";

export interface TrashRequest {
  paths: string[];
  kind: TrashKind;
  knownBytes?: number;
  note?: string;
}

export interface MoveRequest {
  title: string;
  paths: string[];
  fromRoot: string;
  toRoot: string;
  folderName: string;
}

export interface AlertState {
  title: string;
  message: string;
  action?: { label: string; run: () => void };
}

export type TreeRow =
  | { type: "root"; key: string; path: string; depth: 0; state: FolderState; expanded: boolean; busy: boolean }
  | { type: "entry"; key: string; entry: Entry; depth: number; expanded: boolean }
  | { type: "placeholder"; key: string; depth: number; text: MsgKey };

const SEARCH_LIMIT = 10_000;
const LS_EXPANDED = "mf.expanded";
const LS_SORT = "mf.sort";

function readLS<T>(key: string): T | undefined {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : undefined;
  } catch {
    return undefined;
  }
}
function writeLS(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage unavailable: keep it in memory only */
  }
}

const fkey = (p: string) => p.toLowerCase();

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  return typeof e === "string" ? e : JSON.stringify(e);
}

/** "‘a’, ‘b’, ‘c’ and 2 more" */
export function nameList(paths: string[], max = 3): string {
  const names = paths.slice(0, max).map((p) => `‘${basename(p)}’`).join(", ");
  return paths.length > max ? `${names} ${t("외 {0}개", paths.length - max)}` : names;
}

export class AppModel {
  readonly backend: Backend;
  settings: Settings;
  screen: Screen = "collection";

  // ── All Folders ──
  expanded: Set<string>;
  sel: Sel = emptySel();
  filter: Filter = { text: "", category: "all", minSize: 0, withinDays: 0 };
  sort: Sort;
  searchResults: Entry[] = [];
  searchTruncated = false;
  searchPending = false;
  previewOpen = false;
  scrollTo: { key: string; nonce: number } | null = null;
  focusSearchNonce = 0;
  /** Bumps whenever the index may have changed — cached lists are refetched (stale data stays on screen meanwhile). */
  gen = 0;

  // ── status ──
  folderState = new Map<string, FolderState>();
  scanning = new Set<string>();
  startupPending = true;
  progressText = "";
  stats: Stats | null = null;
  summaries = new Map<string, FolderSummary>();
  undoState: UndoState = { canUndo: false };
  lastSummary: string | null = null;

  // ── dialogs ──
  alert: AlertState | null = null;
  trashRequest: TrashRequest | null = null;
  moveRequest: MoveRequest | null = null;

  readonly duplicates: DuplicatesModel;
  readonly compare: CompareModel;
  readonly categories: CategoriesModel;
  readonly leftovers: LeftoversModel;

  private version = 0;
  private listeners = new Set<() => void>();
  private notifyQueued = false;
  private children = new Map<string, { key: string; gen: number; seq: number; entries: Entry[] }>();
  private childrenInflight = new Set<string>();
  private requestSeq = 0;
  private dirInfos = new Map<string, { gen: number; info: DirInfo }>();
  private dirInfoQueue = new Set<string>();
  private dirInfoInflight = new Set<string>();
  private dirInfoTimer: ReturnType<typeof setTimeout> | null = null;
  private entries = new Map<string, Entry>();
  /** Bumped whenever something the tree rows depend on changes (children, expansion, folder states). */
  private treeRev = 0;
  private treeCache: { sig: string; value: { rows: TreeRow[]; need: string[] } } | null = null;
  private searchTimer: ReturnType<typeof setTimeout> | null = null;
  private searchSeq = 0;
  private statsLoading = false;
  private summaryTimer: ReturnType<typeof setTimeout> | null = null;
  private pollTimer: ReturnType<typeof setTimeout> | null = null;
  private invalidateTimer: ReturnType<typeof setTimeout> | null = null;
  private lastSummaryTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(backend: Backend, settings: Settings) {
    this.backend = backend;
    this.settings = settings;
    const stored = readLS<string[]>(LS_EXPANDED);
    this.expanded = new Set(stored ?? settings.folders);
    const sort = readLS<Sort>(LS_SORT);
    this.sort = sort && ["name", "modified", "size", "kind"].includes(sort.field) ? sort : { field: "name", ascending: true };
    this.duplicates = new DuplicatesModel(this);
    this.compare = new CompareModel(this);
    this.categories = new CategoriesModel(this);
    this.leftovers = new LeftoversModel(this);
  }

  // ───────────── subscription ─────────────

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  getVersion = (): number => this.version;

  /** Schedules one re-render for everything that changed in this tick. */
  changed() {
    this.version++;
    if (this.notifyQueued) return;
    this.notifyQueued = true;
    queueMicrotask(() => {
      this.notifyQueued = false;
      for (const fn of this.listeners) fn();
    });
  }

  // ───────────── startup ─────────────

  async start(launch: LaunchArgs) {
    const b = this.backend;
    b.on("scan-progress", (e) => {
      this.scanning.add(fkey(e.root));
      this.folderState.set(fkey(e.root), "scanning");
      this.treeRev++;
      this.progressText = t("{0}개 … {1}", e.scanned, e.current);
      this.changed();
    });
    b.on("scan-finished", (e) => {
      this.scanning.delete(fkey(e.root));
      this.treeRev++;
      if (this.scanning.size === 0) this.progressText = "";
      if (e.error) this.showError(`${basename(e.root)}: ${e.error}`);
      void this.refreshFolderStates();
      this.invalidate();
      if (!this.isScanning) this.duplicates.resumeAfterScan();
    });
    b.on("index-changed", () => this.invalidateSoon());
    b.on("dup-progress", (e) => this.duplicates.onProgress(e));
    b.on("compare-progress", (e) => this.compare.onProgress(e));
    b.on("classify-progress", (e) => this.categories.onProgress(e));
    b.on("classify-finished", (e) => this.categories.onFinished(e));
    b.on("model-progress", (e) => this.categories.onModelProgress(e));
    b.on("model-finished", (e) => this.categories.onModelFinished(e));
    b.on("leftover-progress", (e) => this.leftovers.onProgress(e));
    b.onFileDrop((paths) => void this.addFolderPaths(paths));

    try {
      this.startupPending = await b.isScanning();
    } catch {
      this.startupPending = false;
    }
    await Promise.allSettled([this.refreshFolderStates(), this.refreshUndo(), this.loadSummaries(), this.categories.load()]);
    this.changed();
    this.pollScanning();
    this.applyLaunchArgs(launch);
  }

  private applyLaunchArgs(a: LaunchArgs) {
    if (a.screen) this.screen = a.screen;
    if (a.left) this.compare.setSide("left", a.left);
    if (a.right) this.compare.setSide("right", a.right);
    if (a.search) this.setSearchText(a.search, false);
    if (a.run) {
      switch (this.screen) {
        case "duplicates":
          void this.duplicates.run();
          break;
        case "compare":
          void this.compare.run();
          break;
        case "categories":
          void this.categories.run("new");
          break;
        case "leftovers":
          void this.leftovers.run();
          break;
        default:
          break;
      }
    }
    this.changed();
  }

  /** Safety net for missed events: while anything scans, ask the backend every 1.5 s. */
  private pollScanning() {
    if (this.pollTimer) clearTimeout(this.pollTimer);
    this.pollTimer = setTimeout(async () => {
      this.pollTimer = null;
      if (this.isScanning) {
        try {
          const busy = await this.backend.isScanning();
          if (!busy) {
            const was = this.isScanning;
            this.startupPending = false;
            this.scanning.clear();
            this.treeRev++;
            this.progressText = "";
            await this.refreshFolderStates();
            if (was) {
              this.invalidate();
              this.duplicates.resumeAfterScan();
            }
          }
        } catch {
          /* ignore */
        }
      }
      this.pollScanning();
    }, 1500);
  }

  get isScanning(): boolean {
    return this.startupPending || this.scanning.size > 0;
  }

  async refreshFolderStates() {
    try {
      const states = await this.backend.folderStates();
      this.folderState.clear();
      for (const s of states) {
        this.folderState.set(fkey(s.path), s.state);
        if (s.state === "scanning") this.scanning.add(fkey(s.path));
        else this.scanning.delete(fkey(s.path));
      }
      this.treeRev++;
      this.changed();
    } catch (e) {
      console.error(e);
    }
  }

  stateOf(root: string): FolderState {
    return this.folderState.get(fkey(root)) ?? "indexed";
  }

  isRootScanning(root: string): boolean {
    return this.scanning.has(fkey(root));
  }

  async refreshUndo() {
    try {
      this.undoState = await this.backend.undoState();
      this.changed();
    } catch (e) {
      console.error(e);
    }
  }

  async loadSummaries() {
    try {
      const list = await this.backend.folderSummaries(this.settings.folders);
      this.summaries = new Map(list.map((s) => [fkey(s.path), s]));
      this.changed();
    } catch (e) {
      console.error(e);
    }
  }

  summaryOf(root: string): FolderSummary | undefined {
    return this.summaries.get(fkey(root));
  }

  loadStats() {
    if (this.stats || this.statsLoading) return;
    this.statsLoading = true;
    this.backend
      .stats()
      .then((s) => {
        this.stats = s;
      })
      .catch((e) => console.error(e))
      .finally(() => {
        this.statsLoading = false;
        this.changed();
      });
  }

  // ───────────── screens ─────────────

  setScreen(screen: Screen) {
    if (this.screen === screen) return;
    this.screen = screen;
    this.changed();
    this.refreshStaleScreen();
  }

  /** Results computed before files changed elsewhere are recomputed when their screen comes back. */
  refreshStaleScreen() {
    switch (this.screen) {
      case "duplicates":
        if (this.duplicates.stale) void this.duplicates.run();
        break;
      case "compare":
        if (this.compare.stale) void this.compare.run();
        break;
      case "leftovers":
        if (this.leftovers.stale) void this.leftovers.run();
        break;
      case "categories":
        void this.categories.reload();
        break;
      default:
        break;
    }
  }

  // ───────────── folders ─────────────

  async addFolderViaPicker() {
    try {
      const paths = await this.backend.pickFolders({ multiple: true, title: t("한 화면에 같이 볼 폴더를 고르세요") });
      if (paths.length) await this.addFolderPaths(paths);
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  async addFolderPaths(paths: string[]) {
    if (!paths.length) return;
    try {
      const before = new Set(this.settings.folders.map(fkey));
      this.settings = await this.backend.addFolders(paths);
      for (const f of this.settings.folders) if (!before.has(fkey(f))) this.expanded.add(f);
      this.persistExpanded();
      this.screen = "collection";
      await this.refreshFolderStates();
      void this.loadSummaries();
      this.invalidate();
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  async removeFolder(path: string) {
    try {
      this.settings = await this.backend.removeFolder(path);
      for (const p of [...this.expanded]) if (isUnderOrSame(p, path)) this.expanded.delete(p);
      this.persistExpanded();
      this.invalidate();
      void this.loadSummaries();
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  async moveFolder(path: string, delta: -1 | 1) {
    const list = [...this.settings.folders];
    const i = list.findIndex((f) => samePath(f, path));
    const j = i + delta;
    if (i < 0 || j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    try {
      this.settings = await this.backend.reorderFolders(list);
      this.treeRev++;
      this.changed();
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  async rescan(path?: string) {
    try {
      if (path) {
        this.scanning.add(fkey(path));
        this.folderState.set(fkey(path), this.stateOf(path) === "indexed" ? "indexed" : "scanning");
      } else for (const f of this.settings.folders) this.scanning.add(fkey(f));
      this.treeRev++;
      this.changed();
      await this.backend.rescan(path);
    } catch (e) {
      this.showError(errorText(e));
    }
    void this.refreshFolderStates();
  }

  /** Sidebar click: show the folder in All Folders. */
  showFolder(path: string) {
    this.setScreen("collection");
    if (this.filter.text) this.setSearchText("");
    this.expanded.add(path);
    this.persistExpanded();
    this.sel = { selected: new Set([path]), anchor: path, cursor: path };
    this.scrollTo = { key: path, nonce: Date.now() };
    this.changed();
  }

  // ───────────── tree data ─────────────

  get filterActive(): boolean {
    const f = this.filter;
    return f.category !== "all" || f.minSize > 0 || f.withinDays > 0;
  }

  get isSearching(): boolean {
    return this.filter.text.trim().length > 0;
  }

  private childKey(): string {
    const f = this.filter;
    return `${f.category}|${f.minSize}|${f.withinDays}|${this.sort.field}|${this.sort.ascending}`;
  }

  isRoot(path: string): boolean {
    return this.settings.folders.some((f) => samePath(f, path));
  }

  entryOf(path: string): Entry | undefined {
    return this.entries.get(path);
  }

  isDir(path: string): boolean {
    if (this.isRoot(path)) return true;
    return this.entries.get(path)?.kind === "dir";
  }

  /** Visible rows of the multi-root tree, plus the folders whose children must be (re)loaded. */
  treeRows(): { rows: TreeRow[]; need: string[] } {
    const sig = `${this.treeRev}|${this.gen}|${this.childKey()}|${this.settings.folders.join("\u0000")}`;
    if (this.treeCache?.sig === sig) return this.treeCache.value;
    const value = this.buildTreeRows();
    this.treeCache = { sig, value };
    return value;
  }

  private buildTreeRows(): { rows: TreeRow[]; need: string[] } {
    const rows: TreeRow[] = [];
    const need: string[] = [];
    const key = this.childKey();
    const walk = (path: string, depth: number) => {
      const c = this.children.get(path);
      if (!c || c.key !== key || c.gen !== this.gen) need.push(path);
      if (!c) return;
      if (c.entries.length === 0) {
        rows.push({ type: "placeholder", key: `${path}\u0000empty`, depth, text: this.filterActive ? "필터에 맞는 항목 없음" : "비어 있음" });
        return;
      }
      for (const e of c.entries) {
        const open = e.kind === "dir" && this.expanded.has(e.path);
        rows.push({ type: "entry", key: e.path, entry: e, depth, expanded: open });
        if (open) walk(e.path, depth + 1);
      }
    };
    for (const root of this.settings.folders) {
      let state = this.stateOf(root);
      const busy = this.isRootScanning(root);
      const has = this.children.has(root);
      if (state === "scanning" && has) state = "indexed";
      if (busy && state === "notIndexed") state = "scanning";
      const open = (state === "indexed" || state === "scanning") && this.expanded.has(root);
      rows.push({ type: "root", key: root, path: root, depth: 0, state, expanded: open && state === "indexed", busy });
      if (state === "indexed" && open) walk(root, 1);
    }
    return { rows, need };
  }

  /** Loads children of expanded folders (keeps the old rows visible until the new ones arrive). */
  ensureChildren(paths: string[]) {
    const key = this.childKey();
    const gen = this.gen;
    const filter: Filter = { ...this.filter, text: "" };
    const sort = { ...this.sort };
    for (const p of paths) {
      const flight = `${p}\u0000${key}\u0000${gen}`;
      if (this.childrenInflight.has(flight)) continue;
      this.childrenInflight.add(flight);
      const seq = ++this.requestSeq;
      this.backend
        .listChildren(p, filter, sort)
        .then(
          (entries) => {
            const cur = this.children.get(p);
            if (cur && cur.seq > seq) return;
            this.children.set(p, { key, gen, seq, entries });
            this.treeRev++;
            for (const e of entries) this.entries.set(e.path, e);
          },
          (err) => {
            console.error(err);
            const cur = this.children.get(p);
            if (!cur || cur.seq < seq) this.children.set(p, { key, gen, seq, entries: cur?.entries ?? [] });
            this.treeRev++;
          },
        )
        .finally(() => {
          this.childrenInflight.delete(flight);
          this.changed();
        });
    }
  }

  dirInfoOf(path: string): DirInfo | undefined {
    return this.dirInfos.get(path)?.info;
  }

  /** Batches dirInfo requests for folder rows that just became visible. */
  requestDirInfo(paths: string[]) {
    let added = false;
    for (const p of paths) {
      const d = this.dirInfos.get(p);
      if (d && d.gen === this.gen) continue;
      if (this.dirInfoInflight.has(`${p}\u0000${this.gen}`)) continue;
      this.dirInfoQueue.add(p);
      added = true;
    }
    if (!added || this.dirInfoTimer) return;
    this.dirInfoTimer = setTimeout(() => {
      this.dirInfoTimer = null;
      const batch = [...this.dirInfoQueue];
      this.dirInfoQueue.clear();
      const gen = this.gen;
      for (const p of batch) this.dirInfoInflight.add(`${p}\u0000${gen}`);
      this.backend
        .dirInfo(batch)
        .then((list) => {
          for (const info of list) this.dirInfos.set(info.path, { gen, info });
        })
        .catch((e) => console.error(e))
        .finally(() => {
          for (const p of batch) this.dirInfoInflight.delete(`${p}\u0000${gen}`);
          this.changed();
        });
    }, 30);
  }

  toggleExpanded(path: string, open?: boolean) {
    const want = open ?? !this.expanded.has(path);
    if (want) this.expanded.add(path);
    else this.expanded.delete(path);
    this.persistExpanded();
    this.changed();
  }

  private persistExpanded() {
    this.treeRev++;
    writeLS(LS_EXPANDED, [...this.expanded]);
  }

  // ───────────── filter / sort / search ─────────────

  setFilter(patch: Partial<Omit<Filter, "text">>) {
    this.filter = { ...this.filter, ...patch };
    if (this.isSearching) this.runSearchSoon(0);
    this.changed();
  }

  clearFilters() {
    this.filter = { text: this.filter.text, category: "all", minSize: 0, withinDays: 0 };
    if (this.isSearching) this.runSearchSoon(0);
    this.changed();
  }

  setSort(field: SortField, ascending?: boolean) {
    const asc = ascending ?? (this.sort.field === field ? !this.sort.ascending : true);
    this.sort = { field, ascending: asc };
    writeLS(LS_SORT, this.sort);
    if (this.isSearching) this.runSearchSoon(0);
    this.changed();
  }

  setSearchText(text: string, switchScreen = true) {
    this.filter = { ...this.filter, text };
    if (switchScreen && text && this.screen !== "collection") this.screen = "collection";
    if (text.trim()) {
      this.searchPending = true;
      this.runSearchSoon(150);
    } else {
      this.searchSeq++;
      this.searchResults = [];
      this.searchTruncated = false;
      this.searchPending = false;
    }
    this.changed();
  }

  focusSearch() {
    this.focusSearchNonce++;
    this.changed();
  }

  private runSearchSoon(delay: number) {
    if (this.searchTimer) clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => {
      this.searchTimer = null;
      void this.runSearch();
    }, delay);
  }

  private async runSearch() {
    const text = this.filter.text.trim();
    const seq = ++this.searchSeq;
    if (!text) return;
    try {
      const r = await this.backend.search(text, { ...this.filter, text }, this.sort, SEARCH_LIMIT);
      if (seq !== this.searchSeq) return;
      this.searchResults = r.results;
      this.searchTruncated = r.truncated;
      for (const e of r.results) this.entries.set(e.path, e);
    } catch (e) {
      if (seq !== this.searchSeq) return;
      this.searchResults = [];
      console.error(e);
    }
    this.searchPending = false;
    this.changed();
  }

  /** From a search result to its place in the folder tree. */
  revealInList(path: string) {
    const root = rootOf(path, this.settings.folders);
    if (!root) {
      void this.reveal([path]);
      return;
    }
    this.setSearchText("");
    let p = dirname(path);
    while (p && isUnderOrSame(p, root)) {
      this.expanded.add(p);
      if (samePath(p, root)) break;
      p = dirname(p);
    }
    this.expanded.add(root);
    this.persistExpanded();
    this.sel = { selected: new Set([path]), anchor: path, cursor: path };
    this.scrollTo = { key: path, nonce: Date.now() };
    this.changed();
  }

  // ───────────── selection & activation ─────────────

  setSel(sel: Sel) {
    this.sel = sel;
    this.changed();
  }

  /** Selected paths that file operations may touch — the user's own folders (roots) are never moved or trashed. */
  operablePaths(): string[] {
    return [...this.sel.selected].filter((p) => !this.isRoot(p)).sort();
  }

  /** Double-click / Enter: folders expand or collapse (in search results: show in the tree), files open. */
  activate(paths: string[]) {
    if (paths.length === 1 && this.isDir(paths[0])) {
      if (this.isSearching) this.revealInList(paths[0]);
      else this.toggleExpanded(paths[0]);
      return;
    }
    for (const p of paths) if (!this.isDir(p)) void this.open(p);
  }

  togglePreview(open?: boolean) {
    this.previewOpen = open ?? !this.previewOpen;
    this.changed();
  }

  previewPath(): string | null {
    const { selected, cursor } = this.sel;
    if (cursor && selected.has(cursor)) return cursor;
    const first = [...selected][0];
    return first ?? null;
  }

  // ───────────── shell ─────────────

  async open(path: string) {
    try {
      await this.backend.openPath(path);
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  async reveal(paths: string[]) {
    if (!paths.length) return;
    try {
      await this.backend.revealPaths(paths);
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  async copyPaths(paths: string[]) {
    try {
      await navigator.clipboard.writeText(paths.join("\r\n"));
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  showError(message: string, title?: string, action?: AlertState["action"]) {
    this.alert = { title: title ?? t("실패"), message, action };
    this.changed();
  }

  dismissAlert() {
    this.alert = null;
    this.changed();
  }

  // ───────────── trash (after confirmation only) ─────────────

  requestTrashSelection() {
    let paths = this.operablePaths();
    let note: string | undefined;
    // With filters on, Ctrl+A also picks the folders shown for navigation → leave them out when mixed with files.
    if (this.filterActive && !this.isSearching) {
      const dirs = paths.filter((p) => this.isDir(p));
      if (dirs.length > 0 && dirs.length < paths.length) {
        paths = paths.filter((p) => !this.isDir(p));
        note = t("필터가 켜져 있어 함께 선택된 폴더 {0}개는 뺐습니다.", dirs.length);
      }
    }
    this.requestTrash(paths, { kind: "items", note });
  }

  requestTrash(paths: string[], opts: { kind: TrashKind; knownBytes?: number; note?: string }) {
    const unique = [...new Set(paths)].sort();
    if (!unique.length) return;
    this.trashRequest = { paths: unique, kind: opts.kind, knownBytes: opts.knownBytes, note: opts.note || undefined };
    this.changed();
  }

  trashRequestBytes(): number {
    const r = this.trashRequest;
    if (!r) return 0;
    if (r.knownBytes !== undefined) return r.knownBytes;
    let total = 0;
    for (const p of r.paths) {
      const e = this.entries.get(p);
      if (!e) continue;
      total += e.kind === "dir" ? (this.dirInfoOf(p)?.bytes ?? 0) : e.size;
    }
    return total;
  }

  cancelTrash() {
    this.trashRequest = null;
    this.changed();
  }

  async confirmTrash() {
    const req = this.trashRequest;
    if (!req) return;
    this.trashRequest = null;
    this.changed();
    const b = this.backend;
    try {
      const r =
        req.kind === "duplicates" ? await b.trashDuplicates(req.paths) : req.kind === "leftovers" ? await b.trashLeftovers(req.paths) : await b.trashItems(req.paths);
      this.afterOperation(this.reportOp(r, "trash", ""));
    } catch (e) {
      this.showError(errorText(e));
      this.afterOperation(null);
    }
  }

  // ───────────── move ─────────────

  async requestMoveSelection() {
    await this.requestMovePaths(this.operablePaths());
  }

  async requestMovePaths(paths: string[]) {
    if (!paths.length) return;
    try {
      const picked = await this.backend.pickFolders({ title: t("{0}개 항목을 옮길 폴더", paths.length) });
      if (!picked.length) return;
      await this.performMove(paths, picked[0]);
    } catch (e) {
      this.showError(errorText(e));
    }
  }

  /** Drag & drop onto a folder: moves immediately (the drop is the approval), one undo batch. */
  drop(paths: string[], folder: string) {
    const targets = paths.filter((p) => !samePath(p, folder) && !isInside(folder, p) && !samePath(dirname(p), folder) && !this.isRoot(p));
    if (!targets.length) return;
    void this.performMove(targets, folder).then(() => this.toggleExpanded(folder, true));
  }

  private async performMove(paths: string[], destination: string) {
    try {
      const r = await this.backend.moveItems(paths, destination);
      this.afterOperation(this.reportOp(r, "move", basename(destination)));
    } catch (e) {
      this.showError(errorText(e));
      this.afterOperation(null);
    }
  }

  /** Compare Folders: move one-sided files across, keeping relative paths — confirmed in a dialog first. */
  requestBatchMove(req: MoveRequest) {
    if (!req.paths.length) return;
    this.moveRequest = req;
    this.changed();
  }

  cancelMove() {
    this.moveRequest = null;
    this.changed();
  }

  async confirmMove() {
    const req = this.moveRequest;
    if (!req) return;
    this.moveRequest = null;
    this.changed();
    try {
      const r = await this.backend.moveAcross(req.paths, req.fromRoot, req.toRoot);
      this.afterOperation(this.reportOp(r, "move", req.folderName));
    } catch (e) {
      this.showError(errorText(e));
      this.afterOperation(null);
    }
  }

  /** Alerts for failures / skipped items; returns the status-bar summary. */
  private reportOp(r: OpResult, kind: "move" | "trash", folderName: string): string {
    const failed: OpFailure[] = r.failed ?? [];
    const skipped: OpFailure[] = r.skipped ?? [];
    if (failed.length) {
      const names = nameList(failed.map((f) => f.path));
      this.showError(`${names}\n\n${failed[0].reason}`, kind === "trash" ? t("{0}개를 휴지통으로 옮기지 못했습니다", failed.length) : t("{0}개를 옮기지 못했습니다", failed.length));
    } else if (skipped.length) {
      const names = nameList(skipped.map((f) => f.path));
      this.showError(`${t("다음 항목은 그대로 두었습니다 — 같은 이름이 이미 있거나, 찾은 뒤에 바뀌었거나, 묶음의 마지막 사본입니다.")}\n${names}\n\n${skipped[0].reason}`, t("건너뛴 항목이 있습니다"));
    }
    let summary =
      r.done > 0
        ? kind === "trash"
          ? t("{0}개 항목을 휴지통으로 옮겼습니다 — {1}", r.done, t("Ctrl+Z로 되돌리기"))
          : t("{0}개 항목을 {1}(으)로 옮겼습니다 — Ctrl+Z로 되돌리기", r.done, folderName)
        : t("옮긴 항목이 없습니다");
    if (skipped.length) summary += ` · ${t("{0}개 건너뜀", skipped.length)}`;
    return summary;
  }

  // ───────────── undo ─────────────

  async undo() {
    if (!this.undoState.canUndo) return;
    try {
      const r = await this.backend.undo();
      const parts: string[] = [];
      if (r.restored > 0) parts.push(t("{0}개 항목을 되돌렸습니다", r.restored));
      if (r.missing > 0) parts.push(t("{0}개는 찾을 수 없어 건너뜀", r.missing));
      if (r.blocked > 0)
        this.showError(t("원래 자리에 같은 이름의 항목이 있어 {0}개를 되돌리지 못했습니다.\n\n그 항목을 옮긴 뒤 다시 되돌리기를 누르세요.", r.blocked), t("되돌리지 못한 항목이 있습니다"));
      this.afterOperation(parts.join(" · ") || null);
    } catch (e) {
      this.showError(errorText(e));
      void this.refreshUndo();
    }
  }

  // ───────────── after file changes ─────────────

  private afterOperation(summary: string | null) {
    this.sel = emptySel();
    this.previewOpen = false;
    this.invalidate();
    void this.refreshUndo();
    if (summary) this.setLastSummary(summary);
    const d = this.duplicates, c = this.compare, l = this.leftovers;
    switch (this.screen) {
      case "duplicates":
        d.afterOperation();
        break;
      case "compare":
        if (c.result) void c.run();
        break;
      case "categories":
        void this.categories.reload();
        break;
      case "leftovers":
        void l.run();
        break;
      default:
        break;
    }
    if (this.screen !== "duplicates" && d.report) d.stale = true;
    if (this.screen !== "compare" && c.result) c.stale = true;
    if (this.screen !== "leftovers" && l.report) l.stale = true;
    if (this.screen !== "categories") void this.categories.reload();
    this.changed();
  }

  private setLastSummary(text: string) {
    this.lastSummary = text;
    if (this.lastSummaryTimer) clearTimeout(this.lastSummaryTimer);
    this.lastSummaryTimer = setTimeout(() => {
      this.lastSummary = null;
      this.changed();
    }, 30_000);
  }

  /** Index changed: drop cached numbers and refetch what is on screen. */
  invalidate() {
    this.gen++;
    this.stats = null;
    if (this.summaryTimer) clearTimeout(this.summaryTimer);
    this.summaryTimer = setTimeout(() => void this.loadSummaries(), 50);
    if (this.isSearching) this.runSearchSoon(0);
    this.changed();
  }

  private invalidateSoon() {
    if (this.invalidateTimer) return;
    this.invalidateTimer = setTimeout(() => {
      this.invalidateTimer = null;
      this.invalidate();
    }, 200);
  }

  /** Something is open that should swallow global shortcuts. */
  get modalOpen(): boolean {
    return !!(this.alert || this.trashRequest || this.moveRequest || this.categories.showEditor);
  }

  /** Paths selected on the current screen (Ctrl+Shift+R, Delete, Ctrl+Shift+M act on these only). */
  currentSelectionPaths(): string[] {
    switch (this.screen) {
      case "collection":
        return this.operablePaths();
      case "duplicates":
        return this.duplicates.selectedFiles().map((f) => f.path);
      case "compare":
        return this.compare.selectedPaths(false);
      case "categories":
        return [...this.categories.sel.selected];
      case "leftovers":
        return this.leftovers.selectedPaths();
    }
  }

  trashOnCurrentScreen() {
    switch (this.screen) {
      case "collection":
        this.requestTrashSelection();
        break;
      case "duplicates":
        this.duplicates.trashSelection();
        break;
      case "compare":
        this.compare.trashSelected(null);
        break;
      case "categories":
        this.categories.trashSelection();
        break;
      case "leftovers":
        this.leftovers.trashSelection();
        break;
    }
  }

  moveOnCurrentScreen() {
    switch (this.screen) {
      case "collection":
        void this.requestMoveSelection();
        break;
      case "duplicates":
        void this.requestMovePaths(this.duplicates.selectedFiles().map((f) => f.path));
        break;
      case "compare":
        void this.requestMovePaths(this.compare.selectedPaths(true));
        break;
      case "categories":
        this.categories.moveSelectionToFolder();
        break;
      default:
        break;
    }
  }

  revealOnCurrentScreen() {
    const paths = this.screen === "collection" ? [...this.sel.selected] : this.currentSelectionPaths();
    void this.reveal(paths);
  }
}
