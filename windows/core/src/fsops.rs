//! Low-level file operations that never overwrite.

use std::io;
use std::path::Path;

/// Rename that fails with `AlreadyExists` instead of replacing the destination.
/// (std's `rename` replaces existing files on both Windows and Linux.)
pub(crate) fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
    imp::rename_noreplace(src, dst)
}

/// Is this the "different volume" error of a rename?
pub(crate) fn is_cross_device(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::CrossesDevices {
        return true;
    }
    match e.raw_os_error() {
        Some(code) if cfg!(windows) => code == 17, // ERROR_NOT_SAME_DEVICE
        Some(code) => code == libc_exdev(),
        None => false,
    }
}

#[cfg(unix)]
fn libc_exdev() -> i32 {
    libc::EXDEV
}

#[cfg(not(unix))]
fn libc_exdev() -> i32 {
    18
}

/// Move a file or folder; never overwrites. Across volumes: copy everything (files keep
/// their modification time), then remove the source. A failed copy is rolled back (the
/// partial copy is removed, the source untouched). If removing the source fails after a
/// complete copy, the move still counts: everything is safe at the destination and what
/// could not be removed stays where it was.
pub(crate) fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(dst).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination exists",
        ));
    }
    match rename_noreplace(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            if let Err(copy_err) = copy_tree(src, dst) {
                let _ = remove_tree(dst);
                return Err(copy_err);
            }
            let _ = remove_tree(src);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn remove_tree(p: &Path) -> io::Result<()> {
    let md = std::fs::symlink_metadata(p)?;
    if md.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

/// Recursive copy; refuses to overwrite anything; links are copied as links on unix and
/// skipped elsewhere (they are rare in user folders and cannot be recreated without
/// privileges on Windows).
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    let md = std::fs::symlink_metadata(src)?;
    let ft = md.file_type();
    if ft.is_symlink() {
        #[cfg(unix)]
        {
            let target = std::fs::read_link(src)?;
            return std::os::unix::fs::symlink(target, dst);
        }
        #[cfg(not(unix))]
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cannot move a link across volumes",
            ));
        }
    }
    if ft.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }
    let mut from = std::fs::File::open(src)?;
    let mut to = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    io::copy(&mut from, &mut to)?;
    if let Ok(m) = md.modified() {
        let _ = to.set_modified(m);
    }
    to.sync_all()?;
    Ok(())
}

/// Can items under `path` go to the Recycle Bin? On Windows only fixed drives have one;
/// on network shares and removable drives the shell would delete permanently.
pub(crate) fn has_recycle_bin(path: &str) -> bool {
    imp::has_recycle_bin(path)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub(super) fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
        let s = CString::new(src.as_os_str().as_bytes())?;
        let d = CString::new(dst.as_os_str().as_bytes())?;
        // SAFETY: both pointers are valid NUL-terminated strings for the duration of the call.
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                s.as_ptr(),
                libc::AT_FDCWD,
                d.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // file system without RENAME_NOREPLACE support: check, then rename
            Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::ENOTSUP) => {
                if std::fs::symlink_metadata(dst).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "destination exists",
                    ));
                }
                std::fs::rename(src, dst)
            }
            _ => Err(e),
        }
    }

    pub(super) fn has_recycle_bin(_path: &str) -> bool {
        true
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW, MoveFileExW};

    const DRIVE_FIXED: u32 = 3;

    /// NUL-terminated UTF-16; long absolute paths get the `\\?\` prefix (the raw Win32
    /// calls below do not add it the way std does).
    fn wide(p: &Path) -> Vec<u16> {
        let s = p.as_os_str().to_string_lossy();
        let long = if s.len() > 240 && !s.starts_with(r"\\?\") {
            let b = s.replace('/', "\\");
            if let Some(unc) = b.strip_prefix(r"\\") {
                Some(format!(r"\\?\UNC\{unc}"))
            } else if b.as_bytes().get(1) == Some(&b':') {
                Some(format!(r"\\?\{b}"))
            } else {
                None
            }
        } else {
            None
        };
        match long {
            Some(l) => std::ffi::OsStr::new(&l)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect(),
            None => p
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect(),
        }
    }

    pub(super) fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
        let s = wide(src);
        let d = wide(dst);
        // flags 0: no MOVEFILE_REPLACE_EXISTING (fails with ERROR_ALREADY_EXISTS) and no
        // MOVEFILE_COPY_ALLOWED (fails with ERROR_NOT_SAME_DEVICE across volumes → we copy).
        // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call.
        let ok = unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), 0) };
        if ok != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn has_recycle_bin(path: &str) -> bool {
        let p = wide(Path::new(path));
        let mut buf = vec![0u16; 1024];
        // SAFETY: `p` is NUL-terminated, `buf` has the length we pass.
        let ok = unsafe { GetVolumePathNameW(p.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        let root: Vec<u16> = if ok != 0 {
            buf
        } else {
            // fall back to the drive letter ("C:\"); anything else (UNC) is a network share
            let b = path.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                format!("{}:\\", b[0] as char)
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect()
            } else {
                return false;
            }
        };
        // SAFETY: `root` is a NUL-terminated UTF-16 string.
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        kind == DRIVE_FIXED
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(super) fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
        if std::fs::symlink_metadata(dst).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination exists",
            ));
        }
        std::fs::rename(src, dst)
    }

    pub(super) fn has_recycle_bin(_path: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_never_overwrites() {
        let d = tempfile::Builder::new()
            .prefix("mf-fsops")
            .tempdir()
            .unwrap();
        let a = d.path().join("a.txt");
        let b = d.path().join("b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let e = rename_noreplace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");
        assert!(move_path(&a, &b).is_err());
        let c = d.path().join("c.txt");
        move_path(&a, &c).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "a");
    }

    #[test]
    fn copy_tree_keeps_mtime_and_refuses_overwrite() {
        let d = tempfile::Builder::new()
            .prefix("mf-fsops")
            .tempdir()
            .unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/x.txt"), "x").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        filetime::set_file_mtime(
            src.join("sub/x.txt"),
            filetime::FileTime::from_system_time(old),
        )
        .unwrap();
        let dst = d.path().join("dst");
        copy_tree(&src, &dst).unwrap();
        let m = std::fs::metadata(dst.join("sub/x.txt"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(m, old);
        assert!(copy_tree(&src, &dst).is_err());
        assert!(is_cross_device(&io::Error::from(
            io::ErrorKind::CrossesDevices
        )));
    }
}
