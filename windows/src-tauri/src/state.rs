//! 앱 상태와 백그라운드 작업(스캔·감시). 모든 로직은 mf-core에 있고 여기선 연결만 한다.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{bail, Result};
use mf_core::model::ModelManager;
use mf_core::{
    classify, paths, settings, watcher, Cancel, DuplicateReport, Executor, IndexChangedEvent, IndexStore,
    LaunchArgs, PathStyle, ProtectedPaths, ScanFinishedEvent, ScanProgressEvent, ScanRules, Scanner, Watcher,
};
use tauri::{AppHandle, Emitter, Manager};

pub struct AppState {
    pub data_dir: PathBuf,
    pub locale: String,
    pub lang: &'static str,
    pub version: String,
    pub settings: Mutex<settings::Settings>,
    pub store: Arc<IndexStore>,
    pub rules: ScanRules,
    pub scanner: Scanner,
    pub executor: Executor,
    /// 이 실행에서 처음 쓴 작업 번호 — 되돌리기는 이 뒤의 작업만
    pub session_start: i64,
    /// 스캔 중(대기 포함)인 폴더
    pub scanning: Mutex<Vec<String>>,
    /// 실행 직후 첫 스캔을 시작하기 전(제외 대상 정리 중)도 '스캔 중'으로 본다
    pub startup_pending: AtomicBool,
    pub watcher: Mutex<Option<Watcher>>,
    pub models: ModelManager,
    pub dup_cancel: Cancel,
    pub compare_cancel: Cancel,
    pub classify_cancel: Cancel,
    pub leftover_cancel: Cancel,
    pub classify_running: AtomicBool,
    pub last_dup_report: Mutex<Option<DuplicateReport>>,
    pub launch: LaunchArgs,
}

pub type Shared = Arc<AppState>;

