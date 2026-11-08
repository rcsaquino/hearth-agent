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
    blocks(&mut writer, markdown);
    FormattedText {
        text: writer.text,
        entities: writer.entities,
    }
}

fn blocks(writer: &mut Writer, markdown: &str) {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut index = 0;
    let mut first = true;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if let Some((fence, language)) = fence_start(line) {
            let mut code = Vec::new();
            index += 1;
            while index < lines.len() && !is_fence_close(lines[index], &fence) {
                code.push(lines[index]);
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            if !first {
                writer.append("\n\n");
            }
            first = false;
            let code = code.join("\n");
            let extra = if language.is_empty() {
                None
            } else {
                Some(("language", language.as_str()))
            };
            writer.mark("pre", extra, |writer| writer.append(&code));
            continue;
        }
        let mut paragraph = Vec::new();
        while index < lines.len() {
            let candidate = lines[index];
            if candidate.trim().is_empty() || fence_start(candidate).is_some() {
                break;
            }
            paragraph.push(candidate);
            index += 1;
        }
        if paragraph.is_empty() {
            index += 1;
            continue;
        }
        if !first {
            writer.append("\n\n");
        }
        first = false;
        inline(writer, &paragraph.join("\n"));
    }
}

fn fence_start(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    for marker in ["```", "~~~"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            let fence: String = marker.chars().collect();
            return Some((fence, rest.trim().to_owned()));
        }
    }
    None
}

fn is_fence_close(line: &str, fence: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(fence) && trimmed.trim_end_matches(fence_chars(fence)).trim().is_empty()
}

fn fence_chars(fence: &str) -> &str {
    fence
}

fn inline(writer: &mut Writer, input: &str) {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    'outer: while index < chars.len() {
        for (marker, kind) in [
            ("**", "bold"),
            ("~~", "strikethrough"),
            ("||", "spoiler"),
            ("__", "underline"),
            ("*", "italic"),
            ("_", "italic"),
        ] {
            if let Some(next) = delimited(writer, &chars, index, marker, kind) {
                index = next;
                continue 'outer;
            }
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

fn delimited(
    writer: &mut Writer,
    chars: &[char],
    index: usize,
    marker: &str,
    kind: &str,
) -> Option<usize> {
    if !starts_with(chars, index, marker) {
        return None;
    }
    let width = marker.chars().count();
    let end = find(chars, index + width, marker)?;
    let inner: String = chars[index + width..end].iter().collect();
    writer.mark(kind, None, |writer| inline(writer, &inner));
    Some(end + width)
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

/// Split formatted text into parts that fit Telegram's message limit.
pub fn split_formatted(value: &FormattedText, limit: usize) -> Vec<FormattedText> {
    let units: Vec<u16> = value.text.encode_utf16().collect();
    if units.len() <= limit {
        return vec![FormattedText {
            text: value.text.clone(),
            entities: clip_entities(&value.entities, 0, units.len()),
        }];
    }
    const MARKER_RESERVE: usize = 16;
    let content_limit = limit.saturating_sub(MARKER_RESERVE).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut end = (start + content_limit).min(units.len());
        if end < units.len() && is_high_surrogate(units[end - 1]) && is_low_surrogate(units[end]) {
            end -= 1;
        }
        if end < units.len() && end > start + content_limit / 2 {
            if let Some(position) = rfind_pair(&units, start, end, b'\n', b'\n') {
                end = position + 2;
            } else if let Some(position) = rfind(&units, start, end, b'\n' as u16) {
                end = position + 1;
            } else if let Some(position) = rfind(&units, start, end, b' ' as u16) {
                end = position + 1;
            }
        }
        if end <= start {
            end = (start + content_limit).min(units.len());
        }
        chunks.push((start, end));
        start = end;
    }
    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| {
            let mut text = String::from_utf16_lossy(&units[start..end]);
            text.push_str(&format!("\n\n({}/{})", index + 1, total));
            FormattedText {
                text,
                entities: clip_entities(&value.entities, start, end),
            }
        })
        .collect()
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xd800..=0xdbff).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xdc00..=0xdfff).contains(&unit)
}

fn rfind(units: &[u16], start: usize, end: usize, needle: u16) -> Option<usize> {
    (start..end).rev().find(|index| units[*index] == needle)
}

fn rfind_pair(units: &[u16], start: usize, end: usize, first: u8, second: u8) -> Option<usize> {
    if end < 2 {
        return None;
    }
    (start..end - 1)
        .rev()
        .find(|index| units[*index] == first as u16 && units[index + 1] == second as u16)
}

fn clip_entities(entities: &[Entity], start: usize, end: usize) -> Vec<Entity> {
    let mut clipped: Vec<Entity> = entities
        .iter()
        .filter_map(|entity| {
            let entity_start = entity.offset.max(start);
            let entity_end = (entity.offset + entity.length).min(end);
            if entity_end <= entity_start {
                return None;
            }
            Some(Entity {
                kind: entity.kind.clone(),
                offset: entity_start - start,
                length: entity_end - entity_start,
                url: entity.url.clone(),
                language: entity.language.clone(),
            })
        })
        .collect();
    clipped.sort_by(|left, right| {
        left.offset
            .cmp(&right.offset)
            .then_with(|| right.length.cmp(&left.length))
    });
    clipped.truncate(100);
    clipped
}
