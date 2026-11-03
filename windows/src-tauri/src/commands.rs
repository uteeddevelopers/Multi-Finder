//! Tauri 명령 — SPEC.md의 표와 src/api/types.ts의 모양 그대로. 오래 걸리는 일은 작업 스레드에서.

use std::path::Path;
use std::sync::atomic::Ordering;

use mf_core::analyzer::{self, ProjectRoots};
use mf_core::leftovers::{self, LeftoverOptions};
use mf_core::{
    classify, diff, duplicates, text, Bucket, ClassifiedList, ClassifyFinishedEvent, ClassifyRequest, DiffResult,
    DirInfo, DuplicateReport, DuplicateRequest, Entry, Filter, FolderStatus, FolderSummary, LaunchArgs, LeftoverReport,
    ModelFinishedEvent, ModelProgressEvent, ModelStatus, OpResult, ProgressEvent, SearchResult, Settings, Sort, Stats,
    UndoResult, UndoState,
};
use tauri::{AppHandle, Emitter, State};

use crate::state::{self, lock, Shared};

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// 막히는 일을 작업 스레드에서 돌린다
async fn blocking<T, F>(f: F) -> Res<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(err)
}

// ───────────────────────────── 설정·폴더 ─────────────────────────────

#[tauri::command]
pub fn get_settings(st: State<'_, Shared>) -> Settings {
    st.view()
}

#[tauri::command]
pub fn get_launch_args(st: State<'_, Shared>) -> LaunchArgs {
    st.launch.clone()
}

#[tauri::command]
pub fn add_folders(app: AppHandle, st: State<'_, Shared>, paths: Vec<String>) -> Res<Settings> {
    // 탐색기에서 끌어다 놓은 것에는 파일도 섞일 수 있다 — 폴더만
    let dirs: Vec<String> = paths.into_iter().filter(|p| Path::new(p).is_dir()).collect();
    let added = {
        let mut s = lock(&st.settings);
        let added = s.add_folders(&dirs);
        s.save(&st.data_dir).map_err(|e| e.to_string())?;
        added
    };
    if !added.is_empty() {
        for f in &added {
            state::allow_preview(&app, f);
        }
        state::restart_watcher(&app, &st);
        state::spawn_scan(&app, &st, added);
    }
    Ok(st.view())
}

#[tauri::command]
pub fn remove_folder(app: AppHandle, st: State<'_, Shared>, path: String) -> Res<Settings> {
    {
        let mut s = lock(&st.settings);
        s.remove_folder(&path);
        s.save(&st.data_dir).map_err(|e| e.to_string())?;
    }
    state::restart_watcher(&app, &st);
    Ok(st.view())
}

#[tauri::command]
pub fn reorder_folders(st: State<'_, Shared>, folders: Vec<String>) -> Res<Settings> {
    {
        let mut s = lock(&st.settings);
        s.reorder(&folders);
        s.save(&st.data_dir).map_err(|e| e.to_string())?;
    }
    Ok(st.view())
}

#[tauri::command]
pub async fn folder_states(st: State<'_, Shared>) -> Res<Vec<FolderStatus>> {
    let st = st.inner().clone();
    blocking(move || {
        let scanning = lock(&st.scanning).clone();
        st.store.folder_states(&st.folders(), &scanning)
    })
    .await
}

#[tauri::command]
pub fn rescan(app: AppHandle, st: State<'_, Shared>, path: Option<String>) {
    let roots = match path {
        Some(p) => vec![p],
        None => st.folders(),
    };
    state::spawn_scan(&app, &st, roots);
}

#[tauri::command]
pub fn is_scanning(st: State<'_, Shared>) -> bool {
    st.is_scanning()
}

// ───────────────────────────── 목록·검색 ─────────────────────────────

#[tauri::command]
pub async fn list_children(st: State<'_, Shared>, path: String, filter: Filter, sort: Sort) -> Res<Vec<Entry>> {
    let st = st.inner().clone();
    blocking(move || st.store.list_children(&path, &filter, sort)).await
}

#[tauri::command]
pub async fn dir_info(st: State<'_, Shared>, paths: Vec<String>) -> Res<Vec<DirInfo>> {
    let st = st.inner().clone();
    blocking(move || st.store.dir_info(&paths)).await
}

#[tauri::command]
pub async fn folder_summaries(st: State<'_, Shared>, paths: Vec<String>) -> Res<Vec<FolderSummary>> {
    let st = st.inner().clone();
    blocking(move || st.store.folder_summaries(&paths)).await
}

#[tauri::command]
pub async fn stats(st: State<'_, Shared>) -> Res<Stats> {
    let st = st.inner().clone();
    blocking(move || st.store.stats(&st.folders())).await
}

