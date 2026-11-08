//! The documents tool: extract readable text from PDF, DOCX, PPTX, and XLSX.

use std::ffi::OsStr;
use std::io::{Cursor, Read};
use std::path::Path;

use hearth_model::ToolSpec;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::{Value, json};
use zip::ZipArchive;

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// Reads documents from the workspace, spill, and inbound-media roots.
#[derive(Debug, Clone)]
pub struct DocumentsTool {
    workspace: Workspace,
}

impl DocumentsTool {
    /// Build the tool with the three read roots.
    pub fn new(
        workspace: impl Into<std::path::PathBuf>,
        spill: impl Into<std::path::PathBuf>,
        media: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            workspace: Workspace::with_media(workspace, spill, media),
        }
    }
}

impl Tool for DocumentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "documents".to_owned(),
            description: "Extract readable text from a PDF, DOCX, PPTX, or XLSX file in the workspace, spill, or inbound media folder.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "A document path inside the workspace, spill, or inbound media"}
                },
                "required": ["path"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "path is required"))?;
        let bytes = self.workspace.read_bytes(path)?;
        let extension = Path::new(path)
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let text = match extension.as_str() {
            "pdf" => pdf_text(&bytes)?,
            "docx" => docx_text(&bytes)?,
            "pptx" => pptx_text(&bytes)?,
            "xlsx" => xlsx_text(&bytes)?,
            other => {
                return Err(ToolError::invalid(
                    "unsupported_format",
                    format!("unsupported document format: {other:?}"),
                ));
            }
        };
        if text.trim().is_empty() {
            return Err(ToolError::invalid(
                "empty_document",
                "the document contains no extractable text",
            ));
        }
        Ok(ToolOutput {
            content: text,
            is_error: false,
        })
    }
}

fn pdf_text(bytes: &[u8]) -> Result<String, ToolError> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|error| {
        ToolError::invalid("corrupt_document", format!("could not read PDF: {error}"))
    })
}

fn archive(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, ToolError> {
    ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        ToolError::invalid(
            "corrupt_document",
            format!("invalid document archive: {error}"),
        )
    })
}

fn archive_text(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, ToolError> {
    let mut file = archive
        .by_name(name)
        .map_err(|_| ToolError::invalid("corrupt_document", format!("missing {name}")))?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|error| {
        ToolError::invalid(
            "corrupt_document",
            format!("could not read {name}: {error}"),
        )
    })?;
    Ok(text)
}

fn docx_text(bytes: &[u8]) -> Result<String, ToolError> {
    let mut archive = archive(bytes)?;
    let xml = archive_text(&mut archive, "word/document.xml")?;
    Ok(extract_ooxml_text(&xml, b"t", b"p"))
}

fn pptx_text(bytes: &[u8]) -> Result<String, ToolError> {
    let mut archive = archive(bytes)?;
    let mut slides: Vec<(u32, String)> = archive
        .file_names()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .map(|name| (trailing_number(name), name.to_owned()))
        .collect();
    slides.sort_by_key(|(number, _)| *number);
    let mut out = String::new();
    for (index, (number, name)) in slides.into_iter().enumerate() {
        let xml = archive_text(&mut archive, &name)?;
        let text = extract_ooxml_text(&xml, b"t", b"p");
        if !text.trim().is_empty() {
            out.push_str(&format!(
                "--- Slide {} ---\n{}\n",
                if number == 0 {
                    index + 1
                } else {
                    number as usize
                },
                text.trim_end()
            ));
        }
    }
    Ok(out)
}

