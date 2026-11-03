//! What never enters the index ([`ScanRules`]) and what the executor never touches
//! ([`ProtectedPaths`]). Pure string logic so Windows rules are testable on Linux.

use std::path::Path;

use crate::paths::{self, PathStyle};

/// Directory names excluded anywhere (spec list).
const WINDOWS_EXCLUDED_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "__pycache__",
    ".venv",
    "venv",
    ".cache",
    ".npm",
    ".nvm",
    "$RECYCLE.BIN",
    "System Volume Information",
];

/// The macOS list, kept for non-Windows builds so behavior stays comparable.
const UNIX_EXCLUDED_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    ".build",
    "DerivedData",
    "Pods",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".npm",
    ".nvm",
    ".Trash",
];

/// Well-known Windows folders, read from the environment (with the usual defaults).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsDirs {
    pub system_root: String,
    pub program_files: Vec<String>,
    pub program_data: String,
    pub user_profile: String,
    pub appdata: String,
    pub local_appdata: String,
    /// `%OneDrive%`, `%OneDriveConsumer%`, `%OneDriveCommercial%` when set
    pub onedrive: Vec<String>,
}

impl WindowsDirs {
    pub fn from_env() -> WindowsDirs {
        let var = |k: &str| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| paths::normalize(&v))
        };
        let style = PathStyle::WINDOWS;
        let drive = var("SystemDrive").unwrap_or_else(|| "C:".into());
        let drive = drive.trim_end_matches('\\').to_string();
        let user_profile = var("USERPROFILE").unwrap_or_else(|| format!(r"{drive}\Users\Default"));
        let mut program_files = Vec::new();
        for (k, default) in [
            ("ProgramFiles", format!(r"{drive}\Program Files")),
            ("ProgramFiles(x86)", format!(r"{drive}\Program Files (x86)")),
            ("ProgramW6432", format!(r"{drive}\Program Files")),
        ] {
            let v = var(k).unwrap_or(default);
            if !program_files.iter().any(|p: &String| style.eq(p, &v)) {
                program_files.push(v);
            }
        }
        let mut onedrive = Vec::new();
        for k in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(v) = var(k) {
                if !onedrive.iter().any(|p: &String| style.eq(p, &v)) {
                    onedrive.push(v);
                }
            }
        }
        WindowsDirs {
            system_root: var("SystemRoot").unwrap_or_else(|| format!(r"{drive}\Windows")),
            program_files,
            program_data: var("ProgramData").unwrap_or_else(|| format!(r"{drive}\ProgramData")),
            appdata: var("APPDATA")
                .unwrap_or_else(|| style.join(&user_profile, r"AppData\Roaming")),
            local_appdata: var("LOCALAPPDATA")
                .unwrap_or_else(|| style.join(&user_profile, r"AppData\Local")),
            user_profile,
            onedrive,
        }
    }
}

/// What not to put into the index. Principle (from the macOS app): look everywhere the user
/// asked, but leave out what the user cannot or should not touch (system areas, caches,
/// build output, the Recycle Bin).
#[derive(Clone, Debug)]
pub struct ScanRules {
    /// Skip names starting with "." (and, in the scanner, Windows HIDDEN/SYSTEM entries).
    pub skip_hidden: bool,
    /// This path and everything below it is excluded.
    pub excluded_prefixes: Vec<String>,
    /// Directory names excluded anywhere (stored as comparison keys).
    excluded_names: Vec<String>,
    pub style: PathStyle,
}

impl ScanRules {
    pub fn new<P: AsRef<str>, N: AsRef<str>>(
        skip_hidden: bool,
        prefixes: &[P],
        names: &[N],
        style: PathStyle,
    ) -> ScanRules {
        ScanRules {
            skip_hidden,
            excluded_prefixes: prefixes
                .iter()
                .map(|p| style.trim(p.as_ref()))
                .filter(|p| !p.is_empty())
                .collect(),
            excluded_names: names.iter().map(|n| style.key(n.as_ref())).collect(),
            style,
        }
    }

