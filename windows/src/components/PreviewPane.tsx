// Space: preview pane beside the list (images / video / audio / PDF through the asset protocol, text via
// read_text_preview). Follows the selection like Quick Look.
import { useEffect, useState } from "react";
import { Button, Spinner, Tooltip } from "@fluentui/react-components";
import { DismissRegular, FolderOpenRegular, OpenRegular } from "@fluentui/react-icons";
import { t } from "../i18n";
import { useApp } from "../model/context";
import { basename, dirname, displayPath, extOf } from "../util/paths";
import { formatBytes, formatDateTime } from "../util/format";
import { previewKindOf } from "../util/fileKinds";
import { FileIcon } from "./common";

const TEXT_LIMIT = 64 * 1024;

export function PreviewPane({ path }: { path: string | null }) {
  const app = useApp();
  const entry = path ? app.entryOf(path) : undefined;
  const isDir = path ? app.isDir(path) : false;
  const ext = entry?.ext ?? (path ? extOf(path) : "");
  const kind = !path || isDir || entry?.cloud ? "none" : previewKindOf(ext);
  const url = path && (kind === "image" || kind === "video" || kind === "audio" || kind === "pdf") ? app.backend.previewUrl(path) : "";
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
    setText(null);
    if (!path || kind !== "text") return;
    let alive = true;
    app.backend
      .readTextPreview(path, TEXT_LIMIT)
      .then((s) => alive && setText(s))
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
    };
  }, [path, kind, app.backend]);

  const info = path && isDir ? app.dirInfoOf(path) : undefined;
  useEffect(() => {
    if (path && isDir) app.requestDirInfo([path]);
  }, [path, isDir, app]);

  let body;
  if (!path) body = <div className="pv-empty">{t("미리보기를 표시할 수 없습니다")}</div>;
  else if (failed || ((kind === "image" || kind === "video" || kind === "audio" || kind === "pdf") && !url) || kind === "none") {
    body = (
      <div className="pv-big">
        <FileIcon kind={isDir ? "dir" : (entry?.kind ?? "file")} ext={ext} size={72} />
        {isDir ? (
          <div className="pv-caption">{info ? `${t("{0}개", info.items)} · ${formatBytes(info.bytes)}` : "…"}</div>
        ) : (
          <div className="pv-caption">{entry?.cloud ? t("내용이 이 PC에 없음 (OneDrive 등)") : t("미리보기를 표시할 수 없습니다")}</div>
        )}
      </div>
    );
  } else if (kind === "image") body = <img className="pv-media" src={url} alt="" onError={() => setFailed(true)} />;
  else if (kind === "video") body = <video className="pv-media" src={url} controls onError={() => setFailed(true)} />;
  else if (kind === "audio")
    body = (
      <div className="pv-big">
        <FileIcon kind="file" ext={ext} size={72} />
        <audio src={url} controls onError={() => setFailed(true)} />
      </div>
    );
  else if (kind === "pdf") body = <iframe className="pv-frame" src={url} title={basename(path)} />;
  else body = text === null ? <Spinner size="small" /> : <pre className="pv-text">{text}</pre>;

  return (
    <aside className="preview" aria-label={t("미리보기 (스페이스)")}>
      <div className="pv-head">
        <span className="pv-title ellipsis" title={path ?? ""}>
          {path ? basename(path) : ""}
        </span>
        <Tooltip content={t("미리보기 닫기")} relationship="label">
          <Button appearance="subtle" size="small" icon={<DismissRegular />} onClick={() => app.togglePreview(false)} />
        </Tooltip>
      </div>
      <div className="pv-body">{body}</div>
      {path && (
        <div className="pv-foot">
          <div className="pv-meta">
            {entry && entry.kind === "file" && <span>{formatBytes(entry.size)}</span>}
            {entry && <span>{formatDateTime(entry.modified)}</span>}
            <span className="ellipsis" title={path}>
              {displayPath(dirname(path), app.settings.folders)}
            </span>
          </div>
          <div className="pv-actions">
            {!isDir && (
              <Button size="small" icon={<OpenRegular />} onClick={() => void app.open(path)}>
                {t("열기")}
              </Button>
            )}
            <Button size="small" icon={<FolderOpenRegular />} onClick={() => void app.reveal([path])}>
              {t("탐색기에서 보기")}
            </Button>
          </div>
        </div>
      )}
    </aside>
  );
}
