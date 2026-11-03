//! Path helpers: data dir, root canonicalization, separator-aware string logic.
//!
//! Index paths are plain strings with native separators. All string logic takes a
//! [`PathStyle`] so Windows behavior (`\`, case-insensitive) is testable on Linux.
//!
//! Unicode: on macOS the file system hands out decomposed (NFD) names, so the Swift app
//! stores NFC everywhere. NTFS (and ext4) are *normalization-sensitive*: an NFD name and its
//! NFC twin are two different files, so on Windows paths are stored exactly as the
//! enumerator returns them (`storage_form` is the identity there). NFC is still applied to
//! search keys and name matching ([`fold`]), so typing "한글" finds a decomposed "한글".

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// Separator and case rules of a path family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathStyle {
    pub sep: char,
    pub case_insensitive: bool,
}

impl PathStyle {
    pub const WINDOWS: PathStyle = PathStyle {
        sep: '\\',
        case_insensitive: true,
    };
    pub const UNIX: PathStyle = PathStyle {
        sep: '/',
        case_insensitive: false,
    };

    /// The style of the platform we run on.
    pub fn native() -> PathStyle {
        if cfg!(windows) {
            Self::WINDOWS
        } else {
            Self::UNIX
        }
    }

    /// Comparison key (lowercased when the style is case-insensitive).
    pub fn key(&self, s: &str) -> String {
        if self.case_insensitive {
            s.to_lowercase()
        } else {
            s.to_string()
        }
    }

    /// Equal under this style's case rules.
    pub fn eq(&self, a: &str, b: &str) -> bool {
        if self.case_insensitive {
            a == b || chars_eq_ci(a, b)
        } else {
            a == b
        }
    }

    /// Length in bytes of the prefix of `path` that equals `prefix` (char-wise, honoring case
    /// rules), or None.
    fn match_prefix(&self, path: &str, prefix: &str) -> Option<usize> {
        if path.starts_with(prefix) {
            return Some(prefix.len());
        }
        if !self.case_insensitive {
            return None;
        }
        let mut pi = path.char_indices();
        for pc in prefix.chars() {
            let (_, c) = pi.next()?;
            if c != pc && !char_eq_ci(c, pc) {
                return None;
            }
        }
        Some(pi.next().map(|(i, _)| i).unwrap_or(path.len()))
    }

    /// True for "/" or a drive root like "C:\" (or "C:").
    pub fn is_root(&self, p: &str) -> bool {
        let one_sep = p.len() == self.sep.len_utf8() && p.starts_with(self.sep);
        let drive_root =
            p.len() == 2 + self.sep.len_utf8() && is_bare_drive(&p[..2]) && p.ends_with(self.sep);
        one_sep || drive_root || is_bare_drive(p)
    }

    /// Remove trailing separators, keeping roots (`/`, `C:\`). `C:` becomes `C:\`.
    pub fn trim(&self, p: &str) -> String {
        if is_bare_drive(p) {
            return format!("{p}{}", self.sep);
        }
        let mut s = p;
        while s.len() > 1 && s.ends_with(self.sep) {
            let shorter = &s[..s.len() - 1];
            if is_bare_drive(shorter) {
                break; // "C:\" stays
            }
            s = shorter;
        }
        s.to_string()
    }

    /// `path` is `dir` itself or lies under it.
    pub fn is_same_or_under(&self, path: &str, dir: &str) -> bool {
        self.eq(path, dir) || self.is_under(path, dir)
    }

    /// `path` lies strictly under `dir`.
    pub fn is_under(&self, path: &str, dir: &str) -> bool {
        match self.match_prefix(path, dir) {
            Some(n) if n < path.len() => {
                dir.ends_with(self.sep) || path[n..].starts_with(self.sep) && n + 1 < path.len()
            }
            _ => false,
        }
    }

