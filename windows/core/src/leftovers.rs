//! App Leftovers for Windows: data folders in `%APPDATA%` / `%LOCALAPPDATA%` that belong to
//! no installed app. Principle (from the macOS app): **when in doubt, leave it out** — only
//! clear-cut folders are reported, anything uncertain is not listed at all.
//!
//! A folder is reported only if ALL hold:
//! - its normalized name matches none of the installed-app tokens (both ways, min length 3);
//! - it is not on the protected list (OS, shared vendor and tool folders);
//! - nothing inside changed in the last 30 days;
//! - it contains no `.exe` / `.dll` (could be a portable app);
//! - it is not empty, could be read completely, and is not a link.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use crate::paths;
use crate::progress::Cancel;
use crate::record;
use crate::types::{LeftoverItem, LeftoverPath, LeftoverReport, ProgressEvent};

/// Reason: one folder, no installed app, unchanged 30+ days. (i18n key — Korean source text)
pub const REASON_NOT_INSTALLED: &str = "설치된 앱 목록에 없고 30일 넘게 바뀌지 않은 앱 데이터 폴더";
/// Reason: the same app left folders in more than one place (Roaming and Local).
pub const REASON_NOT_INSTALLED_MULTI: &str =
    "설치된 앱 목록에 없고 30일 넘게 바뀌지 않은 앱 데이터 — 여러 위치에 남아 있음";
/// Reason: an app folder inside a vendor folder.
pub const REASON_VENDOR_CHILD: &str =
    "제조사 폴더 안의 앱 데이터 — 같은 이름의 설치된 앱이 없고 30일 넘게 바뀌지 않음";

/// Every reason string this module emits (for the i18n file).
pub const REASONS: &[&str] = &[
    REASON_NOT_INSTALLED,
    REASON_NOT_INSTALLED_MULTI,
    REASON_VENDOR_CHILD,
];

/// Folder names never offered (case-insensitive, exact).
const PROTECTED_NAMES: &[&str] = &[
    "microsoft",
    "windows",
    "packages",
    "temp",
    "tmp",
    "programs",
    "comms",
    "connecteddevicesplatform",
    "d3dscache",
    "crashdumps",
    "crash reports",
    "crashpad",
    "publishers",
    "google",
    "mozilla",
    "firefox",
    "thunderbird",
    "apple",
    "apple computer",
    "adobe",
    "macromedia",
    "intel",
    "amd",
    "ati",
    "realtek",
    "pip",
    "npm",
    "npm-cache",
    "nuget",
    "yarn",
    "pnpm",
    "pnpm-cache",
    "pypoetry",
    "python",
    "uv",
    "package cache",
    "packagecache",
    "virtualstore",
    "history",
    "temporary internet files",
    "application data",
    "local settings",
    "peerdistrepub",
    "placeholdertilelogofolder",
    "iconcache",
    "fontcache",
    "squirreltemp",
    "sun",
    "oracle",
    "java",
    "jetbrains",
    "code",
    "cursor",
    "vscode",
    "docker",
    "steam",
    "epic",
    "multi-finder",
    "multifinder",
    "shelf",
    "claude",
    "figma",
    "notion",
    "slack",
    "discord",
    "kakaotalk",
    "telegram",
    "telegram desktop",
    "zoom",
    "onedrive",
    "identities",
    "media center programs",
    "isolatedstorage",
    "diagnostics",
    "elevateddiagnostics",
    "gamedvr",
    "microsoftedge",
    "windowsapps",
    "fontconfig",
    "gtk-3.0",
    "openssl",
    "gnupg",
    "ssh",
    "certificates",
    "cache",
    "caches",
    "logs",
    "crashes",
    "updates",
    "downloaded installations",
    "installer",
    "installshield",
    "installshield installation information",
];

/// Folder name prefixes never offered (lowercase).
const PROTECTED_PREFIXES: &[&str] = &[
    "microsoft",
    "windows",
    "nvidia",
    "intel",
    "amd",
    "realtek",
    "google",
    "mozilla",
    "apple",
    "adobe",
    "package cache",
    "msix",
    "visualstudio",
    "dotnet",
    ".",
];

