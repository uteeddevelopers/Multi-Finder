// Categories — a local model reads files and sorts them into the user's buckets (port of CategoriesView.swift).
// Before first use the model has to be downloaded (804.8 MB); files never leave the PC.
import { useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Input,
  Menu,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  ProgressBar,
  SplitButton,
  Textarea,
  ToggleButton,
  Tooltip,
  type MenuButtonProps,
  type MenuProps,
} from "@fluentui/react-components";
import {
  AddRegular,
  ArchiveRegular,
  ArrowDownRegular,
  ArrowDownloadRegular,
  ArrowUpRegular,
  ChevronDownRegular,
  ClockRegular,
  DeleteRegular,
  ErrorCircleRegular,
  FolderArrowRightRegular,
  FolderOpenRegular,
  FolderRegular,
  HandLeftRegular,
  OpenRegular,
  QuestionCircleRegular,
  ShieldLockRegular,
  SparkleRegular,
  StopRegular,
  SubtractCircleRegular,
  TagRegular,
  TextBulletListTreeRegular,
} from "@fluentui/react-icons";
import type { Bucket, Classified } from "../api/types";
import { t } from "../i18n";
import { useApp } from "../model/context";
import { VirtualList, type VirtualListHandle } from "../components/VirtualList";
import { useContextMenu } from "../components/ContextMenu";
import { EmptyState, FileIcon, keepListFocus } from "../components/common";
import { useRowDrag } from "../components/useRowDrag";
import { clickSel, indexOfKeys, moveSel, selectAllSel } from "../util/selection";
import { basename, dirname, extOf, relativeTo } from "../util/paths";
import { formatBytes, formatPercent } from "../util/format";

const ROW_H = 46;

export function CategoriesActions() {
  const app = useApp();
  const cat = app.categories;
  const scope = cat.effectiveScope;
  const onScope: MenuProps["onCheckedValueChange"] = (_, d) => cat.setScope(d.checkedItems[0]);
  const canRun = cat.modelReady && !!scope;
  return (
    <>
      <Menu checkedValues={{ scope: scope ? [scope] : [] }} onCheckedValueChange={onScope}>
        <MenuTrigger disableButtonEnhancement>
          <Tooltip content={t("어느 폴더의 파일을 분류할지")} relationship="description">
            <Button appearance="subtle" icon={<FolderRegular />} disabled={!scope}>
              {scope ? basename(scope) : t("폴더")}
            </Button>
          </Tooltip>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            {app.settings.folders.map((f) => (
              <MenuItemRadio key={f} name="scope" value={f}>
                {basename(f)}
              </MenuItemRadio>
            ))}
          </MenuList>
        </MenuPopover>
      </Menu>
      <Tooltip content={cat.includeSubfolders ? t("하위 폴더의 파일까지 분류 (오래 걸림)") : t("폴더 바로 아래 파일만 분류 — 켜면 하위 폴더까지")} relationship="description">
        <ToggleButton
          appearance="subtle"
          icon={<TextBulletListTreeRegular />}
          checked={cat.includeSubfolders}
          aria-label={t("하위 폴더 포함")}
          onClick={() => cat.setIncludeSubfolders(!cat.includeSubfolders)}
        >
          <span className="tb-label">{t("하위 폴더 포함")}</span>
        </ToggleButton>
      </Tooltip>
      <Tooltip content={t("묶음 이름과 설명 고치기 — 설명이 좋을수록 분류가 정확해집니다")} relationship="description">
        <Button appearance="subtle" icon={<TagRegular />} aria-label={t("묶음 편집")} onClick={() => cat.openEditor(true)}>
          <span className="tb-label">{t("묶음 편집")}</span>
        </Button>
      </Tooltip>
      {cat.running ? (
        <Button icon={<StopRegular />} onClick={() => cat.cancel()}>
          {t("중지")}
        </Button>
      ) : (
        <Menu positioning="below-end">
          <MenuTrigger disableButtonEnhancement>
            {(triggerProps: MenuButtonProps) => (
              <SplitButton appearance="primary" icon={<SparkleRegular />} disabled={!canRun} menuButton={triggerProps} primaryActionButton={{ onClick: () => void cat.run("new") }}>
                {t("분류")}
              </SplitButton>
            )}
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem onClick={() => void cat.run("new")}>{t("새 파일만 분류")}</MenuItem>
              <MenuItem onClick={() => void cat.run("all")}>{t("전부 다시 분류 (직접 고친 건 유지)")}</MenuItem>
            </MenuList>
          </MenuPopover>
        </Menu>
      )}
    </>
  );
}

