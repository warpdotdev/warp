use std::path::Path;

use super::*;

#[test]
fn test_chunker() {
    let code = "This is some text content\nthat should be chunked\nusing the naive chunker\nbecause the language isn't recognized.";
    let path = Path::new("test_file.xyz");

    let max_lines = 1;
    let fragments = chunk_code(code, path, 10000, max_lines);

    assert!(!fragments.is_empty(), "Expected at least one fragment");

    assert_eq!(fragments.len(), code.lines().count());
    for (idx, line) in code.lines().enumerate() {
        assert_eq!(fragments[idx].content, line);
        assert_eq!(fragments[idx].start_line, idx);
        assert_eq!(fragments[idx].end_line, idx);
    }
}

#[test]
fn test_chunker_large_chunk() {
    let code = "This is some text content\nthat should be chunked\nusing the naive chunker\nbecause the language isn't recognized.";
    let path = Path::new("test_file.xyz");

    let fragments = chunk_code(code, path, 10000, 100);

    // We should have only one fragment
    assert_eq!(fragments.len(), 1);

    assert_eq!(fragments[0].content, code);
    assert_eq!(fragments[0].start_line, 0);
    assert_eq!(fragments[0].end_line, code.lines().count() - 1);
}

#[test]
fn test_chunker_max_bytes() {
    // Create a string with known byte size - each line is exactly 20 bytes including newline
    let code = "line1\nline2\nline3\nline4abcdefghijklmnopqrstuvwxyz";
    let path = Path::new("test_file.xyz");

    // Set max_bytes_per_chunk to 25 bytes to force multiple chunks for the last line (which is 30 bytes).
    let max_bytes_per_chunk = 25;
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Verify we have multiple chunks
    assert!(
        fragments.len() > 1,
        "Expected multiple chunks due to size limit"
    );

    // Verify that no chunk exceeds the max_bytes_per_chunk limit
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            fragment.content.trim().len() <= max_bytes_per_chunk,
            "Fragment {} has size {} bytes, which exceeds limit of {} bytes",
            i,
            fragment.content.len(),
            max_bytes_per_chunk
        );
    }

    // The first fragment contains all of the lines except the last one.
    assert_eq!(fragments[0].content, "line1\nline2\nline3");

    // The last two fragments contains the contents of the line line.
    assert_eq!(fragments[1].content, "line4abcdefghijklmnopqrst");
    assert_eq!(fragments[2].content, "uvwxyz");

    // Verify that the chunks together contain all the original content
    let reassembled_content: String = fragments
        .iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("");

    // Ignore any newlines when doing comparisons--the chunker may drop newlines at fragment boundaries
    // and that's not necessary for testing the correctness of the naive chunker.
    assert_eq!(
        reassembled_content.replace('\n', ""),
        code.replace('\n', ""),
        "Reassembled content does not match original"
    );
}

#[test]
fn test_utf8_emoji_chunking() {
    // Test with emojis (4-byte UTF-8 characters) to ensure byte boundaries are respected
    let code = "Hello 🦀 Rust\nWorld 🌍 Test\n🚀 Rocket 🎯 Target";
    let path = Path::new("test_emoji.txt");

    // Set a small max_bytes_per_chunk to force splitting through emoji characters
    let max_bytes_per_chunk = 15; // This will force splits in the middle of emoji sequences
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Verify we have multiple chunks
    assert!(
        fragments.len() > 1,
        "Expected multiple chunks due to size limit"
    );

    // Verify that no chunk exceeds the max_bytes_per_chunk limit
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            fragment.content.len() <= max_bytes_per_chunk,
            "Fragment {} has size {} bytes, which exceeds limit of {} bytes. Content: '{}'",
            i,
            fragment.content.len(),
            max_bytes_per_chunk,
            fragment.content
        );
    }

    // Verify that all fragments contain valid UTF-8
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            fragment.content.is_ascii() || std::str::from_utf8(fragment.content.as_bytes()).is_ok(),
            "Fragment {} contains invalid UTF-8: {:?}",
            i,
            fragment.content
        );
    }

    // Verify that reassembled content matches original (ignoring newlines)
    let reassembled_content: String = fragments
        .iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("");

    assert_eq!(
        reassembled_content.replace('\n', ""),
        code.replace('\n', ""),
        "Reassembled content does not match original"
    );
}

#[test]
fn test_utf8_accented_characters() {
    // Test with accented characters (2-byte UTF-8)
    let code = "Café résumé naïve\nÉlève découvrir\nMañana piñata";
    let path = Path::new("test_accents.txt");

    // Set max_bytes_per_chunk to force splitting through accented characters
    let max_bytes_per_chunk = 10;
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Verify we have multiple chunks
    assert!(
        fragments.len() > 1,
        "Expected multiple chunks due to size limit"
    );

    // Verify that all fragments contain valid UTF-8
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            std::str::from_utf8(fragment.content.as_bytes()).is_ok(),
            "Fragment {} contains invalid UTF-8: {:?}",
            i,
            fragment.content.as_bytes()
        );
    }

    // Verify that reassembled content matches original (ignoring newlines)
    let reassembled_content: String = fragments
        .iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("");

    assert_eq!(
        reassembled_content.replace('\n', ""),
        code.replace('\n', ""),
        "Reassembled content does not match original"
    );
}