    /// Nothing excluded (tests).
    pub fn open(style: PathStyle) -> ScanRules {
        ScanRules::new::<&str, &str>(false, &[], &[], style)
    }

    /// Windows rules from the spec.
    pub fn windows(dirs: &WindowsDirs, data_dir: &str) -> ScanRules {
        let style = PathStyle::WINDOWS;
        let mut prefixes = vec![dirs.system_root.clone()];
        prefixes.extend(dirs.program_files.iter().cloned());
        prefixes.push(dirs.program_data.clone());
        prefixes.push(style.join(&dirs.user_profile, "AppData"));
        // %APPDATA%/%LOCALAPPDATA% can be redirected outside the profile
        prefixes.push(dirs.appdata.clone());
        prefixes.push(dirs.local_appdata.clone());
        if !data_dir.is_empty() {
            prefixes.push(data_dir.to_string());
        }
        ScanRules::new(true, &prefixes, WINDOWS_EXCLUDED_NAMES, style)
    }

    /// macOS/Linux rules (the macOS list plus Linux system dirs and the freedesktop Trash).
    pub fn unix(home: &str, data_dir: &str) -> ScanRules {
        let style = PathStyle::UNIX;
        let mut prefixes: Vec<String> = [
            "/System", "/usr", "/bin", "/sbin", "/dev", "/private", "/cores", "/opt", "/Library",
            "/proc", "/sys",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if !home.is_empty() {
            for sub in [
                "Library/Caches",
                "Library/Containers",
                "Library/Group Containers",
                "Library/Developer",
                "Library/Logs",
                "Library/Metadata",
                ".Trash",
                ".local/share/Trash",
            ] {
                prefixes.push(style.join(home, sub));
            }
        }
        if !data_dir.is_empty() {
            prefixes.push(data_dir.to_string());
        }
        ScanRules::new(true, &prefixes, UNIX_EXCLUDED_NAMES, style)
    }

    /// Rules for the platform we run on.
    pub fn platform_default(data_dir: &Path) -> ScanRules {
        let data = data_dir.to_str().map(paths::normalize).unwrap_or_default();
        if cfg!(windows) {
            ScanRules::windows(&WindowsDirs::from_env(), &data)
        } else {
            ScanRules::unix(&paths::home_string(), &data)
        }
    }

    pub fn is_excluded_name(&self, name: &str) -> bool {
        let k = self.style.key(name);
        self.excluded_names.contains(&k)
    }

    /// Should `path` be skipped? Folders take everything below with them. Checks every
    /// ancestor name too, because the watcher hands in arbitrary deep paths
    /// (`…\node_modules\x\y.js`, `…\$RECYCLE.BIN\…`).
    pub fn is_excluded(&self, path: &str, name: &str) -> bool {
        if self.is_excluded_name(name) {
            return true;
        }
        if self
            .excluded_prefixes
            .iter()
            .any(|p| self.style.is_same_or_under(path, p))
        {
            return true;
        }
        for c in self.style.components(path) {
            if self.is_excluded_name(c) || (self.skip_hidden && c.starts_with('.')) {
                return true;
            }
        }
        false
    }
}

/// Paths the executor refuses to move or trash (they stay visible in the index).
#[derive(Clone, Debug)]
pub struct ProtectedPaths {
    pub prefixes: Vec<String>,
    /// Exact folders that may not be moved/trashed themselves (their contents may).
    pub roots: Vec<String>,
    pub style: PathStyle,
}

impl ProtectedPaths {
    pub fn new<S: AsRef<str>>(prefixes: &[S], roots: &[S], style: PathStyle) -> ProtectedPaths {
        ProtectedPaths {
            prefixes: prefixes
                .iter()
                .map(|p| style.trim(p.as_ref()))
                .filter(|p| !p.is_empty())
                .collect(),
            roots: roots
                .iter()
                .map(|p| style.trim(p.as_ref()))
                .filter(|p| !p.is_empty())
                .collect(),
            style,
        }
    }

    /// Nothing protected except file-system roots (tests).
    pub fn none(style: PathStyle) -> ProtectedPaths {
        ProtectedPaths::new::<&str>(&[], &[], style)
    }