function ModelCard({ compact }: { compact: boolean }) {
  const app = useApp();
  const cat = app.categories;
  const m = cat.model;
  if (!m || m.state === "ready") return null;
  const total = m.totalBytes || 804_753_632;
  const fraction = Math.min(1, m.bytes / total);
  const action =
    m.state === "downloading" ? (
      <div className="mc-progress">
        <ProgressBar value={fraction} thickness="large" />
        <div className="mc-progress-row">
          <span className="dim">
            {t("내려받는 중… {0} / {1}", formatBytes(m.bytes), formatBytes(total))} · {formatPercent(fraction)}
          </span>
          <Button size="small" onClick={() => void cat.cancelDownload()}>
            {t("취소")}
          </Button>
        </div>
      </div>
    ) : m.state === "error" ? (
      <div className="mc-error-box">
        <div className="mc-error">
          <ErrorCircleRegular />
          <span>{t("모델을 내려받지 못했습니다")}</span>
        </div>
        {m.error && <div className="mc-error-detail">{m.error}</div>}
        <Button appearance="primary" icon={<ArrowDownloadRegular />} onClick={() => void cat.downloadModel()}>
          {t("다시 시도")}
        </Button>
      </div>
    ) : (
      <Button appearance="primary" icon={<ArrowDownloadRegular />} onClick={() => void cat.downloadModel()}>
        {t("내려받기 ({0})", formatBytes(total))}
      </Button>
    );
  if (compact) {
    return (
      <div className="banner model-banner">
        <SparkleRegular />
        <div className="banner-text">{t("자동 분류에 쓸 AI 모델이 아직 없습니다 — 내려받으면 새 파일을 분류할 수 있습니다")}</div>
        {m.state === "downloading" ? <span className="dim">{formatPercent(fraction)}</span> : null}
        {m.state !== "downloading" && (
          <Button size="small" appearance="primary" icon={<ArrowDownloadRegular />} onClick={() => void cat.downloadModel()}>
            {m.state === "error" ? t("다시 시도") : t("내려받기 ({0})", formatBytes(total))}
          </Button>
        )}
      </div>
    );
  }
  return (
    <div className="screen-center">
      <div className="model-card">
        <div className="mc-head">
          <div className="mc-icon">
            <SparkleRegular fontSize={28} />
          </div>
          <div>
            <div className="mc-title">{t("온디바이스 AI 모델")}</div>
            <div className="mc-sub">
              {m.name} · {formatBytes(total)}
            </div>
          </div>
        </div>
        <p className="mc-text">{t("자동 분류는 이 PC에서 돌아가는 작은 AI 모델이 파일 내용을 읽어 묶음을 정합니다. 처음 한 번 모델을 내려받아야 합니다.")}</p>
        <p className="mc-note">
          <ShieldLockRegular />
          <span>{t("파일 내용은 이 PC 밖으로 나가지 않습니다. 인터넷은 모델을 받을 때 한 번만 씁니다.")}</span>
        </p>
        {action}
      </div>
    </div>
  );
}

function ConfidenceDots({ value }: { value: number }) {
  return (
    <span className="dots" title={t("확신 {0}", formatPercent(value))}>
      {[0, 1, 2, 3, 4].map((i) => (
        <span key={i} className={i < value * 5 - 0.01 ? "dot on" : "dot"} />
      ))}
    </span>
  );
}

