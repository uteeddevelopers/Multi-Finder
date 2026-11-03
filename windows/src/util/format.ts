import { intlLocale, t } from "../i18n";

/** Decimal units like the macOS app (ByteCountFormatter .file): 12 KB, 2.6 MB, 804.8 MB, 5.83 GB. */
export function formatBytes(bytes: number): string {
  const locale = intlLocale();
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 KB";
  if (bytes < 1000) return t("{0}바이트", bytes);
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 999.95 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  const digits = unit === 0 ? 0 : unit === 1 ? 1 : 2;
  const text = new Intl.NumberFormat(locale, { minimumFractionDigits: 0, maximumFractionDigits: digits }).format(value);
  return `${text} ${units[unit]}`;
}

const dtCache = new Map<string, Intl.DateTimeFormat>();
function formatter(kind: "datetime" | "date"): Intl.DateTimeFormat {
  const key = `${intlLocale()}|${kind}`;
  let f = dtCache.get(key);
  if (!f) {
    f =
      kind === "datetime"
        ? new Intl.DateTimeFormat(intlLocale(), { year: "numeric", month: "2-digit", day: "2-digit", hour: "numeric", minute: "2-digit" })
        : new Intl.DateTimeFormat(intlLocale(), { year: "numeric", month: "2-digit", day: "2-digit" });
    dtCache.set(key, f);
  }
  return f;
}

/** Date + time for list columns ("11/02/2026, 11:04 PM"). */
export function formatDateTime(ms: number): string {
  if (!ms) return "";
  return formatter("datetime").format(new Date(ms));
}

/** Date only ("11/02/2026"). */
export function formatDate(ms: number): string {
  if (!ms) return "";
  return formatter("date").format(new Date(ms));
}

export function formatPercent(value: number): string {
  return new Intl.NumberFormat(intlLocale(), { style: "percent", maximumFractionDigits: 0 }).format(value);
}
