//! Where oversized tool output lives.
//!
//! Spill is scratch data: a redacted copy of one tool result that the model may
//! read once. The home carries only durable state, so the root is under the
//! system temp folder and may vanish between turns.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The fixed segment below the temp base.
const SPILL_ROOT: &str = "hearth/spill";

/// The spill root for this process.
pub fn root() -> PathBuf {
    root_from(std::env::var_os("TMPDIR"))
}

/// The spill root for one temp base value.
///
/// The value is the environment's, not the process's, so a test can exercise an
/// empty or relative `TMPDIR` without writing outside its own directory.
pub(crate) fn root_from(value: Option<OsString>) -> PathBuf {
    let _ = value;
    PathBuf::from("/tmp").join(SPILL_ROOT)
}

/// Create the root privately, or refuse a root that is not safe to write.
pub fn ensure_root(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)
}

/// Create the root for a known owner.
pub(crate) fn ensure_root_owned(root: &Path, owner: u32) -> io::Result<()> {
    let _ = owner;
    ensure_root(root)
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
        assert!(error.to_string().contains("owner"), "{error}");
    }
}