/// Words too generic to identify an app.
const STOP_WORDS: &[&str] = &[
    "inc",
    "llc",
    "ltd",
    "corp",
    "corporation",
    "company",
    "software",
    "technologies",
    "technology",
    "systems",
    "team",
    "the",
    "and",
    "for",
    "update",
    "updater",
    "helper",
    "service",
    "installer",
    "setup",
    "runtime",
    "tools",
    "tool",
    "app",
    "apps",
    "application",
    "edition",
    "version",
    "beta",
    "preview",
    "free",
    "plus",
    "client",
    "desktop",
    "studio",
    "player",
    "manager",
    "driver",
    "drivers",
    "package",
    "redistributable",
    "microsoft",
    "windows",
    "x64",
    "x86",
    "bin",
    "program",
    "files",
    "programs",
    "local",
    "roaming",
    "users",
    "appdata",
    "data",
    "user",
    "common",
];

/// Where installed-app names come from.
pub trait InstalledApps {
    /// Raw names (display names, publishers, folder names, exe stems, process names …).
    fn names(&self) -> Vec<String>;
}

/// Fixed list (tests, other platforms).
#[derive(Clone, Debug, Default)]
pub struct FakeApps(pub Vec<String>);

impl InstalledApps for FakeApps {
    fn names(&self) -> Vec<String> {
        self.0.clone()
    }
}

/// Portable sources: program folders, Start Menu shortcuts, MSIX package folders and the
/// names of running processes.
#[derive(Clone, Debug, Default)]
pub struct FolderApps {
    /// folders whose children (and grandchildren) are app folders (`Program Files`, `Programs`)
    pub program_dirs: Vec<PathBuf>,
    /// Start Menu `Programs` folders (shortcut names, 3 levels)
    pub start_menus: Vec<PathBuf>,
    /// `%LOCALAPPDATA%\Packages` (installed MSIX/Store apps)
    pub packages_dir: Option<PathBuf>,
    pub include_processes: bool,
}

impl FolderApps {
    /// The Windows locations, from the environment.
    pub fn windows_default() -> FolderApps {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let mut program_dirs = Vec::new();
        for k in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(p) = env(k) {
                if !program_dirs.contains(&p) {
                    program_dirs.push(p);
                }
            }
        }
        let local = env("LOCALAPPDATA");
        if let Some(l) = &local {
            program_dirs.push(l.join("Programs"));
        }
        let mut start_menus = Vec::new();
        if let Some(a) = env("APPDATA") {
            start_menus.push(a.join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        if let Some(pd) = env("ProgramData") {
            start_menus.push(pd.join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        FolderApps {
            program_dirs,
            start_menus,
            packages_dir: local.map(|l| l.join("Packages")),
            include_processes: true,
        }
    }
}

fn child_names(dir: &Path) -> Vec<(String, bool)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some((name, is_dir))
        })
        .collect()
}

impl InstalledApps for FolderApps {
    fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for d in &self.program_dirs {
            for (n, is_dir) in child_names(d) {
                if is_dir {
                    for (inner, inner_dir) in child_names(&d.join(&n)) {
                        if inner_dir {
                            out.push(inner);
                        } else if let Some(stem) = inner
                            .strip_suffix(".exe")
                            .or_else(|| inner.strip_suffix(".EXE"))
                        {
                            out.push(stem.to_string());
                        }
                    }
                    out.push(n);
                }
            }
        }
        for sm in &self.start_menus {
            for e in WalkDir::new(sm).max_depth(3).into_iter().flatten() {
                if let Some(n) = e.file_name().to_str() {
                    let stem = n
                        .strip_suffix(".lnk")
                        .or_else(|| n.strip_suffix(".url"))
                        .unwrap_or(n);
                    if e.depth() > 0 {
                        out.push(stem.to_string());
                    }
                }
            }
        }
        if let Some(p) = &self.packages_dir {
            for (n, is_dir) in child_names(p) {
                if !is_dir {
                    continue;
                }
                // "SpotifyAB.SpotifyMusic_zpdnekdrzrea0" → SpotifyAB, SpotifyMusic
                let family = n.split('_').next().unwrap_or(&n).to_string();
                out.extend(family.split('.').map(str::to_string));
                out.push(family);
            }
        }
        if self.include_processes {
            out.extend(running_process_names());
        }
        out
    }
}

/// Names of running processes (without `.exe`) and the folders their executables live in.
pub fn running_process_names() -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let Ok(mut sys) = System::new() else {
        return vec![];
    };
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut out = Vec::new();
    for p in sys.processes().values() {
        if let Some(n) = p.name().to_str() {
            out.push(
                n.strip_suffix(".exe")
                    .or_else(|| n.strip_suffix(".EXE"))
                    .unwrap_or(n)
                    .to_string(),
            );
        }
        if let Some(parent) = p
            .exe()
            .and_then(|e| e.parent())
            .and_then(|d| d.file_name())
            .and_then(|d| d.to_str())
        {
            out.push(parent.to_string());
        }
    }
    out
}

