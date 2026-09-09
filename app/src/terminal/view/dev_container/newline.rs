//! Piped Dev Container text logs use bare LF. A terminal LF moves down a row
//! without returning to column 0, so unnormalized streams staircase.

/// Converts an `LF` that is not immediately preceded by `CR` into `CRLF`,
/// carrying previous-byte state across read chunks.
pub(crate) struct NewlineNormalizer {
    last_was_cr: bool,
}

impl NewlineNormalizer {
    pub(crate) fn new() -> Self {
        Self { last_was_cr: false }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        let extra_crs = chunk.iter().filter(|&&byte| byte == b'\n').count();
        let mut out = Vec::with_capacity(chunk.len() + extra_crs);
        for &byte in chunk {
            match byte {
                b'\n' => {
                    if !self.last_was_cr {
                        out.push(b'\r');
                    }
                    out.push(b'\n');
                    self.last_was_cr = false;
                }
                b'\r' => {
                    out.push(b'\r');
                    self.last_was_cr = true;
                }
                _ => {
                    out.push(byte);
                    self.last_was_cr = false;
                }
            }
        }
        out
    }

    /// A trailing standalone `CR` was already emitted; leave it unchanged.
    pub(crate) fn finish(self) -> Vec<u8> {
        Vec::new()
    }
}

#[cfg(test)]
#[path = "newline_tests.rs"]
mod tests;
