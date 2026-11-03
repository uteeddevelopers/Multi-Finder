// Compare Folders — two folders side by side (port of CompareView.swift).
import { useMemo, useRef, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactElement, type ReactNode } from "react";
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
  Tooltip,
  type MenuProps,
} from "@fluentui/react-components";
import {
  ArrowCircleLeftRegular,
  ArrowCircleRightRegular,
  ArrowSwapRegular,
  ArrowSyncRegular,
  ArrowTurnDownRightRegular,
  CheckmarkCircleFilled,
  ColumnDoubleCompareRegular,
  DeleteRegular,
  FolderFilled,
  FolderOpenRegular,
  WandRegular,
  WarningFilled,
} from "@fluentui/react-icons";
import type { DiffEntry, DiffSide, DiffStatus } from "../api/types";
import { t, type MsgKey } from "../i18n";
import { useApp } from "../model/context";
import type { Side } from "../model/compare";
import { VirtualList, type VirtualListHandle } from "../components/VirtualList";
import { useContextMenu } from "../components/ContextMenu";
import { EmptyState, FileIcon, Progress } from "../components/common";
import { clickSel, indexOfKeys, moveSel, selectAllSel } from "../util/selection";
import { basename, dirname, extOf } from "../util/paths";
import { formatBytes, formatDate } from "../util/format";

const STATUS_ORDER: DiffStatus[] = ["identical", "different", "leftOnly", "rightOnly", "moved"];

export const STATUS: Record<DiffStatus, { label: MsgKey; explain: MsgKey; cls: string; icon: ReactElement }> = {
  identical: { label: "동일", explain: "양쪽 같은 자리에 똑같은 내용이 있음 — 한쪽은 치워도 됨", cls: "st-identical", icon: <CheckmarkCircleFilled /> },
  different: { label: "다름", explain: "같은 자리에 있지만 내용이 다름 — 어느 쪽이 최신인지 확인 필요", cls: "st-different", icon: <WarningFilled /> },
  leftOnly: { label: "왼쪽에만", explain: "왼쪽 폴더에만 있음", cls: "st-one", icon: <ArrowCircleLeftRegular /> },
  rightOnly: { label: "오른쪽에만", explain: "오른쪽 폴더에만 있음", cls: "st-one", icon: <ArrowCircleRightRegular /> },
  moved: { label: "자리만 다름", explain: "반대쪽의 다른 자리에 같은 내용이 있음 (옮겨졌거나 이름이 다름)", cls: "st-moved", icon: <ArrowSwapRegular /> },
};

export function CompareActions() {
  const app = useApp();
  const cmp = app.compare;
  const onFilter: MenuProps["onCheckedValueChange"] = (_, d) => {
    const v = d.checkedItems[0];
    cmp.setStatusFilter(v === "all" ? null : (v as DiffStatus));
  };
  return (
    <>
      {cmp.result && (
        <Menu checkedValues={{ status: [cmp.statusFilter ?? "all"] }} onCheckedValueChange={onFilter}>
          <MenuTrigger disableButtonEnhancement>
            <Tooltip content={t("상태별로 보기")} relationship="description">
              <Button appearance="subtle" icon={cmp.statusFilter ? STATUS[cmp.statusFilter].icon : undefined}>
                {cmp.statusFilter ? t(STATUS[cmp.statusFilter].label) : t("전체")}
              </Button>
            </Tooltip>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItemRadio name="status" value="all">
                {t("전체")}
              </MenuItemRadio>
              <MenuDivider />
              {STATUS_ORDER.map((s) => (
                <MenuItemRadio key={s} name="status" value={s} icon={<span className={STATUS[s].cls}>{STATUS[s].icon}</span>}>
                  {t(STATUS[s].label)}
                </MenuItemRadio>
              ))}
            </MenuList>
          </MenuPopover>
        </Menu>
      )}
      <Tooltip content={t("다시 비교")} relationship="description">
        <Button appearance="primary" icon={<ArrowSyncRegular />} disabled={!cmp.canRun} onClick={() => void cmp.run()}>
          {t("비교")}
        </Button>
      </Tooltip>
    </>
  );
}

function FolderPicker({ side }: { side: Side }) {
  const app = useApp();
  const cmp = app.compare;
  const path = side === "left" ? cmp.left : cmp.right;
  return (
    <button className="cmp-pick" onClick={() => void cmp.pick(side)} title={path ?? t("폴더를 고르세요")}>
      <FolderFilled className="ic-folder" fontSize={30} />
      <div className="two-line">
        {path ? (
          <>
            <div className="name ellipsis strong">{basename(path)}</div>
            <div className="sub ellipsis">{dirname(path)}</div>
          </>
        ) : (
          <>
            <div className="name ellipsis dim">{side === "left" ? t("왼쪽 폴더 선택…") : t("오른쪽 폴더 선택…")}</div>
            <div className="sub ellipsis">{t("클릭해서 고르기")}</div>
          </>
        )}
      </div>
    </button>
  );
}