/// Windows: registry Uninstall keys (HKLM 64/32-bit, HKCU) + [`FolderApps::windows_default`].
#[cfg(windows)]
#[derive(Clone, Debug, Default)]
pub struct WindowsApps;

#[cfg(windows)]
impl InstalledApps for WindowsApps {
    fn names(&self) -> Vec<String> {
        let mut out = registry_names();
        out.extend(FolderApps::windows_default().names());
        out
    }
}

#[cfg(windows)]
fn registry_names() -> Vec<String> {
    use winreg::enums::{KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::{RegKey, HKCU, HKLM};
    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let sources: [(&RegKey, u32); 3] = [
        (HKLM, KEY_READ | KEY_WOW64_64KEY),
        (HKLM, KEY_READ | KEY_WOW64_32KEY),
        (HKCU, KEY_READ),
    ];
    let mut out = Vec::new();
    for (root, flags) in sources {
        let Ok(key) = root.open_subkey_with_flags(UNINSTALL, flags) else {
            continue;
        };
        for sub in key.enum_keys().flatten() {
            let Ok(k) = key.open_subkey_with_flags(&sub, flags) else {
                continue;
            };
            out.push(sub.clone());
            for v in ["DisplayName", "Publisher"] {
                if let Ok(s) = k.get_value::<String, _>(v) {
                    out.push(s);
                }
            }
            if let Ok(loc) = k.get_value::<String, _>("InstallLocation") {
                let t = loc.trim().trim_matches('"').trim_end_matches('\\');
                if let Some(leaf) = t.rsplit('\\').next() {
                    out.push(leaf.to_string());
                }
            }
            for v in ["DisplayIcon", "UninstallString"] {
                if let Ok(s) = k.get_value::<String, _>(v) {
                    out.extend(exe_path_names(&s));
                }
            }
        }
    }
    out
}

/// Useful names in a command line / icon path: the exe stem and the folder it sits in
/// (`"C:\Program Files\Zoom\bin\Zoom.exe",0` → Zoom, bin).
pub fn exe_path_names(s: &str) -> Vec<String> {
    let t = s.trim().trim_start_matches('"');
    let lower = t.to_lowercase();
    let Some(i) = lower.find(".exe") else {
        return vec![];
    };
    let path = &t[..i + 4];
    let mut out = Vec::new();
    let parts: Vec<&str> = path.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    if let Some(last) = parts.last() {
        out.push(last[..last.len() - 4].to_string());
    }
    if parts.len() >= 2 {
        out.push(parts[parts.len() - 2].to_string());
    }
    out
}

/// The installed-app source for this platform.
pub fn system_apps() -> Box<dyn InstalledApps> {
    #[cfg(windows)]
    {
        Box::new(WindowsApps)
    }
    #[cfg(not(windows))]
    {
        Box::new(FolderApps {
            include_processes: true,
            ..Default::default()
        })
    }
}

/// One place to look (`%APPDATA%` …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftoverArea {
    /// shown in `scannedAreas`, e.g. "%APPDATA%"
    pub label: String,
    pub path: PathBuf,
}

/// `%APPDATA%` and `%LOCALAPPDATA%` (empty list on other platforms).
pub fn default_areas() -> Vec<LeftoverArea> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for (label, var) in [("%APPDATA%", "APPDATA"), ("%LOCALAPPDATA%", "LOCALAPPDATA")] {
            if let Some(p) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                out.push(LeftoverArea {
                    label: label.into(),
                    path: PathBuf::from(p),
                });
            }
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct LeftoverOptions {
    /// newest file inside must be older than this
    pub min_age_days: u32,
    /// folders with more entries than this are not judged (uncertain)
    pub max_entries: usize,
    /// more folder names to leave alone (e.g. the app's own WebView2 data folder)
    pub extra_protected: Vec<String>,
    /// "now" in ms (tests)
    pub now_ms: Option<i64>,
}

impl Default for LeftoverOptions {
    fn default() -> Self {
        LeftoverOptions {
            min_age_days: 30,
            max_entries: 200_000,
            extra_protected: vec![],
            now_ms: None,
        }
    }
}

/// Lowercase, NFC, drop version numbers / architecture words / punctuation and spaces.
pub fn normalize_name(s: &str) -> String {
    words(s).concat()
}

