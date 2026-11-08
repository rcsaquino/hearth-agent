//! The documents tool: extract readable text from PDF, DOCX, PPTX, and XLSX.

use std::ffi::OsStr;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use hearth_model::ToolSpec;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::{Value, json};
use zip::ZipArchive;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

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

    fn create(&self, arguments: &Value, path: &str) -> Result<ToolOutput, ToolError> {
        let format = arguments
            .get("format")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolError::invalid("invalid_arguments", "format is required for create")
            })?
            .to_ascii_lowercase();
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolError::invalid("invalid_arguments", "content is required for create")
            })?;
        let title = arguments
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(format.as_str(), "pdf" | "docx" | "pptx" | "xlsx") {
            return Err(ToolError::invalid(
                "unsupported_format",
                format!("unsupported document format: {format:?}"),
            ));
        }
        let extension = Path::new(path)
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let path = if extension.is_empty() {
            format!("{path}.{format}")
        } else if extension == format {
            path.to_owned()
        } else {
            return Err(ToolError::invalid(
                "unsupported_format",
                format!("path extension {extension:?} does not match format {format:?}"),
            ));
        };
        let bytes = document_bytes(&format, title, content)?;
        self.workspace.write_bytes(&path, &bytes)?;
        Ok(ToolOutput {
            content: format!("created {path} ({} bytes)", bytes.len()),
            is_error: false,
        })
    }
}

impl Tool for DocumentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "documents".to_owned(),
            description: "Extract readable text from a PDF, DOCX, PPTX, or XLSX file, or create one from text in the workspace, spill, or inbound media folder.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["extract", "create"], "description": "Defaults to extract"},
                    "path": {"type": "string", "description": "A document path inside the workspace, spill, or inbound media"},
                    "format": {"type": "string", "enum": ["pdf", "docx", "pptx", "xlsx"]},
                    "title": {"type": "string"},
                    "content": {"type": "string"}
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
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("extract");
        if action == "create" {
            return self.create(arguments, path);
        }
        if action != "extract" {
            return Err(ToolError::invalid(
                "unknown_action",
                format!("unknown documents action: {action}"),
            ));
        }
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

fn document_bytes(format: &str, title: &str, content: &str) -> Result<Vec<u8>, ToolError> {
    match format {
        "pdf" => Ok(pdf_document(title, content)),
        "docx" => docx_document(title, content),
        "pptx" => pptx_document(title, content),
        "xlsx" => xlsx_document(title, content),
        other => Err(ToolError::invalid(
            "unsupported_format",
            format!("unsupported document format: {other:?}"),
        )),
    }
}

fn document_lines(title: &str, content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if !title.is_empty() {
        lines.push(title.to_owned());
    }
    lines.extend(content.lines().map(str::to_owned));
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn pdf_document(title: &str, content: &str) -> Vec<u8> {
    let lines = document_lines(title, content);
    let mut stream = String::from("BT\n/F1 12 Tf\n72 720 Td\n");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            stream.push_str("0 -16 Td\n");
        }
        stream.push('(');
        stream.push_str(&pdf_escape(line));
        stream.push_str(") Tj\n");
    }
    stream.push_str("ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{}endstream", stream.len(), stream),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, object));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

fn pdf_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\n' | '\r' => out.push(' '),
            character if character.is_ascii() => out.push(character),
            _ => out.push('?'),
        }
    }
    out
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            character => out.push(character),
        }
    }
    out
}

fn zip_entries(entries: &[(&str, String)]) -> Result<Vec<u8>, ToolError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, content) in entries {
        writer
            .start_file(*name, options)
            .map_err(|error| ToolError::invalid("document_error", error.to_string()))?;
        writer
            .write_all(content.as_bytes())
            .map_err(|error| ToolError::invalid("document_error", error.to_string()))?;
    }
    let cursor = writer
        .finish()
        .map_err(|error| ToolError::invalid("document_error", error.to_string()))?;
    Ok(cursor.into_inner())
}

fn docx_document(title: &str, content: &str) -> Result<Vec<u8>, ToolError> {
    let mut paragraphs = String::new();
    for line in document_lines(title, content) {
        if line.is_empty() {
            paragraphs.push_str("<w:p/>");
        } else {
            paragraphs.push_str(&format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                xml_escape(&line)
            ));
        }
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{paragraphs}</w:body></w:document>"
    );
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>".to_owned();
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".to_owned();
    zip_entries(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("word/document.xml", document),
    ])
}

fn pptx_document(title: &str, content: &str) -> Result<Vec<u8>, ToolError> {
    let mut paragraphs = String::new();
    for line in document_lines(title, content) {
        if line.is_empty() {
            paragraphs.push_str("<a:p/>");
        } else {
            paragraphs.push_str(&format!(
                "<a:p><a:r><a:t>{}</a:t></a:r></a:p>",
                xml_escape(&line)
            ));
        }
    }
    let slide = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:cSld><p:spTree><p:sp><p:txBody>{paragraphs}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"
    );
    let presentation = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId1\"/></p:sldIdLst></p:presentation>".to_owned();
    let presentation_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/></Relationships>".to_owned();
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/></Types>".to_owned();
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>".to_owned();
    zip_entries(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("ppt/presentation.xml", presentation),
        ("ppt/_rels/presentation.xml.rels", presentation_rels),
        ("ppt/slides/slide1.xml", slide),
    ])
}

fn xlsx_document(title: &str, content: &str) -> Result<Vec<u8>, ToolError> {
    let mut rows = String::new();
    for (index, line) in document_lines(title, content).iter().enumerate() {
        let row = index + 1;
        rows.push_str(&format!(
            "<row r=\"{row}\"><c r=\"A{row}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c></row>",
            xml_escape(line)
        ));
    }
    let sheet = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>{rows}</sheetData></worksheet>"
    );
    let workbook = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>".to_owned();
    let workbook_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>".to_owned();
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>".to_owned();
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>".to_owned();
    zip_entries(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", workbook_rels),
        ("xl/worksheets/sheet1.xml", sheet),
    ])
}
