//! Where oversized tool output lives.
//!
//! Spill is scratch data: a redacted copy of one tool result that the model may
//! read once. The home carries only durable state, so the root is under the
//! system temp folder and may vanish between turns.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The fixed segment below the temp base.
const SPILL_ROOT: &str = "hearth/spill";
/// The mode every spill directory is created and kept with.
#[cfg(unix)]
const PRIVATE_DIR: u32 = 0o700;

/// The spill root for this process.
pub fn root() -> PathBuf {
    root_from(std::env::var_os("TMPDIR"))
}

/// The spill root for one temp base value.
///
/// An empty or relative `TMPDIR` is treated as unset, because either would put
/// the root in the daemon's working directory instead of the temp folder.
pub(crate) fn root_from(value: Option<OsString>) -> PathBuf {
    match value {
        Some(value) if usable_base(&value) => PathBuf::from(value).join(SPILL_ROOT),
        _ => PathBuf::from("/tmp").join(SPILL_ROOT),
    }
}

fn usable_base(value: &OsStr) -> bool {
    !value.is_empty() && Path::new(value).is_absolute()
}

/// Create the root privately, or refuse a root that is not safe to write.
pub fn ensure_root(root: &Path) -> io::Result<()> {
    ensure_root_for(root, current_uid())
}

/// Create the root for a known owner, for the ownership check's test.
#[cfg(test)]
pub(crate) fn ensure_root_owned(root: &Path, owner: u32) -> io::Result<()> {
    ensure_root_for(root, Some(owner))
}

fn ensure_root_for(root: &Path, owner: Option<u32>) -> io::Result<()> {
    let parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        ensure_private_directory(parent, owner)?;
    }
    ensure_private_directory(root, owner)
}

/// Create one directory with mode 0700, or verify that the existing one is safe.
fn ensure_private_directory(path: &Path, owner: Option<u32>) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is a symlink, not a directory", path.display()),
                ));
            }
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} is not a directory", path.display()),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if let Some(owner) = owner
                    && metadata.uid() != owner
                {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("{} is not owned by the current user", path.display()),
                    ));
                }
                fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIR))?;
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(PRIVATE_DIR);
            }
            builder.create(path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIR))?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// The effective uid of this process, when the host can report it safely.
///
/// The kernel owns `/proc/<pid>` under the process's euid, so its metadata gives
/// the owner without an `unsafe` libc call.
#[cfg(unix)]
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

#[cfg(not(unix))]
fn current_uid() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_tmpdir_uses_tmp() {
        assert_eq!(
            root_from(None),
            PathBuf::from("/tmp/hearth/spill"),
            "an unset TMPDIR must fall back to /tmp"
        );
    }

    #[test]
    fn an_empty_tmpdir_is_unset() {
        assert_eq!(
            root_from(Some(OsString::new())),
            PathBuf::from("/tmp/hearth/spill"),
            "an empty TMPDIR must not write a relative spill/ into the cwd"
        );
    }

    #[test]
    fn a_tmpdir_with_a_path_is_honoured() {
        assert_eq!(
            root_from(Some(OsString::from("/var/tmp"))),
            PathBuf::from("/var/tmp/hearth/spill")
        );
    }

    #[test]
    fn a_relative_tmpdir_is_unset() {
        assert_eq!(
            root_from(Some(OsString::from("tmp"))),
            PathBuf::from("/tmp/hearth/spill"),
            "a relative temp base would put the root in the daemon's cwd"
        );
    }

    #[test]
    fn a_missing_root_is_created_on_demand() {
        let home = tempfile::TempDir::new().expect("temp home");
        let root = home.path().join(SPILL_ROOT);
        ensure_root(&root).expect("create");
        assert!(root.is_dir());
        ensure_root(&root).expect("an existing private root is accepted");
    }

    #[cfg(unix)]
    #[test]
    fn the_root_is_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::TempDir::new().expect("temp home");
        let root = home.path().join(SPILL_ROOT);
        ensure_root(&root).expect("create");
        let mode = fs::metadata(&root).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the spill root mode was {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_root_is_refused() {
        let home = tempfile::TempDir::new().expect("temp home");
        let target = home.path().join("elsewhere");
        fs::create_dir_all(&target).expect("target");
        let root = home.path().join(SPILL_ROOT);
        fs::create_dir_all(root.parent().expect("parent")).expect("parent");
        std::os::unix::fs::symlink(&target, &root).expect("symlink");

        let error = ensure_root(&root).expect_err("a symlink root must be refused");
        assert!(error.to_string().contains("symlink"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_root_owned_by_another_uid_is_refused() {
        let home = tempfile::TempDir::new().expect("temp home");
        let root = home.path().join(SPILL_ROOT);
        ensure_root(&root).expect("create");

        let error = ensure_root_owned(&root, u32::MAX).expect_err("another owner must be refused");
        assert!(error.to_string().contains("owned"), "{error}");
    }
}
