// List selection like Explorer: click, Ctrl+click, Shift+click, Shift+arrows, Ctrl+A.
// `keys` are the selectable rows in display order (headers / placeholders are left out).

export interface Sel {
  selected: ReadonlySet<string>;
  anchor: string | null; // start of a Shift range
  cursor: string | null; // keyboard focus row
}

export const emptySel = (): Sel => ({ selected: new Set(), anchor: null, cursor: null });

export interface Mods {
  ctrl: boolean;
  shift: boolean;
}

function range(keys: readonly string[], index: ReadonlyMap<string, number>, a: string, b: string): string[] {
  const ia = index.get(a);
  const ib = index.get(b);
  if (ia === undefined || ib === undefined) return [b];
  const [lo, hi] = ia <= ib ? [ia, ib] : [ib, ia];
  return keys.slice(lo, hi + 1);
}

export function indexOfKeys(keys: readonly string[]): Map<string, number> {
  const m = new Map<string, number>();
  keys.forEach((k, i) => m.set(k, i));
  return m;
}

/** Mouse click on a row. */
export function clickSel(sel: Sel, keys: readonly string[], index: ReadonlyMap<string, number>, key: string, mods: Mods): Sel {
  if (mods.shift) {
    const anchor = sel.anchor && index.has(sel.anchor) ? sel.anchor : key;
    const span = range(keys, index, anchor, key);
    const selected = new Set(mods.ctrl ? sel.selected : []);
    for (const k of span) selected.add(k);
    return { selected, anchor, cursor: key };
  }
  if (mods.ctrl) {
    const selected = new Set(sel.selected);
    if (selected.has(key)) selected.delete(key);
    else selected.add(key);
    return { selected, anchor: key, cursor: key };
  }
  return { selected: new Set([key]), anchor: key, cursor: key };
}

/** Keyboard move to `key` (arrows, Home/End, Page keys). Shift extends from the anchor; Ctrl only moves focus. */
export function moveSel(sel: Sel, keys: readonly string[], index: ReadonlyMap<string, number>, key: string, mods: Mods): Sel {
  if (mods.shift) {
    const anchor = sel.anchor && index.has(sel.anchor) ? sel.anchor : sel.cursor && index.has(sel.cursor) ? sel.cursor : key;
    return { selected: new Set(range(keys, index, anchor, key)), anchor, cursor: key };
  }
  if (mods.ctrl) return { ...sel, cursor: key };
  return { selected: new Set([key]), anchor: key, cursor: key };
}

export function selectAllSel(keys: readonly string[], cursor: string | null): Sel {
  return { selected: new Set(keys), anchor: keys[0] ?? null, cursor: cursor ?? keys[0] ?? null };
}

/** Drops keys that are no longer listed (filter changed, rows collapsed…). Returns the same object when nothing changed. */
export function pruneSel(sel: Sel, index: ReadonlyMap<string, number>): Sel {
  let changed = false;
  const selected = new Set<string>();
  for (const k of sel.selected) {
    if (index.has(k)) selected.add(k);
    else changed = true;
  }
  const anchor = sel.anchor && index.has(sel.anchor) ? sel.anchor : null;
  const cursor = sel.cursor && index.has(sel.cursor) ? sel.cursor : null;
  if (!changed && anchor === sel.anchor && cursor === sel.cursor) return sel;
  return { selected, anchor, cursor };
}
