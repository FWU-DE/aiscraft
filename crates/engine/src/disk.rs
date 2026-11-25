//! Every file the engine writes goes through here: crash-safe through craft-atomic, and a full
//! disk is named as such ([`Error::io`]) instead of an unexpected error.

use std::path::Path;

use aiscraft_core::{Error, Result};
use craft_atomic::{AtomicFs, NewFilePermissions, RealFs, RenameRetry, WriteOptions};

/// Replace `path` with `bytes` (exports, saved files, cache files).
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_with(path, bytes, NewFilePermissions::PlatformDefault)
}

/// Like [`write`], with mode 0600 from the first byte (secrets).
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    write_with(path, bytes, NewFilePermissions::Private)
}

fn write_with(path: &Path, bytes: &[u8], permissions: NewFilePermissions) -> Result<()> {
    WriteOptions::default()
        .with_new_file_permissions(permissions)
        .atomic_write_custom(fs_for(path), RenameRetry::platform(), path, bytes)
        .map_err(|e| Error::io(path.display(), &e))
}

/// Create `path` with `bytes`; never replaces a file (attachments). `what` names it in errors.
pub(crate) fn write_new(path: &Path, bytes: &[u8], what: &str) -> Result<()> {
    WriteOptions::default().write_new_custom(fs_for(path), path, bytes).map_err(|e| Error::io(what, &e))
}

/// Mark a program file of an installed package as executable (Unix); elsewhere nothing to do.
pub(crate) fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path).map_err(|e| Error::io(path.display(), &e))?;
        let mut perms = meta.permissions();
        if perms.mode() & 0o100 == 0 {
            perms.set_mode(perms.mode() | 0o700);
            std::fs::set_permissions(path, perms).map_err(|e| Error::io(path.display(), &e))?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Create a folder and its parents.
pub(crate) fn create_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io(format_args!("Ordner {}", dir.display()), &e))
}

#[cfg(not(test))]
fn fs_for(_path: &Path) -> &'static dyn AtomicFs {
    &RealFs
}

#[cfg(test)]
fn fs_for(path: &Path) -> &'static dyn AtomicFs {
    if test_hook::is_full(path) { &test_hook::FullFs } else { &RealFs }
}

/// Tests simulate a full disk for chosen paths (no real disk gets filled): the staged file is
/// created, then flushing it fails with `StorageFull`, as on a full file system.
#[cfg(test)]
pub(crate) mod test_hook {
    use std::fs::File;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use craft_atomic::{AtomicFs, NewFilePermissions, RealFs};

    static FULL: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    /// Writes to `path` fail as if the disk were full. Paths are unique per test, so tests
    /// running in parallel do not see each other's full disks.
    pub(crate) fn fill_disk_at(path: &Path) {
        FULL.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(path.to_path_buf());
    }

    pub(super) fn is_full(path: &Path) -> bool {
        FULL.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().any(|p| p == path)
    }

    pub(super) struct FullFs;

    impl AtomicFs for FullFs {
        fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
            RealFs.create_new(path, permissions)
        }
        fn sync(&self, _file: &File) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::StorageFull))
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            RealFs.rename(from, to)
        }
        fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
            RealFs.hard_link(from, to)
        }
        fn remove(&self, path: &Path) -> io::Result<()> {
            RealFs.remove(path)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            RealFs.sync_dir(dir)
        }
        fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()> {
            RealFs.set_permissions(file, perms)
        }
        #[cfg(unix)]
        fn set_group(&self, file: &File, gid: u32) -> io::Result<()> {
            RealFs.set_group(file, gid)
        }
    }
}