#[tauri::command]
pub async fn search(
    st: State<'_, Shared>,
    text: String,
    filter: Filter,
    sort: Sort,
    limit: usize,
) -> Res<SearchResult> {
    let st = st.inner().clone();
    blocking(move || st.store.search(&text, &st.folders(), &filter, sort, limit)).await
}

// ───────────────────────────── 열기·보여 주기·미리보기 ─────────────────────────────

#[tauri::command]
pub fn open_path(path: String) -> Res<()> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_paths(paths: Vec<String>) -> Res<()> {
    match paths.as_slice() {
        [] => Ok(()),
        [one] => tauri_plugin_opener::reveal_item_in_dir(one).map_err(|e| e.to_string()),
        many => tauri_plugin_opener::reveal_items_in_dir(many).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn read_text_preview(path: String, max_bytes: usize) -> Res<String> {
    blocking(move || Ok(text::read_text_preview(Path::new(&path), max_bytes.min(1 << 20))?)).await
}

// ───────────────────────────── 이동·휴지통·되돌리기 ─────────────────────────────

#[tauri::command]
pub async fn move_items(st: State<'_, Shared>, paths: Vec<String>, destination: String) -> Res<OpResult> {
    let st = st.inner().clone();
    blocking(move || st.executor.move_items(&paths, &destination)).await
}

#[tauri::command]
pub async fn move_across(
    st: State<'_, Shared>,
    paths: Vec<String>,
    from_root: String,
    to_root: String,
) -> Res<OpResult> {
    let st = st.inner().clone();
    blocking(move || st.executor.move_across(&paths, &from_root, &to_root)).await
}

#[tauri::command]
pub async fn trash_items(st: State<'_, Shared>, paths: Vec<String>) -> Res<OpResult> {
    let st = st.inner().clone();
    blocking(move || st.executor.trash_items(&paths)).await
}

#[tauri::command]
pub async fn trash_duplicates(st: State<'_, Shared>, paths: Vec<String>) -> Res<OpResult> {
    let st = st.inner().clone();
    blocking(move || st.executor.trash_duplicates(&paths)).await
}

#[tauri::command]
pub async fn undo(app: AppHandle, st: State<'_, Shared>) -> Res<UndoResult> {
    let shared = st.inner().clone();
    let outcome = blocking(move || shared.executor.undo(shared.session_start)).await?;
    for e in &outcome.errors {
        eprintln!("undo: {e}");
    }
    // 휴지통에서 돌아온 폴더는 안쪽이 인덱스에 없다 → 다시 훑는다
    if !outcome.rescan.is_empty() {
        state::spawn_scan(&app, st.inner(), outcome.rescan.clone());
    }
    Ok(outcome.result)
}

#[tauri::command]
pub async fn undo_state(st: State<'_, Shared>) -> Res<UndoState> {
    let st = st.inner().clone();
    blocking(move || st.executor.undo_state(st.session_start)).await
}

// ───────────────────────────── 중복 ─────────────────────────────

#[tauri::command]
pub async fn find_duplicates(app: AppHandle, st: State<'_, Shared>, req: DuplicateRequest) -> Res<DuplicateReport> {
    let st = st.inner().clone();
    blocking(move || {
        st.dup_cancel.reset();
        st.wait_for_scans(&st.dup_cancel)?;
        let progress = |e: &ProgressEvent| {
            let _ = app.emit("dup-progress", e);
        };
        let report = duplicates::find_duplicates(
            &st.store,
            &req,
            &mut ProjectRoots::platform_default(),
            &progress,
            &st.dup_cancel,
        )?;
        *lock(&st.last_dup_report) = Some(report.clone());
        Ok(report)
    })
    .await
}

#[tauri::command]
pub fn cancel_duplicates(st: State<'_, Shared>) {
    st.dup_cancel.cancel();
}

#[tauri::command]
pub async fn ignore_duplicate_group(st: State<'_, Shared>, id: String) -> Res<()> {
    let st = st.inner().clone();
    blocking(move || {
        let hashes = analyzer::hashes_for_id(lock(&st.last_dup_report).as_ref(), &id);
        st.store.add_ignore_hashes(&hashes)
    })
    .await
}

#[tauri::command]
pub async fn clear_duplicate_ignores(st: State<'_, Shared>) -> Res<()> {
    let st = st.inner().clone();
    blocking(move || st.store.clear_ignores()).await
}

// ───────────────────────────── 폴더 비교 ─────────────────────────────

#[tauri::command]
pub async fn compare_folders(app: AppHandle, st: State<'_, Shared>, left: String, right: String) -> Res<DiffResult> {
    state::allow_preview(&app, &left);
    state::allow_preview(&app, &right);
    let st = st.inner().clone();
    blocking(move || {
        st.compare_cancel.reset();
        let progress = |e: &ProgressEvent| {
            let _ = app.emit("compare-progress", e);
        };
        diff::compare_folders(&st.store, &st.scanner, &left, &right, &progress, &st.compare_cancel)
    })
    .await
}

// ───────────────────────────── 자동 분류 ─────────────────────────────

#[tauri::command]
pub async fn get_buckets(st: State<'_, Shared>) -> Res<Vec<Bucket>> {
    let st = st.inner().clone();
    blocking(move || classify::get_buckets(&st.store, st.lang)).await
}

#[tauri::command]
pub async fn save_buckets(st: State<'_, Shared>, buckets: Vec<Bucket>) -> Res<Vec<Bucket>> {
    let st = st.inner().clone();
    blocking(move || classify::save_buckets(&st.store, &buckets, st.lang)).await
}

#[tauri::command]
pub fn model_status(st: State<'_, Shared>) -> ModelStatus {
    st.models.status()
}

#[tauri::command]
pub fn download_model(app: AppHandle, st: State<'_, Shared>) {
    let st = st.inner().clone();
    std::thread::spawn(move || {
        let progress = |e: &ModelProgressEvent| {
            let _ = app.emit("model-progress", e);
        };
        let finished = match st.models.download(&progress) {
            Ok(_) => ModelFinishedEvent { ok: true, error: None },
            Err(e) => ModelFinishedEvent { ok: false, error: Some(err(e)) },
        };
        let _ = app.emit("model-finished", &finished);
    });
}

#[tauri::command]
pub fn cancel_model_download(st: State<'_, Shared>) {
    st.models.cancel();
}

#[tauri::command]
pub fn classify(app: AppHandle, st: State<'_, Shared>, req: ClassifyRequest) -> Res<()> {
    if st.classify_running.swap(true, Ordering::SeqCst) {
        return Err("classification is already running".into());
    }
    let st = st.inner().clone();
    std::thread::spawn(move || {
        st.classify_cancel.reset();
        let progress = |e: &ProgressEvent| {
            let _ = app.emit("classify-progress", e);
        };
        let models = &st.models;
        let mut factory = || -> anyhow::Result<Box<dyn classify::Engine>> {
            let path = models
                .ready_path()
                .ok_or_else(|| anyhow::anyhow!("the model is not downloaded"))?;
            Ok(Box::new(mf_core::llm::LlamaEngine::load(&path)?))
        };
        let result = st
            .wait_for_scans(&st.classify_cancel)
            .and_then(|_| classify::run_classify(&st.store, &req, st.lang, &mut factory, &progress, &st.classify_cancel));
        let finished = match result {
            Ok(done) => done,
            Err(e) => {
                let cancelled = mf_core::is_cancelled(&e);
                if !cancelled {
                    eprintln!("classify: {e:#}");
                }
                ClassifyFinishedEvent { classified: 0, failed: 0, cancelled }
            }
        };
        st.classify_running.store(false, Ordering::SeqCst);
        let _ = app.emit("classify-finished", &finished);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_classify(st: State<'_, Shared>) {
    st.classify_cancel.cancel();
}

#[tauri::command]
pub async fn list_classified(st: State<'_, Shared>, scope: String, include_subfolders: bool) -> Res<ClassifiedList> {
    let st = st.inner().clone();
    blocking(move || classify::list_classified(&st.store, &scope, include_subfolders)).await
}

#[tauri::command]
pub async fn set_bucket(st: State<'_, Shared>, paths: Vec<String>, bucket: String) -> Res<()> {
    let st = st.inner().clone();
    blocking(move || classify::set_bucket(&st.store, &paths, &bucket)).await
}

// ───────────────────────────── 앱 잔여물 ─────────────────────────────

#[tauri::command]
pub async fn scan_leftovers(app: AppHandle, st: State<'_, Shared>) -> Res<LeftoverReport> {
    let st = st.inner().clone();
    blocking(move || {
        st.leftover_cancel.reset();
        let progress = |e: &ProgressEvent| {
            let _ = app.emit("leftover-progress", e);
        };
        let opts = LeftoverOptions {
            // 이 앱 자신의 WebView2 데이터·인덱스 폴더
            extra_protected: vec!["com.taeholim.multifinder".into(), "Multi-Finder".into()],
            ..LeftoverOptions::default()
        };
        leftovers::scan_leftovers(
            &leftovers::default_areas(),
            &*leftovers::system_apps(),
            &opts,
            &progress,
            &st.leftover_cancel,
        )
    })
    .await
}

#[tauri::command]
pub async fn trash_leftovers(st: State<'_, Shared>, paths: Vec<String>) -> Res<OpResult> {
    let st = st.inner().clone();
    blocking(move || st.executor.trash_items(&paths)).await
}
