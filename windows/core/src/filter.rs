//! View filters (kind / size / date / name) and sorting — port of `FileFilter.swift` and
//! `AppModel.sorted`.

use std::cmp::Ordering;

use crate::paths;
use crate::record::{FileKind, FileRecord};
use crate::types::{FileCategory, Filter, Sort, SortField};

pub const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "ppt", "pptx", "key", "xls", "xlsx", "numbers", "pages", "hwp", "hwpx",
    "txt", "md", "rtf", "csv", "epub",
];
pub const IMAGE: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp", "svg", "psd", "ai",
    "raw", "dng",
];
pub const VIDEO: &[&str] = &["mp4", "mov", "mkv", "avi", "m4v", "webm", "wmv"];
pub const AUDIO: &[&str] = &["mp3", "m4a", "wav", "aac", "flac", "aiff", "ogg"];
pub const ARCHIVE: &[&str] = &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz"];
/// macOS installers plus Windows ones (exe, msi, msix, appx …).
pub const INSTALLER: &[&str] = &[
    "dmg",
    "pkg",
    "iso",
    "mpkg",
    "xip",
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
];
pub const CODE: &[&str] = &[
    "swift", "js", "ts", "tsx", "jsx", "py", "java", "kt", "c", "cpp", "h", "m", "go", "rs", "rb",
    "php", "html", "css", "scss", "json", "yaml", "yml", "toml", "sh", "sql", "cs", "ps1", "bat",
];

/// Extensions of a category (empty for `All` and `Other`).
pub fn extensions(category: FileCategory) -> &'static [&'static str] {
    match category {
        FileCategory::Document => DOCUMENT,
        FileCategory::Image => IMAGE,
        FileCategory::Video => VIDEO,
        FileCategory::Audio => AUDIO,
        FileCategory::Archive => ARCHIVE,
        FileCategory::Installer => INSTALLER,
        FileCategory::Code => CODE,
        FileCategory::All | FileCategory::Other => &[],
    }
}

const ORDER: [FileCategory; 7] = [
    FileCategory::Document,
    FileCategory::Image,
    FileCategory::Video,
    FileCategory::Audio,
    FileCategory::Archive,
    FileCategory::Installer,
    FileCategory::Code,
];

/// Every extension that belongs to some category.
pub fn all_known_extensions() -> Vec<&'static str> {
    ORDER
        .iter()
        .flat_map(|c| extensions(*c).iter().copied())
        .collect()
}

/// Category of a record (non-files are `Other`).
pub fn category_of(kind: FileKind, ext: &str) -> FileCategory {
    if kind != FileKind::File {
        return FileCategory::Other;
    }
    for c in ORDER {
        if extensions(c).contains(&ext) {
            return c;
        }
    }
    FileCategory::Other
}

/// A [`Filter`] prepared for matching many records.
pub struct Matcher {
    needle: Option<String>,
    category: FileCategory,
    min_size: u64,
    cutoff: Option<i64>,
}

impl Matcher {
    pub fn new(filter: &Filter) -> Matcher {
        let t = filter.text.trim();
        Matcher {
            needle: if t.is_empty() {
                None
            } else {
                Some(paths::fold(t))
            },
            category: filter.category,
            min_size: filter.min_size,
            cutoff: paths::window_start_ms(filter.within_days),
        }
    }

    /// Folders always pass kind/size/date (the tree stays navigable); only the text filters them.
    pub fn matches(&self, r: &FileRecord) -> bool {
        if let Some(n) = &self.needle {
            if !paths::fold(&r.name).contains(n.as_str()) {
                return false;
            }
        }
        if r.kind == FileKind::Dir {
            return true;
        }
        if self.category != FileCategory::All && category_of(r.kind, &r.ext) != self.category {
            return false;
        }
        if r.size < self.min_size {
            return false;
        }
        if let Some(c) = self.cutoff {
            if r.modified < c {
                return false;
            }
        }
        true
    }

    pub fn cutoff(&self) -> Option<i64> {
        self.cutoff
    }
}

/// Finder-like name order: case-insensitive, NFC, digit runs compared as numbers
/// ("file2" < "file10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let ka = paths::fold(a);
    let kb = paths::fold(b);
    let mut ia = ka.chars().peekable();
    let mut ib = kb.chars().peekable();
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => break,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut da = String::new();
                while let Some(c) = ia.peek().copied().filter(|c| c.is_ascii_digit()) {
                    da.push(c);
                    ia.next();
                }
                let mut db = String::new();
                while let Some(c) = ib.peek().copied().filter(|c| c.is_ascii_digit()) {
                    db.push(c);
                    ib.next();
                }
                let ta = da.trim_start_matches('0');
                let tb = db.trim_start_matches('0');
                let ord = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| da.len().cmp(&db.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ia.next();
                ib.next();
            }
        }
    }
    a.cmp(b)
}