    pub fn windows(dirs: &WindowsDirs, data_dir: &str) -> ProtectedPaths {
        let style = PathStyle::WINDOWS;
        let mut prefixes = vec![dirs.system_root.clone()];
        prefixes.extend(dirs.program_files.iter().cloned());
        prefixes.push(dirs.program_data.clone());
        // OS-owned data inside the profile (App Leftovers never offers these either)
        prefixes.push(style.join(&dirs.appdata, "Microsoft"));
        prefixes.push(style.join(&dirs.local_appdata, "Microsoft"));
        prefixes.push(style.join(&dirs.local_appdata, "Packages"));
        if !data_dir.is_empty() {
            prefixes.push(data_dir.to_string());
        }
        let up = &dirs.user_profile;
        let mut roots = vec![up.clone(), dirs.appdata.clone(), dirs.local_appdata.clone()];
        for sub in [
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Music",
            "Videos",
            "OneDrive",
            "AppData",
            r"AppData\Local",
            r"AppData\LocalLow",
            r"AppData\Roaming",
            "Favorites",
            "Contacts",
            "Links",
            "Saved Games",
            "Searches",
            "3D Objects",
        ] {
            roots.push(style.join(up, sub));
        }
        if let Some(users) = style.parent(up) {
            roots.push(users.to_string());
            roots.push(style.join(users, "Public"));
        }
        for od in &dirs.onedrive {
            roots.push(od.clone());
            for sub in ["Desktop", "Documents", "Pictures"] {
                roots.push(style.join(od, sub));
            }
        }
        ProtectedPaths::new(&prefixes, &roots, style)
    }

