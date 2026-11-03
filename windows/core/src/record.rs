//! One index row (`FileRecord`) and how to read it from disk.

use std::fs::Metadata;

use crate::paths::{self, PathStyle};
use crate::types::{Entry, Kind};

/// FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
pub const ATTR_HIDDEN: u32 = 0x2;
pub const ATTR_SYSTEM: u32 = 0x4;
/// OneDrive Files On-Demand placeholders: RECALL_ON_DATA_ACCESS, RECALL_ON_OPEN, OFFLINE.
pub const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
pub const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
pub const ATTR_OFFLINE: u32 = 0x1000;

/// Content is not on disk (reading it would start a download).
pub fn is_cloud_attrs(attrs: u32) -> bool {
    attrs & (ATTR_RECALL_ON_DATA_ACCESS | ATTR_RECALL_ON_OPEN | ATTR_OFFLINE) != 0
}

/// Hidden or system entry (desktop.ini, Thumbs.db, $-folders …).
pub fn is_hidden_attrs(attrs: u32) -> bool {
    attrs & (ATTR_HIDDEN | ATTR_SYSTEM) != 0
}

/// Windows file attributes (0 elsewhere).
#[cfg(windows)]
pub fn file_attributes(md: &Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes()
}

#[cfg(not(windows))]
pub fn file_attributes(_md: &Metadata) -> u32 {
    0
}

/// Inode number on unix; 0 (unknown) on Windows.
#[cfg(unix)]
pub fn inode_of(md: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    md.ino()
}

#[cfg(not(unix))]
pub fn inode_of(_md: &Metadata) -> u64 {
    0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    File = 0,
    Dir = 1,
    /// symlink / junction — recorded, never descended
    Link = 2,
}

impl FileKind {
    pub fn from_i64(v: i64) -> FileKind {
        match v {
            1 => FileKind::Dir,
            2 => FileKind::Link,
            _ => FileKind::File,
        }
    }
    pub fn as_i64(self) -> i64 {
        self as i64
    }
    pub fn kind(self) -> Kind {
        match self {
            FileKind::File => Kind::File,
            FileKind::Dir => Kind::Dir,
            FileKind::Link => Kind::Link,
        }
    }
}

/// A file/folder as stored in the index. Every screen is a different view of these rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub name: String,
    pub parent: String,
    /// lowercase extension of files, "" otherwise
    pub ext: String,
    pub size: u64,
    /// ms since epoch
    pub modified: i64,
    pub inode: u64,
    pub kind: FileKind,
    /// online-only placeholder — never read or hashed
    pub cloud: bool,
    /// BLAKE3 (hex) of the content, filled only once it was needed
    pub hash: Option<String>,
}

impl FileRecord {
    /// Build from metadata obtained WITHOUT following links (`symlink_metadata` or a
    /// directory-walk entry).
    pub fn from_metadata(path: &str, md: &Metadata, style: PathStyle) -> FileRecord {
        let ft = md.file_type();
        let kind = if ft.is_symlink() {
            FileKind::Link
        } else if ft.is_dir() {
            FileKind::Dir
        } else {
            FileKind::File
        };
        let path = paths::storage_form(path);
        let mut name = style.file_name(&path).to_string();
        if name.is_empty() {
            name = path.clone(); // a drive root ("D:\") is shown by its path
        }
        let parent = style.parent(&path).unwrap_or("").to_string();
        let ext = if kind == FileKind::File {
            extension_of(&name)
        } else {
            String::new()
        };
        let attrs = file_attributes(md);
        FileRecord {
            size: if kind == FileKind::File { md.len() } else { 0 },
            modified: md.modified().map(paths::system_time_ms).unwrap_or(0),
            inode: inode_of(md),
            cloud: is_cloud_attrs(attrs),
            path,
            name,
            parent,
            ext,
            kind,
            hash: None,
        }
    }

    pub fn to_entry(&self) -> Entry {
        Entry {
            path: self.path.clone(),
            name: self.name.clone(),
            parent: self.parent.clone(),
            ext: self.ext.clone(),
            kind: self.kind.kind(),
            size: self.size,
            modified: self.modified,
            cloud: self.cloud,
        }
    }
}

/// Lowercased extension without the dot ("" for none, ".bashrc" has none).
pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// Read one path from disk (links are not followed). None when it does not exist.
pub fn record_for(path: &str, style: PathStyle) -> Option<FileRecord> {
    let md = std::fs::symlink_metadata(path).ok()?;
    Some(FileRecord::from_metadata(path, &md, style))
}

/// Is the file still the one the index knows (exists, same kind, size and modification time)?
/// If so its stored hash is still valid — hashes are dropped whenever size/mtime change.
pub fn is_unchanged(r: &FileRecord, style: PathStyle) -> bool {
    match record_for(&r.path, style) {
        Some(now) => now.kind == r.kind && now.size == r.size && now.modified == r.modified,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(extension_of("a.TXT"), "txt");
        assert_eq!(extension_of("archive.tar.gz"), "gz");
        assert_eq!(extension_of(".bashrc"), "");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("dot."), "");
    }

    #[test]
    fn attributes() {
        assert!(is_cloud_attrs(0x40_0000 | 0x20));
        assert!(is_cloud_attrs(0x1000));
        assert!(!is_cloud_attrs(0x20));
        assert!(is_hidden_attrs(0x2));
        assert!(is_hidden_attrs(0x4 | 0x10));
        assert!(!is_hidden_attrs(0x10));
    }

    #[test]
    fn unchanged_detects_edits_and_deletion() {
        let dir = tempfile::Builder::new().prefix("mf-rec").tempdir().unwrap();
        let f = dir.path().join("e.txt");
        std::fs::write(&f, "abc").unwrap();
        let style = PathStyle::native();
        let rec = record_for(f.to_str().unwrap(), style).unwrap();
        assert_eq!(rec.size, 3);
        assert_eq!(rec.ext, "txt");
        assert_eq!(rec.kind, FileKind::File);
        assert!(is_unchanged(&rec, style));
        std::fs::write(&f, "abcd").unwrap();
        assert!(!is_unchanged(&rec, style));
        std::fs::remove_file(&f).unwrap();
        assert!(!is_unchanged(&rec, style));
    }
}
