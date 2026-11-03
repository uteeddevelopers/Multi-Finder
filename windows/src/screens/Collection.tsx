// All Folders — several folders in one Explorer-style tree (port of CollectionView.swift).
import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import {
  Button,
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Spinner,
  Tooltip,
  type MenuProps,
} from "@fluentui/react-components";
import {
  AddRegular,
  ArrowClockwiseRegular,
  ArrowSortRegular,
  ChevronDownRegular,
  ChevronRightRegular,
  ChevronUpRegular,
  CloudRegular,
  ColumnDoubleCompareRegular,
  CopyRegular,
  DeleteRegular,
  DocumentCopyRegular,
  EyeRegular,
  FilterFilled,
  FilterRegular,
  FolderAddRegular,
  FolderArrowRightRegular,
  FolderFilled,
  FolderOpenRegular,
  LayerRegular,
  OpenRegular,
  SearchRegular,
  TextBulletListTreeRegular,
} from "@fluentui/react-icons";
import type { Entry, FileCategory, SortField } from "../api/types";
import { t } from "../i18n";
import { useApp } from "../model/context";
import type { TreeRow } from "../model/app";
import { VirtualList, type VirtualListHandle } from "../components/VirtualList";
import { useContextMenu, type MenuSpec } from "../components/ContextMenu";
import { Banner, EmptyState, FileIcon, keepListFocus } from "../components/common";
import { PreviewPane } from "../components/PreviewPane";
import { useRowDrag } from "../components/useRowDrag";
import { clickSel, emptySel, indexOfKeys, moveSel, pruneSel, selectAllSel } from "../util/selection";
import { basename, dirname, displayPath, isInside, samePath } from "../util/paths";
import { formatBytes, formatDateTime } from "../util/format";
import { CATEGORY_LABEL, CATEGORY_ORDER, RECENT_RANGES, SIZE_FLOORS } from "../util/fileKinds";

const ROW_H = 32;
const SEARCH_ROW_H = 46;

type Row = TreeRow | { type: "search"; key: string; entry: Entry };

function sortLabel(field: SortField) {
  return field === "name" ? t("이름") : field === "modified" ? t("수정일") : field === "size" ? t("크기") : t("형식");
}

function useFilterDescription(): string {
  const app = useApp();
  const f = app.filter;
  const parts: string[] = [];
  if (f.category !== "all") parts.push(t(CATEGORY_LABEL[f.category]));
  const size = SIZE_FLOORS.find((s) => s.bytes === f.minSize);
  if (f.minSize > 0) parts.push(size ? t(size.label) : formatBytes(f.minSize));
  const range = RECENT_RANGES.find((r) => r.days === f.withinDays);
  if (f.withinDays > 0) parts.push(range ? t(range.label) : String(f.withinDays));
  return parts.join(" · ");
}

// ───────────────────────────── toolbar ─────────────────────────────