    /// The part of `path` after `root` and its separator ("a\b.txt"), None when not under it.
    pub fn relative<'a>(&self, path: &'a str, root: &str) -> Option<&'a str> {
        let n = self.match_prefix(path, root)?;
        let rest = &path[n..];
        if root.ends_with(self.sep) {
            return if rest.is_empty() { None } else { Some(rest) };
        }
        let rest = rest.strip_prefix(self.sep)?;
        if rest.is_empty() {
            None
        } else {
            Some(rest)
        }
    }

    /// `[lower, upper)` covering every path strictly under `path` — the SQL range used
    /// instead of `LIKE` so the primary-key index is used. The character after `\` is `]`,
    /// after `/` it is `0`.
    pub fn descendant_range(&self, path: &str) -> (String, String) {
        let lower = if path.ends_with(self.sep) {
            path.to_string()
        } else {
            format!("{path}{}", self.sep)
        };
        let next = char::from_u32(self.sep as u32 + 1).unwrap_or(self.sep);
        let mut upper = lower[..lower.len() - self.sep.len_utf8()].to_string();
        upper.push(next);
        (lower, upper)
    }

    pub fn join(&self, base: &str, name: &str) -> String {
        if base.is_empty() {
            name.to_string()
        } else if base.ends_with(self.sep) {
            format!("{base}{name}")
        } else {
            format!("{base}{}{name}", self.sep)
        }
    }

    /// Parent folder ("C:\a" → "C:\", "/a" → "/"); None for roots and bare names.
    pub fn parent<'a>(&self, p: &'a str) -> Option<&'a str> {
        if self.is_root(p) {
            return None;
        }
        let t = p.strip_suffix(self.sep).unwrap_or(p);
        let idx = t.rfind(self.sep)?;
        let head = &t[..idx];
        if head.is_empty() {
            return Some(&t[..self.sep.len_utf8()]); // "/"
        }
        if is_bare_drive(head) {
            return Some(&t[..idx + self.sep.len_utf8()]); // "C:\"
        }
        // UNC share root "\\server\share" is fine as a parent string
        Some(head)
    }

    /// Last component ("" for roots).
    pub fn file_name<'a>(&self, p: &'a str) -> &'a str {
        if self.is_root(p) {
            return "";
        }
        let t = p.strip_suffix(self.sep).unwrap_or(p);
        match t.rfind(self.sep) {
            Some(i) => &t[i + self.sep.len_utf8()..],
            None => t,
        }
    }

    /// Non-empty components.
    pub fn components<'a>(&self, p: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let sep = self.sep;
        p.split(sep).filter(|c| !c.is_empty())
    }

    /// Number of components below the root (used to order work shallow-first).
    pub fn depth(&self, p: &str) -> usize {
        self.components(p).count()
    }
}

fn is_bare_drive(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn char_eq_ci(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

fn chars_eq_ci(a: &str, b: &str) -> bool {
    let mut ai = a.chars();
    let mut bi = b.chars();
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if char_eq_ci(x, y) => {}
            _ => return false,
        }
    }
}

/// NFC (composed) form.
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// Search / matching key: NFC + lowercase.
pub fn fold(s: &str) -> String {
    nfc(s).to_lowercase()
}