    pub fn unix(home: &str, data_dir: &str) -> ProtectedPaths {
        let style = PathStyle::UNIX;
        let mut prefixes: Vec<String> = [
            "/System",
            "/usr",
            "/bin",
            "/sbin",
            "/dev",
            "/private",
            "/Library",
            "/Applications",
            "/etc",
            "/proc",
            "/sys",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut roots = Vec::new();
        if !home.is_empty() {
            prefixes.push(style.join(home, "Library/Mobile Documents")); // iCloud Drive
            roots.push(home.to_string());
            for sub in [
                "Desktop",
                "Documents",
                "Downloads",
                "Library",
                "Pictures",
                "Movies",
                "Music",
                "Public",
                "Videos",
            ] {
                roots.push(style.join(home, sub));
            }
        }
        if !data_dir.is_empty() {
            prefixes.push(data_dir.to_string());
        }
        ProtectedPaths::new(&prefixes, &roots, style)
    }

    pub fn platform_default(data_dir: &Path) -> ProtectedPaths {
        let data = data_dir.to_str().map(paths::normalize).unwrap_or_default();
        if cfg!(windows) {
            ProtectedPaths::windows(&WindowsDirs::from_env(), &data)
        } else {
            ProtectedPaths::unix(&paths::home_string(), &data)
        }
    }

    /// Inside one of the protected areas (used for move destinations; roots such as the
    /// Desktop itself are fine destinations).
    pub fn is_in_protected_area(&self, path: &str) -> bool {
        let p = self.style.trim(path);
        self.prefixes
            .iter()
            .any(|pre| self.style.is_same_or_under(&p, pre))
    }

    pub fn is_protected(&self, path: &str) -> bool {
        let p = self.style.trim(path);
        if self.style.is_root(&p) || p.is_empty() {
            return true;
        }
        if self.roots.iter().any(|r| self.style.eq(r, &p)) {
            return true;
        }
        self.prefixes
            .iter()
            .any(|pre| self.style.is_same_or_under(&p, pre))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win_dirs() -> WindowsDirs {
        WindowsDirs {
            system_root: r"C:\Windows".into(),
            program_files: vec![r"C:\Program Files".into(), r"C:\Program Files (x86)".into()],
            program_data: r"C:\ProgramData".into(),
            user_profile: r"C:\Users\me".into(),
            appdata: r"C:\Users\me\AppData\Roaming".into(),
            local_appdata: r"C:\Users\me\AppData\Local".into(),
            onedrive: vec![r"C:\Users\me\OneDrive".into()],
        }
    }

    #[test]
    fn windows_exclusions_are_case_insensitive_and_look_at_ancestors() {
        let r = ScanRules::windows(&win_dirs(), r"C:\Users\me\AppData\Local\Multi-Finder");
        let ex = |p: &str| r.is_excluded(p, PathStyle::WINDOWS.file_name(p));
        assert!(ex(r"C:\WINDOWS\System32\x.dll"));
        assert!(ex(r"c:\program files\App\a.exe"));
        assert!(ex(r"C:\Program Files (x86)\App"));
        assert!(ex(r"C:\ProgramData\x"));
        assert!(ex(r"C:\Users\me\AppData\Roaming\Zoom\a.txt"));
        assert!(ex(r"D:\$Recycle.Bin\S-1-5\$R1.txt"));
        assert!(ex(r"E:\System Volume Information\x"));
        assert!(ex(r"C:\Users\me\dev\app\node_modules\x\y.js"));
        assert!(ex(r"C:\Users\me\dev\app\.git\HEAD"));
        assert!(ex(r"C:\Users\me\Downloads\.hidden\a.txt"));
        assert!(!ex(r"C:\Users\me\Downloads\a.mp4"));
        assert!(!ex(r"C:\Users\me\Desktop\한글폴더\x.pdf"));
        assert!(!ex(r"C:\Program Files Backup\x.txt"));
        assert!(!ex(r"D:\Windows Stuff\x.txt"));
    }

    #[test]
    fn unix_exclusions_match_the_macos_tests() {
        let r = ScanRules::unix("/Users/me", "");
        assert!(r.is_excluded("/Users/me/.Trash/a.mp4", "a.mp4"));
        assert!(r.is_excluded("/Users/me/dev/app/node_modules/x/y.js", "y.js"));
        assert!(r.is_excluded("/Users/me/Library/Caches/foo", "foo"));
        assert!(!r.is_excluded("/Users/me/Downloads/a.mp4", "a.mp4"));
        assert!(!r.is_excluded("/Users/me/Desktop/한글폴더/x.pdf", "x.pdf"));
        // case-sensitive on unix
        assert!(!r.is_excluded("/Users/me/NODE_MODULES/x", "x"));
    }

    #[test]
    fn protected_paths_windows() {
        let p = ProtectedPaths::windows(&win_dirs(), r"C:\Users\me\AppData\Local\Multi-Finder");
        assert!(p.is_protected(r"C:\"));
        assert!(p.is_protected(r"D:\"));
        assert!(p.is_protected(r"c:\users\me\desktop"));
        assert!(p.is_protected(r"C:\Users\me\Desktop\"));
        assert!(!p.is_protected(r"C:\Users\me\Desktop\some.txt"));
        assert!(p.is_protected(r"C:\Windows\notepad.exe"));
        assert!(p.is_protected(r"C:\Users\me\AppData\Local\Multi-Finder\index.sqlite"));
        assert!(p.is_protected(r"C:\Users\me\AppData\Local\Packages\X"));
        assert!(p.is_protected(r"C:\Users\me\AppData\Roaming"));
        assert!(!p.is_protected(r"C:\Users\me\AppData\Roaming\OldApp"));
        assert!(p.is_protected(r"C:\Users\me\OneDrive\Documents"));
        assert!(!p.is_protected(r"C:\Users\me\OneDrive\Documents\a.docx"));
        assert!(p.is_protected(r"C:\Users"));
    }

    #[test]
    fn protected_paths_unix() {
        let p = ProtectedPaths::unix("/home/me", "/home/me/.local/share/multi-finder");
        assert!(p.is_protected("/home/me/Desktop"));
        assert!(!p.is_protected("/home/me/Desktop/some.txt"));
        assert!(p.is_protected("/home/me/Library/Mobile Documents/com~apple~CloudDocs/a.txt"));
        assert!(p.is_protected("/"));
        assert!(!ProtectedPaths::none(PathStyle::UNIX).is_protected("/tmp/x"));
    }
}
