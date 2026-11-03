import type { FileCategory } from "../api/types";
import type { MsgKey } from "../i18n";

/** Same sets as the macOS FileFilter.swift, plus Windows installers (exe, msi, msix, appx). */
export const CATEGORY_EXTENSIONS: Record<Exclude<FileCategory, "all" | "other">, readonly string[]> = {
  document: ["pdf", "doc", "docx", "ppt", "pptx", "key", "xls", "xlsx", "numbers", "pages", "hwp", "hwpx", "txt", "md", "rtf", "csv", "epub"],
  image: ["jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp", "svg", "psd", "ai", "raw", "dng"],
  video: ["mp4", "mov", "mkv", "avi", "m4v", "webm", "wmv"],
  audio: ["mp3", "m4a", "wav", "aac", "flac", "aiff", "ogg"],
  archive: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz"],
  installer: ["exe", "msi", "msix", "msixbundle", "appx", "appxbundle", "iso", "dmg", "pkg", "mpkg", "xip"],
  code: ["swift", "js", "ts", "tsx", "jsx", "py", "java", "kt", "c", "cpp", "h", "m", "go", "rs", "rb", "php", "html", "css", "scss", "json", "yaml", "yml", "toml", "sh", "sql", "ps1", "bat", "cs"],
};

const byExt = new Map<string, FileCategory>();
for (const [cat, exts] of Object.entries(CATEGORY_EXTENSIONS)) for (const e of exts) if (!byExt.has(e)) byExt.set(e, cat as FileCategory);

export function categoryOfExt(ext: string): FileCategory {
  return byExt.get(ext.toLowerCase()) ?? "other";
}

export const CATEGORY_ORDER: FileCategory[] = ["all", "document", "image", "video", "audio", "archive", "installer", "code", "other"];

export const CATEGORY_LABEL: Record<FileCategory, MsgKey> = {
  all: "모든 형식",
  document: "문서",
  image: "이미지",
  video: "영상",
  audio: "음악",
  archive: "압축",
  installer: "설치 파일",
  code: "코드",
  other: "기타",
};

/** Size floors offered in the UI (bytes) — 0, 1 MB, 10 MB, 100 MB, 1 GB (binary MB like macOS SizeFloor). */
export const SIZE_FLOORS: { bytes: number; label: MsgKey }[] = [
  { bytes: 0, label: "모든 크기" },
  { bytes: 1 * 1024 * 1024, label: "1 MB 이상" },
  { bytes: 10 * 1024 * 1024, label: "10 MB 이상" },
  { bytes: 100 * 1024 * 1024, label: "100 MB 이상" },
  { bytes: 1024 * 1024 * 1024, label: "1 GB 이상" },
];

export const RECENT_RANGES: { days: number; label: MsgKey }[] = [
  { days: 0, label: "모든 기간" },
  { days: 1, label: "오늘" },
  { days: 7, label: "최근 7일" },
  { days: 30, label: "최근 30일" },
  { days: 365, label: "최근 1년" },
];

/** Previewable as an image / video / audio / PDF in WebView2. */
export type PreviewKind = "image" | "video" | "audio" | "pdf" | "text" | "none";

const TEXT_EXTS = new Set(["txt", "md", "csv", "log", "json", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf", "html", "css", "scss", "js", "ts", "tsx", "jsx", "py", "rs", "go", "java", "kt", "c", "cpp", "h", "cs", "rb", "php", "sh", "ps1", "bat", "sql", "swift", "m", "tsv", "srt", "rtf"]);
const IMAGE_PREVIEW = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico", "avif"]);
const VIDEO_PREVIEW = new Set(["mp4", "webm", "m4v", "mov", "ogv"]);
const AUDIO_PREVIEW = new Set(["mp3", "m4a", "wav", "aac", "flac", "ogg", "oga"]);

export function previewKindOf(ext: string): PreviewKind {
  const e = ext.toLowerCase();
  if (IMAGE_PREVIEW.has(e)) return "image";
  if (VIDEO_PREVIEW.has(e)) return "video";
  if (AUDIO_PREVIEW.has(e)) return "audio";
  if (e === "pdf") return "pdf";
  if (TEXT_EXTS.has(e)) return "text";
  return "none";
}
