// Pointer-based drag of list rows onto drop targets (folder rows, sidebar folders, category buckets).
// Pointer events instead of HTML5 drag & drop: works the same in WebView2 whatever Tauri's
// dragDropEnabled setting is, and gives us auto-scroll and spring-loaded folders.
import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { createPortal } from "react-dom";
import { DocumentMultipleRegular } from "@fluentui/react-icons";

export interface DropTarget {
  kind: string; // data-drop-kind
  value: string; // data-drop-value
}

interface Options {
  /** Which drop kinds this drag accepts. */
  kinds: string[];
  canDrop: (target: DropTarget, paths: string[]) => boolean;
  onDrop: (target: DropTarget, paths: string[]) => void;
  label: (paths: string[]) => string;
  /** Scroll container to auto-scroll near its edges. */
  scrollElement?: () => HTMLElement | null;
  /** Hovering a target this long opens it (folders). */
  onHoverLong?: (target: DropTarget) => void;
}

interface DragState {
  paths: string[];
  x: number;
  y: number;
  target: DropTarget | null;
}

function targetAt(x: number, y: number, kinds: string[]): DropTarget | null {
  const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-kind]");
  if (!el) return null;
  const kind = el.dataset.dropKind ?? "";
  const value = el.dataset.dropValue ?? "";
  return kinds.includes(kind) && value ? { kind, value } : null;
}

export function useRowDrag(opts: Options) {
  const [drag, setDrag] = useState<DragState | null>(null);
  const optsRef = useRef(opts);
  optsRef.current = opts;
  const justDragged = useRef(false);

  // Highlight the current drop target row.
  useEffect(() => {
    if (!drag?.target) return;
    const sel = `[data-drop-kind="${CSS.escape(drag.target.kind)}"][data-drop-value="${CSS.escape(drag.target.value)}"]`;
    const els = document.querySelectorAll<HTMLElement>(sel);
    els.forEach((e) => e.classList.add("drop-target"));
    return () => els.forEach((e) => e.classList.remove("drop-target"));
  }, [drag?.target?.kind, drag?.target?.value, drag]);

  const begin = useCallback((e: ReactPointerEvent, getPaths: () => string[]) => {
    if (e.button !== 0) return;
    const startX = e.clientX, startY = e.clientY;
    let state: DragState | null = null;
    let scrollTimer: ReturnType<typeof setInterval> | null = null;
    let hoverTimer: ReturnType<typeof setTimeout> | null = null;
    let hoverKey = "";
    let lastY = startY;

    const stopScroll = () => {
      if (scrollTimer) clearInterval(scrollTimer);
      scrollTimer = null;
    };
    const autoScroll = () => {
      const sc = optsRef.current.scrollElement?.();
      if (!sc) return;
      const r = sc.getBoundingClientRect();
      const edge = 28;
      const dir = lastY < r.top + edge ? -1 : lastY > r.bottom - edge ? 1 : 0;
      if (dir === 0) return stopScroll();
      if (!scrollTimer) scrollTimer = setInterval(() => (sc.scrollTop += dir * 14), 30);
    };
    const update = (x: number, y: number) => {
      if (!state) return;
      let target = targetAt(x, y, optsRef.current.kinds);
      if (target && !optsRef.current.canDrop(target, state.paths)) target = null;
      const key = target ? `${target.kind}:${target.value}` : "";
      if (key !== hoverKey) {
        hoverKey = key;
        if (hoverTimer) clearTimeout(hoverTimer);
        hoverTimer = target && optsRef.current.onHoverLong ? setTimeout(() => optsRef.current.onHoverLong?.(target!), 900) : null;
      }
      state = { ...state, x, y, target };
      setDrag(state);
    };
    const move = (ev: PointerEvent) => {
      lastY = ev.clientY;
      if (!state) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 6) return;
        const paths = getPaths();
        if (!paths.length) return cleanup();
        state = { paths, x: ev.clientX, y: ev.clientY, target: null };
        document.body.classList.add("dragging");
      }
      update(ev.clientX, ev.clientY);
      autoScroll();
    };
    const up = () => {
      const s = state;
      cleanup();
      if (s) {
        justDragged.current = true;
        setTimeout(() => (justDragged.current = false), 0);
        if (s.target) optsRef.current.onDrop(s.target, s.paths);
      }
    };
    const key = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        cleanup();
      }
    };
    const cleanup = () => {
      stopScroll();
      if (hoverTimer) clearTimeout(hoverTimer);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key, true);
      document.body.classList.remove("dragging");
      state = null;
      setDrag(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", key, true);
  }, []);

  const ghost = drag
    ? createPortal(
        <div className={`drag-ghost ${drag.target ? "can-drop" : ""}`} style={{ left: drag.x + 14, top: drag.y + 10 }}>
          <DocumentMultipleRegular fontSize={16} />
          <span>{optsRef.current.label(drag.paths)}</span>
        </div>,
        document.body,
      )
    : null;

  return { begin, ghost, dragging: !!drag, wasDragging: () => justDragged.current };
}
