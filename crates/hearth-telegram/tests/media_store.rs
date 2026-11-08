use std::fs;

use hearth_telegram::{InboundMedia, MediaStoreError};
use tempfile::TempDir;

#[test]
fn a_saved_file_uses_the_utc_date_and_private_mode() {
    let root = TempDir::new().expect("temp media root");
    let store = InboundMedia::new(root.path());
    let path = store
        .save(1_700_000_000, 7, "photo.jpg", b"abc")
        .expect("save media");

    assert_eq!(path, root.path().join("2023-11-14/7-photo.jpg"));
    assert_eq!(fs::read(&path).expect("read media"), b"abc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn a_traversal_name_cannot_leave_the_date_directory() {
    let root = TempDir::new().expect("temp media root");
    let store = InboundMedia::new(root.path());
    let path = store
        .save(1_700_000_000, 8, "../../evil name.txt", b"x")
        .expect("save media");

    let date = root.path().join("2023-11-14");
    assert_eq!(path.parent(), Some(date.as_path()), "path: {}", path.display());
    let name = path.file_name().expect("file name").to_string_lossy();
    assert!(name.starts_with("8-"), "name: {name}");
    assert!(!name.contains('/') && !name.contains('\\'), "name: {name}");
    assert!(!path.components().any(|part| part.as_os_str() == ".."));
}

#[test]
fn a_missing_name_still_saves() {
    let root = TempDir::new().expect("temp media root");
    let store = InboundMedia::new(root.path());
    let path = store
        .save(1_700_000_000, 9, "", b"x")
        .expect("save media");
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("9-file")
    );
    assert!(!matches!(
        store.save(1_700_000_000, 9, "/", b"x"),
        Err(MediaStoreError::Invalid(_))
    ));
}
