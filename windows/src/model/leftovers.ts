// App Leftovers state. Selecting an app row means all of its folders; expanded path rows select one folder.
import type { LeftoverItem, LeftoverPath, LeftoverReport, ProgressEvent } from "../api/types";
import { t } from "../i18n";
import { emptySel, type Sel } from "../util/selection";
import { errorText, type AppModel } from "./app";

export const appKey = (id: string) => `app:${id}`;

export type LeftoverRow =
  | { type: "app"; key: string; item: LeftoverItem; expanded: boolean }
  | { type: "path"; key: string; item: LeftoverItem; path: LeftoverPath };

export class LeftoversModel {
  report: LeftoverReport | null = null;
  running = false;
  progress = "";
  stale = false;
  sel: Sel = emptySel();
  expanded = new Set<string>();

  constructor(private app: AppModel) {}

  async run() {
    if (this.running) return;
    this.running = true;
    this.stale = false;
    this.progress = t("검사 중…");
    this.app.changed();
    try {
      this.report = await this.app.backend.scanLeftovers();
      this.sel = emptySel();
    } catch (e) {
      this.app.showError(errorText(e));
    } finally {
      this.running = false;
      this.progress = "";
      this.app.changed();
    }
  }

  onProgress(e: ProgressEvent) {
    if (!this.running) return;
    this.progress = e.current ? t("{0} 확인 중…", e.current) : t("검사 중…");
    this.app.changed();
  }

  rows(): LeftoverRow[] {
    const rows: LeftoverRow[] = [];
    for (const item of this.report?.items ?? []) {
      const open = this.expanded.has(item.id);
      rows.push({ type: "app", key: appKey(item.id), item, expanded: open });
      if (open) for (const p of item.paths) rows.push({ type: "path", key: p.path, item, path: p });
    }
    return rows;
  }

  toggleExpanded(id: string) {
    if (this.expanded.has(id)) this.expanded.delete(id);
    else this.expanded.add(id);
    this.app.changed();
  }

  /** The folders the selection stands for (an app row = all of its folders). */
  selectedLeftoverPaths(): LeftoverPath[] {
    const out: LeftoverPath[] = [];
    const seen = new Set<string>();
    for (const item of this.report?.items ?? []) {
      const whole = this.sel.selected.has(appKey(item.id));
      for (const p of item.paths)
        if ((whole || this.sel.selected.has(p.path)) && !seen.has(p.path)) {
          seen.add(p.path);
          out.push(p);
        }
    }
    return out;
  }

  selectedPaths(): string[] {
    return this.selectedLeftoverPaths().map((p) => p.path);
  }

  selectedAppCount(): number {
    return (this.report?.items ?? []).filter((i) => this.sel.selected.has(appKey(i.id)) || i.paths.some((p) => this.sel.selected.has(p.path))).length;
  }

  selectAll() {
    const keys = (this.report?.items ?? []).map((i) => appKey(i.id));
    this.sel = { selected: new Set(keys), anchor: keys[0] ?? null, cursor: keys[0] ?? null };
    this.app.changed();
  }

  clearSelection() {
    this.sel = emptySel();
    this.app.changed();
  }

  setSel(sel: Sel) {
    this.sel = sel;
    this.app.changed();
  }

  pathsFor(keys: Iterable<string>): string[] {
    const set = new Set(keys);
    const out: string[] = [];
    for (const item of this.report?.items ?? []) for (const p of item.paths) if (set.has(appKey(item.id)) || set.has(p.path)) out.push(p.path);
    return out;
  }

  trashSelection() {
    const paths = this.selectedLeftoverPaths();
    if (!paths.length) return;
    this.app.requestTrash(
      paths.map((p) => p.path),
      { kind: "leftovers", knownBytes: paths.reduce((s, p) => s + p.bytes, 0) },
    );
  }
}
