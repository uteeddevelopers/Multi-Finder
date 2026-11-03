import { useEffect, useRef } from "react";
import { Button, SearchBox, Tooltip } from "@fluentui/react-components";
import { ArrowClockwiseRegular, FolderAddRegular } from "@fluentui/react-icons";
import type { Screen } from "../api/types";
import { t, type MsgKey } from "../i18n";
import { useApp } from "../model/context";
import { CollectionActions } from "../screens/Collection";
import { DuplicatesActions } from "../screens/Duplicates";
import { CompareActions } from "../screens/Compare";
import { CategoriesActions } from "../screens/Categories";
import { LeftoversActions } from "../screens/Leftovers";

export const SCREEN_TITLE: Record<Screen, MsgKey> = {
  collection: "모아 보기",
  duplicates: "중복 파일",
  compare: "폴더 비교",
  categories: "자동 분류",
  leftovers: "앱 잔여물",
};

/** Title · Add Folder / Rescan · the screen's own actions · search box. */
export function Toolbar() {
  const app = useApp();
  const search = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (app.focusSearchNonce > 0) {
      search.current?.focus();
      search.current?.select();
    }
  }, [app.focusSearchNonce]);
  return (
    <header className="toolbar">
      <h1 className="tb-title">{t(SCREEN_TITLE[app.screen])}</h1>
      <div className="tb-group">
        <Tooltip content={t("모아 보기에 폴더 추가 (Ctrl+O)")} relationship="label">
          <Button appearance="subtle" icon={<FolderAddRegular />} onClick={() => void app.addFolderViaPicker()} />
        </Tooltip>
        <Tooltip content={t("다시 스캔 (F5)")} relationship="label">
          <Button appearance="subtle" icon={<ArrowClockwiseRegular />} disabled={app.isScanning} onClick={() => void app.rescan()} />
        </Tooltip>
      </div>
      <div className="tb-sep" />
      <div className="tb-actions">
        {app.screen === "collection" && <CollectionActions />}
        {app.screen === "duplicates" && <DuplicatesActions />}
        {app.screen === "compare" && <CompareActions />}
        {app.screen === "categories" && <CategoriesActions />}
        {app.screen === "leftovers" && <LeftoversActions />}
      </div>
      <SearchBox
        ref={search}
        className="tb-search"
        placeholder={t("이름으로 찾기")}
        value={app.filter.text}
        onChange={(_, d) => app.setSearchText(d.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") app.setSearchText("");
          if (e.key === "ArrowDown" || e.key === "Enter") {
            const list = document.querySelector<HTMLElement>(".file-list");
            if (list && e.key === "ArrowDown") {
              e.preventDefault();
              list.focus();
            }
          }
        }}
        aria-label={t("이름으로 찾기")}
      />
    </header>
  );
}
