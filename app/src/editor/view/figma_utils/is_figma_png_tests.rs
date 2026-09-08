use super::is_figma_png;

fn build_png_with_text_chunk(keyword: &[u8], text: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let data: Vec<u8> = keyword.iter().chain(b"\x00").chain(text).copied().collect();
    let length = data.len() as u32;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(b"tEXt");
    bytes.extend_from_slice(&data);
    bytes.extend_from_slice(&[0u8; 4]); // fake CRC
    bytes
}

#[test]
fn returns_true_for_figma_export_png() {
    let bytes = include_bytes!("figma-export.png");
    assert!(is_figma_png(bytes));
}

#[test]
fn returns_false_for_non_figma_png() {
    let bytes = include_bytes!("non-figma-export.png");
    assert!(!is_figma_png(bytes));
}

#[test]
fn returns_false_for_empty_bytes() {
    assert!(!is_figma_png(&[]));
}

#[test]
fn returns_false_for_invalid_png_signature() {
    let mut bytes = b"\x00PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0u8; 12]);
    assert!(!is_figma_png(&bytes));
}

#[test]
fn returns_true_for_crafted_png_with_software_figma() {
    let bytes = build_png_with_text_chunk(b"Software", b"Figma");
    assert!(is_figma_png(&bytes));
}

#[test]
fn returns_false_when_text_chunk_keyword_is_not_software() {
    let bytes = build_png_with_text_chunk(b"Author", b"Figma");
    assert!(!is_figma_png(&bytes));
}

#[test]
fn returns_false_when_software_value_is_not_figma() {
    let bytes = build_png_with_text_chunk(b"Software", b"Sketch");
    assert!(!is_figma_png(&bytes));
}

#[test]
fn returns_true_when_figma_text_chunk_follows_another_chunk() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    // A preceding chunk (e.g. tIME)
    let preceding = b"dummy data";
    bytes.extend_from_slice(&(preceding.len() as u32).to_be_bytes());
    bytes.extend_from_slice(b"tIME");
    bytes.extend_from_slice(preceding);
    bytes.extend_from_slice(&[0u8; 4]); // fake CRC
    // tEXt chunk with Software: Figma
    let text_data = b"Software\x00Figma";
    bytes.extend_from_slice(&(text_data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(b"tEXt");
    bytes.extend_from_slice(text_data);
    bytes.extend_from_slice(&[0u8; 4]); // fake CRC
    assert!(is_figma_png(&bytes));
}

#[test]
fn returns_false_for_truncated_chunk_data() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    // Declare length as 100 but provide fewer bytes
    bytes.extend_from_slice(&100u32.to_be_bytes());
    bytes.extend_from_slice(b"tEXt");
    bytes.extend_from_slice(b"Software\x00Figma"); // only 15 bytes, not 100
    assert!(!is_figma_png(&bytes));
}