/// Folders first, then by the chosen field. `dir_bytes` supplies folder totals for size sort.
pub fn sort_records(
    records: &mut [FileRecord],
    sort: Sort,
    dir_bytes: &dyn Fn(&str) -> Option<u64>,
) {
    let size_of = |r: &FileRecord| -> i128 {
        if r.kind == FileKind::Dir {
            dir_bytes(&r.path).map(|b| b as i128).unwrap_or(-1)
        } else {
            r.size as i128
        }
    };
    let mut sizes = std::collections::HashMap::new();
    if sort.field == SortField::Size {
        for r in records.iter() {
            sizes.insert(r.path.clone(), size_of(r));
        }
    }
    records.sort_by(|a, b| {
        let ad = a.kind == FileKind::Dir;
        let bd = b.kind == FileKind::Dir;
        if ad != bd {
            return if ad {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let ord = match sort.field {
            SortField::Name => natural_cmp(&a.name, &b.name),
            SortField::Modified => a
                .modified
                .cmp(&b.modified)
                .then_with(|| natural_cmp(&a.name, &b.name)),
            SortField::Size => {
                let sa = sizes.get(&a.path).copied().unwrap_or(0);
                let sb = sizes.get(&b.path).copied().unwrap_or(0);
                sa.cmp(&sb).then_with(|| natural_cmp(&a.name, &b.name))
            }
            SortField::Kind => a
                .ext
                .cmp(&b.ext)
                .then_with(|| natural_cmp(&a.name, &b.name)),
        };
        if sort.ascending {
            ord
        } else {
            ord.reverse()
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, kind: FileKind, size: u64, modified: i64) -> FileRecord {
        FileRecord {
            path: format!("/r/{name}"),
            name: name.into(),
            parent: "/r".into(),
            ext: crate::record::extension_of(name),
            size,
            modified,
            inode: 0,
            kind,
            cloud: false,
            hash: None,
        }
    }

    #[test]
    fn categories() {
        assert_eq!(category_of(FileKind::File, "pdf"), FileCategory::Document);
        assert_eq!(category_of(FileKind::File, "exe"), FileCategory::Installer);
        assert_eq!(category_of(FileKind::File, "msi"), FileCategory::Installer);
        assert_eq!(category_of(FileKind::File, "xyz"), FileCategory::Other);
        assert_eq!(category_of(FileKind::Dir, "pdf"), FileCategory::Other);
    }

    #[test]
    fn matcher_keeps_folders_navigable() {
        let m = Matcher::new(&Filter {
            text: String::new(),
            category: FileCategory::Image,
            min_size: 10,
            within_days: 0,
        });
        assert!(m.matches(&rec("folder", FileKind::Dir, 0, 0)));
        assert!(m.matches(&rec("a.png", FileKind::File, 10, 0)));
        assert!(!m.matches(&rec("a.png", FileKind::File, 9, 0)));
        assert!(!m.matches(&rec("a.txt", FileKind::File, 100, 0)));
        let t = Matcher::new(&Filter {
            text: "REP".into(),
            ..Default::default()
        });
        assert!(t.matches(&rec("report.pdf", FileKind::File, 1, 0)));
        assert!(!t.matches(&rec("folder", FileKind::Dir, 0, 0)));
        let d = Matcher::new(&Filter {
            within_days: 1,
            ..Default::default()
        });
        assert!(!d.matches(&rec("old.txt", FileKind::File, 1, 0)));
        assert!(d.matches(&rec("new.txt", FileKind::File, 1, paths::now_ms())));
    }

    #[test]
    fn natural_order_and_sorting() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("File", "file"), Ordering::Less); // tie broken by raw string
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        let mut v = vec![
            rec("b10.txt", FileKind::File, 5, 3),
            rec("zdir", FileKind::Dir, 0, 1),
            rec("b2.txt", FileKind::File, 50, 2),
            rec("adir", FileKind::Dir, 0, 9),
        ];
        sort_records(
            &mut v,
            Sort {
                field: SortField::Name,
                ascending: true,
            },
            &|_| None,
        );
        let names: Vec<_> = v.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["adir", "zdir", "b2.txt", "b10.txt"]);
        sort_records(
            &mut v,
            Sort {
                field: SortField::Size,
                ascending: false,
            },
            &|p| Some(if p.ends_with("adir") { 1 } else { 100 }),
        );
        let names: Vec<_> = v.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["zdir", "adir", "b2.txt", "b10.txt"]);
    }
}
