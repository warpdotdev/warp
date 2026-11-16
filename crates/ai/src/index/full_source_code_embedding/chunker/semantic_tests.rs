use std::path::Path;

use languages::language_by_filename;
use warp_util::standardized_path::StandardizedPath;

use super::*;

#[test]
fn test_basic_rust_chunking() {
    let source_code = r#"
#[derive(Debug)]
struct Rectangle {
    width: u32,
    height: u32,
}

impl Rectangle {
    fn area(&self) -> u32 {
        self.width * self.height
    }
}

fn main() {
    let rect1 = Rectangle {
        width: 30,
        height: 50,
    };

    println!(
        "The area of the rectangle is {} square pixels.",
        rect1.area()
    );
}
"#;

    let max_chunk_size = 128;
    let language_path =
        StandardizedPath::try_new("/test.rs").expect("test path should be absolute");

    let chunks = chunk_code(
        source_code,
        Path::new("test.rs"),
        max_chunk_size,
        &language_by_filename(&language_path)
            .expect("Rust language must exist")
            .grammar,
    )
    .unwrap();

    assert_eq!(chunks.len(), 4);

    // None of the chunks should exceed the chunk size.
    for chunk in &chunks {
        assert!(
            chunk.content.len() <= max_chunk_size,
            "Chunk should not exceed max size of {max_chunk_size} but was: {}",
            chunk.content.len()
        );
    }

    assert_eq!(
        chunks[0].content.trim(),
        r#"#[derive(Debug)]
struct Rectangle {
    width: u32,
    height: u32,
}"#
    );
    assert_eq!(
        chunks[1].content.trim(),
        r#"impl Rectangle {
    fn area(&self) -> u32 {
        self.width * self.height
    }
}"#
    );
    assert_eq!(
        chunks[2].content.trim(),
        r#"fn main() {
    let rect1 = Rectangle {
        width: 30,
        height: 50,
    };"#
    );
    assert_eq!(
        chunks[3].content.trim(),
        r#"println!(
        "The area of the rectangle is {} square pixels.",
        rect1.area()
    );
}"#
    );
}