function BucketList() {
  const app = useApp();
  const cat = app.categories;
  const counts = cat.counts();
  const item = (key: string | null, label: string, count: number, icon: ReactNode, hint?: string) => (
    <div
      key={key ?? "\u0001all"}
      className={`bucket ${cat.selectedBucket === key ? "selected" : ""}`}
      title={hint}
      tabIndex={0}
      role="option"
      aria-selected={cat.selectedBucket === key}
      data-drop-kind={key ? "bucket" : undefined}
      data-drop-value={key ?? undefined}
      onClick={() => cat.selectBucket(key)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") cat.selectBucket(key);
      }}
    >
      <span className="bucket-icon">{icon}</span>
      <span className="bucket-name ellipsis">{label}</span>
      <span className="bucket-count">{count}</span>
    </div>
  );
  return (
    <div className="bucket-pane" role="listbox" aria-label={t("묶음")}>
      {item(null, t("전체"), cat.results.length, <ArchiveRegular />)}
      <div className="sb-header">{t("묶음")}</div>
      {cat.buckets.map((b) => item(b.name, b.name, counts.get(b.name) ?? 0, b.isFallback ? <QuestionCircleRegular /> : <TagRegular />, b.hint))}
      {!cat.running && cat.pending > 0 && (
        <div className="bucket pending">
          <span className="bucket-icon">
            <ClockRegular />
          </span>
          <span className="bucket-name">{t("아직 분류 안 된 파일 {0}개", cat.pending)}</span>
        </div>
      )}
    </div>
  );
}

function BucketMenu({ c }: { c: Classified }) {
  const app = useApp();
  const cat = app.categories;
  const targets = () => (cat.sel.selected.has(c.path) ? [...cat.sel.selected] : [c.path]);
  return (
    <Menu>
      <MenuTrigger disableButtonEnhancement>
        <Tooltip content={c.manual ? t("직접 고른 묶음") : t("모델이 고른 묶음 — 클릭해서 바꾸기")} relationship="description">
          <button className="bucket-btn" onPointerDown={keepListFocus}>
            <span className="ellipsis">{c.bucket}</span>
            {c.manual && <HandLeftRegular className="manual" />}
            <ChevronDownRegular fontSize={12} />
          </button>
        </Tooltip>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          {cat.buckets.map((b) => (
            <MenuItem key={b.id} onClick={() => void cat.move(targets(), b.name)}>
              {b.name}
            </MenuItem>
          ))}
        </MenuList>
      </MenuPopover>
    </Menu>
  );
}

