// In-memory backend for a plain browser (npm run dev, Playwright screenshots). Behaves like the real
// one closely enough to exercise every screen: scans with progress, move/trash/undo batches, duplicates
// with copy trees and an ignore list, folder compare, a model that "downloads", classification, leftovers.
//
// URL parameters (mirror the app's --screen/--run/--left/--right/--search launch args, plus mock-only ones):
//   ?lang=ko|en|ja|zh   UI language          ?screen=duplicates|compare|categories|leftovers  ?run=1
//   ?left=Photos 2025&right=Photos backup (names are resolved under Desktop)   ?search=lake
//   ?model=missing|downloading|ready|error   ?classified=1 (model ready + results)   ?fast=1 (short delays)
//   ?big=1 (30,000 extra files in Desktop\Scans — virtualized list check)
import type { Backend, BackendEventName, BackendEvents, PickFolderOptions } from "./backend";
import type {
  Bucket,
  Classified,
  ClassifiedList,
  ClassifyRequest,
  CopyTree,
  DiffEntry,
  DiffResult,
  DirInfo,
  DuplicateReport,
  DuplicateRequest,
  Entry,
  FileGroup,
  Filter,
  FolderStatus,
  FolderSummary,
  LaunchArgs,
  LeftoverItem,
  LeftoverPath,
  LeftoverReport,
  ModelStatus,
  OpFailure,
  OpResult,
  Screen,
  SearchResult,
  Settings,
  Sort,
  Stats,
  UndoResult,
  UndoState,
} from "./types";
import { t, currentLang } from "../i18n";
import { categoryOfExt } from "../util/fileKinds";
import {
  CAMERA,
  DESKTOP,
  DOWNLOADS,
  HOME,
  PHOTOS_LEFT,
  PHOTOS_RIGHT,
  PICTURES,
  SORTED,
  buildDisk,
  classifyRule,
  type MockNode,
} from "./mockData";

const params = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
const SPEED = params.has("fast") ? 0.15 : 1;
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, Math.max(0, ms * SPEED)));
const MODEL_TOTAL = 804_753_632;
const DAY = 86_400_000;
const lower = (s: string) => s.toLowerCase();

function within(path: string, folder: string): boolean {
  const p = lower(path);
  const f = lower(folder);
  return p === f || p.startsWith(f.endsWith("\\") ? f : f + "\\");
}
const strictlyWithin = (path: string, folder: string) => lower(path) !== lower(folder) && within(path, folder);
const extOf = (name: string) => {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
};
const parentOf = (p: string) => p.slice(0, p.lastIndexOf("\\"));
const nameOf = (p: string) => p.slice(p.lastIndexOf("\\") + 1);
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

type Op =
  | { type: "move"; from: string; to: string }
  | { type: "trash"; root: string; nodes: MockNode[] }
  | { type: "mkdir"; path: string }
  | { type: "leftover"; item: LeftoverItem; path: LeftoverPath };
interface Batch {
  id: number;
  kind: "move" | "trash";
  ops: Op[];
  undone: boolean;
}

function makeLeftovers(): LeftoverItem[] {
  const roaming = `${HOME}\\AppData\\Roaming`;
  const local = `${HOME}\\AppData\\Local`;
  const p = (path: string, mb: number, daysAgo: number): LeftoverPath => ({ path, bytes: Math.round(mb * 1_000_000), lastModified: Date.now() - daysAgo * DAY });
  // Reason keys are the Korean source strings mf-core emits (core/src/leftovers.rs REASONS).
  const ONE = "설치된 앱 목록에 없고 30일 넘게 바뀌지 않은 앱 데이터 폴더";
  const MULTI = "설치된 앱 목록에 없고 30일 넘게 바뀌지 않은 앱 데이터 — 여러 위치에 남아 있음";
  const VENDOR = "제조사 폴더 안의 앱 데이터 — 같은 이름의 설치된 앱이 없고 30일 넘게 바뀌지 않음";
  const items: LeftoverItem[] = [
    { id: "oldeditor", app: "OldEditor", reason: MULTI, bytes: 0, paths: [p(`${roaming}\\OldEditor`, 12.4, 421), p(`${local}\\OldEditor`, 231.6, 418)] },
    { id: "retroracer", app: "Retro Racer", reason: ONE, bytes: 0, paths: [p(`${local}\\RetroRacer`, 86.2, 203)] },
    { id: "tunebox", app: "Tunebox", reason: MULTI, bytes: 0, paths: [p(`${roaming}\\Tunebox`, 31.0, 152), p(`${local}\\Tunebox\\Cache`, 412.5, 150)] },
    { id: "inkwell", app: "Inkwell Studio", reason: VENDOR, bytes: 0, paths: [p(`${roaming}\\Inkwell Labs\\Inkwell Studio`, 4.2, 96)] },
  ];
  for (const i of items) i.bytes = i.paths.reduce((s, x) => s + x.bytes, 0);
  return items;
}

// ───────────────────────────── preview images (SVG) ─────────────────────────────

