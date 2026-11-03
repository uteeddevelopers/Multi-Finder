// Compare Folders state (port of CompareModel.swift).
import type { DiffEntry, DiffResult, DiffStatus, ProgressEvent } from "../api/types";
import { t } from "../i18n";
import { emptySel, type Sel } from "../util/selection";
import { basename, isInside, samePath } from "../util/paths";
import { errorText, type AppModel } from "./app";

export type Side = "left" | "right";

export class CompareModel {
  left: string | null = null;
  right: string | null = null;
  result: DiffResult | null = null;
  running = false;
  progress = "";
  stale = false;
  statusFilter: DiffStatus | null = null;
  sel: Sel = emptySel();

  constructor(private app: AppModel) {}

  setSide(side: Side, path: string) {
    if (side === "left") this.left = path;
    else this.right = path;
    this.result = null;
    this.sel = emptySel();
    this.app.changed();
  }

  async pick(side: Side) {
    try {
      const picked = await this.app.backend.pickFolders({ title: side === "left" ? t("왼쪽 폴더 선택…") : t("오른쪽 폴더 선택…") });
      if (picked.length) this.setSide(side, picked[0]);
    } catch (e) {
      this.app.showError(errorText(e));
    }
  }

  swap() {
    [this.left, this.right] = [this.right, this.left];
    this.result = null;
    this.sel = emptySel();
    this.app.changed();
  }

  /** Same folder, or one inside the other, would compare a folder with itself. */
  get canRun(): boolean {
    const { left, right } = this;
    if (!left || !right || this.running) return false;
    return !samePath(left, right) && !isInside(left, right) && !isInside(right, left);
  }

  async run() {
    if (!this.canRun) return;
    const left = this.left!, right = this.right!;
    this.running = true;
    this.stale = false;
    this.progress = t("폴더 스캔 중…");
    this.app.changed();
    try {
      const r = await this.app.backend.compareFolders(left, right);
      this.result = r;
      const ids = new Set(r.entries.map((e) => e.id));
      this.sel = { ...this.sel, selected: new Set([...this.sel.selected].filter((id) => ids.has(id))) };
      this.app.invalidate();
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
    this.progress = e.phase === "hash" ? t("내용 확인 중 … {0}/{1}", e.done, e.total) : t("폴더 스캔 중…");
    this.app.changed();
  }

  setStatusFilter(s: DiffStatus | null) {
    this.statusFilter = s;
    const visible = new Set(this.visibleEntries().map((e) => e.id));
    this.sel = { ...this.sel, selected: new Set([...this.sel.selected].filter((id) => visible.has(id))) };
    this.app.changed();
  }

  visibleEntries(): DiffEntry[] {
    const r = this.result;
    if (!r) return [];
    return this.statusFilter ? r.entries.filter((e) => e.status === this.statusFilter) : r.entries;
  }

  selectedEntries(): DiffEntry[] {
    const r = this.result;
    if (!r) return [];
    return r.entries.filter((e) => this.sel.selected.has(e.id));
  }

  /** Real file paths of the selected rows. `oneSidedOnly` leaves out rows that exist on both sides. */
  selectedPaths(oneSidedOnly: boolean): string[] {
    return this.selectedEntries().flatMap((e) => {
      const sides = [e.left?.path, e.right?.path].filter((p): p is string => !!p);
      return oneSidedOnly && sides.length > 1 ? [] : sides;
    });
  }

  selectionHas(side: Side): boolean {
    return this.selectedEntries().some((e) => (side === "left" ? !!e.left : !!e.right));
  }

  /** One-sided files → the other side, same relative path (confirmed first). */
  moveOnly(from: Side) {
    const r = this.result;
    if (!r) return;
    const status: DiffStatus = from === "left" ? "leftOnly" : "rightOnly";
    const paths = r.entries.filter((e) => e.status === status).map((e) => (from === "left" ? e.left?.path : e.right?.path)).filter((p): p is string => !!p);
    if (!paths.length) return;
    const fromRoot = from === "left" ? r.left : r.right;
    const toRoot = from === "left" ? r.right : r.left;
    const target = basename(toRoot);
    this.app.requestBatchMove({ title: t("{0}개 항목을 {1}(으)로 옮길까요?", paths.length, target), paths, fromRoot, toRoot, folderName: target });
  }

  /** One copy of every identical pair → Recycle Bin (confirmed first). */
  trashIdentical(side: Side) {
    const r = this.result;
    if (!r) return;
    const paths = r.entries
      .filter((e) => e.status === "identical" && e.left && e.right)
      .map((e) => (side === "left" ? e.left!.path : e.right!.path));
    this.app.requestTrash(paths, { kind: "items" });
  }

  /** Selected rows → Recycle Bin. Rows on both sides need a side; with `null` (Delete key) they are skipped. */
  trashSelected(side: Side | null) {
    const paths: string[] = [];
    let bothSides = 0;
    for (const e of this.selectedEntries()) {
      if (e.left && e.right) {
        if (!side) {
          bothSides++;
          continue;
        }
        paths.push(side === "left" ? e.left.path : e.right.path);
      } else if (e.left) {
        if (side !== "right") paths.push(e.left.path);
      } else if (e.right) {
        if (side !== "left") paths.push(e.right.path);
      }
    }
    const note = bothSides > 0 ? t("양쪽에 모두 있는 {0}개는 건너뜁니다 — 오른쪽 클릭 메뉴에서 지울 쪽을 고르세요.", bothSides) : undefined;
    if (!paths.length) {
      if (note) this.app.showError(note, t("어느 쪽을 지울지 골라 주세요"));
      return;
    }
    this.app.requestTrash(paths, { kind: "items", note });
  }

  setSel(sel: Sel) {
    this.sel = sel;
    this.app.changed();
  }
}
