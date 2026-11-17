//! Bounded response-body reads.

use std::io::Read;

/// The most of an error body that is read before the rest is dropped.
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

/// Read a body that must fit in `max_bytes`, or refuse it.
pub(crate) fn capped(mut reader: impl Read, max_bytes: usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("the response body could not be read: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "the response body is larger than {max_bytes} bytes"
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// Read a binary body that must fit in `max_bytes`, or refuse it.
pub(crate) fn capped_bytes(mut reader: impl Read, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("the response body could not be read: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "the response body is larger than {max_bytes} bytes"
        ));
    }
    Ok(bytes)
}
