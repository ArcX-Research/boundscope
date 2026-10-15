//! Bounded reads anchored to open directories. Do not replace these with a
//! metadata check followed by a pathname open: a component can change in between.
use crate::{Result, config::relative_path};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::Path,
};

pub struct Directory(File);

impl Directory {
    pub fn open(path: &Path) -> Result<Self> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map(Self)
            .map_err(|e| format!("opening directory: {e}"))
    }

    fn open_relative(&self, path: &Path, directory: bool) -> Result<File> {
        relative_path(path)?;
        let mut parent = self.0.try_clone().map_err(|e| e.to_string())?;
        let mut parts = path.components().peekable();
        while let Some(part) = parts.next() {
            let name = CString::new(part.as_os_str().as_bytes()).map_err(|_| "NUL in path")?;
            let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
            if parts.peek().is_some() || directory {
                flags |= libc::O_DIRECTORY;
            }
            // parent is a live owned descriptor; name is NUL-terminated. No
            // creation flag is used. Every component is opened without symlinks.
            let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(format!(
                    "opening declared path: {}",
                    io::Error::last_os_error()
                ));
            }
            // openat returned a new descriptor. File now exclusively owns it.
            parent = unsafe { File::from_raw_fd(fd) };
        }
        Ok(parent)
    }

    pub fn directory(&self, path: &Path) -> Result<Self> {
        self.open_relative(path, true).map(Self)
    }

    pub fn read(&self, path: &Path, limit: usize) -> Result<Vec<u8>> {
        read_file(self.open_relative(path, false)?, limit)
    }
}

pub fn read_file(file: File, limit: usize) -> Result<Vec<u8>> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(format!("expected a regular file of at most {limit} bytes"));
    }
    let bound = limit.checked_add(1).ok_or("file size bound overflow")?;
    let mut bytes = Vec::new();
    file.take(bound as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("file exceeds {limit} bytes"));
    }
    Ok(bytes)
}

/// Publish without an exists()/rename() race. Unsupported filesystems fail
/// closed; never fall back to a rename that could replace existing evidence.
pub fn rename_new(source: &Path, destination: &Path) -> Result<()> {
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| "NUL in path")?;
    let destination =
        CString::new(destination.as_os_str().as_bytes()).map_err(|_| "NUL in path")?;
    // Both C strings are live for the call. Exclusive rename is supplied by the
    // two supported platforms and refuses an existing file, directory or link.
    #[cfg(target_os = "macos")]
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Err("exclusive report publication requires macOS or Linux".into());
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if result != 0 {
        return Err(format!(
            "publishing without replacing existing evidence: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn opened_directory_survives_rename_and_rejects_link_substitution() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("project")).unwrap();
        fs::create_dir(root.path().join("outside")).unwrap();
        fs::write(root.path().join("project/input"), b"original").unwrap();
        fs::write(root.path().join("outside/input"), b"secret").unwrap();
        let opened = Directory::open(&root.path().join("project")).unwrap();
        fs::rename(root.path().join("project"), root.path().join("moved")).unwrap();
        symlink("outside", root.path().join("project")).unwrap();
        assert_eq!(opened.read(Path::new("input"), 20).unwrap(), b"original");
        let parent = Directory::open(root.path()).unwrap();
        assert!(parent.read(Path::new("project/input"), 20).is_err());
        symlink("../outside/input", root.path().join("moved/link")).unwrap();
        assert!(opened.read(Path::new("link"), 20).is_err());
        assert!(opened.read(Path::new("../outside/input"), 20).is_err());
    }

    #[test]
    fn publication_does_not_replace_even_an_empty_directory_or_dangling_link() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("staging");
        let destination = root.path().join("saved");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("evidence"), b"new").unwrap();
        fs::create_dir(&destination).unwrap();
        assert!(rename_new(&source, &destination).is_err());
        assert!(!destination.join("evidence").exists());
        fs::remove_dir(&destination).unwrap();
        symlink("missing", &destination).unwrap();
        assert!(rename_new(&source, &destination).is_err());
        fs::remove_file(&destination).unwrap();
        rename_new(&source, &destination).unwrap();
        assert_eq!(fs::read(destination.join("evidence")).unwrap(), b"new");
    }
}
