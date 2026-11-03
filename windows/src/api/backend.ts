// The UI talks to the backend only through this interface. `pick()` returns the Tauri implementation
// inside the app (window.__TAURI_INTERNALS__) and the in-memory mock in a plain browser.
import type {
  Bucket,
  ClassifiedList,
  ClassifyFinishedEvent,
  ClassifyRequest,
  DiffResult,
  DirInfo,
  DuplicateReport,
  DuplicateRequest,
  Entry,
  Filter,
  FolderStatus,
  FolderSummary,
  IndexChangedEvent,
  LaunchArgs,
  LeftoverReport,
  ModelFinishedEvent,
  ModelProgressEvent,
  ModelStatus,
  OpResult,
  ProgressEvent,
  ScanFinishedEvent,
  ScanProgressEvent,
  SearchResult,
  Settings,
  Sort,
  Stats,
  UndoResult,
  UndoState,
} from "./types";

/** Backend → UI events (Tauri `listen`), with their payload types. */
export interface BackendEvents {
  "scan-progress": ScanProgressEvent;
  "scan-finished": ScanFinishedEvent;
  "index-changed": IndexChangedEvent;
  "dup-progress": ProgressEvent;
  "compare-progress": ProgressEvent;
  "classify-progress": ProgressEvent;
  "classify-finished": ClassifyFinishedEvent;
  "model-progress": ModelProgressEvent;
  "model-finished": ModelFinishedEvent;
  "leftover-progress": ProgressEvent;
}
export type BackendEventName = keyof BackendEvents;

export const BACKEND_EVENTS: BackendEventName[] = [
  "scan-progress",
  "scan-finished",
  "index-changed",
  "dup-progress",
  "compare-progress",
  "classify-progress",
  "classify-finished",
  "model-progress",
  "model-finished",
  "leftover-progress",
];

export interface PickFolderOptions {
  /** Allow choosing several folders (Add Folder). Default false. */
  multiple?: boolean;
  /** Dialog title. */
  title?: string;
}

/** One async method per backend command (SPEC.md table; Tauri name = snake_case of the method). */
export interface Backend {
  readonly kind: "tauri" | "mock";

  getSettings(): Promise<Settings>;
  getLaunchArgs(): Promise<LaunchArgs>;
  addFolders(paths: string[]): Promise<Settings>;
  removeFolder(path: string): Promise<Settings>;
  reorderFolders(folders: string[]): Promise<Settings>;
  folderStates(): Promise<FolderStatus[]>;
  rescan(path?: string): Promise<void>;
  isScanning(): Promise<boolean>;
  listChildren(path: string, filter: Filter, sort: Sort): Promise<Entry[]>;
  dirInfo(paths: string[]): Promise<DirInfo[]>;
  folderSummaries(paths: string[]): Promise<FolderSummary[]>;
  stats(): Promise<Stats>;
  search(text: string, filter: Filter, sort: Sort, limit: number): Promise<SearchResult>;
  openPath(path: string): Promise<void>;
  revealPaths(paths: string[]): Promise<void>;
  readTextPreview(path: string, maxBytes: number): Promise<string>;
  moveItems(paths: string[], destination: string): Promise<OpResult>;
  moveAcross(paths: string[], fromRoot: string, toRoot: string): Promise<OpResult>;
  trashItems(paths: string[]): Promise<OpResult>;
  trashDuplicates(paths: string[]): Promise<OpResult>;
  undo(): Promise<UndoResult>;
  undoState(): Promise<UndoState>;
  findDuplicates(req: DuplicateRequest): Promise<DuplicateReport>;
  cancelDuplicates(): Promise<void>;
  ignoreDuplicateGroup(id: string): Promise<void>;
  clearDuplicateIgnores(): Promise<void>;
  compareFolders(left: string, right: string): Promise<DiffResult>;
  getBuckets(): Promise<Bucket[]>;
  saveBuckets(buckets: Bucket[]): Promise<Bucket[]>;
  modelStatus(): Promise<ModelStatus>;
  downloadModel(): Promise<void>;
  cancelModelDownload(): Promise<void>;
  classify(req: ClassifyRequest): Promise<void>;
  cancelClassify(): Promise<void>;
  listClassified(scope: string, includeSubfolders: boolean): Promise<ClassifiedList>;
  setBucket(paths: string[], bucket: string): Promise<void>;
  scanLeftovers(): Promise<LeftoverReport>;
  trashLeftovers(paths: string[]): Promise<OpResult>;

  /** Subscribes to a backend event; returns the unsubscribe function. */
  on<E extends BackendEventName>(event: E, handler: (payload: BackendEvents[E]) => void): () => void;
  /** Native folder picker. Empty array = cancelled. */
  pickFolders(options?: PickFolderOptions): Promise<string[]>;
  /** URL the web view can load for a local file (Tauri asset protocol). "" when not previewable. */
  previewUrl(path: string): string;
  /** Files/folders dropped onto the window from Explorer (Tauri drag-drop event). */
  onFileDrop(handler: (paths: string[]) => void): () => void;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Tauri implementation inside the app, mock in a plain browser. */
export async function pick(): Promise<Backend> {
  if (isTauri()) {
    const { TauriBackend } = await import("./tauri");
    return new TauriBackend();
  }
  const { MockBackend } = await import("./mock");
  return new MockBackend();
}
