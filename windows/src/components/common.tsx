import type { ReactNode } from "react";
import { Spinner } from "@fluentui/react-components";
import {
  AppGenericRegular,
  BoxRegular,
  CodeRegular,
  DocumentPdfRegular,
  DocumentRegular,
  DocumentTextRegular,
  FolderFilled,
  FolderZipRegular,
  ImageRegular,
  LinkRegular,
  MusicNote2Regular,
  SlideTextRegular,
  TableRegular,
  VideoRegular,
} from "@fluentui/react-icons";
import type { Kind } from "../api/types";
import { categoryOfExt } from "../util/fileKinds";
import { boldParts } from "../i18n";

/** Icon for a file or folder, colored by kind (Explorer-like). */
export function FileIcon({ kind, ext, size = 16 }: { kind: Kind; ext: string; size?: number }) {
  const props = { fontSize: size, className: "ficon" };
  if (kind === "dir") return <FolderFilled {...props} className="ficon ic-folder" />;
  if (kind === "link") return <LinkRegular {...props} />;
  const e = ext.toLowerCase();
  if (e === "pdf") return <DocumentPdfRegular {...props} className="ficon ic-pdf" />;
  if (["xls", "xlsx", "csv", "numbers", "tsv"].includes(e)) return <TableRegular {...props} className="ficon ic-sheet" />;
  if (["ppt", "pptx", "key"].includes(e)) return <SlideTextRegular {...props} className="ficon ic-slides" />;
  if (["doc", "docx", "hwp", "hwpx", "pages", "rtf"].includes(e)) return <DocumentTextRegular {...props} className="ficon ic-doc" />;
  switch (categoryOfExt(e)) {
    case "image":
      return <ImageRegular {...props} className="ficon ic-image" />;
    case "video":
      return <VideoRegular {...props} className="ficon ic-video" />;
    case "audio":
      return <MusicNote2Regular {...props} className="ficon ic-audio" />;
    case "archive":
      return <FolderZipRegular {...props} className="ficon ic-archive" />;
    case "installer":
      return e === "iso" ? <BoxRegular {...props} className="ficon ic-installer" /> : <AppGenericRegular {...props} className="ficon ic-installer" />;
    case "code":
      return <CodeRegular {...props} className="ficon ic-code" />;
    case "document":
      return <DocumentTextRegular {...props} className="ficon ic-doc" />;
    default:
      return <DocumentRegular {...props} />;
  }
}

/** Big centered message for empty / first-run states (like ContentUnavailableView). */
export function EmptyState({ icon, title, children, actions }: { icon?: ReactNode; title: string; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="empty-state">
      {icon && <div className="empty-icon">{icon}</div>}
      <div className="empty-title">{title}</div>
      {children && <div className="empty-text">{children}</div>}
      {actions && <div className="empty-actions">{actions}</div>}
    </div>
  );
}

export function Progress({ text }: { text: string }) {
  return (
    <div className="empty-state">
      <Spinner size="medium" />
      <div className="empty-text progress-text">{text}</div>
    </div>
  );
}

/** Renders **bold** segments of a translated string. */
export function Rich({ text }: { text: string }) {
  return (
    <>
      {boldParts(text).map((part, i) => (i % 2 === 1 ? <strong key={i}>{part}</strong> : <span key={i}>{part}</span>))}
    </>
  );
}

export function Banner({ icon, children, actions }: { icon?: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <div className="banner">
      {icon && <span className="banner-icon">{icon}</span>}
      <div className="banner-text">{children}</div>
      {actions}
    </div>
  );
}

/** Mouse-down handler for buttons inside list rows: keeps keyboard focus on the list. */
export const keepListFocus = (e: { preventDefault(): void; stopPropagation(): void }) => {
  e.preventDefault();
  e.stopPropagation();
};