function hashNum(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return Math.abs(h);
}
function svgUrl(svg: string) {
  return "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
}
function photoSvg(seed: number): string {
  const hue = seed % 360;
  return svgUrl(
    `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1200 800'><defs><linearGradient id='s' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='hsl(${hue},65%,70%)'/><stop offset='1' stop-color='hsl(${(hue + 35) % 360},70%,88%)'/></linearGradient><linearGradient id='w' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='hsl(${(hue + 190) % 360},45%,45%)'/><stop offset='1' stop-color='hsl(${(hue + 200) % 360},50%,28%)'/></linearGradient></defs>` +
      `<rect width='1200' height='800' fill='url(#s)'/><circle cx='${820 + (seed % 200)}' cy='210' r='70' fill='hsl(48,100%,92%)' opacity='.9'/>` +
      `<path d='M0 520 L230 290 L420 470 L640 240 L880 500 L1040 380 L1200 470 L1200 800 L0 800Z' fill='hsl(${(hue + 210) % 360},20%,38%)'/>` +
      `<path d='M560 300 L640 240 L720 318 L690 300 L655 330 L620 300Z' fill='white' opacity='.85'/>` +
      `<rect y='560' width='1200' height='240' fill='url(#w)'/><path d='M0 560 C200 545 420 575 640 560 S1000 548 1200 562 L1200 590 L0 590Z' fill='white' opacity='.18'/></svg>`,
  );
}
function screenshotSvg(seed: number): string {
  const hue = (seed % 6) * 50 + 190;
  return svgUrl(
    `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1280 800'><rect width='1280' height='800' fill='hsl(${hue},30%,92%)'/>` +
      `<rect x='120' y='90' width='1040' height='620' rx='10' fill='white' stroke='hsl(${hue},10%,80%)'/><rect x='120' y='90' width='1040' height='44' rx='10' fill='hsl(${hue},20%,96%)'/>` +
      `<rect x='120' y='134' width='220' height='576' fill='hsl(${hue},25%,97%)'/>` +
      Array.from({ length: 7 }, (_, i) => `<rect x='145' y='${165 + i * 42}' width='${120 + ((seed + i * 31) % 50)}' height='14' rx='7' fill='hsl(${hue},15%,82%)'/>`).join("") +
      `<rect x='380' y='170' width='420' height='26' rx='6' fill='hsl(${hue},60%,45%)'/>` +
      Array.from({ length: 6 }, (_, i) => `<rect x='380' y='${230 + i * 36}' width='${540 + ((seed * (i + 3)) % 180)}' height='12' rx='6' fill='hsl(${hue},10%,86%)'/>`).join("") +
      `<rect x='380' y='470' width='160' height='40' rx='6' fill='hsl(${hue},70%,50%)'/></svg>`,
  );
}

// ───────────────────────────── the mock ─────────────────────────────

export class MockBackend implements Backend {
  readonly kind = "mock" as const;

  private disk: Map<string, MockNode> = buildDisk(params.has("big"));
  private indexed = new Set<string>([DOWNLOADS, DESKTOP].map(lower));
  private scanning = new Set<string>();
  private settings: Settings;
  private listeners = new Map<string, Set<(payload: unknown) => void>>();
  private batches: Batch[] = [];
  private nextBatch = 1;
  private ignored = new Set<string>();
  private buckets: Bucket[] | null = null;
  private classified = new Map<string, Classified>();
  private model: ModelStatus;
  private modelTimer: ReturnType<typeof setInterval> | null = null;
  private classifyRun: { cancelled: boolean } | null = null;
  private dupRun: { cancelled: boolean } | null = null;
  private leftovers: LeftoverItem[] = makeLeftovers();

  constructor() {
    const lang = params.get("lang");
    const locale =
      lang === "ko" ? "ko-KR" : lang === "ja" ? "ja-JP" : lang?.startsWith("zh") ? "zh-CN" : lang === "en" ? "en-US" : navigator.language || "en-US";
    this.settings = { folders: [DOWNLOADS, DESKTOP, CAMERA], locale, dataDir: `${HOME}\\AppData\\Local\\Multi-Finder`, version: "1.0.0" };
    this.model = { state: "missing", name: "Qwen3 0.6B (Q8_0)", bytes: 0, totalBytes: MODEL_TOTAL };
    const m = params.get("model");
    if (m === "ready" || params.has("classified")) this.model = { ...this.model, state: "ready", bytes: MODEL_TOTAL };
    else if (m === "downloading") this.model = { ...this.model, state: "downloading", bytes: Math.round(MODEL_TOTAL * 0.37) };
    else if (m === "error") this.model = { ...this.model, state: "error", bytes: 301_989_888, error: "Download interrupted: connection reset by huggingface.co (os error 10054)" };
  }

  // ───── plumbing ─────

  private emit<E extends BackendEventName>(event: E, payload: BackendEvents[E]) {
    for (const h of this.listeners.get(event) ?? []) h(payload);
  }

  on<E extends BackendEventName>(event: E, handler: (payload: BackendEvents[E]) => void): () => void {
    let set = this.listeners.get(event);
    if (!set) this.listeners.set(event, (set = new Set()));
    const h = handler as (payload: unknown) => void;
    set.add(h);
    return () => set!.delete(h);
  }