function ResultsTable() {
  const app = useApp();
  const cat = app.categories;
  const menu = useContextMenu();
  const listRef = useRef<VirtualListHandle>(null);
  const items = cat.visible();
  const keys = useMemo(() => items.map((c) => c.path), [items]);
  const keyIndex = useMemo(() => indexOfKeys(keys), [keys]);
  const sel = cat.sel;
  const focusIndex = sel.cursor ? (keyIndex.get(sel.cursor) ?? -1) : -1;
  const scope = cat.effectiveScope ?? "";
  const pending = useRef<string | null>(null);

  const drag = useRowDrag({
    kinds: ["bucket"],
    canDrop: () => true,
    onDrop: (target, paths) => void cat.move(paths, target.value),
    label: (paths) => (paths.length === 1 ? basename(paths[0]) : t("{0}개", paths.length)),
    scrollElement: () => listRef.current?.element() ?? null,
  });

  const onPointerDown = (e: PointerEvent, key: string) => {
    if (e.button !== 0) return;
    listRef.current?.focus();
    const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    pending.current = null;
    if (mods.ctrl || mods.shift || !cat.sel.selected.has(key)) cat.setSel(clickSel(cat.sel, keys, keyIndex, key, mods));
    else pending.current = key;
    if (!mods.ctrl && !mods.shift) drag.begin(e, () => (cat.sel.selected.has(key) ? [...cat.sel.selected] : [key]));
  };
  const onClick = (key: string) => {
    if (drag.wasDragging()) return;
    if (pending.current === key) cat.setSel({ selected: new Set([key]), anchor: key, cursor: key });
    pending.current = null;
  };
  const onContextMenu = (e: MouseEvent, key: string) => {
    e.preventDefault();
    if (!cat.sel.selected.has(key)) cat.setSel({ selected: new Set([key]), anchor: key, cursor: key });
    const paths = [...cat.sel.selected];
    menu(e, [
      { label: t("열기"), icon: <OpenRegular />, onClick: () => paths.forEach((p) => void app.open(p)) },
      { label: t("탐색기에서 보기"), icon: <FolderOpenRegular />, onClick: () => void app.reveal(paths) },
      { label: t("묶음 바꾸기"), icon: <TagRegular />, submenu: cat.buckets.map((b) => ({ label: b.name, onClick: () => void cat.move(paths, b.name) })) },
      { divider: true },
      { label: t("폴더로 이동…"), icon: <FolderArrowRightRegular />, onClick: () => cat.moveSelectionToFolder() },
      { label: t("휴지통으로 이동…"), icon: <DeleteRegular />, onClick: () => cat.trashSelection() },
    ]);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      [...cat.sel.selected].forEach((p) => void app.open(p));
    } else if (e.ctrlKey && (e.key === "a" || e.key === "A")) {
      e.preventDefault();
      cat.setSel(selectAllSel(keys, cat.sel.cursor));
    }
  };

  const renderRow = (i: number, style: CSSProperties): ReactNode => {
    const c = items[i];
    const selected = sel.selected.has(c.path);
    const rel = relativeTo(dirname(c.path), scope) ?? dirname(c.path);
    return (
      <div
        key={c.path}
        style={style}
        role="option"
        aria-selected={selected}
        className={`row cat-row ${selected ? "selected" : ""} ${sel.cursor === c.path ? "cursor" : ""}`}
        title={c.path}
        onPointerDown={(e) => onPointerDown(e, c.path)}
        onClick={() => onClick(c.path)}
        onDoubleClick={() => void app.open(c.path)}
        onContextMenu={(e) => onContextMenu(e, c.path)}
      >
        <div className="cat-file">
          <FileIcon kind="file" ext={extOf(c.path)} />
          <div className="two-line">
            <div className="name ellipsis">{basename(c.path)}</div>
            <div className="sub ellipsis">{rel || basename(scope)}</div>
          </div>
        </div>
        <div className="cat-bucket">
          <BucketMenu c={c} />
        </div>
        <div className="cat-conf">
          <ConfidenceDots value={c.confidence} />
        </div>
        <div className="cat-summary ellipsis" title={c.summary}>
          {c.summary}
        </div>
      </div>
    );
  };

  return (
    <>
      <div className="cat-colheader">
        <span className="cat-file">{t("파일")}</span>
        <span className="cat-bucket">{t("묶음 (열 제목)")}</span>
        <span className="cat-conf">{t("확신")}</span>
        <span className="cat-summary">{t("한 줄 요약")}</span>
      </div>
      <VirtualList
        ref={listRef}
        count={items.length}
        rowHeight={ROW_H}
        renderRow={renderRow}
        focusIndex={focusIndex}
        onNavigate={(i, mods) => cat.setSel(moveSel(cat.sel, keys, keyIndex, items[i].path, mods))}
        onKeyDown={onKeyDown}
        onBackgroundMouseDown={() => cat.setSel({ selected: new Set(), anchor: null, cursor: null })}
        ariaLabel={t("자동 분류")}
        className="cat-list"
      />
      <div className="actionbar">
        {cat.selectedBucket !== null ? (
          <>
            <span className="dim">{t("‘{0}’ {1}개", cat.selectedBucket, items.length)}</span>
            <Button size="small" onClick={() => cat.setSel(selectAllSel(keys, null))}>
              {t("이 묶음 전부 선택")}
            </Button>
          </>
        ) : (
          <span className="dim">{t("{0}개 분류됨", cat.results.length)}</span>
        )}
        <span className="spacer" />
        {sel.selected.size > 0 && (
          <>
            <span className="dim">{t("{0}개 선택", sel.selected.size)}</span>
            <Button icon={<FolderArrowRightRegular />} onClick={() => cat.moveSelectionToFolder()}>
              {t("폴더로 이동…")}
            </Button>
            <Button icon={<DeleteRegular />} onClick={() => cat.trashSelection()}>
              {t("휴지통으로…")}
            </Button>
          </>
        )}
      </div>
      {drag.ghost}
    </>
  );
}

