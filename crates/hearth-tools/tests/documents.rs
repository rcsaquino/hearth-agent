use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use hearth_tools::{DocumentsTool, Tool};
use serde_json::json;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn roots() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let directory = TempDir::new().expect("temp directory");
    let workspace = directory.path().join("workspace");
    let spill = directory.path().join("spill");
    let media = directory.path().join("media/inbound");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&spill).expect("spill");
    fs::create_dir_all(media.join("2026-11-09")).expect("media");
    (directory, workspace, spill, media)
}

fn tool(workspace: &Path, spill: &Path, media: &Path) -> DocumentsTool {
    DocumentsTool::new(workspace, spill, media)
}

fn write_zip(path: &Path, entries: &[(&str, &str)]) {
    let file = File::create(path).expect("zip file");
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, content) in entries {
        zip.start_file(*name, options).expect("zip entry");
        zip.write_all(content.as_bytes()).expect("zip content");
    }
    zip.finish().expect("zip finish");
}

fn write_docx(path: &Path) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Hello DOCX</w:t></w:r></w:p>
    <w:p><w:r><w:t>Second line</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
    write_zip(path, &[("word/document.xml", xml)]);
}

fn write_pptx(path: &Path) {
    let slide = |text: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#
        )
    };
    let one = slide("Slide one");
    let two = slide("Slide two");
    write_zip(
        path,
        &[
            ("ppt/slides/slide1.xml", one.as_str()),
            ("ppt/slides/slide2.xml", two.as_str()),
        ],
    );
}

fn write_xlsx(path: &Path) {
    let shared = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <si><t>Hello XLSX</t></si>
  <si><t>Second cell</t></si>
</sst>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
    <row r="2"><c r="A2"><v>42</v></c></row>
  </sheetData>
</worksheet>"#;
    write_zip(
        path,
        &[
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet),
        ],
    );
}

#[test]
fn extracts_pdf_docx_pptx_and_xlsx() {
    let (_directory, workspace, spill, media) = roots();
    let documents = tool(&workspace, &spill, &media);

    fs::write(
        workspace.join("sample.pdf"),
        include_bytes!("fixtures/sample.pdf"),
    )
    .expect("pdf");
    write_docx(&workspace.join("sample.docx"));
    write_pptx(&workspace.join("sample.pptx"));
    write_xlsx(&workspace.join("sample.xlsx"));

    let pdf = documents
        .call(&json!({"path": "sample.pdf"}))
        .expect("pdf extraction");
    assert!(pdf.content.contains("Hello Hearth PDF"), "{}", pdf.content);

    let docx = documents
        .call(&json!({"path": "sample.docx"}))
        .expect("docx extraction");
    assert!(docx.content.contains("Hello DOCX"), "{}", docx.content);
    assert!(docx.content.contains("Second line"), "{}", docx.content);

    let pptx = documents
        .call(&json!({"path": "sample.pptx"}))
        .expect("pptx extraction");
    assert!(pptx.content.contains("Slide one"), "{}", pptx.content);
    assert!(pptx.content.contains("Slide two"), "{}", pptx.content);

    let xlsx = documents
        .call(&json!({"path": "sample.xlsx"}))
        .expect("xlsx extraction");
    assert!(xlsx.content.contains("Hello XLSX"), "{}", xlsx.content);
    assert!(xlsx.content.contains("Second cell"), "{}", xlsx.content);
    assert!(xlsx.content.contains("42"), "{}", xlsx.content);
}

#[test]
fn reads_the_inbound_media_root_and_rejects_escapes() {
    let (_directory, workspace, spill, media) = roots();
    let documents = tool(&workspace, &spill, &media);
    let inbound = media.join("2026-11-09/incoming.docx");
    write_docx(&inbound);

    let relative = documents
        .call(&json!({"path": "media/2026-11-09/incoming.docx"}))
        .expect("media-relative extraction");
    assert!(
        relative.content.contains("Hello DOCX"),
        "{}",
        relative.content
    );

    let absolute = documents
        .call(&json!({"path": inbound.to_string_lossy()}))
        .expect("absolute inbound extraction");
    assert!(
        absolute.content.contains("Hello DOCX"),
        "{}",
        absolute.content
    );

    let escape = documents
        .call(&json!({"path": "../outside.docx"}))
        .expect_err("parent escape");
    assert_eq!(escape.code, "invalid_path");

    let outside = documents
        .call(&json!({"path": "/etc/hostname"}))
        .expect_err("absolute escape");
    assert!(
        outside.code == "not_found" || outside.code == "path_escape",
        "{}",
        outside.code
    );
}

#[test]
fn rejects_unsupported_corrupt_and_empty_documents() {
    let (_directory, workspace, spill, media) = roots();
    let documents = tool(&workspace, &spill, &media);

    fs::write(workspace.join("plain.txt"), "hello").expect("text file");
    let unsupported = documents
        .call(&json!({"path": "plain.txt"}))
        .expect_err("unsupported format");
    assert_eq!(unsupported.code, "unsupported_format");

    fs::write(workspace.join("broken.docx"), "not a zip").expect("broken file");
    let corrupt = documents
        .call(&json!({"path": "broken.docx"}))
        .expect_err("corrupt document");
    assert_eq!(corrupt.code, "corrupt_document");

    let missing = documents
        .call(&json!({"path": "missing.pdf"}))
        .expect_err("missing document");
    assert_eq!(missing.code, "not_found");
}