  private isIndexed(path: string): boolean {
    for (const r of this.indexed) if (within(lower(path), r)) return true;
    return false;
  }

  private entry(n: MockNode): Entry {
    return { path: n.path, name: n.name, parent: n.parent, ext: n.kind === "file" ? extOf(n.name) : "", kind: n.kind, size: n.kind === "file" ? n.size : 0, modified: n.modified, cloud: n.cloud };
  }

  private children(path: string): MockNode[] {
    const out: MockNode[] = [];
    for (const n of this.disk.values()) if (n.parent === path) out.push(n);
    return out;
  }

  private under(path: string): MockNode[] {
    const out: MockNode[] = [];
    for (const n of this.disk.values()) if (strictlyWithin(n.path, path)) out.push(n);
    return out;
  }

  private bytesUnder(path: string): number {
    let total = 0;
    for (const n of this.disk.values()) if (n.kind === "file" && strictlyWithin(n.path, path)) total += n.size;
    return total;
  }

  private passes(n: MockNode, f: Filter, useText: boolean): boolean {
    if (useText && f.text && !lower(n.name).includes(lower(f.text))) return false;
    if (n.kind === "dir") return true;
    if (f.category !== "all" && categoryOfExt(extOf(n.name)) !== f.category) return false;
    if (n.size < f.minSize) return false;
    if (f.withinDays > 0) {
      const midnight = new Date();
      midnight.setHours(0, 0, 0, 0);
      if (n.modified < midnight.getTime() - f.withinDays * DAY) return false;
    }
    return true;
  }

  private sorted(nodes: MockNode[], sort: Sort): MockNode[] {
    const sizeOf = (n: MockNode) => (n.kind === "dir" ? this.bytesUnder(n.path) : n.size);
    const cmp = (a: MockNode, b: MockNode) => {
      let r = 0;
      if (sort.field === "modified") r = a.modified - b.modified;
      else if (sort.field === "size") r = sizeOf(a) - sizeOf(b);
      else if (sort.field === "kind") r = collator.compare(extOf(a.name), extOf(b.name));
      if (r === 0) r = collator.compare(a.name, b.name);
      return sort.ascending ? r : -r;
    };
    return [...nodes].sort((a, b) => (a.kind === "dir") !== (b.kind === "dir") ? (a.kind === "dir" ? -1 : 1) : cmp(a, b));
  }

  // ───── settings & folders ─────

  async getSettings(): Promise<Settings> {
    return { ...this.settings, folders: [...this.settings.folders] };
  }

  async getLaunchArgs(): Promise<LaunchArgs> {
    const resolve = (v: string | null) => (v ? (v.includes("\\") ? v : `${DESKTOP}\\${v}`) : undefined);
    const screen = params.get("screen") as Screen | null;
    return {
      screen: screen ?? undefined,
      run: params.get("run") === "1" || params.get("run") === "true",
      left: resolve(params.get("left")) ?? (screen === "compare" ? PHOTOS_LEFT : undefined),
      right: resolve(params.get("right")) ?? (screen === "compare" ? PHOTOS_RIGHT : undefined),
      search: params.get("search") ?? undefined,
    };
  }

  async addFolders(paths: string[]): Promise<Settings> {
    for (const p of paths) {
      if (this.settings.folders.some((f) => lower(f) === lower(p))) continue;
      this.settings.folders.push(p);
      if (this.disk.has(p) && !this.isIndexed(p)) void this.runScan(p);
    }
    return this.getSettings();
  }

  async removeFolder(path: string): Promise<Settings> {
    this.settings.folders = this.settings.folders.filter((f) => lower(f) !== lower(path));
    return this.getSettings();
  }

  async reorderFolders(folders: string[]): Promise<Settings> {
    this.settings.folders = [...folders];
    return this.getSettings();
  }

  async folderStates(): Promise<FolderStatus[]> {
    return this.settings.folders.map((path) => ({
      path,
      state: !this.disk.has(path) ? "missing" : this.scanning.has(lower(path)) ? "scanning" : this.isIndexed(path) ? "indexed" : "notIndexed",
    }));
  }

  async rescan(path?: string): Promise<void> {
    const targets = path ? [path] : this.settings.folders;
    for (const p of targets) if (this.disk.has(p)) void this.runScan(p);
  }

  private async runScan(root: string) {
    const key = lower(root);
    if (this.scanning.has(key)) return;
    this.scanning.add(key);
    const nodes = this.under(root);
    const steps = 5;
    for (let i = 1; i <= steps; i++) {
      await sleep(220);
      const n = nodes[Math.min(nodes.length - 1, Math.floor((nodes.length * i) / steps) - 1)];
      this.emit("scan-progress", { root, scanned: Math.round((nodes.length * i) / steps), current: n?.path ?? root });
    }
    this.indexed.add(key);
    this.scanning.delete(key);
    this.emit("scan-finished", { root });
  }

  async isScanning(): Promise<boolean> {
    return this.scanning.size > 0;
  }

  // ───── listing ─────

  async listChildren(path: string, filter: Filter, sort: Sort): Promise<Entry[]> {
    if (!this.isIndexed(path)) return [];
    return this.sorted(this.children(path).filter((n) => this.passes(n, filter, true)), sort).map((n) => this.entry(n));
  }