function SideCell({ side, entry }: { side: "left" | "right"; entry: DiffEntry }) {
  const s: DiffSide | undefined = side === "left" ? entry.left : entry.right;
  // For a moved pair the right column shows where the same content sits on the right.
  const relocated = entry.status === "moved" && entry.otherRelPath && (side === "right" || !s) ? entry.otherRelPath : null;
  if (relocated) {
    return (
      <span className="side-cell relocated" title={t("같은 내용이 이 자리에 있음")}>
        <ArrowTurnDownRightRegular />
        <span className="ellipsis">{relocated}</span>
      </span>
    );
  }
  if (!s) return <span className="side-cell none">{t("없음")}</span>;
  return (
    <span className="side-cell">
      <span className="num">{formatBytes(s.size)}</span>
      <span className="dim num">{formatDate(s.modified)}</span>
    </span>
  );
}

export function CompareView() {
  const app = useApp();
  const cmp = app.compare;
  let body: ReactNode;
  if (cmp.running) body = <Progress text={cmp.progress} />;
  else if (cmp.result) body = <CompareResults />;
  else
    body = (
      <EmptyState
        icon={<ColumnDoubleCompareRegular fontSize={48} />}
        title={t("폴더 두 개를 고르세요")}
        actions={
          <Button appearance="primary" disabled={!cmp.canRun} onClick={() => void cmp.run()}>
            {t("비교")}
          </Button>
        }
      >
        {t("같은 자리의 파일끼리 맞춰 보고, 한쪽에만 있는 파일은 반대쪽 다른 자리에 같은 내용이 있는지도 찾습니다.")}
      </EmptyState>
    );
  return (
    <div className="screen">
      <div className="cmp-header">
        <FolderPicker side="left" />
        <Tooltip content={t("좌우 바꾸기")} relationship="label">
          <Button appearance="subtle" icon={<ArrowSwapRegular />} disabled={!cmp.left && !cmp.right} onClick={() => cmp.swap()} />
        </Tooltip>
        <FolderPicker side="right" />
      </div>
      {body}
    </div>
  );
}