/// How a path string is stored in the index: NFC on macOS (normalization-insensitive file
/// system that returns NFD), unchanged elsewhere (NTFS/ext4 are normalization-sensitive).
pub fn storage_form(s: &str) -> String {
    if cfg!(target_os = "macos") {
        nfc(s)
    } else {
        s.to_string()
    }
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\host\share` → `\\host\share`.
pub fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// Light normalization for paths coming from the UI or the OS: no `\\?\`, no trailing
/// separator (roots keep theirs), storage form. Does not touch the disk.
pub fn normalize(p: &str) -> String {
    let style = PathStyle::native();
    style.trim(&storage_form(&strip_verbatim(p)))
}

/// Canonical form of a user folder: symlinks/junctions resolved, on-disk letter case,
/// never `\\?\`. Fails when the path does not exist.
pub fn canonicalize_root(p: &str) -> std::io::Result<String> {
    let c = dunce::canonicalize(Path::new(p))?;
    let s = c.to_str().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "path is not valid Unicode")
    })?;
    Ok(normalize(s))
}

/// The user's home folder (`%USERPROFILE%` on Windows, `$HOME` elsewhere).
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Home as an index-style string ("" when unknown).
pub fn home_string() -> String {
    home_dir()
        .and_then(|h| h.to_str().map(normalize))
        .unwrap_or_default()
}

/// Where settings, the index and models live. `MF_DATA_DIR` overrides; Windows:
/// `%LOCALAPPDATA%\com.taeholim.multifinder` (the app identifier — the same folder Tauri uses
/// for WebView2 data, so the installer's "delete app data" option removes it and an install
/// folder named after the product never mixes with data); elsewhere `$XDG_DATA_HOME/multi-finder` or
/// `~/.local/share/multi-finder`.
/// Folder name under `%LOCALAPPDATA%` on Windows (= the Tauri app identifier).
pub const WINDOWS_DATA_DIR: &str = "com.taeholim.multifinder";

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MF_DATA_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        if let Some(l) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            return PathBuf::from(l).join(WINDOWS_DATA_DIR);
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(x) = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            return x.join("multi-finder");
        }
    }
    if let Some(h) = home_dir() {
        if cfg!(windows) {
            return h.join("AppData").join("Local").join(WINDOWS_DATA_DIR);
        }
        return h.join(".local").join("share").join("multi-finder");
    }
    std::env::temp_dir().join("multi-finder")
}

/// `data_dir()/models`
pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

/// Replace the home prefix with `~` (for prompts and progress text).
pub fn display_with_home(path: &str, home: &str, style: PathStyle) -> String {
    if home.is_empty() {
        return path.to_string();
    }
    if style.eq(path, home) {
        return "~".to_string();
    }
    match style.relative(path, home) {
        Some(rest) => format!("~{}{}", style.sep, rest),
        None => path.to_string(),
    }
}

/// Milliseconds since the Unix epoch (negative before 1970).
pub fn system_time_ms(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis().min(i64::MAX as u128) as i64,
        Err(e) => -(e.duration().as_millis().min(i64::MAX as u128) as i64),
    }
}

/// Nanoseconds since the Unix epoch (negative before 1970).
pub fn system_time_ns(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos().min(i64::MAX as u128) as i64,
        Err(e) => -(e.duration().as_nanos().min(i64::MAX as u128) as i64),
    }
}

/// Now in ms since epoch.
pub fn now_ms() -> i64 {
    system_time_ms(std::time::SystemTime::now())
}

/// Start of a filter's date window: local midnight today minus `within_days - 1` days
/// (1 = today, 7 = the last 7 days including today). None when `within_days` is 0.
pub fn window_start_ms(within_days: u32) -> Option<i64> {
    use chrono::{Duration, Local, TimeZone};
    if within_days == 0 {
        return None;
    }
    let today = Local::now().date_naive();
    let start = today.checked_sub_signed(Duration::days(i64::from(within_days) - 1))?;
    let midnight = start.and_hms_opt(0, 0, 0)?;
    let local = Local.from_local_datetime(&midnight).earliest()?;
    Some(local.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: PathStyle = PathStyle::WINDOWS;
    const U: PathStyle = PathStyle::UNIX;

    #[test]
    fn descendant_ranges() {
        assert_eq!(
            U.descendant_range("/t/b"),
            ("/t/b/".to_string(), "/t/b0".to_string())
        );
        assert_eq!(U.descendant_range("/"), ("/".to_string(), "0".to_string()));
        assert_eq!(
            W.descendant_range(r"C:\Users\me"),
            (r"C:\Users\me\".to_string(), r"C:\Users\me]".to_string())
        );
        assert_eq!(
            W.descendant_range(r"D:\"),
            (r"D:\".to_string(), "D:]".to_string())
        );
    }

    #[test]
    fn under_and_relative_follow_case_rules() {
        assert!(W.is_under(r"c:\users\ME\Downloads\a.txt", r"C:\Users\me"));
        assert!(!W.is_under(r"C:\Users\me2\a.txt", r"C:\Users\me"));
        assert!(!W.is_under(r"C:\Users\me", r"C:\Users\me"));
        assert!(W.is_same_or_under(r"C:\USERS\ME", r"C:\Users\me"));
        assert!(W.is_under(r"D:\x", r"D:\"));
        assert!(!U.is_under("/t/b c", "/t/b"));
        assert!(!U.is_under("/T/b/c", "/t/b"));
        assert!(U.is_under("/t/b/c", "/t/b"));
        assert_eq!(
            W.relative(r"C:\Users\Me\Pics\a.jpg", r"c:\users\me"),
            Some(r"Pics\a.jpg")
        );
        assert_eq!(W.relative(r"D:\a\b", r"D:\"), Some(r"a\b"));
        assert_eq!(U.relative("/a/b", "/a"), Some("b"));
        assert_eq!(U.relative("/a", "/a"), None);
        assert_eq!(U.relative("/ab", "/a"), None);
    }

    #[test]
    fn parents_names_and_trim() {
        assert_eq!(W.parent(r"C:\a"), Some(r"C:\"));
        assert_eq!(W.parent(r"C:\a\b"), Some(r"C:\a"));
        assert_eq!(W.parent(r"C:\"), None);
        assert_eq!(U.parent("/a"), Some("/"));
        assert_eq!(U.parent("/a/b"), Some("/a"));
        assert_eq!(U.parent("/"), None);
        assert_eq!(W.file_name(r"C:\a\b.txt"), "b.txt");
        assert_eq!(W.file_name(r"C:\"), "");
        assert_eq!(W.trim(r"C:\a\"), r"C:\a");
        assert_eq!(W.trim(r"C:\"), r"C:\");
        assert_eq!(W.trim("C:"), r"C:\");
        assert_eq!(U.trim("/a//"), "/a");
        assert_eq!(U.trim("/"), "/");
        assert_eq!(W.join(r"C:\", "a"), r"C:\a");
        assert_eq!(W.join(r"C:\a", "b"), r"C:\a\b");
    }

    #[test]
    fn verbatim_prefixes_are_stripped() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\me"), r"C:\Users\me");
        assert_eq!(strip_verbatim(r"\\?\UNC\nas\share\x"), r"\\nas\share\x");
        assert_eq!(strip_verbatim(r"C:\x"), r"C:\x");
    }

    #[test]
    fn fold_matches_nfd_and_case() {
        let nfd: String = "한글 Report".nfd().collect();
        assert_ne!(nfd, "한글 Report");
        assert_eq!(fold(&nfd), fold("한글 REPORT"));
        assert!(W.eq("ÄBC", "äbc"));
    }

    #[test]
    fn home_display() {
        assert_eq!(
            display_with_home(r"C:\Users\me\Downloads", r"C:\Users\me", W),
            r"~\Downloads"
        );
        assert_eq!(display_with_home("/home/me", "/home/me", U), "~");
        assert_eq!(display_with_home("/srv/x", "/home/me", U), "/srv/x");
    }

    #[test]
    fn window_start_is_local_midnight() {
        assert_eq!(window_start_ms(0), None);
        let today = window_start_ms(1).unwrap();
        let week = window_start_ms(7).unwrap();
        assert!(today <= now_ms());
        assert!(now_ms() - today <= 25 * 3600 * 1000);
        let diff = today - week;
        // 6 days, ± one hour for daylight-saving changes
        assert!((diff - 6 * 86_400_000).abs() <= 3_600_000, "{diff}");
    }
}