fn trailing_number(name: &str) -> u32 {
    name.rsplit('/')
        .next()
        .unwrap_or(name)
        .trim_start_matches("slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(0)
}

fn xlsx_text(bytes: &[u8]) -> Result<String, ToolError> {
    let mut archive = archive(bytes)?;
    let shared = match archive.by_name("xl/sharedStrings.xml") {
        Ok(mut file) => {
            let mut xml = String::new();
            file.read_to_string(&mut xml).map_err(|error| {
                ToolError::invalid(
                    "corrupt_document",
                    format!("could not read shared strings: {error}"),
                )
            })?;
            shared_strings(&xml)
        }
        Err(_) => Vec::new(),
    };
    let mut sheets: Vec<(u32, String)> = archive
        .file_names()
        .filter(|name| name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml"))
        .map(|name| (trailing_number(name), name.to_owned()))
        .collect();
    sheets.sort_by_key(|(number, _)| *number);
    let mut out = String::new();
    for (index, (number, name)) in sheets.into_iter().enumerate() {
        let xml = archive_text(&mut archive, &name)?;
        let text = sheet_text(&xml, &shared);
        if !text.trim().is_empty() {
            out.push_str(&format!(
                "--- Sheet {} ---\n{}\n",
                if number == 0 {
                    index + 1
                } else {
                    number as usize
                },
                text.trim_end()
            ));
        }
    }
    Ok(out)
}

fn extract_ooxml_text(xml: &str, text_tag: &[u8], paragraph_tag: &[u8]) -> String {
    let mut reader = Reader::from_str(xml);
    let mut text = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = event.name();
                if local_name(name.as_ref()) == text_tag {
                    in_text = true;
                }
            }
            Ok(Event::Text(event)) => {
                if in_text && let Ok(value) = event.unescape() {
                    text.push_str(&value);
                }
            }
            Ok(Event::End(event)) => {
                let qualified = event.name();
                let name = local_name(qualified.as_ref());
                if name == text_tag {
                    in_text = false;
                } else if name == paragraph_tag {
                    text.push('\n');
                }
            }
            Ok(Event::Empty(event)) => {
                let qualified = event.name();
                let name = local_name(qualified.as_ref());
                if name == b"tab" {
                    text.push('\t');
                } else if name == b"br" {
                    text.push('\n');
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    text
}

fn shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_item = false;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let qualified = event.name();
                let name = local_name(qualified.as_ref());
                if name == b"si" {
                    in_item = true;
                    current.clear();
                } else if name == b"t" && in_item {
                    in_text = true;
                }
            }
            Ok(Event::Text(event)) => {
                if in_item
                    && in_text
                    && let Ok(value) = event.unescape()
                {
                    current.push_str(&value);
                }
            }
            Ok(Event::End(event)) => {
                let qualified = event.name();
                let name = local_name(qualified.as_ref());
                if name == b"t" {
                    in_text = false;
                } else if name == b"si" {
                    values.push(current.clone());
                    in_item = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    values
}

fn sheet_text(xml: &str, shared: &[String]) -> String {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut row: Vec<String> = Vec::new();
    let mut value = String::new();
    let mut cell_type = String::new();
    let mut in_value = false;
    let mut in_inline_text = false;
    let mut in_cell = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let qualified = event.name();
                let name = local_name(qualified.as_ref());
                match name {
                    b"c" => {
                        in_cell = true;
                        value.clear();
                        cell_type.clear();
                        for attribute in event.attributes().flatten() {
                            if attribute.key.as_ref() == b"t" {
                                cell_type =
                                    String::from_utf8_lossy(attribute.value.as_ref()).to_string();
                            }
                        }
                    }
                    b"v" => in_value = true,
                    b"t" if in_cell => in_inline_text = true,
                    _ => {}
                }
            }
            Ok(Event::Text(event)) => {
                if (in_value || in_inline_text)
                    && let Ok(text) = event.unescape()
                {
                    value.push_str(&text);
                }
            }
            Ok(Event::End(event)) => {
                let qualified = event.name();
                let name = local_name(qualified.as_ref());
                match name {
                    b"v" => in_value = false,
                    b"t" => in_inline_text = false,
                    b"c" => {
                        let resolved = if cell_type == "s" {
                            value
                                .trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| shared.get(index))
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            value.trim().to_owned()
                        };
                        row.push(resolved);
                        in_cell = false;
                    }
                    b"row" => {
                        out.push_str(&row.join("\t"));
                        out.push('\n');
                        row.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}
