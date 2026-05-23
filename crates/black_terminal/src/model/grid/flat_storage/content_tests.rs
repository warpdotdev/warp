use super::*;

#[test]
fn test_large_grapheme_starts_new_chunk() {
    let mut content = Content::new();
    let a = Grapheme::new_from_str("a");
    for _ in 0..Chunk::CHUNK_SIZE - 1 {
        content.push_grapheme(&a);
    }

    assert!(content.filled_chunks.is_empty());

    let grapheme = Grapheme::new_from_str("🚀");
    assert!(grapheme.len().as_usize() > 1);
    assert!(content.end_offset() + grapheme.len().as_usize() > Chunk::CHUNK_SIZE);

    content.push_grapheme(&grapheme);

    assert_eq!(content.filled_chunks.len(), 1);
    assert_eq!(grapheme.len().as_usize(), content.active_chunk.len());
    assert_eq!(
        content.active_chunk.start_offset,
        ByteOffset::from(Chunk::CHUNK_SIZE - 1)
    );
}

#[test]
fn test_truncate_front_drops_old_chunks() {
    let mut content = Content::new();
    let a = Grapheme::new_from_str("a");
    for _ in 0..Chunk::CHUNK_SIZE - 1 {
        content.push_grapheme(&a);
    }
    let grapheme = Grapheme::new_from_str("🚀");
    content.push_grapheme(&grapheme);

    // Drop everything before the active chunk except for one byte.
    content.truncate_front(content.active_chunk.start_offset - 1);
    // This should't affect the one filled chunk.
    assert_eq!(content.filled_chunks.len(), 1);

    // Drop everything before the active chunk.
    content.truncate_front(content.active_chunk.start_offset);
    // Ensure the filled chunk was dropped.
    assert!(content.filled_chunks.is_empty());
}
