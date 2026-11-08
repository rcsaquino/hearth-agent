use hearth_telegram::{format_markdown, split_formatted};

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

#[test]
fn splits_long_text_with_markers() {
    let value = format_markdown(&"word ".repeat(30));
    let parts = split_formatted(&value, 40);
    assert!(parts.len() > 1);
    let total = parts.len();
    for (index, part) in parts.iter().enumerate() {
        assert!(part.text.encode_utf16().count() <= 40, "part too long: {:?}", part.text);
        assert!(part.text.ends_with(&format!("({}/{})", index + 1, total)), "missing marker: {:?}", part.text);
    }
}

#[test]
fn keeps_split_code_inside_pre_entities() {
    let markdown = format!("```rust\n{}\n```", "a".repeat(5000));
    let value = format_markdown(&markdown);
    let parts = split_formatted(&value, 4096);
    assert!(parts.len() > 1);
    for (index, part) in parts.iter().enumerate() {
        assert!(part.text.encode_utf16().count() <= 4096);
        assert!(part.text.ends_with(&format!("({}/{})", index + 1, parts.len())));
        assert!(
            part.entities.iter().any(|entity| entity.kind == "pre" && entity.language.as_deref() == Some("rust")),
            "part {} lost its pre entity: {:?}",
            index + 1,
            part.entities
        );
    }
}

#[test]
fn formats_italic_strikethrough_spoiler_and_underline() {
    let value = format_markdown("*it* ~~gone~~ ||secret|| __under__");
    assert_eq!(value.text, "it gone secret under");
    assert_eq!((entity(&value, "italic").offset, entity(&value, "italic").length), (0, 2));
    assert_eq!(
        (entity(&value, "strikethrough").offset, entity(&value, "strikethrough").length),
        (3, 4)
    );
    assert_eq!((entity(&value, "spoiler").offset, entity(&value, "spoiler").length), (8, 6));
    assert_eq!((entity(&value, "underline").offset, entity(&value, "underline").length), (15, 5));
}

#[test]
fn formats_headings_lists_quotes_and_tables() {
    let value = format_markdown(
        "# Title\n\n- one\n- two\n\n> quoted\n\n| A | B |\n|---|---|\n| 1 | 2 |",
    );
    let heading = entity(&value, "bold");
    assert_eq!(
        &value.text[heading.offset..heading.offset + heading.length],
        "Title"
    );
    assert!(value.text.contains("• one\n• two"), "{}", value.text);
    assert!(value.text.contains("› quoted"), "{}", value.text);
    assert!(value.text.contains("A: 1"), "{}", value.text);
    assert!(value.text.contains("B: 2"), "{}", value.text);
}

#[test]
fn keeps_entities_out_of_inline_code() {
    let value = format_markdown("**`code`** and **bold**");
    let code = entity(&value, "code");
    assert_eq!((code.offset, code.length), (0, 4));
    for entity in &value.entities {
        if entity.kind == "code" || entity.kind == "pre" {
            continue;
        }
        assert!(
            entity.offset >= code.offset + code.length || entity.offset + entity.length <= code.offset,
            "entity overlaps code: {:?}",
            entity
        );
    }
    assert!(value.entities.iter().any(|entity| entity.kind == "bold" && entity.offset >= 9));
}
