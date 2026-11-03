// Real backend: Tauri v2 commands (snake_case names, camelCase args — see SPEC.md) and events.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import type { Backend, BackendEventName, BackendEvents, PickFolderOptions } from "./backend";
import type {
  Bucket,
  ClassifiedList,
  ClassifyRequest,
  DiffResult,
  DirInfo,
  DuplicateReport,
  DuplicateRequest,
  Entry,
  Filter,
  FolderStatus,
  FolderSummary,
  LaunchArgs,
  LeftoverReport,
  ModelStatus,
  OpResult,
  SearchResult,
  Settings,
  Sort,
  Stats,
  UndoResult,
  UndoState,
} from "./types";

/** Turns a Promise<UnlistenFn> into a synchronous unsubscribe. */
function syncUnlisten(p: Promise<() => void>): () => void {
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  p.then((f) => {
    if (cancelled) f();
    else unlisten = f;
  }).catch((e) => console.error(e));
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

export class TauriBackend implements Backend {
  readonly kind = "tauri" as const;

  getSettings(): Promise<Settings> {
    return invoke("get_settings");
  }
  getLaunchArgs(): Promise<LaunchArgs> {
    return invoke("get_launch_args");
  }
  addFolders(paths: string[]): Promise<Settings> {
    return invoke("add_folders", { paths });
  }
  removeFolder(path: string): Promise<Settings> {
    return invoke("remove_folder", { path });
  }
  reorderFolders(folders: string[]): Promise<Settings> {
    return invoke("reorder_folders", { folders });
  }
  folderStates(): Promise<FolderStatus[]> {
    return invoke("folder_states");
  }
  rescan(path?: string): Promise<void> {
    return invoke("rescan", { path: path ?? null });
  }
  isScanning(): Promise<boolean> {
    return invoke("is_scanning");
  }
  listChildren(path: string, filter: Filter, sort: Sort): Promise<Entry[]> {
    return invoke("list_children", { path, filter, sort });
  }
  dirInfo(paths: string[]): Promise<DirInfo[]> {
    return invoke("dir_info", { paths });
  }
  folderSummaries(paths: string[]): Promise<FolderSummary[]> {
    return invoke("folder_summaries", { paths });
  }
  stats(): Promise<Stats> {
    return invoke("stats");
  }
  search(text: string, filter: Filter, sort: Sort, limit: number): Promise<SearchResult> {
    return invoke("search", { text, filter, sort, limit });
  }
  openPath(path: string): Promise<void> {
    return invoke("open_path", { path });
  }
  revealPaths(paths: string[]): Promise<void> {
    return invoke("reveal_paths", { paths });
  }
  readTextPreview(path: string, maxBytes: number): Promise<string> {
    return invoke("read_text_preview", { path, maxBytes });
  }
  moveItems(paths: string[], destination: string): Promise<OpResult> {
    return invoke("move_items", { paths, destination });
  }
  moveAcross(paths: string[], fromRoot: string, toRoot: string): Promise<OpResult> {
    return invoke("move_across", { paths, fromRoot, toRoot });
  }
  trashItems(paths: string[]): Promise<OpResult> {
    return invoke("trash_items", { paths });
  }
  trashDuplicates(paths: string[]): Promise<OpResult> {
    return invoke("trash_duplicates", { paths });
  }
  undo(): Promise<UndoResult> {
    return invoke("undo");
  }
  undoState(): Promise<UndoState> {
    return invoke("undo_state");
  }
  findDuplicates(req: DuplicateRequest): Promise<DuplicateReport> {
    return invoke("find_duplicates", { req });
  }
  cancelDuplicates(): Promise<void> {
    return invoke("cancel_duplicates");
  }
  ignoreDuplicateGroup(id: string): Promise<void> {
    return invoke("ignore_duplicate_group", { id });
  }
  clearDuplicateIgnores(): Promise<void> {
    return invoke("clear_duplicate_ignores");
  }
  compareFolders(left: string, right: string): Promise<DiffResult> {
    return invoke("compare_folders", { left, right });
  }
  getBuckets(): Promise<Bucket[]> {
    return invoke("get_buckets");
  }
  saveBuckets(buckets: Bucket[]): Promise<Bucket[]> {
    return invoke("save_buckets", { buckets });
  }
  modelStatus(): Promise<ModelStatus> {
    return invoke("model_status");
  }
  downloadModel(): Promise<void> {
    return invoke("download_model");
  }
  cancelModelDownload(): Promise<void> {
    return invoke("cancel_model_download");
  }
  classify(req: ClassifyRequest): Promise<void> {
    return invoke("classify", { req });
  }
  cancelClassify(): Promise<void> {
    return invoke("cancel_classify");
  }
  listClassified(scope: string, includeSubfolders: boolean): Promise<ClassifiedList> {
    return invoke("list_classified", { scope, includeSubfolders });
  }
  setBucket(paths: string[], bucket: string): Promise<void> {
    return invoke("set_bucket", { paths, bucket });
  }
  scanLeftovers(): Promise<LeftoverReport> {
    return invoke("scan_leftovers");
  }
  trashLeftovers(paths: string[]): Promise<OpResult> {
    return invoke("trash_leftovers", { paths });
  }

  on<E extends BackendEventName>(event: E, handler: (payload: BackendEvents[E]) => void): () => void {
    return syncUnlisten(listen<BackendEvents[E]>(event, (e) => handler(e.payload)));
  }

  async pickFolders(options?: PickFolderOptions): Promise<string[]> {
    const picked = await open({ directory: true, multiple: options?.multiple ?? false, title: options?.title });
    if (picked === null) return [];
    return Array.isArray(picked) ? picked : [picked];
  }

  previewUrl(path: string): string {
    return convertFileSrc(path);
  }

  onFileDrop(handler: (paths: string[]) => void): () => void {
    return syncUnlisten(
      getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop" && event.payload.paths.length > 0) handler(event.payload.paths);
      }),
    );
  }
}