  async dirInfo(paths: string[]): Promise<DirInfo[]> {
    return paths.map((path) => ({ path, items: this.isIndexed(path) ? this.children(path).length : 0, bytes: this.isIndexed(path) ? this.bytesUnder(path) : 0 }));
  }

  async folderSummaries(paths: string[]): Promise<FolderSummary[]> {
    return paths.map((path) => {
      if (!this.isIndexed(path)) return { path, files: 0, bytes: 0 };
      const files = this.under(path).filter((n) => n.kind === "file");
      return { path, files: files.length, bytes: files.reduce((s, n) => s + n.size, 0) };
    });
  }

  async stats(): Promise<Stats> {
    const seen = new Set<string>();
    let files = 0, folders = 0, bytes = 0;
    for (const root of this.settings.folders) {
      if (!this.isIndexed(root)) continue;
      for (const n of this.under(root)) {
        if (seen.has(n.path)) continue;
        seen.add(n.path);
        if (n.kind === "dir") folders++;
        else {
          files++;
          bytes += n.size;
        }
      }
    }
    return { files, folders, bytes };
  }

  async search(text: string, filter: Filter, sort: Sort, limit: number): Promise<SearchResult> {
    await sleep(40);
    const q = lower(text);
    const hits: MockNode[] = [];
    for (const n of this.disk.values()) {
      if (!lower(n.name).includes(q)) continue;
      if (!this.settings.folders.some((r) => strictlyWithin(n.path, r)) || !this.isIndexed(n.path)) continue;
      if (this.passes(n, filter, false)) hits.push(n);
    }
    const all = this.sorted(hits, sort);
    return { results: all.slice(0, limit).map((n) => this.entry(n)), truncated: all.length > limit };
  }

  async openPath(path: string): Promise<void> {
    console.info("[mock] open", path);
  }

  async revealPaths(paths: string[]): Promise<void> {
    console.info("[mock] reveal in Explorer", paths);
  }

  async readTextPreview(path: string, maxBytes: number): Promise<string> {
    const n = this.disk.get(path);
    if (!n) throw new Error("not found");
    const text = n.text || `${n.name}\n\n(sample text)\n`;
    return text.slice(0, maxBytes);
  }

  // ───── operations ─────

  private newBatch(kind: Batch["kind"]): Batch {
    return { id: this.nextBatch++, kind, ops: [], undone: false };
  }

  private finish(batch: Batch, done: number, failed: OpFailure[], skipped: OpFailure[]): OpResult {
    if (done > 0) this.batches.push(batch);
    return { done, failed, skipped, batch: done > 0 ? batch.id : 0 };
  }

  /** Moves the node at `from` (and everything inside) to the exact path `to`. */
  private relocate(from: string, to: string) {
    const n = this.disk.get(from)!;
    const moved = [n, ...this.under(from)];
    for (const m of moved) this.disk.delete(m.path);
    for (const m of moved) {
      const tail = m.path.slice(from.length);
      if (m === n) m.parent = parentOf(to);
      else m.parent = to + m.parent.slice(from.length);
      m.path = to + tail;
      m.name = nameOf(m.path);
      this.disk.set(m.path, m);
    }
  }

  private moveOne(path: string, destDir: string, batch: Batch, failed: OpFailure[], skipped: OpFailure[]): boolean {
    const n = this.disk.get(path);
    const dest = this.disk.get(destDir);
    if (!n) return failed.push({ path, reason: "no such item" }), false;
    if (!dest || dest.kind !== "dir") return failed.push({ path, reason: "destination is not a folder" }), false;
    if (within(destDir, path)) return skipped.push({ path, reason: "cannot move a folder into itself" }), false;
    if (lower(n.parent) === lower(destDir)) return skipped.push({ path, reason: "already in that folder" }), false;
    const to = `${destDir}\\${n.name}`;
    if (this.disk.has(to)) return skipped.push({ path, reason: "an item with the same name exists at the destination (not overwritten)" }), false;
    this.relocate(path, to);
    batch.ops.push({ type: "move", from: path, to });
    return true;
  }

  async moveItems(paths: string[], destination: string): Promise<OpResult> {
    await sleep(60);
    const batch = this.newBatch("move");
    const failed: OpFailure[] = [], skipped: OpFailure[] = [];
    let done = 0;
    for (const p of paths) if (this.moveOne(p, destination, batch, failed, skipped)) done++;
    return this.finish(batch, done, failed, skipped);
  }

  async moveAcross(paths: string[], fromRoot: string, toRoot: string): Promise<OpResult> {
    await sleep(60);
    const batch = this.newBatch("move");
    const failed: OpFailure[] = [], skipped: OpFailure[] = [];
    let done = 0;
    for (const p of paths) {
      if (!within(p, fromRoot)) {
        failed.push({ path: p, reason: "not inside the source folder" });
        continue;
      }
      const rel = p.slice(fromRoot.length + 1);
      const relDir = rel.includes("\\") ? rel.slice(0, rel.lastIndexOf("\\")) : "";
      let dir = toRoot;
      for (const part of relDir ? relDir.split("\\") : []) {
        const next = `${dir}\\${part}`;
        if (!this.disk.has(next)) {
          this.disk.set(next, { path: next, name: part, parent: dir, kind: "dir", size: 0, modified: Date.now(), cloud: false, hash: "" });
          batch.ops.push({ type: "mkdir", path: next });
        }
        dir = next;
      }
      if (this.moveOne(p, dir, batch, failed, skipped)) done++;
    }
    return this.finish(batch, done, failed, skipped);
  }