function CompareResults() {
  const app = useApp();
  const cmp = app.compare;
  const r = cmp.result!;
  const menu = useContextMenu();
  const listRef = useRef<VirtualListHandle>(null);
  const entries = cmp.visibleEntries();
  const heights = useMemo(() => entries.map((e) => (e.relPath.includes("\\") || e.relPath.includes("/") ? 46 : 36)), [entries]);
  const keys = useMemo(() => entries.map((e) => e.id), [entries]);
  const keyIndex = useMemo(() => indexOfKeys(keys), [keys]);
  const sel = cmp.sel;
  const focusIndex = sel.cursor ? (keyIndex.get(sel.cursor) ?? -1) : -1;
  const leftName = basename(r.left);
  const rightName = basename(r.right);

  const onPointerDown = (e: PointerEvent, id: string) => {
    if (e.button !== 0) return;
    listRef.current?.focus();
    cmp.setSel(clickSel(cmp.sel, keys, keyIndex, id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey }));
  };
  const onContextMenu = (e: MouseEvent, id: string) => {
    e.preventDefault();
    if (!cmp.sel.selected.has(id)) cmp.setSel({ selected: new Set([id]), anchor: id, cursor: id });
    const rows = cmp.selectedEntries();
    const paths = rows.flatMap((x) => [x.left?.path, x.right?.path].filter((p): p is string => !!p));
    menu(e, [
      { label: t("탐색기에서 보기"), icon: <FolderOpenRegular />, onClick: () => void app.reveal(paths) },
      { divider: true },
      { label: t("왼쪽 파일 휴지통으로…"), icon: <DeleteRegular />, disabled: !rows.some((x) => x.left), onClick: () => cmp.trashSelected("left") },
      { label: t("오른쪽 파일 휴지통으로…"), icon: <DeleteRegular />, disabled: !rows.some((x) => x.right), onClick: () => cmp.trashSelected("right") },
    ]);
  };
  const open = (e: DiffEntry) => {
    const p = e.left?.path ?? e.right?.path;
    if (p) void app.open(p);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      cmp.selectedEntries().forEach(open);
    } else if (e.ctrlKey && (e.key === "a" || e.key === "A")) {
      e.preventDefault();
      cmp.setSel(selectAllSel(keys, cmp.sel.cursor));
    }
  };

  const renderRow = (i: number, style: CSSProperties): ReactNode => {
    const e = entries[i];
    const selected = sel.selected.has(e.id);
    const dir = e.relPath.includes("\\") ? e.relPath.slice(0, e.relPath.lastIndexOf("\\")) : e.relPath.includes("/") ? e.relPath.slice(0, e.relPath.lastIndexOf("/")) : "";
    const st = STATUS[e.status];
    return (
      <div
        key={e.id}
        style={style}
        role="option"
        aria-selected={selected}
        className={`row cmp-row ${selected ? "selected" : ""} ${sel.cursor === e.id ? "cursor" : ""}`}
        title={e.left?.path ?? e.right?.path}
        onPointerDown={(ev) => onPointerDown(ev, e.id)}
        onDoubleClick={() => open(e)}
        onContextMenu={(ev) => onContextMenu(ev, e.id)}
      >
        <div className="cmp-name">
          <FileIcon kind="file" ext={extOf(e.name)} />
          <div className="two-line">
            <div className="name ellipsis">{e.name}</div>
            {dir && <div className="sub ellipsis">{dir}</div>}
          </div>
        </div>
        <div className={`cmp-status ${st.cls}`} title={t(st.explain)}>
          {st.icon}
          <span className="ellipsis">{t(st.label)}</span>
        </div>
        <div className="cmp-side">
          <SideCell side="left" entry={e} />
        </div>
        <div className="cmp-side">
          <SideCell side="right" entry={e} />
        </div>
      </div>
    );
  };

  const c = r.counts;
  const count = (s: DiffStatus) => c[s] ?? 0;
  return (
    <>
      <div className="cmp-colheader">
        <span className="cmp-name">{t("이름")}</span>
        <span className="cmp-status">{t("상태")}</span>
        <span className="cmp-side ellipsis" title={r.left}>
          {leftName}
        </span>
        <span className="cmp-side ellipsis" title={r.right}>
          {rightName}
        </span>
      </div>
      <VirtualList
        ref={listRef}
        count={entries.length}
        rowHeight={heights}
        renderRow={renderRow}
        focusIndex={focusIndex}
        onNavigate={(i, mods) => cmp.setSel(moveSel(cmp.sel, keys, keyIndex, entries[i].id, mods))}
        onKeyDown={onKeyDown}
        onBackgroundMouseDown={() => cmp.setSel({ selected: new Set(), anchor: null, cursor: null })}
        ariaLabel={t("폴더 비교")}
        className="cmp-list"
      />
      <div className="actionbar">
        <div className="chips">
          {STATUS_ORDER.filter((s) => count(s) > 0).map((s) => (
            <Tooltip key={s} content={t(STATUS[s].explain)} relationship="description">
              <button className={`chip ${STATUS[s].cls} ${cmp.statusFilter === s ? "on" : ""}`} onClick={() => cmp.setStatusFilter(cmp.statusFilter === s ? null : s)}>
                {STATUS[s].icon}
                <span>
                  {t(STATUS[s].label)} {count(s)}
                </span>
              </button>
            </Tooltip>
          ))}
        </div>
        <span className="spacer" />
        {sel.selected.size > 0 && (
          <>
            <span className="dim">{t("{0}개 선택", sel.selected.size)}</span>
            <Menu>
              <MenuTrigger disableButtonEnhancement>
                <Button icon={<DeleteRegular />}>{t("휴지통으로…")}</Button>
              </MenuTrigger>
              <MenuPopover>
                <MenuList>
                  <MenuItem disabled={!cmp.selectionHas("left")} onClick={() => cmp.trashSelected("left")}>
                    {t("왼쪽 파일 휴지통으로…")}
                  </MenuItem>
                  <MenuItem disabled={!cmp.selectionHas("right")} onClick={() => cmp.trashSelected("right")}>
                    {t("오른쪽 파일 휴지통으로…")}
                  </MenuItem>
                </MenuList>
              </MenuPopover>
            </Menu>
          </>
        )}
        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <Tooltip content={t("옮기기·휴지통 모두 확인을 거치고, Ctrl+Z로 한 번에 되돌릴 수 있습니다")} relationship="description">
              <Button appearance="primary" icon={<WandRegular />}>
                {t("정리")}
              </Button>
            </Tooltip>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuGroup>
                <MenuGroupHeader>{t("한쪽에만 있는 파일 옮기기 — 자리(상대 경로) 유지")}</MenuGroupHeader>
                <MenuItem icon={<ArrowCircleRightRegular />} disabled={count("leftOnly") === 0} onClick={() => cmp.moveOnly("left")}>
                  {t("{0}에만 있는 {1}개 → {2}", leftName, count("leftOnly"), rightName)}
                </MenuItem>
                <MenuItem icon={<ArrowCircleLeftRegular />} disabled={count("rightOnly") === 0} onClick={() => cmp.moveOnly("right")}>
                  {t("{0}에만 있는 {1}개 → {2}", rightName, count("rightOnly"), leftName)}
                </MenuItem>
              </MenuGroup>
              <MenuDivider />
              <MenuGroup>
                <MenuGroupHeader>{t("양쪽에 똑같이 있는 {0}개 중 한쪽 치우기", count("identical"))}</MenuGroupHeader>
                <MenuItem icon={<DeleteRegular />} disabled={count("identical") === 0} onClick={() => cmp.trashIdentical("left")}>
                  {t("{0} 쪽 사본 휴지통으로…", leftName)}
                </MenuItem>
                <MenuItem icon={<DeleteRegular />} disabled={count("identical") === 0} onClick={() => cmp.trashIdentical("right")}>
                  {t("{0} 쪽 사본 휴지통으로…", rightName)}
                </MenuItem>
              </MenuGroup>
            </MenuList>
          </MenuPopover>
        </Menu>
      </div>
    </>
  );
}
