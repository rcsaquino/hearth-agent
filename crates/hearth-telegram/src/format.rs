//! Markdown to Telegram plain text plus native entities.

use serde::Serialize;

/// One Telegram message entity. Offsets and lengths are UTF-16 code units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entity {
    #[serde(rename = "type")]
    pub kind: String,
    pub offset: usize,
    pub length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Plain text and the entities that style it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedText {
    pub text: String,
    pub entities: Vec<Entity>,
}

struct Writer {
    text: String,
    utf16: usize,
    entities: Vec<Entity>,
}

impl Writer {
    fn new() -> Self {
        Self {
            text: String::new(),
            utf16: 0,
            entities: Vec::new(),
        }
    }

    fn append(&mut self, text: &str) {
        self.text.push_str(text);
        self.utf16 += text.encode_utf16().count();
    }

    fn mark(&mut self, kind: &str, extra: Option<(&str, &str)>, render: impl FnOnce(&mut Self)) {
        let offset = self.utf16;
        render(self);
        let length = self.utf16 - offset;
        if length == 0 {
            return;
        }
        let (url, language) = match extra {
            Some(("url", value)) => (Some(value.to_owned()), None),
            Some(("language", value)) => (None, Some(value.to_owned())),
            _ => (None, None),
        };
        self.entities.push(Entity {
            kind: kind.to_owned(),
            offset,
            length,
            url,
            language,
        });
    }
}

/// Convert Markdown into plain text plus Telegram entities.
pub fn format_markdown(markdown: &str) -> FormattedText {
    let mut writer = Writer::new();
    inline(&mut writer, markdown);
    FormattedText {
        text: writer.text,
        entities: writer.entities,
    }
}

fn inline(writer: &mut Writer, input: &str) {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if starts_with(&chars, index, "**")
            && let Some(end) = find(&chars, index + 2, "**")
        {
            let inner: String = chars[index + 2..end].iter().collect();
            writer.mark("bold", None, |writer| writer.append(&inner));
            index = end + 2;
            continue;
        }
        if chars[index] == '[' {
            if let Some((label, url, next)) = parse_link(&chars, index) {
                let safe = is_safe_url(&url);
                if safe {
                    writer.mark("text_link", Some(("url", &url)), |writer| {
                        writer.append(&label)
                    });
                } else {
                    writer.append(&label);
                }
                index = next;
                continue;
            }
        }
        if chars[index] == '`'
            && let Some(end) = find_char(&chars, index + 1, '`')
        {
            let inner: String = chars[index + 1..end].iter().collect();
            writer.mark("code", None, |writer| writer.append(&inner));
            index = end + 1;
            continue;
        }
        writer.append(&chars[index].to_string());
        index += 1;
    }
}

fn starts_with(chars: &[char], index: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    chars.get(index..index + needle.len()) == Some(needle.as_slice())
}

fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return None;
    }
    let mut index = from;
    while index + needle.len() <= chars.len() {
        if chars[index..index + needle.len()] == needle[..] {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn find_char(chars: &[char], from: usize, needle: char) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, candidate)| **candidate == needle)
        .map(|(index, _)| index)
}

fn parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let close = find_char(chars, start + 1, ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let url_end = find_char(chars, close + 2, ')')?;
    let label: String = chars[start + 1..close].iter().collect();
    let url: String = chars[close + 2..url_end].iter().collect();
    Some((label, url, url_end + 1))
}

fn is_safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["https://", "http://", "mailto:", "tel:", "tg:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}