fn words(s: &str) -> Vec<String> {
    let lower = paths::fold(s);
    let mut out = Vec::new();
    for raw in lower.split(|c: char| {
        c.is_whitespace() || matches!(c, '_' | '(' | ')' | '[' | ']' | ',' | '/' | '\\' | '+')
    }) {
        let w = raw.trim_matches(|c: char| !c.is_alphanumeric());
        if w.is_empty() || is_version(w) || is_arch(w) {
            continue;
        }
        let cleaned: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
        if !cleaned.is_empty() {
            out.push(cleaned);
        }
    }
    out
}

fn is_version(w: &str) -> bool {
    let t = w.strip_prefix('v').unwrap_or(w);
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        && t.chars().any(|c| c.is_ascii_digit())
}

fn is_arch(w: &str) -> bool {
    matches!(
        w,
        "x64"
            | "x86"
            | "x86-64"
            | "amd64"
            | "arm64"
            | "64-bit"
            | "32-bit"
            | "64bit"
            | "32bit"
            | "win64"
            | "win32"
    )
}

/// Matching tokens from raw app names: the whole normalized name (≥ 3 chars) and its
/// distinctive words (≥ 4 chars, not generic).
pub fn tokens_for(names: &[String]) -> Vec<String> {
    let mut set = HashSet::new();
    for n in names {
        let ws = words(n);
        let full = ws.concat();
        if full.chars().count() >= 3 {
            set.insert(full);
        }
        for w in ws {
            if w.chars().count() >= 4 && !STOP_WORDS.contains(&w.as_str()) {
                set.insert(w);
            }
        }
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

/// Does a normalized folder name match an installed-app token (either contains the other)?
pub fn matches_installed(folder_norm: &str, tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|t| folder_norm.contains(t.as_str()) || t.contains(folder_norm))
}

/// Protected folder name (OS, shared vendor and tool folders).
pub fn is_protected_name(name: &str, extra: &[String]) -> bool {
    let l = name.trim().to_lowercase();
    PROTECTED_NAMES.contains(&l.as_str())
        || PROTECTED_PREFIXES.iter().any(|p| l.starts_with(p))
        || extra.iter().any(|e| e.to_lowercase() == l)
}

/// What is inside a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Inside {
    bytes: u64,
    newest_ms: i64,
    has_binary: bool,
    incomplete: bool,
}

fn inspect(dir: &Path, max_entries: usize, cancel: &Cancel) -> Inside {
    let mut r = Inside::default();
    if let Ok(m) = std::fs::symlink_metadata(dir) {
        r.newest_ms = m.modified().map(paths::system_time_ms).unwrap_or(0);
    }
    for (count, item) in WalkDir::new(dir)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        if count >= max_entries || cancel.is_cancelled() {
            r.incomplete = true;
            break;
        }
        let Ok(e) = item else {
            r.incomplete = true;
            break;
        };
        let Ok(m) = e.metadata() else {
            r.incomplete = true;
            break;
        };
        r.newest_ms = r
            .newest_ms
            .max(m.modified().map(paths::system_time_ms).unwrap_or(0));
        if m.is_file() {
            r.bytes += m.len();
            let ext = record::extension_of(&e.file_name().to_string_lossy());
            if ext == "exe" || ext == "dll" {
                r.has_binary = true;
            }
        }
    }
    r
}

struct Candidate {
    key: String,
    app: String,
    path: PathBuf,
    inside: Inside,
    vendor_child: bool,
}

/// Should `path` (a folder named `name`) be judged at all?
fn eligible_name(name: &str, tokens: &[String], opts: &LeftoverOptions) -> bool {
    if is_protected_name(name, &opts.extra_protected) {
        return false;
    }
    let norm = normalize_name(name);
    if norm.chars().count() < 3 {
        return false; // too short to tell
    }
    !matches_installed(&norm, tokens)
}

fn is_plain_dir(p: &Path) -> bool {
    match std::fs::symlink_metadata(p) {
        Ok(m) => {
            m.is_dir()
                && !m.file_type().is_symlink()
                && !record::is_hidden_attrs(record::file_attributes(&m))
        }
        Err(_) => false,
    }
}

fn clear_cut(i: &Inside, cutoff_ms: i64) -> bool {
    !i.incomplete && !i.has_binary && i.bytes > 0 && i.newest_ms < cutoff_ms
}