/// 잠금이 독(poison)되어도 계속 쓴다 — 한 작업의 패닉이 앱 전체를 멈추지 않게
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
    pub fn new() -> Result<AppState> {
        let data_dir = paths::data_dir();
        std::fs::create_dir_all(&data_dir)?;
        let locale = std::env::var("MF_LOCALE")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(sys_locale::get_locale)
            .unwrap_or_else(|| "en-US".into());
        let lang = classify::ui_language(&locale);
        let store = Arc::new(IndexStore::open_in_or_reset(&data_dir)?);
        let rules = ScanRules::platform_default(&data_dir);
        let executor = Executor::new(store.clone(), ProtectedPaths::platform_default(&data_dir));
        let session_start = store.max_op_id()?;
        let models = ModelManager::new(&paths::models_dir(&data_dir));
        let args: Vec<String> = std::env::args().collect();
        Ok(AppState {
            settings: Mutex::new(settings::Settings::load(&data_dir)),
            locale,
            lang,
            version: env!("CARGO_PKG_VERSION").to_string(),
            scanner: Scanner::new(rules.clone()),
            rules,
            store,
            executor,
            session_start,
            scanning: Mutex::new(Vec::new()),
            startup_pending: AtomicBool::new(true),
            watcher: Mutex::new(None),
            models,
            dup_cancel: Cancel::new(),
            compare_cancel: Cancel::new(),
            classify_cancel: Cancel::new(),
            leftover_cancel: Cancel::new(),
            classify_running: AtomicBool::new(false),
            last_dup_report: Mutex::new(None),
            launch: LaunchArgs::parse(&args),
            data_dir,
        })
    }

    pub fn folders(&self) -> Vec<String> {
        lock(&self.settings).folders.clone()
    }

    pub fn view(&self) -> mf_core::Settings {
        lock(&self.settings).view(&self.locale, &self.data_dir, &self.version)
    }

    pub fn is_scanning(&self) -> bool {
        self.startup_pending.load(Ordering::SeqCst) || !lock(&self.scanning).is_empty()
    }

    /// 중복 찾기·분류 전에: 폴더를 훑는 중이면 끝날 때까지 기다린다 (반쯤 쌓인 인덱스로 답하지 않게)
    pub fn wait_for_scans(&self, cancel: &Cancel) -> Result<()> {
        while self.is_scanning() {
            if cancel.is_cancelled() {
                bail!(mf_core::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}

/// 스캔 목록에 올린다. 이미 스캔 중인 폴더는 뺀 나머지를 돌려준다.
fn mark_scanning(st: &AppState, roots: Vec<String>) -> Vec<String> {
    let style = PathStyle::native();
    let mut scanning = lock(&st.scanning);
    let mut fresh = Vec::new();
    for r in roots {
        if !scanning.iter().any(|s| style.eq(s, &r)) && !fresh.iter().any(|s: &String| style.eq(s, &r)) {
            scanning.push(r.clone());
            fresh.push(r);
        }
    }
    fresh
}

/// 표시된 폴더를 하나씩 스캔하고(폴더마다 scan-finished), 끝나면 목록에서 뺀다. 호출한 스레드에서 돈다.
fn scan_marked(app: &AppHandle, st: &AppState, roots: &[String]) {
    let style = PathStyle::native();
    for root in roots {
        let progress = |e: &ScanProgressEvent| {
            let _ = app.emit("scan-progress", e);
        };
        let finished = match st.scanner.scan(&st.store, std::slice::from_ref(root), &progress, &Cancel::new()) {
            Ok(outcome) => {
                let error = outcome.roots.into_iter().find_map(|r| r.error);
                ScanFinishedEvent { root: root.clone(), error }
            }
            Err(e) => ScanFinishedEvent { root: root.clone(), error: Some(format!("{e:#}")) },
        };
        lock(&st.scanning).retain(|s| !style.eq(s, root));
        let _ = app.emit("scan-finished", &finished);
    }
    let _ = app.emit("index-changed", IndexChangedEvent { paths: roots.to_vec() });
}

/// 백그라운드 스캔 (rescan 명령, 폴더 추가, 되돌리기로 돌아온 폴더)
pub fn spawn_scan(app: &AppHandle, st: &Shared, roots: Vec<String>) {
    let roots = mark_scanning(st, roots);
    if roots.is_empty() {
        return;
    }
    let app = app.clone();
    let st = st.clone();
    std::thread::spawn(move || scan_marked(&app, &st, &roots));
}

/// 폴더 목록이 바뀌면 감시를 새로 건다
pub fn restart_watcher(app: &AppHandle, st: &AppState) {
    let folders = st.folders();
    let mut slot = lock(&st.watcher);
    if let Some(old) = slot.take() {
        old.stop();
    }
    if folders.is_empty() {
        return;
    }
    let emitter = app.clone();
    match Watcher::start(
        st.store.clone(),
        st.rules.clone(),
        folders,
        watcher::DEFAULT_DEBOUNCE,
        move |paths: Vec<String>| {
            let _ = emitter.emit("index-changed", IndexChangedEvent { paths });
        },
    ) {
        Ok(w) => *slot = Some(w),
        Err(e) => eprintln!("watcher: {e:#}"),
    }
}

/// 미리보기(asset 프로토콜)는 사용자가 고른 폴더 안의 파일만 열 수 있게
pub fn allow_preview(app: &AppHandle, dir: &str) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        eprintln!("asset scope {dir}: {e}");
    }
}

/// 실행 직후: 예전에 들어온 제외 대상 정리 → 감시 시작 → 모든 폴더 스캔
pub fn startup(app: &AppHandle, st: &Shared) {
    for f in st.folders() {
        allow_preview(app, &f);
    }
    let app = app.clone();
    let st = st.clone();
    std::thread::spawn(move || {
        if let Err(e) = st.store.remove_excluded(&st.rules) {
            eprintln!("remove_excluded: {e:#}");
        }
        restart_watcher(&app, &st);
        let roots = mark_scanning(&st, st.folders());
        st.startup_pending.store(false, Ordering::SeqCst);
        scan_marked(&app, &st, &roots);
    });
}
