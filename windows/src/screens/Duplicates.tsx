// Duplicates — identical contents wherever they are (port of DuplicatesView.swift).
import { useMemo, useRef, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import {
  Button,
  Menu,
  MenuDivider,
  MenuItem,
  MenuItemCheckbox,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Tooltip,
  type MenuProps,
} from "@fluentui/react-components";
import {
  CheckmarkCircleRegular,
  ChevronDownRegular,
  ChevronRightRegular,
  DeleteRegular,
  DocumentCopyRegular,
  FolderFilled,
  FolderOpenRegular,
  HardDriveRegular,
  InfoRegular,
  MoreHorizontalRegular,
  OpenRegular,
  OptionsRegular,
  SearchRegular,
  TaskListLtrRegular,
} from "@fluentui/react-icons";
import type { CopyTree, DuplicateReport, FileGroup } from "../api/types";
import { t } from "../i18n";
import { useApp } from "../model/context";
import type { DupRow } from "../model/duplicates";
import { VirtualList, type VirtualListHandle } from "../components/VirtualList";
import { useContextMenu } from "../components/ContextMenu";
import { EmptyState, FileIcon, Progress, Rich, keepListFocus } from "../components/common";
import { clickSel, indexOfKeys, moveSel, selectAllSel } from "../util/selection";
import { basename, displayPath } from "../util/paths";
import { formatBytes, formatDate } from "../util/format";
import { SIZE_FLOORS } from "../util/fileKinds";

const HEIGHT: Record<DupRow["type"], number> = { section: 34, tree: 56, group: 36, file: 46 };

function hiddenSummary(r: DuplicateReport): string {
  const parts: string[] = [];
  if (r.hiddenWithinProject > 0) parts.push(t("프로젝트 내부 {0}묶음 숨김", r.hiddenWithinProject));
  if (r.hiddenByIgnoreList > 0) parts.push(t("무시 목록 {0}묶음", r.hiddenByIgnoreList));
  return parts.join(" · ");
}

export function DuplicatesActions() {
  const app = useApp();
  const dup = app.duplicates;
  const sizeLabel = SIZE_FLOORS.find((s) => s.bytes === dup.minSize)?.label ?? "모든 크기";
  const onScope: MenuProps["onCheckedValueChange"] = (_, d) => dup.setOption({ scope: d.checkedItems[0] === "" ? null : d.checkedItems[0] });
  const onSize: MenuProps["onCheckedValueChange"] = (_, d) => dup.setOption({ minSize: Number(d.checkedItems[0]) });
  const onOpts: MenuProps["onCheckedValueChange"] = (_, d) => {
    const on = new Set(d.checkedItems);
    dup.setOption({ hideWithinProject: on.has("project"), foldCopyTrees: on.has("trees") });
  };
  return (
    <>
      <Menu checkedValues={{ scope: [dup.scope ?? ""] }} onCheckedValueChange={onScope}>
        <MenuTrigger disableButtonEnhancement>
          <Tooltip content={t("어느 폴더의 파일이 포함된 중복을 볼지")} relationship="description">
            <Button appearance="subtle" icon={<FolderOpenRegular />} iconPosition="before">
              {dup.scopeLabel()}
            </Button>
          </Tooltip>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuItemRadio name="scope" value="">
              {t("전체 인덱스")}
            </MenuItemRadio>
            <MenuDivider />
            {app.settings.folders.map((f) => (
              <MenuItemRadio key={f} name="scope" value={f}>
                {t("{0} 안", basename(f))}
              </MenuItemRadio>
            ))}
          </MenuList>
        </MenuPopover>
      </Menu>
      <Menu checkedValues={{ size: [String(dup.minSize)] }} onCheckedValueChange={onSize}>
        <MenuTrigger disableButtonEnhancement>
          <Tooltip content={t("이보다 작은 파일은 무시")} relationship="description">
            <Button appearance="subtle">{t(sizeLabel)}</Button>
          </Tooltip>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            {SIZE_FLOORS.map((s) => (
              <MenuItemRadio key={s.bytes} name="size" value={String(s.bytes)}>
                {t(s.label)}
              </MenuItemRadio>
            ))}
          </MenuList>
        </MenuPopover>
      </Menu>
      <Menu
        checkedValues={{ opts: [...(dup.hideWithinProject ? ["project"] : []), ...(dup.foldCopyTrees ? ["trees"] : [])] }}
        onCheckedValueChange={onOpts}
      >
        <MenuTrigger disableButtonEnhancement>
          <Button appearance="subtle" icon={<OptionsRegular />} aria-label={t("옵션")}>
            <span className="tb-label">{t("옵션")}</span>
          </Button>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuItemCheckbox name="opts" value="project">
              {t("같은 프로젝트 안의 중복 숨기기")}
            </MenuItemCheckbox>
            <MenuItemCheckbox name="opts" value="trees">
              {t("폴더끼리 겹치면 폴더 단위로 묶기")}
            </MenuItemCheckbox>
            {(dup.report?.hiddenByIgnoreList ?? 0) > 0 && (
              <>
                <MenuDivider />
                <MenuItem onClick={() => void dup.clearIgnores()}>{t("무시 목록 비우기")}</MenuItem>
              </>
            )}
          </MenuList>
        </MenuPopover>
      </Menu>
      <Tooltip content={t("현재 범위·크기 조건으로 중복 찾기")} relationship="description">
        <Button appearance="primary" icon={<SearchRegular />} disabled={dup.running} onClick={() => void dup.run()}>
          {dup.report ? t("다시 찾기") : t("찾기")}
        </Button>
      </Tooltip>
    </>
  );
}

export function DuplicatesView() {
  const app = useApp();
  const dup = app.duplicates;
  if (dup.running) {
    return (
      <div className="screen-center">
        <Progress text={dup.progress} />
        <Button onClick={() => dup.cancel()}>{t("중지")}</Button>
      </div>
    );
  }
  const r = dup.report;
  if (!r) {
    return (
      <EmptyState
        icon={<DocumentCopyRegular fontSize={48} />}
        title={t("중복 파일 찾기")}
        actions={
          <Button appearance="primary" onClick={() => void dup.run()}>
            {t("찾기")}
          </Button>
        }
      >
        {t("이름이나 위치가 달라도 내용이 완전히 같으면 중복으로 봅니다.\n찾기만 할 뿐, 지우는 건 직접 고른 뒤 확인을 거칩니다.")}
      </EmptyState>
    );
  }
  if (r.fileGroups.length === 0 && r.copyTrees.length === 0) {
    return (
      <EmptyState icon={<CheckmarkCircleRegular fontSize={48} />} title={t("정리할 중복이 없습니다")}>
        {hiddenSummary(r)}
      </EmptyState>
    );
  }
  return <DuplicateResults report={r} />;
}

function DuplicateResults({ report }: { report: DuplicateReport }) {
  const app = useApp();
  const dup = app.duplicates;
  const menu = useContextMenu();
  const listRef = useRef<VirtualListHandle>(null);
  const rows = dup.rows();
  const heights = useMemo(() => rows.map((r) => HEIGHT[r.type]), [rows]);
  const keys = useMemo(() => rows.filter((r) => r.type === "file").map((r) => r.key), [rows]);
  const keyIndex = useMemo(() => indexOfKeys(keys), [keys]);
  const rowIndex = useMemo(() => indexOfKeys(rows.map((r) => r.key)), [rows]);
  const sel = dup.sel;
  const focusIndex = sel.cursor ? (rowIndex.get(sel.cursor) ?? -1) : -1;
  const files = dup.selectedFiles();
  const selBytes = files.reduce((s, f) => s + f.size, 0);
  const shown = dup.shownGroupCount();

  const onPointerDown = (e: PointerEvent, key: string) => {
    if (e.button !== 0) return;
    listRef.current?.focus();
    dup.setSel(clickSel(sel, keys, keyIndex, key, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey }));
  };
  const onContextMenu = (e: MouseEvent, key: string) => {
    e.preventDefault();
    let cur = dup.sel;
    if (!cur.selected.has(key)) {
      dup.setSel({ selected: new Set([key]), anchor: key, cursor: key });
      cur = dup.sel;
    }
    const paths = cur.selected.size ? [...cur.selected] : [key];
    menu(e, [
      { label: t("탐색기에서 보기"), icon: <FolderOpenRegular />, onClick: () => void app.reveal(paths) },
      { label: t("열기"), icon: <OpenRegular />, onClick: () => paths.forEach((p) => void app.open(p)) },
      { divider: true },
      { label: t("선택 항목 휴지통으로…"), icon: <DeleteRegular />, disabled: cur.selected.size === 0, onClick: () => dup.trashSelection() },
    ]);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      [...dup.sel.selected].forEach((p) => void app.open(p));
    } else if (e.ctrlKey && (e.key === "a" || e.key === "A")) {
      e.preventDefault();
      dup.setSel(selectAllSel(keys, dup.sel.cursor));
    } else if (e.key === "Escape") dup.clearSelection();
  };

  const groupMenu = (g: FileGroup) => (
    <Menu>
      <MenuTrigger disableButtonEnhancement>
        <Button appearance="subtle" size="small" icon={<MoreHorizontalRegular />} onPointerDown={keepListFocus} aria-label={t("옵션")} />
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuItem onClick={() => void dup.ignoreGroup(g.id)}>{t("이 묶음은 중복으로 보지 않기")}</MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>
  );
  const treeMenu = (tree: CopyTree) => (
    <Menu>
      <MenuTrigger disableButtonEnhancement>
        <Button appearance="subtle" size="small" icon={<MoreHorizontalRegular />} onPointerDown={keepListFocus} onClick={(e) => e.stopPropagation()} aria-label={t("옵션")} />
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuItem onClick={() => void dup.ignoreTree(tree)}>{t("이 폴더들 사이의 중복은 무시")}</MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>
  );

  const renderRow = (i: number, style: CSSProperties): ReactNode => {
    const r = rows[i];
    const roots = app.settings.folders;
    switch (r.type) {
      case "section":
        return (
          <div key={r.key} style={style} className="row section-row">
            {r.text}
          </div>
        );
      case "tree": {
        const tr = r.tree;
        return (
          <div key={r.key} style={style} className="row tree-group-row" onClick={() => dup.toggleTree(tr.id)} title={`${tr.left}\n${tr.right}`}>
            <span className="chev">{r.expanded ? <ChevronDownRegular fontSize={12} /> : <ChevronRightRegular fontSize={12} />}</span>
            <FolderFilled className="ficon ic-folder" fontSize={20} />
            <div className="two-line">
              <div className="name ellipsis">
                {displayPath(tr.left, roots)}
                <span className="approx">≈</span>
                {displayPath(tr.right, roots)}
              </div>
              <div className="sub ellipsis">{t("{0}개 파일이 양쪽에 똑같이 있음 · 예: {1}", tr.fileCount, tr.examples.join(", "))}</div>
            </div>
            <span className="num dim">{formatBytes(tr.bytes)}</span>
            <Button
              size="small"
              onPointerDown={keepListFocus}
              onClick={(e) => {
                e.stopPropagation();
                app.compare.setSide("left", tr.left);
                app.compare.setSide("right", tr.right);
                app.setScreen("compare");
                void app.compare.run();
              }}
            >
              {t("비교")}
            </Button>
            {treeMenu(tr)}
          </div>
        );
      }
      case "group":
        return (
          <div key={r.key} style={style} className={`row group-row indent-${r.indent}`}>
            <DocumentCopyRegular className="ficon dim" fontSize={16} />
            <span className="name auto ellipsis strong">{r.group.files[0]?.name}</span>
            <span className="dim">{t("× {0}", r.group.files.length)}</span>
            <span className="spacer" />
            <span className="num dim">{formatBytes(r.group.size)}</span>
            {groupMenu(r.group)}
          </div>
        );
      case "file": {
        const f = r.file;
        const selected = sel.selected.has(f.path);
        return (
          <div
            key={r.key}
            style={style}
            role="option"
            aria-selected={selected}
            className={`row file-row indent-${r.indent} ${selected ? "selected" : ""} ${sel.cursor === f.path ? "cursor" : ""} ${r.last ? "last-in-group" : ""}`}
            title={f.path}
            onPointerDown={(e) => onPointerDown(e, f.path)}
            onDoubleClick={() => void app.open(f.path)}
            onContextMenu={(e) => onContextMenu(e, f.path)}
          >
            <FileIcon kind={f.kind} ext={f.ext} />
            <div className="two-line">
              <div className="name ellipsis">{f.name}</div>
              <div className="sub ellipsis">{displayPath(f.parent, roots)}</div>
            </div>
            <span className="col-date">{formatDate(f.modified)}</span>
          </div>
        );
      }
    }
  };

  return (
    <div className="screen">
      <div className="summary-bar">
        <HardDriveRegular />
        <span>
          <Rich text={t("똑같은 파일을 하나씩만 남기면 **{0}** 가 비워집니다", formatBytes(report.reclaimableBytes))} />
        </span>
        {hiddenSummary(report) && <span className="dim">{hiddenSummary(report)}</span>}
        <span className="spacer" />
        {report.rawGroupCount > 0 && <span className="dim">{t("전체 {0}묶음 중 {1}묶음 표시", report.rawGroupCount, shown)}</span>}
      </div>
      <VirtualList
        ref={listRef}
        count={rows.length}
        rowHeight={heights}
        renderRow={renderRow}
        focusIndex={focusIndex}
        isSelectable={(i) => rows[i]?.type === "file"}
        onNavigate={(i, mods) => dup.setSel(moveSel(dup.sel, keys, keyIndex, rows[i].key, mods))}
        onKeyDown={onKeyDown}
        onBackgroundMouseDown={() => dup.clearSelection()}
        ariaLabel={t("중복 파일")}
        className="dup-list"
      />
      <div className="actionbar">
        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <Button icon={<TaskListLtrRegular />} iconPosition="before">
              {t("빠른 선택")}
            </Button>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem onClick={() => dup.selectAllButFirst()}>{t("각 묶음에서 첫 번째만 남기고 선택")}</MenuItem>
              <MenuDivider />
              {app.settings.folders.map((f) => (
                <MenuItem key={f} onClick={() => dup.selectUnder(f)}>
                  {t("{0} 쪽 사본 선택", basename(f))}
                </MenuItem>
              ))}
              <MenuDivider />
              <MenuItem onClick={() => dup.clearSelection()}>{t("선택 해제")}</MenuItem>
            </MenuList>
          </MenuPopover>
        </Menu>
        {dup.keepOneHint && (
          <span className="hint">
            <InfoRegular /> {t("묶음마다 사본 하나는 남겨 둡니다 — 모든 사본을 고를 수는 없습니다")}
          </span>
        )}
        <span className="spacer" />
        {files.length > 0 && (
          <>
            <span className="dim">{t("{0}개 선택 · {1}", files.length, formatBytes(selBytes))}</span>
            <Button icon={<FolderOpenRegular />} onClick={() => void app.reveal(files.map((f) => f.path))}>
              {t("탐색기에서 보기")}
            </Button>
          </>
        )}
        <Button appearance="primary" icon={<DeleteRegular />} disabled={files.length === 0} onClick={() => dup.trashSelection()}>
          {t("선택 항목 휴지통으로…")}
        </Button>
      </div>
    </div>
  );
}