export function CollectionActions() {
  const app = useApp();
  const filterText = useFilterDescription();
  const operable = app.operablePaths().length > 0;
  const f = app.filter;
  const onFilter: MenuProps["onCheckedValueChange"] = (_, { name, checkedItems }) => {
    const v = checkedItems[0];
    if (name === "kind") app.setFilter({ category: v as FileCategory });
    if (name === "size") app.setFilter({ minSize: Number(v) });
    if (name === "date") app.setFilter({ withinDays: Number(v) });
  };
  const onSort: MenuProps["onCheckedValueChange"] = (_, { name, checkedItems }) => {
    const v = checkedItems[0];
    if (name === "field") app.setSort(v as SortField, app.sort.ascending);
    if (name === "order") app.setSort(app.sort.field, v === "asc");
  };
  return (
    <>
      <Tooltip content={t("선택 항목을 다른 폴더로 이동 (Ctrl+Shift+M)")} relationship="label">
        <Button appearance="subtle" icon={<FolderArrowRightRegular />} disabled={!operable} onClick={() => void app.requestMoveSelection()} />
      </Tooltip>
      <Tooltip content={t("선택 항목을 휴지통으로 (Delete)")} relationship="label">
        <Button appearance="subtle" icon={<DeleteRegular />} disabled={!operable} onClick={() => app.requestTrashSelection()} />
      </Tooltip>
      <Tooltip content={t("미리보기 (스페이스)")} relationship="label">
        <Button
          appearance={app.previewOpen ? "secondary" : "subtle"}
          icon={<EyeRegular />}
          aria-pressed={app.previewOpen}
          disabled={!app.previewOpen && app.sel.selected.size === 0}
          onClick={() => app.togglePreview()}
        />
      </Tooltip>
      <Menu checkedValues={{ field: [app.sort.field], order: [app.sort.ascending ? "asc" : "desc"] }} onCheckedValueChange={onSort}>
        <MenuTrigger disableButtonEnhancement>
          <Tooltip content={t("정렬: {0} {1}", sortLabel(app.sort.field), app.sort.ascending ? "↑" : "↓")} relationship="description">
            <Button appearance="subtle" icon={<ArrowSortRegular />} aria-label={t("정렬")}>
              <span className="tb-label">{t("정렬")}</span>
            </Button>
          </Tooltip>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuGroup>
              <MenuGroupHeader>{t("정렬 기준")}</MenuGroupHeader>
              {(["name", "modified", "size"] as SortField[]).map((field) => (
                <MenuItemRadio key={field} name="field" value={field}>
                  {sortLabel(field)}
                </MenuItemRadio>
              ))}
            </MenuGroup>
            <MenuDivider />
            <MenuGroup>
              <MenuGroupHeader>{t("순서")}</MenuGroupHeader>
              <MenuItemRadio name="order" value="asc">
                {t("오름차순")}
              </MenuItemRadio>
              <MenuItemRadio name="order" value="desc">
                {t("내림차순")}
              </MenuItemRadio>
            </MenuGroup>
          </MenuList>
        </MenuPopover>
      </Menu>
      <Menu checkedValues={{ kind: [f.category], size: [String(f.minSize)], date: [String(f.withinDays)] }} onCheckedValueChange={onFilter}>
        <MenuTrigger disableButtonEnhancement>
          <Tooltip content={app.filterActive ? t("필터: {0}", filterText) : t("형식·크기·기간으로 거르기")} relationship="description">
            <Button appearance={app.filterActive ? "secondary" : "subtle"} icon={app.filterActive ? <FilterFilled /> : <FilterRegular />} aria-label={t("필터")}>
              <span className="tb-label">{t("필터")}</span>
            </Button>
          </Tooltip>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <Menu checkedValues={{ kind: [f.category] }} onCheckedValueChange={onFilter}>
              <MenuTrigger disableButtonEnhancement>
                <MenuItem secondaryContent={t(CATEGORY_LABEL[f.category])}>{t("형식")}</MenuItem>
              </MenuTrigger>
              <MenuPopover>
                <MenuList>
                  {CATEGORY_ORDER.map((c) => (
                    <MenuItemRadio key={c} name="kind" value={c}>
                      {t(CATEGORY_LABEL[c])}
                    </MenuItemRadio>
                  ))}
                </MenuList>
              </MenuPopover>
            </Menu>
            <Menu checkedValues={{ size: [String(f.minSize)] }} onCheckedValueChange={onFilter}>
              <MenuTrigger disableButtonEnhancement>
                <MenuItem secondaryContent={t(SIZE_FLOORS.find((s) => s.bytes === f.minSize)?.label ?? "모든 크기")}>{t("크기")}</MenuItem>
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
            <Menu checkedValues={{ date: [String(f.withinDays)] }} onCheckedValueChange={onFilter}>
              <MenuTrigger disableButtonEnhancement>
                <MenuItem secondaryContent={t(RECENT_RANGES.find((r) => r.days === f.withinDays)?.label ?? "모든 기간")}>{t("기간")}</MenuItem>
              </MenuTrigger>
              <MenuPopover>
                <MenuList>
                  {RECENT_RANGES.map((r) => (
                    <MenuItemRadio key={r.days} name="date" value={String(r.days)}>
                      {t(r.label)}
                    </MenuItemRadio>
                  ))}
                </MenuList>
              </MenuPopover>
            </Menu>
            {app.filterActive && (
              <>
                <MenuDivider />
                <MenuItem onClick={() => app.clearFilters()}>{t("필터 지우기")}</MenuItem>
              </>
            )}
          </MenuList>
        </MenuPopover>
      </Menu>
    </>
  );
}

// ───────────────────────────── list ─────────────────────────────