  private trashOne(path: string, batch: Batch, failed: OpFailure[]): boolean {
    const n = this.disk.get(path);
    if (!n) return failed.push({ path, reason: "no such item" }), false;
    const nodes = [n, ...this.under(path)];
    for (const m of nodes) this.disk.delete(m.path);
    batch.ops.push({ type: "trash", root: path, nodes });
    return true;
  }

  async trashItems(paths: string[]): Promise<OpResult> {
    await sleep(60);
    const batch = this.newBatch("trash");
    const failed: OpFailure[] = [];
    let done = 0;
    for (const p of paths) if (this.trashOne(p, batch, failed)) done++;
    return this.finish(batch, done, failed, []);
  }

  async trashDuplicates(paths: string[]): Promise<OpResult> {
    await sleep(60);
    const batch = this.newBatch("trash");
    const failed: OpFailure[] = [], skipped: OpFailure[] = [];
    let done = 0;
    for (const p of paths) {
      const n = this.disk.get(p);
      if (!n) {
        failed.push({ path: p, reason: "no such item" });
        continue;
      }
      const others = [...this.disk.values()].filter((m) => m !== n && m.kind === "file" && m.hash === n.hash && this.isIndexed(m.path));
      if (others.length === 0) {
        skipped.push({ path: p, reason: "last remaining copy of its duplicate set" });
        continue;
      }
      if (this.trashOne(p, batch, failed)) done++;
    }
    return this.finish(batch, done, failed, skipped);
  }

  async undo(): Promise<UndoResult> {
    await sleep(60);
    const batch = [...this.batches].reverse().find((b) => !b.undone);
    const result: UndoResult = { restored: 0, missing: 0, blocked: 0 };
    if (!batch) return result;
    batch.undone = true;
    for (const op of [...batch.ops].reverse()) {
      if (op.type === "move") {
        if (!this.disk.has(op.to)) result.missing++;
        else if (this.disk.has(op.from)) result.blocked++;
        else {
          this.relocate(op.to, op.from);
          result.restored++;
        }
      } else if (op.type === "trash") {
        if (this.disk.has(op.root)) result.blocked++;
        else {
          for (const m of op.nodes) this.disk.set(m.path, m);
          result.restored++;
        }
      } else if (op.type === "mkdir") {
        if (this.disk.has(op.path) && this.children(op.path).length === 0) this.disk.delete(op.path);
      } else {
        let item = this.leftovers.find((i) => i.id === op.item.id);
        if (!item) {
          item = { ...op.item, paths: [], bytes: 0 };
          this.leftovers.push(item);
        }
        item.paths.push(op.path);
        item.bytes = item.paths.reduce((s, x) => s + x.bytes, 0);
        result.restored++;
      }
    }
    return result;
  }

  async undoState(): Promise<UndoState> {
    const b = [...this.batches].reverse().find((x) => !x.undone);
    if (!b) return { canUndo: false };
    return { canUndo: true, kind: b.kind, count: b.ops.filter((o) => o.type !== "mkdir").length };
  }

  // ───── duplicates ─────

