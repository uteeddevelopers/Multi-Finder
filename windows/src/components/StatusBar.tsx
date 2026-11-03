import { useEffect } from "react";
import { Link, Spinner, Tooltip } from "@fluentui/react-components";
import { LocationRegular } from "@fluentui/react-icons";
import { t } from "../i18n";
import { useApp } from "../model/context";
import { formatBytes } from "../util/format";

/** Bottom bar: scan progress · selected item's path · last operation · index totals, and Undo. */
export function StatusBar() {
  const app = useApp();
  const single = app.screen === "collection" && app.sel.selected.size === 1 ? [...app.sel.selected][0] : null;
  const showStats = !app.isScanning && !single && !app.lastSummary;
  useEffect(() => {
    if (showStats) app.loadStats();
  });

  let left = null;
  if (app.isScanning) {
    left = (
      <>
        <Spinner size="extra-tiny" />
        <span className="sb-text ellipsis">{app.progressText || t("스캔 중…")}</span>
      </>
    );
  } else if (single) {
    left = (
      <>
        <LocationRegular className="sb-icon" />
        <span className="sb-text ellipsis selectable">{single}</span>
      </>
    );
  } else if (app.lastSummary) {
    left = <span className="sb-text ellipsis">{app.lastSummary}</span>;
  } else if (app.stats) {
    left = <span className="sb-text ellipsis">{t("파일 {0}개 · 폴더 {1}개 · {2} · 스페이스: 미리보기 · 끌어 놓기: 이동", app.stats.files, app.stats.folders, formatBytes(app.stats.bytes))}</span>;
  }

  const many = app.screen === "collection" && app.sel.selected.size > 1 ? app.sel.selected.size : 0;
  return (
    <footer className="statusbar">
      <div className="sb-left">{left}</div>
      <div className="sb-right">
        {many > 0 && <span>{t("{0}개 선택", many)}</span>}
        {app.undoState.canUndo && (
          <Tooltip content={t("마지막 이동·휴지통 작업 되돌리기 (Ctrl+Z)")} relationship="description">
            <Link as="button" className="sb-undo" onClick={() => void app.undo()}>
              {t("되돌리기")}
            </Link>
          </Tooltip>
        )}
      </div>
    </footer>
  );
}
