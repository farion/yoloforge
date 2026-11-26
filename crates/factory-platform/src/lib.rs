//! OS primitives used by managed runtimes. Security checks must not become no-ops on Windows.
use anyhow::{ensure, Result};
use std::{fs, path::Path};
pub mod runner_control;

pub const GUI_LIFETIME_PIPE: &str = "YOLOFORGE_GUI_LIFETIME_PIPE";

/// Only GUI-owned services use stdin as a lifetime pipe. A plain thread lets
/// independent shutdown paths exit without joining a blocked async stdin read.
pub fn watch_gui_lifetime(on_close: impl FnOnce() + Send + 'static) -> Result<()> {
    if std::env::var_os(GUI_LIFETIME_PIPE).as_deref() == Some(std::ffi::OsStr::new("1")) {
        std::thread::Builder::new().name("gui-lifetime".into()).spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            on_close();
        })?;
    }
    Ok(())
}

#[cfg(windows)]
mod windows;

pub fn private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    windows::restrict(path)?;
    ensure_private(path)
}

pub fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    #[cfg(windows)]
    windows::create_private_dir(path)?;
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_dir(),
        "Private directory must not be a symlink: {}",
        path.display()
    );
    ensure_private(path)
}

pub fn ensure_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o077 == 0,
            "Runtime path must be owned by this user and private: {}",
            path.display()
        );
    }
    #[cfg(windows)]
    windows::ensure_private(path)?;
    Ok(())
}

pub fn set_private_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    windows::restrict(path)?;
    Ok(())
}

pub fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        // MCPs are launched directly, never by implicitly invoking cmd.exe.
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com"))
    }
}

/// Windows has no supported equivalent of fsync on a directory handle.
/// File data is flushed before atomic publication on every platform.
pub fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

/// Open a regular file and verify the *opened handle's* location, closing the
/// canonicalize/open race when an agent renames a directory or swaps a symlink.
pub fn open_rooted(path: &Path, roots: &[impl AsRef<Path>]) -> Result<fs::File> {
    let path = path.canonicalize()?;
    ensure!(
        roots.iter().any(|r| path.starts_with(r)),
        "File outside allowed roots"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(&path)?;
    ensure!(file.metadata()?.is_file(), "Not a regular file");
    let opened = opened_path(&file)?;
    ensure!(
        roots.iter().any(|r| opened.starts_with(r)),
        "Opened file outside allowed roots"
    );
    Ok(file)
}

#[cfg(target_os = "linux")]
fn opened_path(file: &fs::File) -> Result<std::path::PathBuf> {
    use std::os::fd::AsRawFd;
    Ok(fs::canonicalize(format!(
        "/proc/self/fd/{}",
        file.as_raw_fd()
    ))?)
}

#[cfg(target_os = "macos")]
fn opened_path(file: &fs::File) -> Result<std::path::PathBuf> {
    use std::{
        ffi::CStr,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };
    let mut buffer = [0 as libc::c_char; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path into a PATH_MAX buffer.
    ensure!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == 0,
        "Cannot resolve opened file: {}",
        std::io::Error::last_os_error()
    );
    let path = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    Ok(fs::canonicalize(std::ffi::OsStr::from_bytes(
        path.to_bytes(),
    ))?)
}

#[cfg(windows)]
fn opened_path(file: &fs::File) -> Result<std::path::PathBuf> {
    windows::opened_path(file)
}

/// Atomically publish a prepared directory without ever replacing another project.
pub fn rename_noreplace(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(windows)]
    windows::rename_noreplace(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_never_replaces_an_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("prepared");
        let target = root.path().join("project");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("user-work"), "keep").unwrap();
        assert!(rename_noreplace(&source, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("user-work")).unwrap(),
            "keep"
        );
        fs::remove_file(target.join("user-work")).unwrap();
        fs::remove_dir(&target).unwrap();
        rename_noreplace(&source, &target).unwrap();
        assert!(target.is_dir());
    }

    #[test]
    fn rooted_reads_reject_outside_files_and_directories() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let allowed = root.path().canonicalize().unwrap();
        fs::write(root.path().join("result"), b"result").unwrap();
        fs::write(outside.path().join("secret"), b"secret").unwrap();
        assert!(open_rooted(&root.path().join("result"), &[&allowed]).is_ok());
        assert!(open_rooted(&outside.path().join("secret"), &[&allowed]).is_err());
        assert!(open_rooted(root.path(), &[&allowed]).is_err());
    }

    #[test]
    fn managed_directories_are_private_and_reusable() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("runtime");
        private_dir(&private).unwrap();
        private_dir(&private).unwrap();
        ensure_private(&private).unwrap();
    }

    #[test]
    fn newly_created_secret_files_can_be_restricted_and_rechecked() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret");
        fs::write(&path, b"fixture").unwrap();
        private_file(&path).unwrap();
        ensure_private(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"fixture");
    }

    #[test]
    #[cfg(unix)]
    fn existing_public_runtime_directories_are_rejected_not_silently_changed() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(root.path()).is_err());
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
