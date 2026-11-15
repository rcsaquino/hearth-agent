//! Bounded response-body reads for error messages.

use std::io::Read;

/// The most of an error body that is read before the rest is dropped.
///
/// Only a few hundred characters reach a message, so reading more is waste at
/// best and an allocation the peer controls at worst.
pub(crate) const ERROR_BODY_BYTES: usize = 8 * 1024;

/// Read at most `max_bytes` from a response body as text.
pub(crate) fn snippet(mut reader: impl Read, max_bytes: usize) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    while bytes.len() < max_bytes {
        let want = (max_bytes - bytes.len()).min(buffer.len());
        match reader.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(read) => bytes.extend_from_slice(&buffer[..read]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snippet_stops_at_the_cap() {
        let body = b"x".repeat(1024);
        let snippet = snippet(&body[..], 100);
        assert_eq!(snippet.len(), 100);
    }
}