  async findDuplicates(req: DuplicateRequest): Promise<DuplicateReport> {
    const run = { cancelled: false };
    this.dupRun = run;
    const files = [...this.disk.values()].filter((n) => n.kind === "file" && !n.cloud && n.size >= Math.max(1, req.minSize) && this.isIndexed(n.path) && this.settings.folders.some((r) => within(n.path, r)));
    this.emit("dup-progress", { phase: "size", done: 0, total: files.length });
    await sleep(350);
    const bySize = new Map<number, MockNode[]>();
    for (const f of files) bySize.set(f.size, [...(bySize.get(f.size) ?? []), f]);
    const candidates = [...bySize.values()].filter((g) => g.length > 1).flat();
    for (let i = 0; i <= candidates.length; i += Math.max(1, Math.ceil(candidates.length / 6))) {
      if (run.cancelled) throw new Error("cancelled");
      this.emit("dup-progress", { phase: "hash", done: i, total: candidates.length, current: candidates[i]?.path });
      await sleep(180);
    }
    if (run.cancelled) throw new Error("cancelled");
    const byHash = new Map<string, MockNode[]>();
    for (const f of candidates) byHash.set(f.hash, [...(byHash.get(f.hash) ?? []), f]);
    let groups: FileGroup[] = [...byHash.entries()]
      .filter(([, g]) => g.length > 1)
      .map(([hash, g]) => ({ id: `blake3:${hashNum(hash).toString(16).padStart(8, "0")}`, size: g[0].size, files: g.map((n) => this.entry(n)).sort((a, b) => collator.compare(a.path, b.path)) }));
    const rawGroupCount = groups.length;
    if (req.scope) groups = groups.filter((g) => g.files.some((f) => within(f.path, req.scope!)));
    const ignoredBefore = groups.length;
    groups = groups.filter((g) => !this.ignored.has(g.id));
    const hiddenByIgnoreList = ignoredBefore - groups.length;
    let hiddenWithinProject = 0;
    if (req.hideWithinProject) {
      const projectOf = (p: string) => {
        let dir = parentOf(p);
        while (dir.length > 3) {
          if (this.disk.has(`${dir}\\package.json`) || this.disk.has(`${dir}\\.git`) || this.disk.has(`${dir}\\Cargo.toml`)) return lower(dir);
          dir = parentOf(dir);
        }
        return null;
      };
      const kept: FileGroup[] = [];
      for (const g of groups) {
        const projects = g.files.map((f) => projectOf(f.path));
        if (projects[0] && projects.every((x) => x === projects[0])) hiddenWithinProject++;
        else kept.push(g);
      }
      groups = kept;
    }
    let copyTrees: CopyTree[] = [];
    if (req.foldCopyTrees) {
      const topOf = (p: string) => {
        const root = this.settings.folders.find((r) => within(p, r));
        if (!root) return null;
        const rel = p.slice(root.length + 1);
        return rel.includes("\\") ? `${root}\\${rel.slice(0, rel.indexOf("\\"))}` : null;
      };
      const pairs = new Map<string, FileGroup[]>();
      for (const g of groups) {
        if (g.files.length !== 2) continue;
        const a = topOf(g.files[0].path), b = topOf(g.files[1].path);
        if (!a || !b || a === b) continue;
        const key = `${a}|${b}`;
        pairs.set(key, [...(pairs.get(key) ?? []), g]);
      }
      for (const [key, gs] of pairs) {
        if (gs.length < 3) continue;
        const [left, right] = key.split("|");
        copyTrees.push({ id: `tree:${hashNum(key)}`, left, right, groups: gs, fileCount: gs.length, bytes: gs.reduce((s, g) => s + g.size, 0), examples: gs.slice(0, 3).map((g) => g.files[0].name) });
      }
      const folded = new Set(copyTrees.flatMap((tr) => tr.groups.map((g) => g.id)));
      groups = groups.filter((g) => !folded.has(g.id));
      copyTrees = copyTrees.sort((a, b) => b.bytes - a.bytes);
    }
    groups.sort((a, b) => b.size * (b.files.length - 1) - a.size * (a.files.length - 1));
    const reclaimableBytes = [...groups, ...copyTrees.flatMap((x) => x.groups)].reduce((s, g) => s + g.size * (g.files.length - 1), 0);
    this.dupRun = null;
    return { fileGroups: groups, copyTrees, rawGroupCount, hiddenWithinProject, hiddenByIgnoreList, reclaimableBytes };
  }

  async cancelDuplicates(): Promise<void> {
    if (this.dupRun) this.dupRun.cancelled = true;
  }

  async ignoreDuplicateGroup(id: string): Promise<void> {
    this.ignored.add(id);
  }

  async clearDuplicateIgnores(): Promise<void> {
    this.ignored.clear();
  }

  // ───── compare ─────

  async compareFolders(left: string, right: string): Promise<DiffResult> {
    if (!this.disk.has(left) || !this.disk.has(right)) throw new Error(`folder not found: ${!this.disk.has(left) ? left : right}`);
    this.emit("compare-progress", { phase: "scan", done: 0, total: 0 });
    await sleep(300);
    const files = (root: string) => new Map(this.under(root).filter((n) => n.kind === "file").map((n) => [n.path.slice(root.length + 1), n] as const));
    const L = files(left), R = files(right);
    const total = L.size + R.size;
    for (let i = 0; i <= total; i += Math.max(1, Math.ceil(total / 5))) {
      this.emit("compare-progress", { phase: "hash", done: i, total });
      await sleep(140);
    }
    const side = (n: MockNode) => ({ path: n.path, size: n.size, modified: n.modified });
    const entries: DiffEntry[] = [];
    const leftOnly: [string, MockNode][] = [];
    const rightOnly = new Map<string, MockNode>();
    for (const [rel, n] of R) if (!L.has(rel)) rightOnly.set(rel, n);
    for (const [rel, n] of L) {
      const r = R.get(rel);
      if (r) entries.push({ id: `e${entries.length}`, relPath: rel, name: n.name, status: r.hash === n.hash ? "identical" : "different", left: side(n), right: side(r) });
      else leftOnly.push([rel, n]);
    }
    for (const [rel, n] of leftOnly) {
      const match = [...rightOnly.entries()].find(([, r]) => r.hash === n.hash);
      if (match) {
        rightOnly.delete(match[0]);
        entries.push({ id: `e${entries.length}`, relPath: rel, name: n.name, status: "moved", left: side(n), right: side(match[1]), otherRelPath: match[0] });
      } else entries.push({ id: `e${entries.length}`, relPath: rel, name: n.name, status: "leftOnly", left: side(n) });
    }
    for (const [rel, n] of rightOnly) entries.push({ id: `e${entries.length}`, relPath: rel, name: n.name, status: "rightOnly", right: side(n) });
    entries.sort((a, b) => collator.compare(a.relPath, b.relPath));
    const counts = { identical: 0, different: 0, leftOnly: 0, rightOnly: 0, moved: 0 };
    for (const e of entries) counts[e.status]++;
    return { left, right, entries, counts };
  }

