/// Returns `true` if `bytes` is a PNG whose `tEXt` metadata contains `Software: Figma`.
///
/// Figma exports PNGs with a `tEXt` chunk where the keyword is `Software` and the value
/// is `Figma`. We scan the raw chunk stream for this marker without pulling in an image
/// parsing library, keeping the check lightweight.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn is_figma_png(bytes: &[u8]) -> bool {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return false;
    }
    let mut offset = 8usize;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let type_start = offset + 4;
        let data_start = type_start + 4;
        let Some(data_end) = data_start.checked_add(length) else {
            break;
        };
        if data_end + 4 > bytes.len() {
            break;
        }
        // tEXt chunk: data is `keyword\0text`
        if &bytes[type_start..data_start] == b"tEXt"
            && bytes[data_start..data_end].starts_with(b"Software\x00Figma")
        {
            return true;
        }
        // Advance past: length(4) + type(4) + data(length) + CRC(4)
        offset = data_end + 4;
    }
    false
}

#[cfg(test)]
#[path = "is_figma_png_tests.rs"]
mod tests;
