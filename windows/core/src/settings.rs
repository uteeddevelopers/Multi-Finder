//! Persisted settings (`settings.json` in the data dir).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::{self, PathStyle};

pub const FILE_NAME: &str = "settings.json";

/// What is stored on disk. The UI-facing shape is [`crate::types::Settings`].
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The user's folders in sidebar order (canonical paths).
    #[serde(default)]
    pub folders: Vec<String>,
}

impl Settings {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Missing or unreadable file → defaults. A corrupt file is kept as `settings.json.bad`.
    pub fn load(data_dir: &Path) -> Settings {
        let path = Self::path(data_dir);
        let Ok(bytes) = std::fs::read(&path) else {
            return Settings::default();
        };
        match serde_json::from_slice::<Settings>(&bytes) {
            Ok(s) => s,
            Err(_) => {
                let _ = std::fs::rename(&path, data_dir.join(format!("{FILE_NAME}.bad")));
                Settings::default()
            }
        }
    }

    /// Atomic write: temp file in the same folder, flushed, then renamed over the old one.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let path = Self::path(data_dir);
        let tmp = data_dir.join(format!("{FILE_NAME}.tmp"));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &path)
    }

    /// Canonicalize (dunce), dedupe (case-insensitive on Windows) and append.
    /// Returns the folders that were actually added; paths that do not exist are ignored.
    pub fn add_folders<S: AsRef<str>>(&mut self, raw: &[S]) -> Vec<String> {
        let style = PathStyle::native();
        let mut added = Vec::new();
        for p in raw {
            let Ok(c) = paths::canonicalize_root(p.as_ref()) else {
                continue;
            };
            if self.folders.iter().any(|f| style.eq(f, &c)) {
                continue;
            }
            self.folders.push(c.clone());
            added.push(c);
        }
        added
    }

    pub fn remove_folder(&mut self, path: &str) {
        let style = PathStyle::native();
        self.folders.retain(|f| !style.eq(f, path));
    }

    /// Take the order from `order`; folders missing from it keep their relative order at the
    /// end, unknown paths are ignored.
    pub fn reorder<S: AsRef<str>>(&mut self, order: &[S]) {
        let style = PathStyle::native();
        let mut out: Vec<String> = Vec::new();
        for o in order {
            if let Some(f) = self.folders.iter().find(|f| style.eq(f, o.as_ref())) {
                if !out.iter().any(|x| style.eq(x, f)) {
                    out.push(f.clone());
                }
            }
        }
        for f in &self.folders {
            if !out.iter().any(|x| style.eq(x, f)) {
                out.push(f.clone());
            }
        }
        self.folders = out;
    }

    /// The UI shape (`get_settings`).
    pub fn view(&self, locale: &str, data_dir: &Path, version: &str) -> crate::types::Settings {
        crate::types::Settings {
            folders: self.folders.clone(),
            locale: locale.to_string(),
            data_dir: data_dir.to_string_lossy().into_owned(),
            version: version.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip_and_folder_edits() {
        let dir = tempfile::Builder::new()
            .prefix("mf-settings")
            .tempdir()
            .unwrap();
        let a = dir.path().join("A");
        let b = dir.path().join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut s = Settings::load(dir.path());
        assert!(s.folders.is_empty());
        let added = s.add_folders(&[
            a.to_str().unwrap().to_string(),
            format!("{}/", b.to_str().unwrap()),
            a.to_str().unwrap().to_string(),
            dir.path().join("missing").to_str().unwrap().to_string(),
        ]);
        assert_eq!(added.len(), 2);
        assert!(!added[1].ends_with('/'));
        s.reorder(&[added[1].clone()]);
        assert_eq!(s.folders, vec![added[1].clone(), added[0].clone()]);
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        s.remove_folder(&added[1]);
        assert_eq!(s.folders, vec![added[0].clone()]);

        std::fs::write(Settings::path(dir.path()), b"{ not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
        let v = s.view("ko-KR", dir.path(), "1.0.0");
        assert_eq!(v.locale, "ko-KR");
    }
}