  // ───── categories ─────

  private defaultBuckets(): Bucket[] {
    const mk = (id: string, name: string, hint: string, isFallback = false): Bucket => ({ id, name, hint, isFallback });
    return [
      mk("b-work", t("업무"), t("회사·업무 문서 — 보고서, 기획서, 계약서, 회의 자료, 거래처와 주고받은 파일")),
      mk("b-school", t("학교·공부"), t("수업 자료, 강의 노트, 과제·레포트, 시험 자료, 학사 서류, 공부용 교재")),
      mk("b-personal", t("개인"), t("개인 생활 — 영수증, 계약서, 여행, 취미, 건강·금융 등 사적인 문서")),
      mk("b-dev", t("개발"), t("소스 코드, 프로젝트 압축본, 설정 파일, 개발 문서, 기술 자료")),
      mk("b-media", t("디자인·미디어"), t("로고, 사진, 이미지, 영상, 음악, 폰트, 디자인 시안")),
      mk("b-install", t("설치·임시"), t("exe·msi 설치 파일, 압축 해제 전 zip, 임시 캡처, 다시 받을 수 있는 것")),
      mk("b-other", t("기타"), t("위 어디에도 분명히 속하지 않음"), true),
    ];
  }

  private ensureBuckets(): Bucket[] {
    if (!this.buckets) {
      this.buckets = this.defaultBuckets();
      if (params.has("classified")) this.seedClassified();
    }
    return this.buckets;
  }

  async getBuckets(): Promise<Bucket[]> {
    return this.ensureBuckets().map((b) => ({ ...b }));
  }

  async saveBuckets(buckets: Bucket[]): Promise<Bucket[]> {
    const cleaned = buckets.filter((b) => b.name.trim()).map((b) => ({ ...b, name: b.name.trim() }));
    let fallback = cleaned.find((b) => b.isFallback);
    if (!fallback) fallback = { id: "b-other", name: t("기타"), hint: t("위 어디에도 분명히 속하지 않음"), isFallback: true };
    this.buckets = [...cleaned.filter((b) => !b.isFallback), fallback];
    const names = new Set(this.buckets.map((b) => b.name));
    for (const c of this.classified.values()) if (!names.has(c.bucket)) c.bucket = fallback.name;
    return this.getBuckets();
  }

  async modelStatus(): Promise<ModelStatus> {
    return { ...this.model };
  }

  async downloadModel(): Promise<void> {
    if (this.model.state === "ready" || this.modelTimer) return;
    this.model = { ...this.model, state: "downloading", error: undefined };
    const fail = params.has("modelfail");
    this.modelTimer = setInterval(() => {
      const bytes = Math.min(MODEL_TOTAL, this.model.bytes + Math.round(MODEL_TOTAL * 0.035));
      this.model = { ...this.model, bytes };
      this.emit("model-progress", { bytes, totalBytes: MODEL_TOTAL });
      if (fail && bytes > MODEL_TOTAL * 0.45) {
        clearInterval(this.modelTimer!);
        this.modelTimer = null;
        this.model = { ...this.model, state: "error", error: "Download interrupted: connection reset (os error 10054)" };
        this.emit("model-finished", { ok: false, error: this.model.error });
      } else if (bytes >= MODEL_TOTAL) {
        clearInterval(this.modelTimer!);
        this.modelTimer = null;
        this.model = { ...this.model, state: "ready", bytes: MODEL_TOTAL };
        this.emit("model-finished", { ok: true });
      }
    }, 120 * SPEED);
  }

  async cancelModelDownload(): Promise<void> {
    if (this.modelTimer) clearInterval(this.modelTimer);
    this.modelTimer = null;
    if (this.model.state === "downloading") this.model = { ...this.model, state: "missing" };
  }

  private inScope(path: string, scope: string, includeSubfolders: boolean): boolean {
    return includeSubfolders ? strictlyWithin(path, scope) : lower(parentOf(path)) === lower(scope);
  }

  private classifiable(scope: string, includeSubfolders: boolean): MockNode[] {
    return [...this.disk.values()]
      .filter((n) => n.kind === "file" && !n.cloud && this.isIndexed(n.path) && this.inScope(n.path, scope, includeSubfolders))
      .sort((a, b) => collator.compare(a.path, b.path));
  }

  private verdict(n: MockNode): Classified {
    const buckets = this.ensureBuckets();
    const [slot, confidence, ko, en] = classifyRule(n.name);
    const regular = buckets.filter((b) => !b.isFallback);
    const fallback = buckets.find((b) => b.isFallback) ?? buckets[buckets.length - 1];
    const bucket = slot >= 0 && regular[slot] ? regular[slot] : fallback;
    const installer = ["exe", "msi", "msix", "iso"].includes(extOf(n.name));
    const summary = installer ? t("설치 파일 — 설치했다면 지워도 됨") : currentLang() === "ko" ? ko : en;
    return { path: n.path, bucket: bucket.name, confidence, summary, manual: false };
  }

