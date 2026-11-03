//! Second pass over duplicate groups: keep what is worth cleaning. Port of
//! `DuplicateAnalyzer.swift` + `ProjectRoots.swift`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::paths::{self, PathStyle};
use crate::record::FileRecord;
use crate::types::{CopyTree, DuplicateReport, FileGroup};

/// Files with identical content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupGroup {
    pub hash: String,
    /// size of one file
    pub size: u64,
    /// sorted by path
    pub files: Vec<FileRecord>,
}

impl DupGroup {
    /// Bytes freed by keeping one copy.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len().saturating_sub(1) as u64)
    }

    pub fn to_file_group(&self) -> FileGroup {
        FileGroup {
            id: self.hash.clone(),
            size: self.size,
            files: self.files.iter().map(FileRecord::to_entry).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateOptions {
    /// hide groups whose files all sit in the same project (repo, package …)
    pub hide_within_project: bool,
    /// fold groups that are copies between the same two folders into one row
    pub fold_copy_trees: bool,
    /// at least this many groups between the same folders to fold them
    pub min_groups_to_fold: usize,
    pub apply_ignore_list: bool,
}

impl Default for DuplicateOptions {
    fn default() -> Self {
        DuplicateOptions {
            hide_within_project: true,
            fold_copy_trees: true,
            min_groups_to_fold: 3,
            apply_ignore_list: true,
        }
    }
}

/// Hashes and folders the user chose to ignore (persisted in the index).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnoreList {
    pub hashes: HashSet<String>,
    pub folders: Vec<String>,
}

impl IgnoreList {
    pub fn covers(&self, path: &str, style: PathStyle) -> bool {
        self.folders.iter().any(|f| style.is_same_or_under(path, f))
    }
}

/// Finds the project (code repository, package …) a path belongs to.
pub struct ProjectRoots {
    cache: HashMap<String, Option<String>>,
    ceilings: Vec<String>,
    style: PathStyle,
}

impl ProjectRoots {
    /// A folder containing one of these is a project root.
    pub const MARKERS: &'static [&'static str] = &[
        ".git",
        "package.json",
        "Package.swift",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "go.mod",
        "Gemfile",
        "composer.json",
        "Podfile",
        "pubspec.yaml",
        "CMakeLists.txt",
        "Makefile",
    ];
    /// ... or an entry with one of these suffixes (Xcode, Visual Studio).
    pub const SUFFIXES: &'static [&'static str] = &[
        ".xcodeproj",
        ".xcworkspace",
        ".sln",
        ".csproj",
        ".vcxproj",
        ".fsproj",
        ".vbproj",
    ];

    /// `ceilings` (and everything above them) are never project roots.
    pub fn new(ceilings: Vec<String>, style: PathStyle) -> ProjectRoots {
        ProjectRoots {
            cache: HashMap::new(),
            ceilings: ceilings.iter().map(|c| style.trim(c)).collect(),
            style,
        }
    }

    /// Ceilings: the home folder (file-system roots are always ceilings).
    pub fn platform_default() -> ProjectRoots {
        let home = paths::home_string();
        ProjectRoots::new(
            if home.is_empty() { vec![] } else { vec![home] },
            PathStyle::native(),
        )
    }

    /// The closest project root containing `path`, if any.
    pub fn root_of(&mut self, path: &str) -> Option<String> {
        let style = self.style;
        let mut dir = style.parent(path).map(str::to_string);
        let mut visited = Vec::new();
        let mut found = None;
        while let Some(d) = dir {
            if style.is_root(&d) || self.ceilings.iter().any(|c| style.eq(c, &d)) {
                break;
            }
            if let Some(cached) = self.cache.get(&d) {
                found = cached.clone();
                break;
            }
            visited.push(d.clone());
            if Self::is_project_root(&d) {
                found = Some(d);
                break;
            }
            dir = style.parent(&d).map(str::to_string);
        }
        for v in visited {
            self.cache.insert(v, found.clone());
        }
        found
    }

    fn is_project_root(dir: &str) -> bool {
        let d = Path::new(dir);
        if Self::MARKERS
            .iter()
            .any(|m| std::fs::symlink_metadata(d.join(m)).is_ok())
        {
            return true;
        }
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten().take(5000) {
                if let Some(n) = e.file_name().to_str() {
                    let l = n.to_lowercase();
                    if Self::SUFFIXES.iter().any(|s| l.ends_with(s)) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Groups folded into "folder A ⇄ folder B".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyTreeGroup {
    pub left: String,
    pub right: String,
    /// biggest first
    pub groups: Vec<DupGroup>,
    /// a few relative paths
    pub samples: Vec<String>,
}

impl CopyTreeGroup {
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(DupGroup::wasted).sum()
    }

    /// Stable id: "tree:" + short hash of the two folders.
    pub fn id(&self) -> String {
        let h = blake3::hash(format!("{}\n{}", self.left, self.right).as_bytes());
        format!("tree:{}", &h.to_hex()[..16])
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    pub file_groups: Vec<DupGroup>,
    pub copy_trees: Vec<CopyTreeGroup>,
    pub hidden_within_project: u64,
    pub hidden_by_ignore_list: u64,
}

/// Apply the ignore list, hide within-project copies, fold copy trees.
pub fn analyze(
    groups: &[DupGroup],
    options: &DuplicateOptions,
    ignore: &IgnoreList,
    projects: &mut ProjectRoots,
    style: PathStyle,
) -> Analysis {
    let mut out = Analysis::default();
    let mut kept: Vec<DupGroup> = Vec::new();
    for g in groups {
        // 1) ignore list
        if options.apply_ignore_list {
            if ignore.hashes.contains(&g.hash) {
                out.hidden_by_ignore_list += 1;
                continue;
            }
            let remaining: Vec<FileRecord> = g
                .files
                .iter()
                .filter(|f| !ignore.covers(&f.path, style))
                .cloned()
                .collect();
            if remaining.len() < 2 {
                out.hidden_by_ignore_list += 1;
                continue;
            }
            if remaining.len() != g.files.len() {
                kept.push(DupGroup {
                    hash: g.hash.clone(),
                    size: g.size,
                    files: remaining,
                });
                continue;
            }
        }
        // 2) all copies inside one project → intended
        if options.hide_within_project {
            let roots: HashSet<Option<String>> =
                g.files.iter().map(|f| projects.root_of(&f.path)).collect();
            if roots.len() == 1 && roots.iter().next().is_some_and(|r| r.is_some()) {
                out.hidden_within_project += 1;
                continue;
            }
        }
        kept.push(g.clone());
    }

    // 3) fold copy trees (pairs of folders only — the UI shows left ⇄ right)
    if options.fold_copy_trees {
        let mut by_pair: HashMap<(String, String), Vec<(DupGroup, String)>> = HashMap::new();
        let mut order: Vec<(String, String)> = Vec::new();
        for g in &kept {
            let paths: Vec<&str> = g.files.iter().map(|f| f.path.as_str()).collect();
            let Some((roots, rel)) = copy_roots(&paths, style) else {
                continue;
            };
            if roots.len() != 2 {
                continue;
            }
            let key = (roots[0].clone(), roots[1].clone());
            if !by_pair.contains_key(&key) {
                order.push(key.clone());
            }
            by_pair.entry(key).or_default().push((g.clone(), rel));
        }
        let mut folded: HashSet<String> = HashSet::new();
        for key in order {
            let items = &by_pair[&key];
            if items.len() < options.min_groups_to_fold.max(1) {
                continue;
            }
            let mut gs: Vec<DupGroup> = items.iter().map(|(g, _)| g.clone()).collect();
            gs.sort_by(|a, b| {
                b.wasted()
                    .cmp(&a.wasted())
                    .then_with(|| a.hash.cmp(&b.hash))
            });
            for g in &gs {
                folded.insert(g.hash.clone());
            }
            out.copy_trees.push(CopyTreeGroup {
                left: key.0.clone(),
                right: key.1.clone(),
                groups: gs,
                samples: items.iter().take(3).map(|(_, r)| r.clone()).collect(),
            });
        }
        kept.retain(|g| !folded.contains(&g.hash));
        out.copy_trees.sort_by(|a, b| {
            b.wasted()
                .cmp(&a.wasted())
                .then_with(|| a.left.cmp(&b.left))
        });
    }
    kept.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then_with(|| a.hash.cmp(&b.hash))
    });
    out.file_groups = kept;
    out
}

/// The "copy roots" of a group: strip the common trailing part (file name included) from
/// every path. None when the names differ or two copies share a folder (not a tree copy).
pub fn copy_roots(paths: &[&str], style: PathStyle) -> Option<(Vec<String>, String)> {
    let comps: Vec<Vec<&str>> = paths
        .iter()
        .map(|p| style.components(p).collect())
        .collect();
    let shortest = comps.iter().map(Vec::len).min()?;
    if shortest <= 1 {
        return None;
    }
    let mut suffix = 0;
    'outer: while suffix < shortest - 1 {
        let candidate = comps[0][comps[0].len() - 1 - suffix];
        for c in comps.iter().skip(1) {
            if c[c.len() - 1 - suffix] != candidate {
                break 'outer;
            }
        }
        suffix += 1;
    }
    if suffix == 0 {
        return None;
    }
    let sep = style.sep.to_string();
    let roots: Vec<String> = paths
        .iter()
        .zip(&comps)
        .map(|(p, c)| {
            let lead: String = p.chars().take_while(|ch| *ch == style.sep).collect();
            let mut r = format!("{lead}{}", c[..c.len() - suffix].join(&sep));
            if style.is_root(&r) && !r.ends_with(style.sep) {
                r.push(style.sep); // "C:" → "C:\"
            }
            r
        })
        .collect();
    let unique: HashSet<String> = roots.iter().map(|r| style.key(r)).collect();
    if unique.len() != roots.len() {
        return None; // copies within the same folder
    }
    let rel = comps[0][comps[0].len() - suffix..].join(&sep);
    let mut sorted = roots;
    sorted.sort();
    Some((sorted, rel))
}

