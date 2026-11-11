//! Small multi-line text editor used by the operator surface.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a text editor is changing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorKind {
    Settings,
    Soul,
    Guidance,
    Skill { name: String },
}

/// One open editor and the save behavior attached to it.
#[derive(Debug, Clone)]
pub struct OpenEditor {
    pub kind: EditorKind,
    pub text: TextEditor,
}

/// One editable text file.
#[derive(Debug, Clone)]
pub struct TextEditor {
    pub path: PathBuf,
    pub title: String,
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    pub dirty: bool,
    pub confirm_discard: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
}

impl TextEditor {
    /// Read a file, or start empty when it does not exist yet.
    pub fn open(path: impl Into<PathBuf>, title: impl Into<String>) -> io::Result<Self> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        Ok(Self::from_text(path, title, text))
    }

    /// Build an editor from text already in memory.
    pub fn from_text(path: impl Into<PathBuf>, title: impl Into<String>, text: String) -> Self {
        let text = text
            .replace("\r\n", "\n")
            .replace('\r', "")
            .replace('\t', "    ");
        let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        Self {
            path: path.into(),
            title: title.into(),
            lines,
            row: 0,
            col: 0,
            dirty: false,
            confirm_discard: false,
            error: None,
            notice: None,
        }
    }

    /// The complete file text.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replace the buffer text and keep the cursor inside it.
    pub fn replace_text(&mut self, text: String) {
        let text = text
            .replace("\r\n", "\n")
            .replace('\r', "")
            .replace('\t', "    ");
        self.lines = text.split('\n').map(str::to_owned).collect();
        self.row = self.row.min(self.lines.len().saturating_sub(1));
        self.col = self.col.min(self.line().chars().count());
    }

    /// The current line.
    pub fn line(&self) -> &str {
        self.lines.get(self.row).map(String::as_str).unwrap_or("")
    }

    /// Consume one editing key.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Home => {
                    self.row = 0;
                    self.col = 0;
                    true
                }
                KeyCode::End => {
                    self.row = self.lines.len().saturating_sub(1);
                    self.col = self.line().chars().count();
                    true
                }
                _ => false,
            };
        }
        match key.code {
            KeyCode::Char(character) => {
                self.insert_text(&character.to_string());
                true
            }
            KeyCode::Enter => {
                let indent: String = self
                    .line()
                    .chars()
                    .take_while(|character| *character == ' ')
                    .collect();
                self.insert_text(&format!("\n{indent}"));
                true
            }
            KeyCode::Tab => {
                self.insert_text("    ");
                true
            }
            KeyCode::Backspace => {
                self.backspace();
                true
            }
            KeyCode::Delete => {
                self.delete();
                true
            }
            KeyCode::Left => {
                self.left();
                true
            }
            KeyCode::Right => {
                self.right();
                true
            }
            KeyCode::Up => {
                self.up();
                true
            }
            KeyCode::Down => {
                self.down();
                true
            }
            KeyCode::Home => {
                self.col = 0;
                true
            }
            KeyCode::End => {
                self.col = self.line().chars().count();
                true
            }
            KeyCode::PageUp => {
                self.row = self.row.saturating_sub(15);
                self.clamp_col();
                true
            }
            KeyCode::PageDown => {
                self.row = (self.row + 15).min(self.lines.len().saturating_sub(1));
                self.clamp_col();
                true
            }
            _ => false,
        }
    }

    /// Insert text at the cursor, including newlines from a paste.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "");
        let parts: Vec<&str> = text.split('\n').collect();
        let line = self.lines.get(self.row).cloned().unwrap_or_default();
        let split = byte_index(&line, self.col);
        let before = line[..split].to_owned();
        let after = line[split..].to_owned();
        if parts.len() == 1 {
            self.lines[self.row] = format!("{before}{}{after}", parts[0]);
            self.col += parts[0].chars().count();
        } else {
            self.lines[self.row] = format!("{before}{}", parts[0]);
            let last = parts.len() - 1;
            for part in &parts[1..last] {
                self.row += 1;
                self.lines.insert(self.row, (*part).to_owned());
            }
            self.row += 1;
            let tail = format!("{}{after}", parts[last]);
            self.lines.insert(self.row, tail);
            self.col = parts[last].chars().count();
        }
        self.mark_dirty();
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            let line = self.lines[self.row].clone();
            let start = byte_index(&line, self.col - 1);
            let end = byte_index(&line, self.col);
            self.lines[self.row].replace_range(start..end, "");
            self.col -= 1;
            self.mark_dirty();
        } else if self.row > 0 {
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&current);
            self.mark_dirty();
        }
    }

    fn delete(&mut self) {
        let line = self.lines[self.row].clone();
        if self.col < line.chars().count() {
            let start = byte_index(&line, self.col);
            let end = byte_index(&line, self.col + 1);
            self.lines[self.row].replace_range(start..end, "");
            self.mark_dirty();
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
            self.mark_dirty();
        }
    }

    fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line().chars().count();
        }
    }

    fn right(&mut self) {
        if self.col < self.line().chars().count() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.clamp_col();
        }
    }

    fn down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.clamp_col();
        }
    }

    fn clamp_col(&mut self) {
        self.col = self.col.min(self.line().chars().count());
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.confirm_discard = false;
        self.notice = None;
    }

    /// Write the file atomically with mode 0600 where supported.
    pub fn save(&self) -> io::Result<()> {
        write_private(&self.path, &self.text())
    }
}

/// Write one file atomically with mode 0600 where supported.
pub fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_enter_and_backspace_track_the_cursor() {
        let mut editor = TextEditor::from_text("x", "x", "    alpha\nbeta".to_owned());
        editor.handle_key(key(KeyCode::End));
        editor.handle_key(key(KeyCode::Enter));
        editor.insert_text("gamma");
        assert_eq!(editor.text(), "    alpha\n    gamma\nbeta");
        assert_eq!(editor.row, 1);
        editor.handle_key(key(KeyCode::Home));
        editor.handle_key(key(KeyCode::Backspace));
        assert_eq!(editor.text(), "    alpha    gamma\nbeta");
        assert_eq!(editor.row, 0);
    }

    #[test]
    fn paste_can_insert_multiple_lines() {
        let mut editor = TextEditor::from_text("x", "x", "ab".to_owned());
        editor.col = 1;
        editor.insert_text("1\n2\n3");
        assert_eq!(editor.text(), "a1\n2\n3b");
        assert_eq!(editor.row, 2);
        assert_eq!(editor.col, 1);
    }

    #[test]
    fn save_writes_the_file_with_mode_0600() {
        let home = tempfile::TempDir::new().expect("temp home");
        let path = home.path().join("note.md");
        let mut editor = TextEditor::from_text(&path, "note.md", "old".to_owned());
        editor.insert_text("new ");
        editor.save().expect("save");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new old");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("mode").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn control_keys_do_not_insert_text() {
        let mut editor = TextEditor::from_text("x", "x", "body".to_owned());
        let consumed = editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!consumed);
        assert_eq!(editor.text(), "body");
    }
}