  private seedClassified() {
    for (const n of this.classifiable(DOWNLOADS, false)) this.classified.set(n.path, this.verdict(n));
    const csv = this.classified.get(`${DOWNLOADS}\\sample-data.csv`);
    const dev = this.buckets?.[3];
    if (csv && dev) Object.assign(csv, { bucket: dev.name, manual: true });
    // leave a few unclassified so "N files not yet categorized" shows up
    this.classified.delete(`${DOWNLOADS}\\readme-first.txt`);
    this.classified.delete(`${DOWNLOADS}\\podcast-episode-42.mp3`);
  }

  async classify(req: ClassifyRequest): Promise<void> {
    if (this.model.state !== "ready") throw new Error("The model is not downloaded yet");
    if (this.classifyRun) throw new Error("Already categorizing");
    this.ensureBuckets();
    if (req.mode === "all") for (const [p, c] of this.classified) if (!c.manual && this.inScope(p, req.scope, req.includeSubfolders)) this.classified.delete(p);
    const pending = this.classifiable(req.scope, req.includeSubfolders).filter((n) => !this.classified.has(n.path));
    const run = { cancelled: false };
    this.classifyRun = run;
    void (async () => {
      let done = 0;
      for (const n of pending) {
        if (run.cancelled) break;
        this.emit("classify-progress", { phase: "classify", done, total: pending.length, current: n.name });
        await sleep(260);
        if (run.cancelled) break;
        this.classified.set(n.path, this.verdict(n));
        done++;
      }
      this.emit("classify-progress", { phase: "classify", done, total: pending.length });
      this.classifyRun = null;
      this.emit("classify-finished", { classified: done, failed: 0, cancelled: run.cancelled });
    })();
  }

  async cancelClassify(): Promise<void> {
    if (this.classifyRun) this.classifyRun.cancelled = true;
  }

  async listClassified(scope: string, includeSubfolders: boolean): Promise<ClassifiedList> {
    this.ensureBuckets();
    const files = this.classifiable(scope, includeSubfolders);
    const results: Classified[] = [];
    let pending = 0;
    for (const n of files) {
      const c = this.classified.get(n.path);
      if (c) results.push({ ...c });
      else pending++;
    }
    return { results, pending };
  }

  async setBucket(paths: string[], bucket: string): Promise<void> {
    for (const p of paths) {
      const c = this.classified.get(p);
      if (c) Object.assign(c, { bucket, manual: true });
      else this.classified.set(p, { path: p, bucket, confidence: 1, summary: "", manual: true });
    }
  }

  // ───── leftovers ─────

  async scanLeftovers(): Promise<LeftoverReport> {
    const areas = ["%APPDATA%", "%LOCALAPPDATA%"];
    const names = ["Microsoft", "Google", "OldEditor", "Mozilla", "RetroRacer", "Programs", "Tunebox", "Inkwell Labs", "NVIDIA", "Temp"];
    for (let i = 0; i < names.length; i++) {
      this.emit("leftover-progress", { phase: "scan", done: i, total: names.length, current: `${areas[i % 2]}\\${names[i]}` });
      await sleep(110);
    }
    return { items: this.leftovers.map((i) => ({ ...i, paths: i.paths.map((p) => ({ ...p })) })), scannedAreas: areas };
  }

  async trashLeftovers(paths: string[]): Promise<OpResult> {
    await sleep(60);
    const batch = this.newBatch("trash");
    const failed: OpFailure[] = [];
    let done = 0;
    for (const p of paths) {
      const item = this.leftovers.find((i) => i.paths.some((x) => x.path === p));
      if (!item) {
        failed.push({ path: p, reason: "not a reported leftover" });
        continue;
      }
      const lp = item.paths.find((x) => x.path === p)!;
      item.paths = item.paths.filter((x) => x !== lp);
      item.bytes = item.paths.reduce((s, x) => s + x.bytes, 0);
      if (item.paths.length === 0) this.leftovers = this.leftovers.filter((i) => i !== item);
      batch.ops.push({ type: "leftover", item: { ...item, paths: [] }, path: lp });
      done++;
    }
    return this.finish(batch, done, failed, []);
  }

  // ───── shell ─────

  async pickFolders(options?: PickFolderOptions): Promise<string[]> {
    await sleep(80);
    if (options?.multiple) return [PICTURES];
    if (options?.title === t("왼쪽 폴더 선택…")) return [PHOTOS_LEFT];
    if (options?.title === t("오른쪽 폴더 선택…")) return [PHOTOS_RIGHT];
    return [SORTED];
  }

  previewUrl(path: string): string {
    const name = nameOf(path);
    const ext = extOf(name);
    if (!["jpg", "jpeg", "png", "gif", "webp"].includes(ext)) return "";
    return /^screenshot/i.test(name) ? screenshotSvg(hashNum(name)) : photoSvg(hashNum(name));
  }

  onFileDrop(): () => void {
    return () => {};
  }
}

