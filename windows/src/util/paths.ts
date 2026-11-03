// Path helpers that work for Windows paths (C:\Users\me) and, in the browser mock or on Linux, `/` paths.
// Windows path comparisons are case-insensitive.

export function separatorOf(path: string): "\\" | "/" {
  return path.includes("\\") || /^[A-Za-z]:/.test(path) ? "\\" : "/";
}

function trimTrailing(path: string): string {
  if (path.length <= 1) return path;
  if (/^[A-Za-z]:\\$/.test(path)) return path; // "C:\"
  return path.replace(/[\\/]+$/, "");
}

export function basename(path: string): string {
  const p = trimTrailing(path);
  const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  const name = i >= 0 ? p.slice(i + 1) : p;
  return name || p;
}

export function dirname(path: string): string {
  const p = trimTrailing(path);
  const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  if (i < 0) return "";
  if (i === 2 && /^[A-Za-z]:/.test(p)) return p.slice(0, 3); // "C:\x" → "C:\"
  if (i === 0) return p.slice(0, 1);
  return p.slice(0, i);
}

export function join(dir: string, ...parts: string[]): string {
  const sep = separatorOf(dir);
  let out = trimTrailing(dir);
  for (const part of parts) {
    if (!part) continue;
    const clean = part.replace(/^[\\/]+|[\\/]+$/g, "").replace(/[\\/]/g, sep);
    if (!clean) continue;
    out = out.endsWith(sep) ? out + clean : out + sep + clean;
  }
  return out;
}

function norm(path: string): string {
  const p = trimTrailing(path).replace(/\//g, "\\");
  return separatorOf(path) === "\\" ? p.toLowerCase() : p;
}

export function samePath(a: string, b: string): boolean {
  return norm(a) === norm(b);
}

/** True when `path` is `folder` itself or inside it. */
export function isUnderOrSame(path: string, folder: string): boolean {
  const p = norm(path);
  const f = norm(folder);
  if (p === f) return true;
  const prefix = f.endsWith("\\") ? f : f + "\\";
  return p.startsWith(prefix);
}

/** True when `path` is strictly inside `folder`. */
export function isInside(path: string, folder: string): boolean {
  return !samePath(path, folder) && isUnderOrSame(path, folder);
}

/** `path` relative to `root` ("" for the root itself); null when not under it. */
export function relativeTo(path: string, root: string): string | null {
  if (!isUnderOrSame(path, root)) return null;
  const r = trimTrailing(root);
  const rest = path.slice(r.length).replace(/^[\\/]+/, "");
  return rest;
}

/** The user folder (root) that contains `path` — the deepest one. */
export function rootOf(path: string, roots: readonly string[]): string | undefined {
  let best: string | undefined;
  for (const r of roots) if (isUnderOrSame(path, r) && (!best || r.length > best.length)) best = r;
  return best;
}

/**
 * Short form for subtitles: a path inside one of the user's folders is shown from that folder's name
 * ("Desktop\Photos backup\edited"), anything else as is.
 */
export function displayPath(path: string, roots: readonly string[]): string {
  const root = rootOf(path, roots);
  if (!root) return path;
  const rel = relativeTo(path, root) ?? "";
  const sep = separatorOf(path);
  return rel ? basename(root) + sep + rel : basename(root);
}

/** Leftover areas: C:\Users\me\AppData\Roaming\X → %APPDATA%\X, …\AppData\Local\X → %LOCALAPPDATA%\X. */
export function displayAppDataPath(path: string): string {
  const m = /^[A-Za-z]:\\Users\\[^\\]+\\AppData\\(Roaming|Local|LocalLow)(\\.*)?$/i.exec(path);
  if (!m) return path;
  const area = m[1].toLowerCase() === "roaming" ? "%APPDATA%" : m[1].toLowerCase() === "local" ? "%LOCALAPPDATA%" : "%USERPROFILE%\\AppData\\LocalLow";
  return area + (m[2] ?? "");
}

export function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}
