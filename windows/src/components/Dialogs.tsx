// App-level dialogs: Recycle Bin confirmation, batch-move confirmation, alerts.
import { Button, Dialog, DialogActions, DialogBody, DialogContent, DialogSurface, DialogTitle } from "@fluentui/react-components";
import { t } from "../i18n";
import { useApp } from "../model/context";
import { basename, dirname, displayPath, extOf } from "../util/paths";
import { formatBytes } from "../util/format";
import { FileIcon } from "./common";

function PathLines({ paths }: { paths: string[] }) {
  const app = useApp();
  const shown = paths.slice(0, 5);
  const more = paths.length - shown.length;
  return (
    <div className="dlg-lines">
      {shown.map((p) => {
        const e = app.entryOf(p);
        return (
          <div className="dlg-line" key={p} title={p}>
            <FileIcon kind={e?.kind ?? (extOf(p) ? "file" : "dir")} ext={e?.ext ?? extOf(p)} />
            <span className="dlg-name">{basename(p)}</span>
            <span className="dlg-dim">— {displayPath(dirname(p), app.settings.folders)}</span>
          </div>
        );
      })}
      {more > 0 && <div className="dlg-more">… {t("외 {0}개", more)}</div>}
    </div>
  );
}

export function TrashDialog() {
  const app = useApp();
  const req = app.trashRequest;
  if (!req) return null;
  return (
    <Dialog open modalType="alert" onOpenChange={(_, d) => !d.open && app.cancelTrash()}>
      <DialogSurface className="dlg">
        <DialogBody>
          <DialogTitle>{t("{0}개 항목을 휴지통으로 옮길까요?", req.paths.length)}</DialogTitle>
          <DialogContent>
            <PathLines paths={req.paths} />
            <p className="dlg-text">
              {formatBytes(app.trashRequestBytes())} · {t("휴지통에서 복구하거나 Ctrl+Z로 되돌릴 수 있습니다.")}
            </p>
            {req.note && <p className="dlg-note">{req.note}</p>}
          </DialogContent>
          <DialogActions>
            <Button appearance="primary" onClick={() => void app.confirmTrash()}>
              {t("휴지통으로 이동")}
            </Button>
            <Button appearance="secondary" onClick={() => app.cancelTrash()}>
              {t("취소")}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

export function MoveDialog() {
  const app = useApp();
  const req = app.moveRequest;
  if (!req) return null;
  return (
    <Dialog open modalType="alert" onOpenChange={(_, d) => !d.open && app.cancelMove()}>
      <DialogSurface className="dlg">
        <DialogBody>
          <DialogTitle>{req.title}</DialogTitle>
          <DialogContent>
            <PathLines paths={req.paths} />
            <p className="dlg-text">{t("옮긴 뒤에도 Ctrl+Z로 한 번에 되돌릴 수 있습니다.")}</p>
          </DialogContent>
          <DialogActions>
            <Button appearance="primary" onClick={() => void app.confirmMove()}>
              {t("이동")}
            </Button>
            <Button appearance="secondary" onClick={() => app.cancelMove()}>
              {t("취소")}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

export function AlertDialog() {
  const app = useApp();
  const alert = app.alert;
  if (!alert) return null;
  return (
    <Dialog open modalType="alert" onOpenChange={(_, d) => !d.open && app.dismissAlert()}>
      <DialogSurface className="dlg">
        <DialogBody>
          <DialogTitle>{alert.title}</DialogTitle>
          <DialogContent>
            <p className="dlg-text pre">{alert.message}</p>
          </DialogContent>
          <DialogActions>
            {alert.action && (
              <Button
                appearance="secondary"
                onClick={() => {
                  alert.action?.run();
                  app.dismissAlert();
                }}
              >
                {alert.action.label}
              </Button>
            )}
            <Button appearance="primary" onClick={() => app.dismissAlert()}>
              {t("확인")}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