export function CategoriesView() {
  const app = useApp();
  const cat = app.categories;
  const scope = cat.effectiveScope;
  const ready = cat.modelReady;
  let right: ReactNode;
  if (!scope) {
    right = <EmptyState icon={<FolderRegular fontSize={48} />} title={t("모은 폴더가 없습니다")} />;
  } else if (cat.results.length === 0 && !cat.running) {
    right = ready ? (
      <EmptyState
        icon={<SparkleRegular fontSize={48} />}
        title={t("아직 분류된 파일이 없습니다")}
        actions={
          <Button appearance="primary" icon={<SparkleRegular />} onClick={() => void cat.run("new")}>
            {t("분류 시작")}
          </Button>
        }
      >
        {t("‘분류’를 누르면 {0} 폴더의 파일을 읽어 묶음에 넣습니다.\n내용은 이 PC 안의 모델로만 읽고 밖으로 보내지 않습니다. 파일은 그대로 두고, 옮기는 건 고른 뒤에만.", basename(scope))}
      </EmptyState>
    ) : (
      <ModelCard compact={false} />
    );
  } else {
    right = (
      <>
        {!ready && <ModelCard compact />}
        <ResultsTable />
      </>
    );
  }
  return (
    <div className="categories">
      <BucketList />
      <div className="cat-main">
        {cat.running && (
          <div className="classify-progress">
            <ProgressBar value={cat.progressTotal ? cat.progressDone / cat.progressTotal : undefined} className="cp-bar" />
            <span className="dim ellipsis">{t("{0}/{1} · {2}", cat.progressDone, cat.progressTotal, cat.currentFile)}</span>
          </div>
        )}
        {right}
      </div>
      {cat.showEditor && <BucketEditor />}
    </div>
  );
}

function BucketEditor() {
  const app = useApp();
  const cat = app.categories;
  const [list, setList] = useState<Bucket[]>(() => cat.buckets.map((b) => ({ ...b })));
  const update = (i: number, patch: Partial<Bucket>) => setList((l) => l.map((b, j) => (j === i ? { ...b, ...patch } : b)));
  const move = (i: number, d: -1 | 1) =>
    setList((l) => {
      const j = i + d;
      if (j < 0 || j >= l.length || l[i].isFallback || l[j].isFallback) return l;
      const c = [...l];
      [c[i], c[j]] = [c[j], c[i]];
      return c;
    });
  const add = () =>
    setList((l) => {
      const at = l.findIndex((b) => b.isFallback);
      const nb: Bucket = { id: `new-${Date.now()}`, name: t("새 묶음"), hint: "", isFallback: false };
      const c = [...l];
      c.splice(at === -1 ? c.length : at, 0, nb);
      return c;
    });
  const close = () => cat.openEditor(false);
  return (
    <Dialog open onOpenChange={(_, d) => !d.open && close()}>
      <DialogSurface className="dlg dlg-wide">
        <DialogBody>
          <DialogTitle>{t("묶음 편집")}</DialogTitle>
          <DialogContent>
            <p className="dlg-text dim">{t("설명은 모델이 파일을 배정할 때 보는 기준입니다. 어떤 파일이 여기 들어가야 하는지 구체적으로 적을수록 정확해집니다.")}</p>
            <div className="be-list">
              {list.map((b, i) => (
                <div className="be-row" key={b.id}>
                  <Input className="be-name" value={b.name} placeholder={t("이름")} onChange={(_, d) => update(i, { name: d.value })} aria-label={t("이름")} />
                  <Textarea
                    className="be-hint"
                    value={b.hint}
                    placeholder={t("설명 (어떤 파일이 여기 속하는지)")}
                    resize="vertical"
                    rows={2}
                    onChange={(_, d) => update(i, { hint: d.value })}
                    aria-label={t("설명 (어떤 파일이 여기 속하는지)")}
                  />
                  <div className="be-buttons">
                    <Tooltip content={t("위로 이동")} relationship="label">
                      <Button appearance="subtle" size="small" icon={<ArrowUpRegular />} disabled={b.isFallback || i === 0} onClick={() => move(i, -1)} />
                    </Tooltip>
                    <Tooltip content={t("아래로 이동")} relationship="label">
                      <Button appearance="subtle" size="small" icon={<ArrowDownRegular />} disabled={b.isFallback || list[i + 1]?.isFallback !== false} onClick={() => move(i, 1)} />
                    </Tooltip>
                    <Tooltip content={t("제거")} relationship="label">
                      <Button appearance="subtle" size="small" icon={<SubtractCircleRegular />} disabled={b.isFallback} onClick={() => setList((l) => l.filter((_, j) => j !== i))} />
                    </Tooltip>
                  </div>
                </div>
              ))}
            </div>
          </DialogContent>
          <DialogActions position="start">
            <Button icon={<AddRegular />} onClick={add}>
              {t("묶음 추가")}
            </Button>
          </DialogActions>
          <DialogActions position="end">
            <Button
              appearance="primary"
              onClick={() => {
                close();
                void cat.saveBuckets(list);
              }}
            >
              {t("저장")}
            </Button>
            <Button onClick={close}>{t("취소")}</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