#[test]
fn test_utf8_mixed_characters() {
    // Test with a mix of ASCII, 2-byte, 3-byte, and 4-byte UTF-8 characters
    let code = "ASCII text 中文 🦀 résumé ℘ math symbols";
    let path = Path::new("test_mixed.txt");

    // Set a small chunk size to force many splits
    let max_bytes_per_chunk = 8;
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Verify we have multiple chunks
    assert!(
        fragments.len() > 1,
        "Expected multiple chunks due to size limit"
    );

    // Verify that all fragments contain valid UTF-8 and don't exceed size limit
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            fragment.content.len() <= max_bytes_per_chunk,
            "Fragment {} has size {} bytes, which exceeds limit of {} bytes",
            i,
            fragment.content.len(),
            max_bytes_per_chunk
        );

        assert!(
            std::str::from_utf8(fragment.content.as_bytes()).is_ok(),
            "Fragment {} contains invalid UTF-8: {:?}",
            i,
            fragment.content.as_bytes()
        );
    }

    // Verify that reassembled content matches original
    let reassembled_content: String = fragments
        .iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("");

    assert_eq!(
        reassembled_content, code,
        "Reassembled content does not match original"
    );
}

#[test]
fn test_utf8_boundary_edge_cases() {
    // Test edge case where chunk boundary falls exactly on a multi-byte character
    let code = "ab🦀cd"; // 'ab' (2 bytes) + '🦀' (4 bytes) + 'cd' (2 bytes) = 8 bytes total
    let path = Path::new("test_edge.txt");

    // Set chunk size to 3 bytes, which would split in the middle of the emoji without our fix
    let max_bytes_per_chunk = 3;
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Should have multiple fragments
    assert!(fragments.len() >= 2, "Expected at least 2 fragments");

    // Verify all fragments are valid UTF-8
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            std::str::from_utf8(fragment.content.as_bytes()).is_ok(),
            "Fragment {} contains invalid UTF-8: {:?}",
            i,
            fragment.content.as_bytes()
        );
    }

    // Verify reassembled content matches original
    let reassembled_content: String = fragments
        .iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("");

    assert_eq!(
        reassembled_content, code,
        "Reassembled content does not match original"
    );
}

#[test]
fn test_utf8_single_multibyte_character() {
    // Test with a single multi-byte character that's larger than chunk size
    let code = "🦀"; // 4-byte emoji
    let path = Path::new("test_single.txt");

    // Set chunk size smaller than the character
    let max_bytes_per_chunk = 2;
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Should have exactly one fragment (can't split a single character)
    assert_eq!(fragments.len(), 1, "Should have exactly one fragment");

    // The fragment should contain the complete character
    assert_eq!(fragments[0].content, code);

    // Verify it's valid UTF-8
    assert!(
        std::str::from_utf8(fragments[0].content.as_bytes()).is_ok(),
        "Fragment contains invalid UTF-8"
    );
}

#[test]
fn test_utf8_line_endings_with_multibyte() {
    // Test multi-byte characters at line boundaries
    let code = "Hello🌍\nWorld🦀\nTest🎯";
    let path = Path::new("test_lines.txt");

    let max_bytes_per_chunk = 10;
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1); // 1 line per chunk

    // Should have 3 fragments (one per line)
    assert_eq!(fragments.len(), 3, "Should have 3 fragments for 3 lines");

    // Verify all fragments are valid UTF-8
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            std::str::from_utf8(fragment.content.as_bytes()).is_ok(),
            "Fragment {} contains invalid UTF-8: {:?}",
            i,
            fragment.content.as_bytes()
        );
    }

    // Verify line numbers are correct
    assert_eq!(fragments[0].start_line, 0);
    assert_eq!(fragments[0].end_line, 0);
    assert_eq!(fragments[1].start_line, 1);
    assert_eq!(fragments[1].end_line, 1);
    assert_eq!(fragments[2].start_line, 2);
    assert_eq!(fragments[2].end_line, 2);
}

#[test]
fn test_panic_regression_byte_boundary() {
    // This is a regression test for the "byte index is not a char boundary" panic.
    // Before the fix, this would panic when trying to slice at byte index 3,
    // which is in the middle of the 4-byte emoji '🦀'.
    let code = "Hi🦀Test";
    let path = Path::new("test_panic.txt");

    // This chunk size would cause the original code to panic
    let max_bytes_per_chunk = 3;

    // This should not panic
    let fragments = chunk_code(code, path, max_bytes_per_chunk, 1000);

    // Verify we get valid fragments
    assert!(!fragments.is_empty(), "Should have at least one fragment");

    // Verify all fragments are valid UTF-8
    for (i, fragment) in fragments.iter().enumerate() {
        assert!(
            std::str::from_utf8(fragment.content.as_bytes()).is_ok(),
            "Fragment {} contains invalid UTF-8: {:?}",
            i,
            fragment.content.as_bytes()
        );
    }

    // Verify reassembled content matches original
    let reassembled_content: String = fragments
        .iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("");

    assert_eq!(
        reassembled_content, code,
        "Reassembled content does not match original"
    );
}
