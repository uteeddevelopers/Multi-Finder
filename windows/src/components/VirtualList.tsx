// Virtualized row list: only the rows in view (plus a few) are in the DOM, so 30k+ rows stay fast.
// Row heights are fixed per row (one number for all rows, or an array). Keyboard: arrows, Home/End,
// PageUp/PageDown (with Shift to extend); other keys go to onKeyDown.
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";

export interface VirtualListHandle {
  focus(): void;
  scrollToIndex(index: number, align?: "auto" | "center"): void;
  element(): HTMLDivElement | null;
}

export interface VirtualListProps {
  count: number;
  rowHeight: number | readonly number[];
  renderRow: (index: number, style: CSSProperties) => ReactNode;
  /** Keyboard cursor row; kept in view when it changes. */
  focusIndex?: number;
  isSelectable?: (index: number) => boolean;
  onNavigate?: (index: number, mods: { shift: boolean; ctrl: boolean }) => void;
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void;
  /** First and last row index actually in view (no overscan). */
  onRangeChange?: (first: number, last: number) => void;
  onBackgroundMouseDown?: (e: MouseEvent<HTMLDivElement>) => void;
  onBackgroundContextMenu?: (e: MouseEvent<HTMLDivElement>) => void;
  scrollRequest?: { index: number; nonce: number; align?: "auto" | "center" } | null;
  className?: string;
  ariaLabel?: string;
  overscan?: number;
  children?: ReactNode;
}

export const VirtualList = forwardRef<VirtualListHandle, VirtualListProps>(function VirtualList(props, ref) {
  const { count, rowHeight, renderRow, focusIndex, isSelectable, onNavigate, onRangeChange, scrollRequest, overscan = 6 } = props;
  const el = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(600);

  const offsets = useMemo(() => {
    if (typeof rowHeight === "number") return null;
    const o = new Float64Array(count + 1);
    for (let i = 0; i < count; i++) o[i + 1] = o[i] + (rowHeight[i] ?? 32);
    return o;
  }, [count, rowHeight]);

  const fixed = typeof rowHeight === "number" ? rowHeight : 0;
  const total = offsets ? offsets[count] : count * fixed;
  const topOf = useCallback((i: number) => (offsets ? offsets[i] : i * fixed), [offsets, fixed]);
  const heightOf = useCallback((i: number) => (offsets ? offsets[i + 1] - offsets[i] : fixed), [offsets, fixed]);
  const indexAt = useCallback(
    (y: number) => {
      if (count === 0) return 0;
      if (!offsets) return Math.min(count - 1, Math.max(0, Math.floor(y / fixed)));
      let lo = 0, hi = count - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if (offsets[mid] <= y) lo = mid;
        else hi = mid - 1;
      }
      return lo;
    },
    [offsets, fixed, count],
  );

  useLayoutEffect(() => {
    const node = el.current;
    if (!node) return;
    const update = () => setViewport(node.clientHeight);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(node);
    return () => ro.disconnect();
  }, []);

  // Keep the scroll position valid when the list shrinks.
  useLayoutEffect(() => {
    const node = el.current;
    if (node && node.scrollTop > Math.max(0, total - node.clientHeight)) {
      node.scrollTop = Math.max(0, total - node.clientHeight);
      setScrollTop(node.scrollTop);
    }
  }, [total]);

  const first = indexAt(scrollTop);
  const last = indexAt(scrollTop + viewport - 1);
  const start = Math.max(0, first - overscan);
  const end = Math.min(count - 1, last + overscan);

  useEffect(() => {
    if (count > 0) onRangeChange?.(first, Math.min(last, count - 1));
  }, [first, last, count, onRangeChange]);

  const ensureVisible = useCallback(
    (i: number, align: "auto" | "center" = "auto") => {
      const node = el.current;
      if (!node || i < 0 || i >= count) return;
      const top = topOf(i), h = heightOf(i);
      const vh = node.clientHeight;
      if (align === "center") node.scrollTop = Math.max(0, top - vh / 2 + h / 2);
      else if (top < node.scrollTop) node.scrollTop = top;
      else if (top + h > node.scrollTop + vh) node.scrollTop = top + h - vh;
      setScrollTop(node.scrollTop);
    },
    [count, topOf, heightOf],
  );

  useImperativeHandle(ref, () => ({
    focus: () => el.current?.focus({ preventScroll: true }),
    scrollToIndex: ensureVisible,
    element: () => el.current,
  }));

  useEffect(() => {
    if (focusIndex !== undefined && focusIndex >= 0) ensureVisible(focusIndex);
  }, [focusIndex, ensureVisible]);

  useEffect(() => {
    if (scrollRequest && scrollRequest.index >= 0) ensureVisible(scrollRequest.index, scrollRequest.align ?? "center");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scrollRequest?.nonce, scrollRequest?.index]);

  const selectable = useCallback((i: number) => (isSelectable ? isSelectable(i) : true), [isSelectable]);

  const step = (from: number, dir: 1 | -1): number => {
    for (let i = from; i >= 0 && i < count; i += dir) if (selectable(i)) return i;
    return -1;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const mods = { shift: e.shiftKey, ctrl: e.ctrlKey };
    const cur = focusIndex !== undefined && focusIndex >= 0 ? focusIndex : -1;
    const page = Math.max(1, Math.floor(viewport / (fixed || 32)) - 1);
    let target = -1;
    switch (e.key) {
      case "ArrowDown":
        target = cur < 0 ? step(0, 1) : step(cur + 1, 1);
        break;
      case "ArrowUp":
        target = cur < 0 ? step(0, 1) : step(cur - 1, -1);
        break;
      case "Home":
        target = step(0, 1);
        break;
      case "End":
        target = step(count - 1, -1);
        break;
      case "PageDown":
        target = step(Math.min(count - 1, Math.max(0, cur) + page), 1);
        if (target < 0) target = step(count - 1, -1);
        break;
      case "PageUp":
        target = step(Math.max(0, cur - page), -1);
        if (target < 0) target = step(0, 1);
        break;
      default:
        props.onKeyDown?.(e);
        return;
    }
    e.preventDefault();
    if (target >= 0) onNavigate?.(target, mods);
  };

  const rows: ReactNode[] = [];
  for (let i = start; i <= end && i < count; i++) {
    rows.push(renderRow(i, { position: "absolute", top: topOf(i), height: heightOf(i), left: 0, right: 0 }));
  }

  const isBackground = (e: MouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    return target === el.current || target.classList.contains("vlist-inner");
  };

  return (
    <div
      ref={el}
      className={`vlist ${props.className ?? ""}`}
      tabIndex={0}
      role="listbox"
      aria-multiselectable="true"
      aria-label={props.ariaLabel}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      onKeyDown={onKeyDown}
      onMouseDown={(e) => {
        if (isBackground(e)) props.onBackgroundMouseDown?.(e);
      }}
      onContextMenu={(e) => {
        if (isBackground(e)) {
          e.preventDefault();
          props.onBackgroundContextMenu?.(e);
        }
      }}
    >
      <div className="vlist-inner" style={{ height: Math.max(total, 1), position: "relative" }}>
        {rows}
      </div>
      {props.children}
    </div>
  );
});
