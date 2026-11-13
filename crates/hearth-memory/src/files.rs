//! Atomic file replacement, ported from `src/files.ts`.
//!
//! `standing.md` is replaced by writing a sibling temporary file and renaming it.
//! A reader therefore sees either the previous complete file or the new complete
//! file, never a half-written one. This matters because the standing file is injected
//! into the prompt on every turn.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::errors::Result;

/// Replace `path` atomically with `text`.
///
/// The temporary file is created in the same directory so the rename stays on
/// one filesystem, which is what makes it atomic. The file is created mode 0600
/// and flushed before the rename, then the directory is synced so the rename
/// itself survives a crash.
pub fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("memory"),
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<()> {
        let mut file = {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&temporary)?
        };
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        // Directory fsync is not supported on every platform; a failure here is
        // not a reason to report the write as failed.
        if let Ok(handle) = File::open(directory) {
            let _ = handle.sync_all();
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// True when an I/O error means the file simply is not there.
pub fn is_missing(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_replaces_the_whole_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("standing.md");
        atomic_write(&path, "first\n").unwrap();
        atomic_write(&path, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("standing.md");
        atomic_write(&path, "content\n").unwrap();
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn a_missing_file_is_recognized() {
        let directory = tempfile::tempdir().unwrap();
        let error = fs::read_to_string(directory.path().join("nope.md")).unwrap_err();
        assert!(is_missing(&error));
    }
}
