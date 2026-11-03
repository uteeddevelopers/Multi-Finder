import { Spinner } from "@fluentui/react-components";
import {
  AddRegular,
  ArrowClockwiseRegular,
  ArrowDownRegular,
  ArrowUpRegular,
  BroomRegular,
  ColumnDoubleCompareRegular,
  DismissRegular,
  DocumentCopyRegular,
  FolderFilled,
  FolderOpenRegular,
  LayerRegular,
  SparkleRegular,
} from "@fluentui/react-icons";
import type { ReactElement } from "react";
import type { Screen } from "../api/types";
import { t } from "../i18n";
import { useApp } from "../model/context";
import { basename } from "../util/paths";
import { formatBytes } from "../util/format";
import { useContextMenu } from "./ContextMenu";

function NavItem({ screen, label, icon }: { screen: Screen; label: string; icon: ReactElement }) {
  const app = useApp();
  const selected = app.screen === screen;
  return (
    <div
      className={`nav-item ${selected ? "selected" : ""}`}
      role="tab"
      aria-selected={selected}
      tabIndex={0}
      onClick={() => app.setScreen(screen)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") app.setScreen(screen);
      }}
    >
      <span className="nav-icon">{icon}</span>
      <span className="nav-label">{label}</span>
    </div>
  );
}

function FolderItem({ path, index, count }: { path: string; index: number; count: number }) {
  const app = useApp();
  const menu = useContextMenu();
  const state = app.stateOf(path);
  const busy = app.isRootScanning(path) || state === "scanning";
  const summary = app.summaryOf(path);
  let right: string | null = null;
  if (state === "missing") right = t("폴더가 없음");
  else if (state === "notIndexed" && !busy) right = t("인덱스 없음");
  else if (summary && state === "indexed") right = formatBytes(summary.bytes);
  return (
    <div
      className="nav-item folder-item"
      title={path}
      tabIndex={0}
      data-drop-kind="folder"
      data-drop-value={path}
      onClick={() => app.showFolder(path)}
      onKeyDown={(e) => {
        if (e.key === "Enter") app.showFolder(path);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        menu(e, [
          { label: t("탐색기에서 보기"), icon: <FolderOpenRegular />, onClick: () => void app.reveal([path]) },
          { label: t("다시 스캔"), icon: <ArrowClockwiseRegular />, onClick: () => void app.rescan(path) },
          {
            label: t("이 폴더 안 중복 찾기"),
            icon: <DocumentCopyRegular />,
            onClick: () => {
              app.duplicates.setOption({ scope: path });
              app.setScreen("duplicates");
              void app.duplicates.run();
            },
          },
          { divider: true },
          { label: t("위로 이동"), icon: <ArrowUpRegular />, disabled: index === 0, onClick: () => void app.moveFolder(path, -1) },
          { label: t("아래로 이동"), icon: <ArrowDownRegular />, disabled: index === count - 1, onClick: () => void app.moveFolder(path, 1) },
          { divider: true },
          { label: t("목록에서 제거"), icon: <DismissRegular />, onClick: () => void app.removeFolder(path) },
        ]);
      }}
    >
      <span className="nav-icon">
        <FolderFilled className="ic-folder" />
      </span>
      <span className="nav-label">{basename(path)}</span>
      {busy ? <Spinner size="extra-tiny" /> : right && <span className="nav-meta">{right}</span>}
    </div>
  );
}

export function Sidebar() {
  const app = useApp();
  const folders = app.settings.folders;
  return (
    <nav className="sidebar" aria-label={t("Multi-Finder")}>
      <div className="sb-section">
        <div className="sb-header">{t("보기")}</div>
        <NavItem screen="collection" label={t("모아 보기")} icon={<LayerRegular />} />
        <NavItem screen="duplicates" label={t("중복 파일")} icon={<DocumentCopyRegular />} />
        <NavItem screen="compare" label={t("폴더 비교")} icon={<ColumnDoubleCompareRegular />} />
        <NavItem screen="categories" label={t("자동 분류")} icon={<SparkleRegular />} />
      </div>
      <div className="sb-section">
        <div className="sb-header">{t("정리")}</div>
        <NavItem screen="leftovers" label={t("앱 잔여물")} icon={<BroomRegular />} />
      </div>
      <div className="sb-section">
        <div className="sb-header">{t("모은 폴더")}</div>
        {folders.map((f, i) => (
          <FolderItem key={f} path={f} index={i} count={folders.length} />
        ))}
        <div
          className="nav-item add-folder"
          tabIndex={0}
          onClick={() => void app.addFolderViaPicker()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void app.addFolderViaPicker();
          }}
        >
          <span className="nav-icon">
            <AddRegular />
          </span>
          <span className="nav-label">{t("폴더 추가…")}</span>
        </div>
      </div>
    </nav>
  );
}