function ColumnHeader() {
  const app = useApp();
  const head = (field: SortField, cls: string) => {
    const active = app.sort.field === field;
    return (
      <button
        className={`ch ${cls} ${active ? "active" : ""}`}
        title={t("{0}순 정렬", sortLabel(field))}
        onClick={() => app.setSort(field)}
      >
        <span>{sortLabel(field)}</span>
        {active && (app.sort.ascending ? <ChevronUpRegular fontSize={12} /> : <ChevronDownRegular fontSize={12} />)}
      </button>
    );
  };
  return (
    <div className="col-header">
      {head("name", "ch-name")}
      {head("modified", "col-date")}
      {head("size", "col-size")}
    </div>
  );
}

function SizeCell({ entry, path }: { entry?: Entry; path: string }) {
  const app = useApp();
  if (entry && entry.kind === "file") return <>{formatBytes(entry.size)}</>;
  if (entry && entry.kind === "link") return <>{t("--")}</>;
  const info = app.dirInfoOf(path);
  return <>{info ? `${t("{0}개", info.items)} · ${formatBytes(info.bytes)}` : "…"}</>;
}

export function CollectionView() {
  const app = useApp();
  const menu = useContextMenu();
  const listRef = useRef<VirtualListHandle>(null);
  const pendingCollapse = useRef<string | null>(null);
  const [scrollReq, setScrollReq] = useState<{ index: number; nonce: number } | null>(null);
  const [range, setRange] = useState<[number, number]>([0, 40]);
  const filterText = useFilterDescription();

  const searching = app.isSearching;
  const tree = searching ? null : app.treeRows();
  const searchRows = useMemo<Row[]>(() => app.searchResults.map((e) => ({ type: "search" as const, key: e.path, entry: e })), [app.searchResults]);
  const rows: Row[] = searching ? searchRows : tree!.rows;
  const needKey = tree ? tree.need.join("\n") : "";

  useEffect(() => {
    if (tree && tree.need.length) app.ensureChildren(tree.need);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [needKey, app]);

  const keys = useMemo(() => rows.filter((r) => r.type !== "placeholder").map((r) => r.key), [rows]);
  const keyIndex = useMemo(() => indexOfKeys(keys), [keys]);
  const rowIndex = useMemo(() => indexOfKeys(rows.map((r) => r.key)), [rows]);

  // Selection follows what is listed (filters, collapsed folders) — but not while rows are still loading.
  const loading = (tree?.need.length ?? 0) > 0 || app.searchPending;
  useEffect(() => {
    if (loading) return;
    const pruned = pruneSel(app.sel, keyIndex);
    if (pruned !== app.sel) app.setSel(pruned);
  });

  // Scroll to a row requested by the sidebar / "Show in Folder List".
  useEffect(() => {
    const target = app.scrollTo;
    if (!target) return;
    const i = rowIndex.get(target.key);
    if (i !== undefined) {
      setScrollReq({ index: i, nonce: target.nonce });
      app.scrollTo = null;
      listRef.current?.focus();
    }
  });

  // Folder sizes for the rows in view.
  useEffect(() => {
    const [a, b] = range;
    const paths: string[] = [];
    for (let i = Math.max(0, a - 5); i <= Math.min(rows.length - 1, b + 5); i++) {
      const r = rows[i];
      if (r.type === "root" && r.state === "indexed") paths.push(r.path);
      else if ((r.type === "entry" || r.type === "search") && r.entry.kind === "dir") paths.push(r.entry.path);
    }
    if (paths.length) app.requestDirInfo(paths);
  });

  const drag = useRowDrag({
    kinds: ["folder"],
    canDrop: (target, paths) => paths.some((p) => !samePath(p, target.value) && !isInside(target.value, p) && !samePath(dirname(p), target.value)),
    onDrop: (target, paths) => app.drop(paths, target.value),
    label: (paths) => (paths.length === 1 ? basename(paths[0]) : t("{0}개", paths.length)),
    scrollElement: () => listRef.current?.element() ?? null,
    onHoverLong: (target) => {
      if (!searching && !app.expanded.has(target.value)) app.toggleExpanded(target.value, true);
    },
  });

  if (app.settings.folders.length === 0) {
    return (
      <EmptyState
        icon={<LayerRegular fontSize={48} />}
        title={t("모은 폴더가 없습니다")}
        actions={
          <Button appearance="primary" icon={<FolderAddRegular />} onClick={() => void app.addFolderViaPicker()}>
            {t("폴더 추가…")}
          </Button>
        }
      >
        {t("한 화면에 같이 볼 폴더를 추가하세요. 데스크탑·다운로드처럼 자주 어지러워지는 폴더부터.")}
      </EmptyState>
    );
  }

  const sel = app.sel;
  const focusIndex = sel.cursor ? (rowIndex.get(sel.cursor) ?? -1) : -1;

  const dragPayload = (key: string) => {
    const base = sel.selected.has(key) ? [...sel.selected] : [key];
    return base.filter((p) => !app.isRoot(p));
  };

  const onRowPointerDown = (e: PointerEvent, key: string) => {
    if (e.button !== 0) return;
    const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    listRef.current?.focus();
    pendingCollapse.current = null;
    const cur = app.sel;
    if (mods.ctrl || mods.shift || !cur.selected.has(key)) app.setSel(clickSel(cur, keys, keyIndex, key, mods));
    else {
      pendingCollapse.current = key;
      app.setSel({ ...cur, anchor: key, cursor: key });
    }
    if (!mods.ctrl && !mods.shift) drag.begin(e, () => dragPayload(key));
  };

  const onRowClick = (key: string) => {
    if (drag.wasDragging()) return;
    if (pendingCollapse.current === key) app.setSel({ selected: new Set([key]), anchor: key, cursor: key });
    pendingCollapse.current = null;
  };

  const buildMenu = (paths: string[]): MenuSpec[] => {
    const single = paths.length === 1 ? paths[0] : null;
    const singleDir = single && app.isDir(single) ? single : null;
    const operable = paths.filter((p) => !app.isRoot(p));
    const items: MenuSpec[] = [
      { label: t("열기"), icon: <OpenRegular />, onClick: () => paths.forEach((p) => void app.open(p)) },
    ];
    if (searching && single) items.push({ label: t("폴더 목록에서 보기"), icon: <TextBulletListTreeRegular />, onClick: () => app.revealInList(single) });
    items.push(
      { label: t("미리보기 (스페이스)"), icon: <EyeRegular />, onClick: () => app.togglePreview(true) },
      { label: t("탐색기에서 보기"), icon: <FolderOpenRegular />, onClick: () => void app.reveal(paths) },
      { label: t("경로 복사"), icon: <CopyRegular />, onClick: () => void app.copyPaths(paths) },
    );
    if (singleDir) {
      const side = app.compare.left === null ? "left" : "right";
      items.push(
        { divider: true },
        { label: t("이 폴더를 모아 보기에 추가"), icon: <AddRegular />, disabled: app.isRoot(singleDir), onClick: () => void app.addFolderPaths([singleDir]) },
        {
          label: t("이 폴더 안 중복 찾기"),
          icon: <DocumentCopyRegular />,
          onClick: () => {
            app.duplicates.setOption({ scope: app.isRoot(singleDir) ? singleDir : singleDir });
            app.setScreen("duplicates");
            void app.duplicates.run();
          },
        },
        {
          label: side === "left" ? t("폴더 비교의 왼쪽으로") : t("폴더 비교의 오른쪽으로"),
          icon: <ColumnDoubleCompareRegular />,
          onClick: () => {
            app.compare.setSide(side, singleDir);
            if (app.compare.canRun) {
              app.setScreen("compare");
              void app.compare.run();
            }
          },
        },
      );
    }
    items.push(
      { divider: true },
      { label: t("이동…"), icon: <FolderArrowRightRegular />, disabled: !operable.length, onClick: () => void app.requestMovePaths(operable.sort()) },
      { label: t("휴지통으로 이동…"), icon: <DeleteRegular />, disabled: !operable.length, danger: true, onClick: () => app.requestTrashSelection() },
    );
    return items;
  };

  const onRowContextMenu = (e: MouseEvent, key: string) => {
    e.preventDefault();
    let cur = app.sel;
    if (!cur.selected.has(key)) {
      cur = { selected: new Set([key]), anchor: key, cursor: key };
      app.setSel(cur);
    }
    menu(e, buildMenu([...cur.selected]));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const cur = app.sel;
    const cursorRow = cur.cursor ? rows[rowIndex.get(cur.cursor) ?? -1] : undefined;
    if (e.key === "Enter") {
      e.preventDefault();
      app.activate([...cur.selected]);
    } else if (e.key === " ") {
      e.preventDefault();
      app.togglePreview();
    } else if (e.key === "Escape") {
      if (app.previewOpen) app.togglePreview(false);
      else app.setSel(emptySel());
    } else if (e.ctrlKey && (e.key === "a" || e.key === "A")) {
      e.preventDefault();
      app.setSel(selectAllSel(keys.filter((k) => !app.isRoot(k)), cur.cursor));
    } else if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && cursorRow && !searching) {
      e.preventDefault();
      const isDirRow = cursorRow.type === "root" || (cursorRow.type === "entry" && cursorRow.entry.kind === "dir");
      const expanded = (cursorRow.type === "root" || cursorRow.type === "entry") && cursorRow.expanded;
      const path = cursorRow.key;
      if (e.key === "ArrowRight") {
        if (isDirRow && !expanded) app.toggleExpanded(path, true);
        else if (isDirRow && expanded) {
          const next = rows[(rowIndex.get(path) ?? 0) + 1];
          if (next && next.type !== "placeholder" && next.type !== "root") app.setSel(moveSel(cur, keys, keyIndex, next.key, { shift: false, ctrl: false }));
        }
      } else if (isDirRow && expanded) app.toggleExpanded(path, false);
      else if (cursorRow.type === "entry") {
        const parent = cursorRow.entry.parent;
        if (rowIndex.has(parent)) app.setSel(moveSel(cur, keys, keyIndex, parent, { shift: false, ctrl: false }));
      }
    }
  };

  const renderRow = (i: number, style: CSSProperties): ReactNode => {
    const r = rows[i];
    if (r.type === "placeholder") {
      return (
        <div key={r.key} style={style} className="row tree-row placeholder">
          <span style={{ width: r.depth * 18 + 26 }} className="indent" />
          <span className="dim">{t(r.text)}</span>
        </div>
      );
    }
    const selected = sel.selected.has(r.key);
    const cls = `row ${selected ? "selected" : ""} ${sel.cursor === r.key ? "cursor" : ""}`;
    const common = {
      style,
      role: "option",
      "aria-selected": selected,
      onPointerDown: (e: PointerEvent) => onRowPointerDown(e, r.key),
      onClick: () => onRowClick(r.key),
      onDoubleClick: () => app.activate([r.key]),
      onContextMenu: (e: MouseEvent) => onRowContextMenu(e, r.key),
    };
    if (r.type === "search") {
      const e = r.entry;
      return (
        <div key={r.key} {...common} className={`${cls} search-row`} title={e.path} data-drop-kind={e.kind === "dir" ? "folder" : undefined} data-drop-value={e.kind === "dir" ? e.path : undefined}>
          <FileIcon kind={e.kind} ext={e.ext} />
          <div className="two-line">
            <div className="name ellipsis">
              {e.name}
              {e.cloud && (
                <span title={t("내용이 이 PC에 없음 (OneDrive 등)")} className="cloud">
                  <CloudRegular />
                </span>
              )}
            </div>
            <div className="sub ellipsis">{displayPath(e.parent, app.settings.folders)}</div>
          </div>
          <span className="col-date">{formatDateTime(e.modified)}</span>
          <span className="col-size">
            <SizeCell entry={e} path={e.path} />
          </span>
        </div>
      );
    }
    const chevron = (open: boolean, path: string) => (
      <button
        className="chev"
        tabIndex={-1}
        aria-label={open ? t("접기") : t("펼치기")}
        onPointerDown={keepListFocus}
        onClick={(ev) => {
          ev.stopPropagation();
          app.toggleExpanded(path);
        }}
      >
        {open ? <ChevronDownRegular fontSize={12} /> : <ChevronRightRegular fontSize={12} />}
      </button>
    );
    if (r.type === "root") {
      const indexed = r.state === "indexed";
      return (
        <div key={r.key} {...common} className={`${cls} tree-row root-row`} title={r.path} data-drop-kind={indexed ? "folder" : undefined} data-drop-value={indexed ? r.path : undefined}>
          <span className="indent" style={{ width: 2 }} />
          {indexed ? chevron(r.expanded, r.path) : <span className="chev" />}
          <FolderFilled className="ficon ic-folder" fontSize={16} />
          <span className="name ellipsis">
            <strong>{basename(r.path)}</strong>
            <span className="root-parent">{dirname(r.path)}</span>
          </span>
          {r.busy && indexed && <Spinner size="extra-tiny" />}
          {r.state === "notIndexed" && (
            <span className="root-state">
              <span className="dim">{t("인덱스 없음")}</span>
              <Button size="small" icon={<ArrowClockwiseRegular />} onPointerDown={keepListFocus} onClick={() => void app.rescan(r.path)}>
                {t("스캔")}
              </Button>
            </span>
          )}
          {r.state === "scanning" && (
            <span className="root-state">
              <Spinner size="extra-tiny" />
              <span className="dim">{t("스캔 중…")}</span>
            </span>
          )}
          {r.state === "missing" && (
            <span className="root-state">
              <span className="dim">{t("폴더가 없음")}</span>
              <Button size="small" onPointerDown={keepListFocus} onClick={() => void app.removeFolder(r.path)}>
                {t("제거")}
              </Button>
            </span>
          )}
          {indexed && (
            <>
              <span className="col-date" />
              <span className="col-size">
                <SizeCell path={r.path} />
              </span>
            </>
          )}
        </div>
      );
    }
    const e = r.entry;
    const isDir = e.kind === "dir";
    return (
      <div key={r.key} {...common} className={`${cls} tree-row`} title={e.path} data-drop-kind={isDir ? "folder" : undefined} data-drop-value={isDir ? e.path : undefined}>
        <span className="indent" style={{ width: r.depth * 18 + 2 }} />
        {isDir ? chevron(r.expanded, e.path) : <span className="chev" />}
        <FileIcon kind={e.kind} ext={e.ext} />
        <span className="name-cell">
          <span className="name ellipsis">{e.name}</span>
          {e.cloud && (
            <span title={t("내용이 이 PC에 없음 (OneDrive 등)")} className="cloud">
              <CloudRegular />
            </span>
          )}
        </span>
        <span className="col-date">{formatDateTime(e.modified)}</span>
        <span className="col-size">
          <SizeCell entry={e} path={e.path} />
        </span>
      </div>
    );
  };

  const count = app.searchResults.length;
  const previewPath = app.previewOpen ? app.previewPath() : null;

  return (
    <div className="collection">
      {searching ? (
        <Banner
          icon={<SearchRegular />}
          actions={
            <Button size="small" onClick={() => app.setSearchText("")}>
              {t("검색 지우기")}
            </Button>
          }
        >
          <span>{t("‘{0}’ 검색 결과 {1}개", app.filter.text.trim(), count)}</span>
          {app.filterActive && <span className="dim"> {t("· 필터 적용 중")}</span>}
          {app.searchTruncated && <span className="dim"> · {t("결과가 많아 처음 {0}개만 보여 줍니다", count)}</span>}
        </Banner>
      ) : (
        app.filterActive && (
          <Banner
            icon={<FilterFilled />}
            actions={
              <Button size="small" onClick={() => app.clearFilters()}>
                {t("지우기")}
              </Button>
            }
          >
            {t("필터 적용 중: {0}", filterText)}
          </Banner>
        )
      )}
      <div className="split">
        <div className="list-pane">
          <ColumnHeader />
          <VirtualList
            ref={listRef}
            count={rows.length}
            rowHeight={searching ? SEARCH_ROW_H : ROW_H}
            renderRow={renderRow}
            focusIndex={focusIndex}
            isSelectable={(i) => rows[i]?.type !== "placeholder"}
            onNavigate={(i, mods) => app.setSel(moveSel(app.sel, keys, keyIndex, rows[i].key, mods))}
            onKeyDown={onKeyDown}
            onRangeChange={(a, b) => setRange((old) => (old[0] === a && old[1] === b ? old : [a, b]))}
            onBackgroundMouseDown={() => app.setSel(emptySel())}
            onBackgroundContextMenu={(e) => menu(e, [{ label: t("다시 스캔"), icon: <ArrowClockwiseRegular />, onClick: () => void app.rescan() }])}
            scrollRequest={scrollReq}
            ariaLabel={t("모아 보기")}
            className="file-list"
          />
          {searching && !app.searchPending && count === 0 && (
            <div className="list-overlay">
              <EmptyState icon={<SearchRegular fontSize={40} />} title={t("‘{0}’에 대한 결과 없음", app.filter.text.trim())} />
            </div>
          )}
        </div>
        {app.previewOpen && <PreviewPane path={previewPath} />}
      </div>
      {drag.ghost}
    </div>
  );
}