/// Contract shape (`find_duplicates` result).
pub fn to_report(a: &Analysis, raw_group_count: usize) -> DuplicateReport {
    let file_groups: Vec<FileGroup> = a.file_groups.iter().map(DupGroup::to_file_group).collect();
    let copy_trees: Vec<CopyTree> = a
        .copy_trees
        .iter()
        .map(|t| CopyTree {
            id: t.id(),
            left: t.left.clone(),
            right: t.right.clone(),
            groups: t.groups.iter().map(DupGroup::to_file_group).collect(),
            file_count: t.groups.len() as u64,
            bytes: t.wasted(),
            examples: t.samples.clone(),
        })
        .collect();
    let reclaimable = a.file_groups.iter().map(DupGroup::wasted).sum::<u64>()
        + a.copy_trees.iter().map(CopyTreeGroup::wasted).sum::<u64>();
    DuplicateReport {
        file_groups,
        copy_trees,
        raw_group_count: raw_group_count as u64,
        hidden_within_project: a.hidden_within_project,
        hidden_by_ignore_list: a.hidden_by_ignore_list,
        reclaimable_bytes: reclaimable,
    }
}

/// Hashes to put on the ignore list for `ignore_duplicate_group(id)`: the id itself for a
/// file group (ids are content hashes), every folded group's hash for a copy tree.
pub fn hashes_for_id(report: Option<&DuplicateReport>, id: &str) -> Vec<String> {
    if let Some(r) = report {
        if let Some(t) = r.copy_trees.iter().find(|t| t.id == id) {
            return t.groups.iter().map(|g| g.id.clone()).collect();
        }
    }
    if id.starts_with("tree:") {
        return vec![]; // a tree id without its report cannot be resolved
    }
    vec![id.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::FileKind;

    fn rec(p: &str, style: PathStyle) -> FileRecord {
        FileRecord {
            path: p.into(),
            name: style.file_name(p).into(),
            parent: style.parent(p).unwrap_or("").into(),
            ext: String::new(),
            size: 10,
            modified: 0,
            inode: 0,
            kind: FileKind::File,
            cloud: false,
            hash: None,
        }
    }

    fn group(hash: &str, paths: &[&str], style: PathStyle) -> DupGroup {
        DupGroup {
            hash: hash.into(),
            size: 10,
            files: paths.iter().map(|p| rec(p, style)).collect(),
        }
    }

    #[test]
    fn copy_roots_unix_and_windows() {
        let u = PathStyle::UNIX;
        let (roots, rel) =
            copy_roots(&["/a/proj/src/x.txt", "/b/backup/proj/src/x.txt"], u).unwrap();
        assert_eq!(roots, vec!["/a".to_string(), "/b/backup".to_string()]);
        assert_eq!(rel, "proj/src/x.txt");
        assert!(copy_roots(&["/a/x.txt", "/a/y.txt"], u).is_none());
        assert!(copy_roots(&["/a/x.txt", "/a/sub/../x.txt"], u).is_some());
        let w = PathStyle::WINDOWS;
        let (roots, rel) =
            copy_roots(&[r"C:\Users\me\Pics\a.jpg", r"D:\Backup\Pics\a.jpg"], w).unwrap();
        assert_eq!(
            roots,
            vec![r"C:\Users\me".to_string(), r"D:\Backup".to_string()]
        );
        assert_eq!(rel, r"Pics\a.jpg");
        let (roots, _) = copy_roots(&[r"C:\x.txt", r"D:\x.txt"], w).unwrap();
        assert_eq!(roots, vec![r"C:\".to_string(), r"D:\".to_string()]);
        let (roots, _) = copy_roots(&[r"\\nas\share\a\x.txt", r"C:\b\x.txt"], w).unwrap();
        assert!(roots.contains(&r"\\nas\share\a".to_string()), "{roots:?}");
    }

    #[test]
    fn ignore_list_project_hiding_and_folding() {
        let u = PathStyle::UNIX;
        let base = tempfile::Builder::new().prefix("mf-an").tempdir().unwrap();
        let b = base.path().to_str().unwrap().to_string();
        // a project with a repeated asset
        std::fs::create_dir_all(format!("{b}/proj/public")).unwrap();
        std::fs::write(format!("{b}/proj/package.json"), "{}").unwrap();
        let mut projects = ProjectRoots::new(vec![b.clone()], u);
        let groups = vec![
            group(
                "p",
                &[
                    &format!("{b}/proj/public/x.mp4"),
                    &format!("{b}/proj/marketing/x.mp4"),
                ],
                u,
            ),
            group("i", &["/d/a.txt", "/e/a.txt"], u),
            group("f", &["/keep/a.txt", "/ignored/a.txt"], u),
            group("t1", &["/L/one/1.txt", "/R/one/1.txt"], u),
            group("t2", &["/L/two/2.txt", "/R/two/2.txt"], u),
            group("t3", &["/L/3.txt", "/R/3.txt"], u),
            group("solo", &["/m/s.txt", "/n/s.txt"], u),
        ];
        let ignore = IgnoreList {
            hashes: ["i".to_string()].into_iter().collect(),
            folders: vec!["/ignored".into()],
        };
        let a = analyze(
            &groups,
            &DuplicateOptions::default(),
            &ignore,
            &mut projects,
            u,
        );
        assert_eq!(a.hidden_within_project, 1);
        assert_eq!(a.hidden_by_ignore_list, 2);
        assert_eq!(a.copy_trees.len(), 1);
        assert_eq!(
            (
                a.copy_trees[0].left.as_str(),
                a.copy_trees[0].right.as_str()
            ),
            ("/L", "/R")
        );
        assert_eq!(a.copy_trees[0].groups.len(), 3);
        let left: Vec<&str> = a.file_groups.iter().map(|g| g.hash.as_str()).collect();
        assert_eq!(left, ["solo"]);
        let report = to_report(&a, groups.len());
        assert_eq!(report.raw_group_count, 7);
        assert_eq!(report.reclaimable_bytes, 40);
        assert_eq!(report.copy_trees[0].file_count, 3);
        let tid = report.copy_trees[0].id.clone();
        assert_eq!(hashes_for_id(Some(&report), &tid).len(), 3);
        assert_eq!(hashes_for_id(None, "abc"), vec!["abc".to_string()]);
        assert!(hashes_for_id(None, &tid).is_empty());

        let off = DuplicateOptions {
            hide_within_project: false,
            fold_copy_trees: false,
            ..Default::default()
        };
        let a2 = analyze(&groups, &off, &IgnoreList::default(), &mut projects, u);
        assert_eq!(a2.file_groups.len(), 7);
    }
}
