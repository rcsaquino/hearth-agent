use hearth_telegram::format_markdown;

fn entity<'a>(value: &'a hearth_telegram::FormattedText, kind: &str) -> &'a hearth_telegram::Entity {
    value
        .entities
        .iter()
        .find(|entity| entity.kind == kind)
        .unwrap_or_else(|| panic!("missing {kind} entity in {:?}", value.entities))
}

#[test]
fn formats_bold_links_and_inline_code() {
    let value = format_markdown("Hello **bold** and [link](https://example.com) and `code`.");
    assert_eq!(value.text, "Hello bold and link and code.");
    let bold = entity(&value, "bold");
    assert_eq!((bold.offset, bold.length), (6, 4));
    let link = entity(&value, "text_link");
    assert_eq!((link.offset, link.length), (15, 4));
    assert_eq!(link.url.as_deref(), Some("https://example.com"));
    let code = entity(&value, "code");
    assert_eq!((code.offset, code.length), (24, 4));
}

#[test]
fn formats_a_fenced_code_block_with_its_language() {
    let value = format_markdown("Before\n\n```rust\nfn main() {}\n```\n\nAfter");
    let code = entity(&value, "pre");
    assert_eq!(code.language.as_deref(), Some("rust"));
    assert_eq!(
        &value.text[code.offset..code.offset + code.length],
        "fn main() {}"
    );
    assert!(value.text.starts_with("Before"));
    assert!(value.text.ends_with("After"));
}

#[test]
fn counts_offsets_in_utf16_units() {
    let value = format_markdown("😀 **ok**");
    let bold = entity(&value, "bold");
    assert_eq!((bold.offset, bold.length), (3, 2));
}
