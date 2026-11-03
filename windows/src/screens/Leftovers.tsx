// App Leftovers — settings and caches that uninstalled apps left in %APPDATA% / %LOCALAPPDATA%.
// Only clear-cut items are reported by the backend; anything uncertain is not listed at all.
import { useMemo, useRef, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { Button, Tooltip } from "@fluentui/react-components";
import {
  AppGenericRegular,
  ArrowClockwiseRegular,
  BroomRegular,
  CheckmarkCircleRegular,
  ChevronDownRegular,
  ChevronRightRegular,
  DeleteRegular,
  FolderOpenRegular,
  FolderRegular,
  SearchRegular,
} from "@fluentui/react-icons";
import { t, tDynamic } from "../i18n";
import { useApp } from "../model/context";
import type { LeftoverRow } from "../model/leftovers";
import { VirtualList, type VirtualListHandle } from "../components/VirtualList";
import { useContextMenu } from "../components/ContextMenu";
import { EmptyState, Progress, keepListFocus } from "../components/common";
import { clickSel, indexOfKeys, moveSel, selectAllSel } from "../util/selection";
import { displayAppDataPath } from "../util/paths";
import { formatBytes, formatDate } from "../util/format";

export function LeftoversActions() {
  const app = useApp();
  const lo = app.leftovers;
  return (
    <Button appearance="primary" icon={lo.report ? <ArrowClockwiseRegular /> : <SearchRegular />} disabled={lo.running} onClick={() => void lo.run()}>
      {lo.report ? t("다시 검사") : t("검사")}
    </Button>
  );
}

export function LeftoversView() {
  const app = useApp();
  const lo = app.leftovers;
  if (lo.running) return <Progress text={lo.progress || t("검사 중…")} />;
  if (!lo.report) {
    return (
      <EmptyState
        icon={<BroomRegular fontSize={48} />}
        title={t("지운 앱의 흔적 찾기")}
        actions={
          <Button appearance="primary" onClick={() => void lo.run()}>
            {t("검사")}
          </Button>
        }
      >
        {t("앱을 지워도 설정·캐시는 AppData 폴더에 남습니다.\n설치된 앱과 대조해 주인 없는 것만 찾고, 애매하면 남깁니다. 지우는 건 고른 뒤 확인을 거칩니다.")}
      </EmptyState>
    );
  }
  if (lo.report.items.length === 0) {
    return (
      <EmptyState icon={<CheckmarkCircleRegular fontSize={48} />} title={t("남은 흔적이 없습니다")}>
        {t("지운 앱의 잔여물을 찾지 못했습니다.")}
      </EmptyState>
    );
  }
  return <LeftoverList />;
}

function LeftoverList() {
  const app = useApp();
  const lo = app.leftovers;
  const report = lo.report!;
  const menu = useContextMenu();
  const listRef = useRef<VirtualListHandle>(null);
  const rows = lo.rows();
  const heights = useMemo(() => rows.map((r) => (r.type === "app" ? 54 : 36)), [rows]);
  const keys = useMemo(() => rows.map((r) => r.key), [rows]);
  const keyIndex = useMemo(() => indexOfKeys(keys), [keys]);
  const sel = lo.sel;
  const focusIndex = sel.cursor ? (keyIndex.get(sel.cursor) ?? -1) : -1;
  const chosen = lo.selectedLeftoverPaths();
  const bytes = chosen.reduce((s, p) => s + p.bytes, 0);

  const onPointerDown = (e: PointerEvent, key: string) => {
    if (e.button !== 0) return;
    listRef.current?.focus();
    lo.setSel(clickSel(lo.sel, keys, keyIndex, key, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey }));
  };
  const onContextMenu = (e: MouseEvent, key: string) => {
    e.preventDefault();
    if (!lo.sel.selected.has(key)) lo.setSel({ selected: new Set([key]), anchor: key, cursor: key });
    const paths = lo.pathsFor(lo.sel.selected);
    menu(e, [
      { label: t("탐색기에서 보기"), icon: <FolderOpenRegular />, onClick: () => void app.reveal(paths) },
      { divider: true },
      { label: t("휴지통으로 이동…"), icon: <DeleteRegular />, onClick: () => lo.trashSelection() },
    ]);
  };
  const onDoubleClick = (r: LeftoverRow) => {
    if (r.type === "app") lo.toggleExpanded(r.item.id);
    else void app.reveal([r.path.path]);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const cur = lo.sel.cursor ? rows[keyIndex.get(lo.sel.cursor) ?? -1] : undefined;
    if (e.ctrlKey && (e.key === "a" || e.key === "A")) {
      e.preventDefault();
      lo.setSel(selectAllSel(rows.filter((r) => r.type === "app").map((r) => r.key), lo.sel.cursor));
    } else if (cur?.type === "app" && ((e.key === "ArrowRight" && !cur.expanded) || (e.key === "ArrowLeft" && cur.expanded))) {
      e.preventDefault();
      lo.toggleExpanded(cur.item.id);
    } else if (e.key === "Enter" && cur) {
      e.preventDefault();
      onDoubleClick(cur);
    }
  };

  const renderRow = (i: number, style: CSSProperties): ReactNode => {
    const r = rows[i];
    const selected = sel.selected.has(r.key);
    const base = {
      style,
      role: "option",
      "aria-selected": selected,
      onPointerDown: (e: PointerEvent) => onPointerDown(e, r.key),
      onDoubleClick: () => onDoubleClick(r),
      onContextMenu: (e: MouseEvent) => onContextMenu(e, r.key),
    };
    const cls = `row ${selected ? "selected" : ""} ${sel.cursor === r.key ? "cursor" : ""}`;
    if (r.type === "app") {
      const newest = Math.max(...r.item.paths.map((p) => p.lastModified));
      return (
        <div key={r.key} {...base} className={`${cls} lo-app`} title={tDynamic(r.item.reason)}>
          <button
            className="chev"
            tabIndex={-1}
            onPointerDown={keepListFocus}
            onClick={(e) => {
              e.stopPropagation();
              lo.toggleExpanded(r.item.id);
            }}
          >
            {r.expanded ? <ChevronDownRegular fontSize={12} /> : <ChevronRightRegular fontSize={12} />}
          </button>
          <AppGenericRegular className="ficon lo-icon" fontSize={22} />
          <div className="two-line">
            <div className="name ellipsis">
              <strong>{r.item.app}</strong> <span className="dim">{t("{0}개", r.item.paths.length)}</span>
            </div>
            <div className="sub ellipsis">{tDynamic(r.item.reason)}</div>
          </div>
          <span className="lo-date dim">{t("마지막 변경 {0}", formatDate(newest))}</span>
          <span className="lo-size num">{formatBytes(r.item.bytes)}</span>
        </div>
      );
    }
    return (
      <div key={r.key} {...base} className={`${cls} lo-path`} title={r.path.path}>
        <FolderRegular className="ficon dim" />
        <span className="name ellipsis">{displayAppDataPath(r.path.path)}</span>
        <span className="lo-date dim">{formatDate(r.path.lastModified)}</span>
        <span className="lo-size num dim">{formatBytes(r.path.bytes)}</span>
      </div>
    );
  };

  return (
    <div className="screen">
      <div className="summary-bar">
        <BroomRegular />
        <span>{t("지운 앱의 흔적 — 설치된 앱과 맞는 것이 없고 오래 쓰이지 않은 폴더 ({0}개 앱)", report.items.length)}</span>
        <span className="spacer" />
        {report.scannedAreas.length > 0 && <span className="dim">{t("검사한 위치: {0}", report.scannedAreas.join(", "))}</span>}
      </div>
      <VirtualList
        ref={listRef}
        count={rows.length}
        rowHeight={heights}
        renderRow={renderRow}
        focusIndex={focusIndex}
        onNavigate={(i, mods) => lo.setSel(moveSel(lo.sel, keys, keyIndex, rows[i].key, mods))}
        onKeyDown={onKeyDown}
        onBackgroundMouseDown={() => lo.clearSelection()}
        ariaLabel={t("앱 잔여물")}
        className="lo-list"
      />
      <div className="actionbar">
        <Button onClick={() => lo.selectAll()}>{t("지운 앱 흔적 모두 선택")}</Button>
        {sel.selected.size > 0 && <Button onClick={() => lo.clearSelection()}>{t("선택 해제")}</Button>}
        <span className="spacer" />
        {chosen.length > 0 && <span className="dim">{t("{0}개 앱 · {1}개 항목 · {2}", lo.selectedAppCount(), chosen.length, formatBytes(bytes))}</span>}
        <Tooltip content={t("휴지통으로 이동…")} relationship="description">
          <Button appearance="primary" icon={<DeleteRegular />} disabled={chosen.length === 0} onClick={() => lo.trashSelection()}>
            {t("휴지통으로…")}
          </Button>
        </Tooltip>
      </div>
    </div>
  );
}
