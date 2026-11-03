// Categories screen state (port of CategoriesModel.swift). Files stay where they are; buckets are virtual.
import type { Bucket, Classified, ClassifyFinishedEvent, ModelFinishedEvent, ModelProgressEvent, ModelStatus, ProgressEvent } from "../api/types";
import { emptySel, type Sel } from "../util/selection";
import { samePath } from "../util/paths";
import { errorText, type AppModel } from "./app";

export class CategoriesModel {
  scope: string | null = null;
  includeSubfolders = false;
  buckets: Bucket[] = [];
  results: Classified[] = [];
  pending = 0;
  selectedBucket: string | null = null;
  sel: Sel = emptySel();
  running = false;
  progressDone = 0;
  progressTotal = 0;
  currentFile = "";
  model: ModelStatus | null = null;
  showEditor = false;
  private lastReload = 0;
  private reloadSeq = 0;

  constructor(private app: AppModel) {}

  get effectiveScope(): string | null {
    const folders = this.app.settings.folders;
    if (this.scope && folders.some((f) => samePath(f, this.scope!))) return this.scope;
    return folders[0] ?? null;
  }

  async load() {
    const b = this.app.backend;
    try {
      const [buckets, model] = await Promise.all([b.getBuckets(), b.modelStatus()]);
      this.buckets = buckets;
      this.model = model;
      this.app.changed();
    } catch (e) {
      console.error(e);
    }
    await this.reload();
  }

  async reload() {
    const scope = this.effectiveScope;
    const seq = ++this.reloadSeq;
    this.lastReload = Date.now();
    if (!scope) {
      this.results = [];
      this.pending = 0;
      this.app.changed();
      return;
    }
    try {
      const r = await this.app.backend.listClassified(scope, this.includeSubfolders);
      if (seq !== this.reloadSeq) return;
      this.results = r.results;
      this.pending = r.pending;
      const visible = new Set(this.visible().map((c) => c.path));
      this.sel = { ...this.sel, selected: new Set([...this.sel.selected].filter((p) => visible.has(p))) };
      this.app.changed();
    } catch (e) {
      console.error(e);
    }
  }

  setScope(scope: string) {
    this.scope = scope;
    this.sel = emptySel();
    void this.reload();
    this.app.changed();
  }

  setIncludeSubfolders(on: boolean) {
    this.includeSubfolders = on;
    void this.reload();
    this.app.changed();
  }

  selectBucket(name: string | null) {
    this.selectedBucket = name;
    const visible = new Set(this.visible().map((c) => c.path));
    this.sel = { ...this.sel, selected: new Set([...this.sel.selected].filter((p) => visible.has(p))) };
    this.app.changed();
  }

  counts(): Map<string, number> {
    const m = new Map<string, number>();
    for (const c of this.results) m.set(c.bucket, (m.get(c.bucket) ?? 0) + 1);
    return m;
  }

  /** Files of the chosen bucket, most confident first. */
  visible(): Classified[] {
    const base = this.selectedBucket === null ? this.results : this.results.filter((c) => c.bucket === this.selectedBucket);
    return [...base].sort((a, b) => (a.confidence !== b.confidence ? b.confidence - a.confidence : a.path.localeCompare(b.path)));
  }

  get modelReady(): boolean {
    return this.model?.state === "ready";
  }

  async run(mode: "new" | "all") {
    const scope = this.effectiveScope;
    if (this.running || !scope || !this.modelReady) return;
    this.running = true;
    this.progressDone = 0;
    this.progressTotal = 0;
    this.currentFile = "";
    this.app.changed();
    try {
      await this.app.backend.classify({ scope, includeSubfolders: this.includeSubfolders, mode });
    } catch (e) {
      this.running = false;
      this.app.showError(errorText(e));
    }
  }

  cancel() {
    void this.app.backend.cancelClassify().catch(() => undefined);
  }

  onProgress(e: ProgressEvent) {
    this.running = true;
    this.progressDone = e.done;
    this.progressTotal = e.total;
    this.currentFile = e.current ?? "";
    if (Date.now() - this.lastReload > 1500) void this.reload();
    this.app.changed();
  }

  onFinished(_e: ClassifyFinishedEvent) {
    this.running = false;
    this.currentFile = "";
    void this.reload();
    this.app.changed();
  }

  async move(paths: string[], bucket: string) {
    if (!paths.length) return;
    try {
      await this.app.backend.setBucket(paths, bucket);
      await this.reload();
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  moveSelectionToFolder() {
    void this.app.requestMovePaths([...this.sel.selected].sort());
  }

  trashSelection() {
    this.app.requestTrash([...this.sel.selected], { kind: "items" });
  }

  setSel(sel: Sel) {
    this.sel = sel;
    this.app.changed();
  }

  openEditor(open: boolean) {
    this.showEditor = open;
    this.app.changed();
  }

  async saveBuckets(list: Bucket[]) {
    try {
      this.buckets = await this.app.backend.saveBuckets(list);
      if (this.selectedBucket && !this.buckets.some((b) => b.name === this.selectedBucket)) this.selectedBucket = null;
      await this.reload();
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  // ───── model download ─────

  async downloadModel() {
    if (!this.model) return;
    this.model = { ...this.model, state: "downloading", error: undefined };
    this.app.changed();
    try {
      await this.app.backend.downloadModel();
    } catch (e) {
      this.model = { ...this.model, state: "error", error: errorText(e) };
      this.app.changed();
    }
  }

  async cancelDownload() {
    try {
      await this.app.backend.cancelModelDownload();
      this.model = await this.app.backend.modelStatus();
      this.app.changed();
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  onModelProgress(e: ModelProgressEvent) {
    if (!this.model) return;
    this.model = { ...this.model, state: "downloading", bytes: e.bytes, totalBytes: e.totalBytes };
    this.app.changed();
  }

  async onModelFinished(e: ModelFinishedEvent) {
    try {
      this.model = await this.app.backend.modelStatus();
    } catch {
      if (this.model) this.model = { ...this.model, state: e.ok ? "ready" : "error", error: e.error };
    }
    if (!e.ok && this.model && this.model.state !== "error") this.model = { ...this.model, state: "error", error: e.error };
    this.app.changed();
  }
}
