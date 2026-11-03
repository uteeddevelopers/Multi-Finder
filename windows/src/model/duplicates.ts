// Duplicates screen state (port of DuplicatesModel.swift).
// Safety: the UI never lets every file of a group be selected; the backend's trash_duplicates checks again.
import type { CopyTree, DuplicateReport, Entry, FileGroup, ProgressEvent } from "../api/types";
import { t } from "../i18n";
import { emptySel, type Sel } from "../util/selection";
import { basename, isUnderOrSame } from "../util/paths";
import { errorText, type AppModel } from "./app";

export type DupRow =
  | { type: "section"; key: string; text: string }
  | { type: "tree"; key: string; tree: CopyTree; expanded: boolean }
  | { type: "group"; key: string; group: FileGroup; indent: number }
  | { type: "file"; key: string; file: Entry; group: FileGroup; indent: number; first: boolean; last: boolean };

export class DuplicatesModel {
  scope: string | null = null;
  minSize = 1024 * 1024;
  hideWithinProject = true;
  foldCopyTrees = true;
  report: DuplicateReport | null = null;
  running = false;
  progress = "";
  stale = false;
  sel: Sel = emptySel();
  expandedTrees = new Set<string>();
  /** Shown in the action bar when a click would have selected every copy of a set. */
  keepOneHint = false;
  private cancelRequested = false;
  private waitingForScan = false;
  private hintTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private app: AppModel) {}

  async run() {
    if (this.running) return;
    this.running = true;
    this.stale = false;
    this.cancelRequested = false;
    this.waitingForScan = this.app.isScanning;
    this.progress = this.waitingForScan ? t("폴더 스캔 중…") : t("크기가 같은 파일 찾는 중…");
    this.app.changed();
    try {
      const report = await this.app.backend.findDuplicates({
        scope: this.scope ?? undefined,
        minSize: Math.max(1, this.minSize),
        hideWithinProject: this.hideWithinProject,
        foldCopyTrees: this.foldCopyTrees,
      });
      this.report = report;
      const files = new Set(this.allGroups().flatMap((g) => g.files.map((f) => f.path)));
      const kept = new Set([...this.sel.selected].filter((p) => files.has(p)));
      this.sel = this.enforceKeepOne(emptySel(), { ...this.sel, selected: kept });
      const treeIds = new Set(report.copyTrees.map((tr) => tr.id));
      for (const id of [...this.expandedTrees]) if (!treeIds.has(id)) this.expandedTrees.delete(id);
    } catch (e) {
      if (!this.cancelRequested) this.app.showError(errorText(e));
    } finally {
      this.running = false;
      this.progress = "";
      this.waitingForScan = false;
      this.app.changed();
    }
  }

  /** The backend waits for scans itself; this only refreshes the progress line. */
  resumeAfterScan() {
    if (this.running && this.waitingForScan) {
      this.waitingForScan = false;
      this.progress = t("크기가 같은 파일 찾는 중…");
      this.app.changed();
    }
  }

  onProgress(e: ProgressEvent) {
    if (!this.running) return;
    this.waitingForScan = false;
    this.progress = e.phase === "hash" ? t("내용 확인 중 … {0}/{1}", e.done, e.total) : t("크기가 같은 파일 찾는 중…");
    this.app.changed();
  }

  cancel() {
    this.cancelRequested = true;
    void this.app.backend.cancelDuplicates().catch(() => undefined);
  }

  setOption(patch: Partial<Pick<DuplicatesModel, "scope" | "minSize" | "hideWithinProject" | "foldCopyTrees">>) {
    Object.assign(this, patch);
    this.app.changed();
  }

  allGroups(): FileGroup[] {
    const r = this.report;
    if (!r) return [];
    return [...r.fileGroups, ...r.copyTrees.flatMap((tr) => tr.groups)];
  }

  /** Groups whose files are on screen (file groups + groups of expanded copy trees). */
  visibleGroups(): FileGroup[] {
    const r = this.report;
    if (!r) return [];
    return [...r.copyTrees.filter((tr) => this.expandedTrees.has(tr.id)).flatMap((tr) => tr.groups), ...r.fileGroups];
  }

  shownGroupCount(): number {
    const r = this.report;
    if (!r) return 0;
    return r.fileGroups.length + r.copyTrees.reduce((s, tr) => s + tr.groups.length, 0);
  }

  rows(): DupRow[] {
    const r = this.report;
    if (!r) return [];
    const rows: DupRow[] = [];
    const pushGroup = (g: FileGroup, indent: number) => {
      rows.push({ type: "group", key: `g:${g.id}`, group: g, indent });
      g.files.forEach((f, i) => rows.push({ type: "file", key: f.path, file: f, group: g, indent, first: i === 0, last: i === g.files.length - 1 }));
    };
    if (r.copyTrees.length) {
      rows.push({ type: "section", key: "s:trees", text: t("폴더끼리 겹침 — ‘비교’를 누르면 폴더 단위로 정리할 수 있습니다") });
      for (const tr of r.copyTrees) {
        const open = this.expandedTrees.has(tr.id);
        rows.push({ type: "tree", key: `t:${tr.id}`, tree: tr, expanded: open });
        if (open) for (const g of tr.groups) pushGroup(g, 1);
      }
    }
    if (r.fileGroups.length) {
      rows.push({ type: "section", key: "s:groups", text: t("똑같은 파일 {0}묶음 — 남길 것만 빼고 고른 뒤 휴지통으로", r.fileGroups.length) });
      for (const g of r.fileGroups) pushGroup(g, 0);
    }
    return rows;
  }

  toggleTree(id: string) {
    if (this.expandedTrees.has(id)) this.expandedTrees.delete(id);
    else this.expandedTrees.add(id);
    this.app.changed();
  }

  /**
   * Never every copy of a set: when a change would select all files of a group, the file that was
   * just added first (in path order) is left unselected — so a Ctrl+click on the last copy is refused
   * and Ctrl+A / Shift ranges keep the first copy.
   */
  enforceKeepOne(prev: Sel, next: Sel): Sel {
    const selected = new Set(next.selected);
    let refused = false;
    for (const g of this.allGroups()) {
      if (!g.files.every((f) => selected.has(f.path))) continue;
      const added = g.files.find((f) => !prev.selected.has(f.path));
      selected.delete((added ?? g.files[0]).path);
      refused = true;
    }
    if (refused) this.flashKeepOneHint();
    return refused ? { ...next, selected } : next;
  }

  setSel(next: Sel) {
    this.sel = this.enforceKeepOne(this.sel, next);
    this.app.changed();
  }

  private flashKeepOneHint() {
    this.keepOneHint = true;
    if (this.hintTimer) clearTimeout(this.hintTimer);
    this.hintTimer = setTimeout(() => {
      this.keepOneHint = false;
      this.app.changed();
    }, 4000);
  }

  // ───── quick select ─────

  selectAllButFirst() {
    const selected = new Set(this.visibleGroups().flatMap((g) => g.files.slice(1).map((f) => f.path)));
    this.sel = { selected, anchor: null, cursor: this.sel.cursor };
    this.app.changed();
  }

  /** Copies inside one folder (e.g. the Downloads copies). A set entirely inside it keeps its first file. */
  selectUnder(folder: string) {
    const selected = new Set<string>();
    for (const g of this.visibleGroups()) {
      const inside = g.files.filter((f) => isUnderOrSame(f.path, folder));
      const pick = inside.length === g.files.length ? inside.slice(1) : inside;
      for (const f of pick) selected.add(f.path);
    }
    this.sel = { selected, anchor: null, cursor: this.sel.cursor };
    this.app.changed();
  }

  clearSelection() {
    this.sel = emptySel();
    this.app.changed();
  }

  selectedFiles(): Entry[] {
    const seen = new Set<string>();
    const out: Entry[] = [];
    for (const g of this.allGroups())
      for (const f of g.files)
        if (this.sel.selected.has(f.path) && !seen.has(f.path)) {
          seen.add(f.path);
          out.push(f);
        }
    return out;
  }

  // ───── ignore ─────

  async ignoreGroup(id: string) {
    try {
      await this.app.backend.ignoreDuplicateGroup(id);
      void this.run();
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  async ignoreTree(tree: CopyTree) {
    try {
      for (const g of tree.groups) await this.app.backend.ignoreDuplicateGroup(g.id);
      void this.run();
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  async clearIgnores() {
    try {
      await this.app.backend.clearDuplicateIgnores();
      void this.run();
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  // ───── trash ─────

  /** Selected copies → Recycle Bin (after confirmation). At least one copy of every set always stays. */
  trashSelection() {
    const toTrash: Entry[] = [];
    let keptOne = 0;
    for (const g of this.allGroups()) {
      const chosen = g.files.filter((f) => this.sel.selected.has(f.path));
      if (!chosen.length) continue;
      if (chosen.length === g.files.length) {
        chosen.shift();
        keptOne++;
      }
      toTrash.push(...chosen);
    }
    if (!toTrash.length) return;
    const note = keptOne > 0 ? t("{0}개 묶음은 모든 사본이 선택되어 하나씩 남깁니다.", keptOne) : undefined;
    this.app.requestTrash(
      toTrash.map((f) => f.path),
      { kind: "duplicates", knownBytes: toTrash.reduce((s, f) => s + f.size, 0), note },
    );
  }

  afterOperation() {
    this.sel = emptySel();
    void this.run();
  }

  scopeLabel(): string {
    return this.scope ? t("{0} 안", basename(this.scope)) : t("전체 인덱스");
  }
}