/// `scan_leftovers`. Progress: phase "scan", done/total = folders looked at.
pub fn scan_leftovers(
    areas: &[LeftoverArea],
    apps: &dyn InstalledApps,
    opts: &LeftoverOptions,
    progress: &dyn Fn(&ProgressEvent),
    cancel: &Cancel,
) -> Result<LeftoverReport> {
    progress(&ProgressEvent::new("scan", 0, 0, None));
    let tokens = tokens_for(&apps.names());
    let now = opts.now_ms.unwrap_or_else(paths::now_ms);
    let cutoff = now - i64::from(opts.min_age_days) * 86_400_000;

    let mut tops: Vec<(PathBuf, String)> = Vec::new();
    for a in areas {
        for (name, is_dir) in child_names(&a.path) {
            if is_dir {
                tops.push((a.path.join(&name), name));
            }
        }
    }
    let total = tops.len() as u64;
    let mut cands: Vec<Candidate> = Vec::new();
    for (i, (path, name)) in tops.iter().enumerate() {
        cancel.check()?;
        progress(&ProgressEvent::new(
            "scan",
            i as u64,
            total,
            Some(name.clone()),
        ));
        if !is_plain_dir(path) || !eligible_name(name, &tokens, opts) {
            continue;
        }
        let children = child_names(path);
        // A vendor folder (only sub-folders) where one app is still installed: judge the other
        // app folders one by one. Otherwise the folder is judged as a whole.
        let only_dirs = !children.is_empty() && children.iter().all(|(_, d)| *d);
        let live_child = only_dirs
            && children.iter().any(|(c, _)| {
                let n = normalize_name(c);
                n.chars().count() >= 3 && matches_installed(&n, &tokens)
            });
        if live_child {
            for (cname, _) in &children {
                let cp = path.join(cname);
                if !is_plain_dir(&cp) || !eligible_name(cname, &tokens, opts) {
                    continue;
                }
                let inside = inspect(&cp, opts.max_entries, cancel);
                if clear_cut(&inside, cutoff) {
                    cands.push(Candidate {
                        key: format!("{}/{}", normalize_name(name), normalize_name(cname)),
                        app: format!("{name} {cname}"),
                        path: cp,
                        inside,
                        vendor_child: true,
                    });
                }
            }
        } else {
            let inside = inspect(path, opts.max_entries, cancel);
            if clear_cut(&inside, cutoff) {
                cands.push(Candidate {
                    key: normalize_name(name),
                    app: name.clone(),
                    path: path.clone(),
                    inside,
                    vendor_child: false,
                });
            }
        }
    }
    cancel.check()?;
    progress(&ProgressEvent::new("scan", total, total, None));

    // one item per app (Roaming + Local folders of the same app together)
    let mut by_app: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    for c in cands {
        by_app.entry(c.key.clone()).or_default().push(c);
    }
    let mut items: Vec<LeftoverItem> = by_app
        .into_iter()
        .map(|(key, list)| {
            let reason = if list.len() > 1 {
                REASON_NOT_INSTALLED_MULTI
            } else if list[0].vendor_child {
                REASON_VENDOR_CHILD
            } else {
                REASON_NOT_INSTALLED
            };
            let paths: Vec<LeftoverPath> = list
                .iter()
                .map(|c| LeftoverPath {
                    path: paths::normalize(&c.path.to_string_lossy()),
                    bytes: c.inside.bytes,
                    last_modified: c.inside.newest_ms,
                })
                .collect();
            LeftoverItem {
                id: format!("leftover:{key}"),
                app: list[0].app.clone(),
                bytes: paths.iter().map(|p| p.bytes).sum(),
                paths,
                reason: reason.to_string(),
            }
        })
        .collect();
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.app.cmp(&b.app)));
    Ok(LeftoverReport {
        items,
        scanned_areas: areas.iter().map(|a| a.label.clone()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::no_progress;
    use filetime::FileTime;

    #[test]
    fn normalization_and_tokens() {
        assert_eq!(normalize_name("7-Zip 23.01 (x64)"), "7zip");
        assert_eq!(normalize_name("Python 3.11.4 (64-bit)"), "python");
        assert_eq!(normalize_name("Notepad++"), "notepad");
        assert_eq!(normalize_name("Zoom"), "zoom");
        assert_eq!(normalize_name("Visual Studio 2019 v16.11"), "visualstudio");
        let t = tokens_for(&[
            "Zoom Video Communications, Inc.".into(),
            "Hancom Office 2022".into(),
            "ab".into(),
        ]);
        assert!(t.contains(&"zoomvideocommunicationsinc".to_string()));
        assert!(t.contains(&"hancom".to_string()));
        assert!(!t.contains(&"inc".to_string()));
        assert!(!t.contains(&"ab".to_string()));
        assert!(matches_installed("zoom", &t));
        assert!(matches_installed("hancomoffice", &t));
        assert!(!matches_installed("oldtool", &t));
    }

    #[test]
    fn protected_names() {
        for n in [
            "Microsoft",
            "Packages",
            "Temp",
            "NVIDIA Corporation",
            "Google",
            "npm-cache",
            "Package Cache",
            "Programs",
            ".config",
        ] {
            assert!(is_protected_name(n, &[]), "{n}");
        }
        assert!(!is_protected_name("OldTool", &[]));
        assert!(is_protected_name(
            "com.uteed.multifinder",
            &["com.uteed.multifinder".into()]
        ));
    }

    #[test]
    fn exe_paths() {
        assert_eq!(
            exe_path_names(r#""C:\Program Files\Zoom\bin\Zoom.exe",0"#),
            vec!["Zoom", "bin"]
        );
        assert_eq!(exe_path_names(r"MsiExec.exe /X{123}"), vec!["MsiExec"]);
        assert!(exe_path_names("no exe here").is_empty());
    }

    fn age(path: &Path, days: i64) {
        let t =
            std::time::SystemTime::now() - std::time::Duration::from_secs((days * 86_400) as u64);
        let ft = FileTime::from_system_time(t);
        for e in WalkDir::new(path)
            .contents_first(true)
            .into_iter()
            .flatten()
        {
            let _ = filetime::set_file_times(e.path(), ft, ft);
        }
    }

    fn mk(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn only_clear_cut_folders_are_reported() {
        let base = tempfile::Builder::new()
            .prefix("mf-left")
            .tempdir()
            .unwrap();
        let roaming = base.path().join("Roaming");
        let local = base.path().join("Local");
        mk(&roaming, "OldTool/settings.json", "{}"); // leftover (in both areas)
        mk(&local, "OldTool/cache/blob.bin", "xxxx");
        mk(&roaming, "Zoom/data.db", "z"); // installed
        mk(&roaming, "FreshApp/state.json", "1"); // recently used
        mk(&roaming, "PortableThing/tool.exe", "MZ"); // has an exe
        mk(&roaming, "Microsoft/Windows/x.dat", "m"); // protected
        mk(&roaming, "EmptyApp/.keep", ""); // zero bytes
        mk(&local, "Acme/Widget/w.cfg", "w"); // vendor folder, all apps gone → whole vendor
        mk(&local, "Contoso/Gone/g.cfg", "g"); // vendor folder with one installed child
        mk(&local, "Contoso/LiveApp/l.cfg", "l");
        mk(&local, "ab/x.txt", "x"); // name too short to judge
        for p in [&roaming, &local] {
            age(p, 90);
        }
        // FreshApp was touched yesterday
        let fresh = roaming.join("FreshApp/state.json");
        let y = FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(86_400),
        );
        filetime::set_file_times(&fresh, y, y).unwrap();

        let apps = FakeApps(vec![
            "Zoom Video Communications, Inc.".into(),
            "LiveApp".into(),
        ]);
        let areas = vec![
            LeftoverArea {
                label: "%APPDATA%".into(),
                path: roaming.clone(),
            },
            LeftoverArea {
                label: "%LOCALAPPDATA%".into(),
                path: local.clone(),
            },
        ];
        let rep = scan_leftovers(
            &areas,
            &apps,
            &LeftoverOptions::default(),
            &no_progress,
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(
            rep.scanned_areas,
            vec!["%APPDATA%".to_string(), "%LOCALAPPDATA%".to_string()]
        );
        let by_app: BTreeMap<String, &LeftoverItem> =
            rep.items.iter().map(|i| (i.app.clone(), i)).collect();
        let names: Vec<&String> = by_app.keys().collect();
        assert_eq!(
            names,
            ["Acme", "Contoso Gone", "OldTool"],
            "{:#?}",
            rep.items
        );
        let old = by_app["OldTool"];
        assert_eq!(old.paths.len(), 2);
        assert_eq!(old.bytes, 6);
        assert_eq!(old.reason, REASON_NOT_INSTALLED_MULTI);
        assert_eq!(by_app["Acme"].reason, REASON_NOT_INSTALLED);
        assert_eq!(by_app["Contoso Gone"].reason, REASON_VENDOR_CHILD);
        assert!(by_app["Contoso Gone"].paths[0].path.ends_with("Gone"));
        assert!(REASONS.iter().all(|r| !r.is_empty()));
    }
}
