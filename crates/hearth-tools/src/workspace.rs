//! Confined path resolution for files and spill output.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::tool::ToolError;

/// One directory entry returned by a list call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
}

/// A workspace root plus optional spill and inbound-media roots for reads.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    spill: PathBuf,
    media: Option<PathBuf>,
    allow_absolute: bool,
}

impl Workspace {
    /// Build a workspace with a separate spill root.
    pub fn new(root: impl Into<PathBuf>, spill: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            spill: spill.into(),
            media: None,
            allow_absolute: false,
        }
    }

    /// Build a workspace with an additional read-only inbound-media root.
    pub fn with_media(
        root: impl Into<PathBuf>,
        spill: impl Into<PathBuf>,
        media: impl Into<PathBuf>,
    ) -> Self {
        Self {
            root: root.into(),
            spill: spill.into(),
            media: Some(media.into()),
            allow_absolute: true,
        }
    }

    /// Build a workspace where reads and writes share one root.
    pub fn root_only(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            root: root.clone(),
            spill: root,
            media: None,
            allow_absolute: false,
        }
    }

    /// Read a text file. A path starting with `spill/` reads the spill root,
    /// `media/` reads the inbound-media root, and an absolute path must live
    /// under one of the configured roots.
    pub fn read_text(&self, path: &str) -> Result<String, ToolError> {
        let target = self.resolve_read(path)?;
        if !target.is_file() {
            return Err(ToolError::invalid(
                "not_a_file",
                format!("not a file: {path}"),
            ));
        }
        fs::read_to_string(&target).map_err(ToolError::from)
    }

    /// Read file bytes under the workspace, spill, or inbound-media root.
    pub fn read_bytes(&self, path: &str) -> Result<Vec<u8>, ToolError> {
        let target = self.resolve_read(path)?;
        if !target.is_file() {
            return Err(ToolError::invalid(
                "not_a_file",
                format!("not a file: {path}"),
            ));
        }
        fs::read(&target).map_err(ToolError::from)
    }

    /// Write a text file under the workspace root.
    pub fn write_text(&self, path: &str, content: &str) -> Result<(), ToolError> {
        let target = self.resolve_write(path)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&target)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// List a directory under the workspace root or the spill root.
    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntryInfo>, ToolError> {
        let target = self.resolve_dir(path)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&target)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let file_type = metadata.file_type();
            let kind = if file_type.is_dir() {
                "dir"
            } else if file_type.is_symlink() {
                "symlink"
            } else if file_type.is_file() {
                "file"
            } else {
                "other"
            };
            entries.push(DirEntryInfo {
                name: entry.file_name().to_string_lossy().to_string(),
                kind,
                size: metadata.len(),
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    /// Resolve a directory strictly under the workspace root.
    pub fn resolve_workspace_directory(&self, path: &str) -> Result<PathBuf, ToolError> {
        let relative = validate_relative(path)?;
        fs::create_dir_all(&self.root)?;
        let root = fs::canonicalize(&self.root)?;
        let target = fs::canonicalize(root.join(relative))
            .map_err(|_| ToolError::invalid("not_found", format!("not found: {path}")))?;
        if !target.starts_with(&root) {
            return Err(escape(path));
        }
        if !target.is_dir() {
            return Err(ToolError::invalid(
                "not_a_dir",
                format!("not a directory: {path}"),
            ));
        }
        Ok(target)
    }

    /// Resolve a readable path under the workspace or spill root.
    pub fn resolve_read_path(&self, path: &str) -> Result<PathBuf, ToolError> {
        self.resolve_read(path)
    }

    fn resolve_read(&self, path: &str) -> Result<PathBuf, ToolError> {
        if Path::new(path).is_absolute() {
            if !self.allow_absolute {
                return Err(ToolError::invalid("invalid_path", "path must be relative"));
            }
            let target = fs::canonicalize(path)
                .map_err(|_| ToolError::invalid("not_found", format!("not found: {path}")))?;
            let roots = self.read_roots()?;
            if !roots.iter().any(|root| target.starts_with(root)) {
                return Err(escape(path));
            }
            return Ok(target);
        }
        let (root, relative) = self.split_read(path)?;
        let root = fs::canonicalize(&root)
            .map_err(|_| ToolError::invalid("missing_root", "the root does not exist"))?;
        let target = fs::canonicalize(root.join(relative))
            .map_err(|_| ToolError::invalid("not_found", format!("not found: {path}")))?;
        if !target.starts_with(&root) {
            return Err(escape(path));
        }
        Ok(target)
    }

    fn read_roots(&self) -> Result<Vec<PathBuf>, ToolError> {
        let mut roots = vec![
            fs::canonicalize(&self.root).map_err(|_| {
                ToolError::invalid("missing_root", "the workspace root does not exist")
            })?,
            fs::canonicalize(&self.spill)
                .map_err(|_| ToolError::invalid("missing_root", "the spill root does not exist"))?,
        ];
        if let Some(media) = &self.media {
            roots.push(fs::canonicalize(media).map_err(|_| {
                ToolError::invalid("missing_root", "the media root does not exist")
            })?);
        }
        Ok(roots)
    }

    fn resolve_write(&self, path: &str) -> Result<PathBuf, ToolError> {
        let relative = validate_relative(path)?;
        if relative
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == "spill")
        {
            return Err(ToolError::invalid(
                "invalid_path",
                "writes may not use the spill prefix",
            ));
        }
        fs::create_dir_all(&self.root)?;
        let root = fs::canonicalize(&self.root)?;
        let target = root.join(&relative);
        let parent = target
            .parent()
            .ok_or_else(|| ToolError::invalid("invalid_path", "path has no parent"))?;
        fs::create_dir_all(parent)?;
        let canonical_parent = fs::canonicalize(parent)?;
        if !canonical_parent.starts_with(&root) {
            return Err(escape(path));
        }
        match fs::symlink_metadata(&target) {
            Ok(_) => {
                let canonical_target = fs::canonicalize(&target).map_err(|_| escape(path))?;
                if !canonical_target.starts_with(&root) {
                    return Err(escape(path));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(target)
    }

    fn resolve_dir(&self, path: &str) -> Result<PathBuf, ToolError> {
        let (root, relative) = self.split_read(path)?;
        let root = fs::canonicalize(&root)
            .map_err(|_| ToolError::invalid("missing_root", "the root does not exist"))?;
        let target = fs::canonicalize(root.join(relative))
            .map_err(|_| ToolError::invalid("not_found", format!("not found: {path}")))?;
        if !target.starts_with(&root) {
            return Err(escape(path));
        }
        if !target.is_dir() {
            return Err(ToolError::invalid(
                "not_a_dir",
                format!("not a directory: {path}"),
            ));
        }
        Ok(target)
    }

    fn split_read(&self, path: &str) -> Result<(PathBuf, PathBuf), ToolError> {
        let relative = validate_relative(path)?;
        let mut components = relative.components();
        if let Some(Component::Normal(first)) = components.next()
            && first == "spill"
        {
            let rest: PathBuf = components.collect();
            if rest.as_os_str().is_empty() {
                return Err(ToolError::invalid(
                    "invalid_path",
                    "a spill path needs a file name",
                ));
            }
            return Ok((self.spill.clone(), rest));
        }
        if let Some(Component::Normal(first)) = relative.components().next()
            && first == "media"
        {
            let media = self.media.as_ref().ok_or_else(|| {
                ToolError::invalid("invalid_path", "the inbound-media root is not configured")
            })?;
            let rest: PathBuf = relative.components().skip(1).collect();
            if rest.as_os_str().is_empty() {
                return Err(ToolError::invalid(
                    "invalid_path",
                    "a media path needs a file name",
                ));
            }
            return Ok((media.clone(), rest));
        }
        Ok((self.root.clone(), relative))
    }
}

fn validate_relative(path: &str) -> Result<PathBuf, ToolError> {
    if path.trim().is_empty() {
        return Err(ToolError::invalid("invalid_path", "path is empty"));
    }
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(ToolError::invalid("invalid_path", "path must be relative"));
    }
    for component in relative.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(ToolError::invalid(
                    "invalid_path",
                    "path must not contain .. or a root",
                ));
            }
        }
    }
    Ok(relative.to_path_buf())
}

fn escape(path: &str) -> ToolError {
    ToolError::invalid("path_escape", format!("path escapes the workspace: {path}"))
}
