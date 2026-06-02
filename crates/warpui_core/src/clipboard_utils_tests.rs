use super::*;
use crate::clipboard::{ClipboardContent, ImageData};

// ============================================================================
// HELPER FUNCTIONS (shared across tests)
// ============================================================================

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
fn create_rgba_data(w: usize, h: usize) -> Vec<u8> {
    // Simple test pattern: red gradient
    (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| [((x * 255) / w) as u8, ((y * 255) / h) as u8, 128, 255])
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
fn create_simple_png() -> Vec<u8> {
    // PNG header for 1x1 red pixel
    vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] // PNG signature
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
fn create_simple_jpeg() -> Vec<u8> {
    // JPEG header
    vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46]
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
fn create_simple_gif() -> Vec<u8> {
    // GIF header
    let mut data = Vec::new();
    data.extend_from_slice(b"GIF87a");
    data.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0]); // minimal 1x1 GIF
    data
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
fn create_simple_webp() -> Vec<u8> {
    // WebP header
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&[12, 0, 0, 0]); // file size
    data.extend_from_slice(b"WEBP");
    data.extend_from_slice(b"VP8 ");
    data
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
fn assert_valid_png(result: Option<ImageData>) {
    let image_data = result.expect("Should process image successfully");
    assert_eq!(image_data.mime_type, "image/png");
    assert_eq!(&image_data.data[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]); // PNG header
}

// ============================================================================
// FILENAME EXTRACTION TESTS
// ============================================================================

#[test]
fn test_extract_filename_from_html() {
    // Test extraction from src attribute with file:// URL (common on macOS)
    let html1 = r##"<img src="file:///Users/test/Pictures/screenshot.png" alt="Screenshot">"##;
    let filename = extract_filename_from_html(html1);
    assert_eq!(filename, Some("screenshot.png".to_string()));

    // Test extraction from src attribute with http URL
    let html2 = r##"<img src="https://example.com/images/photo.jpg" alt="Photo">"##;
    let filename = extract_filename_from_html(html2);
    assert_eq!(filename, Some("photo.jpg".to_string()));

    // Test extraction from title attribute
    let html3 = r##"<img title="document.gif" src="data:image/gif;base64,R0lGOD...">"##;
    let filename = extract_filename_from_html(html3);
    assert_eq!(filename, Some("document.gif".to_string()));

    // Test extraction from alt attribute
    let html4 = r##"<img alt="image.webp" src="data:image/webp;base64,UklGR...">"##;
    let filename = extract_filename_from_html(html4);
    assert_eq!(filename, Some("image.webp".to_string()));

    // Test extraction from free text
    let html5 = r##"<div>Here is my image: myfile.jpeg that I copied</div>"##;
    let filename = extract_filename_from_html(html5);
    assert_eq!(filename, Some("myfile.jpeg".to_string()));

    // Test no filename found
    let html6 = r##"<div>Just some text with no image references</div>"##;
    let filename = extract_filename_from_html(html6);
    assert_eq!(filename, None);

    // Test non-image extension ignored
    let html7 = r##"<div>document.pdf and archive.zip should be ignored</div>"##;
    let filename = extract_filename_from_html(html7);
    assert_eq!(filename, None);

    // Test complex path extraction with Windows-style paths
    let html8 =
        r##"<img src="file://C:\Users\John%20Doe\Desktop\My%20Images\vacation-photo.png">"##;
    let filename = extract_filename_from_html(html8);
    assert_eq!(filename, Some("vacation-photo.png".to_string()));

    // Test case-insensitive extension matching
    let html9 = r##"<img src="test.PNG" alt="Test">"##;
    let filename = extract_filename_from_html(html9);
    assert_eq!(filename, Some("test.PNG".to_string()));

    // Test extraction with various punctuation
    let html10 = r##"<div>Look at "my-image.jpg", (another.gif), or <file.webp>!</div>"##;
    let filename = extract_filename_from_html(html10);
    // Should find the first one
    assert_eq!(filename, Some("my-image.jpg".to_string()));
}

#[test]
fn test_extract_filename_from_text() {
    // Test full file path
    let file_path = "/Users/test/Documents/screenshot.png";
    let result = extract_filename_from_text(file_path);
    assert_eq!(result, Some("screenshot.png".to_string()));

    // Test Windows path
    let windows_path = "C:\\Users\\test\\Documents\\image.jpg";
    let result = extract_filename_from_text(windows_path);
    assert_eq!(result, Some("image.jpg".to_string()));

    // Test file:// URL
    let file_url = "file:///Users/test/screenshot.gif";
    let result = extract_filename_from_text(file_url);
    assert_eq!(result, Some("screenshot.gif".to_string()));

    // Test multiline with file path
    let multiline = "Some text\n/path/to/image.webp\nMore text";
    let result = extract_filename_from_text(multiline);
    assert_eq!(result, Some("image.webp".to_string()));

    // Test non-image file (should return None)
    let text_file = "/Users/test/document.txt";
    let result = extract_filename_from_text(text_file);
    assert_eq!(result, None);

    // Test no file path
    let plain_text = "Just some plain text";
    let result = extract_filename_from_text(plain_text);
    assert_eq!(result, None);

    // Test just filename
    let just_filename = "my-screenshot.png";
    let result = extract_filename_from_text(just_filename);
    assert_eq!(result, Some("my-screenshot.png".to_string()));

    // Test empty string
    let empty = "";
    let result = extract_filename_from_text(empty);
    assert_eq!(result, None);
}

#[test]
fn test_extract_filename_from_clipboard_content() {
    // Test HTML takes precedence over text
    let html_content = Some(r##"<img src="test.png" alt="Test">"##.to_string());
    let text_content = "other-file.jpg";
    let result = extract_filename_from_clipboard_content(&html_content, text_content);
    assert_eq!(result, Some("test.png".to_string()));

    // Test fallback to text when HTML has no filename
    let html_content = Some("<div>No images here</div>".to_string());
    let text_content = "/path/to/image.gif";
    let result = extract_filename_from_clipboard_content(&html_content, text_content);
    assert_eq!(result, Some("image.gif".to_string()));

    // Test fallback to text when no HTML
    let html_content = None;
    let text_content = "screenshot.webp";
    let result = extract_filename_from_clipboard_content(&html_content, text_content);
    assert_eq!(result, Some("screenshot.webp".to_string()));

    // Test no filename found
    let html_content = Some("<div>Just text</div>".to_string());
    let text_content = "No images here either";
    let result = extract_filename_from_clipboard_content(&html_content, text_content);
    assert_eq!(result, None);
}

// ============================================================================
// IMAGE PROCESSING TESTS (Linux/Windows platforms only)
// ============================================================================

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
mod image_processing_tests {
    use super::*;

    #[test]
    fn test_rgba_bitmap_processing() {
        let arboard_image = arboard::ImageData {
            width: 8,
            height: 6,
            bytes: create_rgba_data(8, 6).into(),
        };
        assert_valid_png(process_clipboard_image(&arboard_image, None));
    }

    #[test]
    fn test_invalid_data_rejection() {
        let arboard_image = arboard::ImageData {
            width: 10,
            height: 10,
            bytes: vec![1, 2, 3, 4, 5].into(),
        };
        assert!(process_clipboard_image(&arboard_image, None).is_none());
    }

    #[test]
    fn test_various_dimensions() {
        for (w, h) in [(100, 100), (782, 297), (1, 1)] {
            let arboard_image = arboard::ImageData {
                width: w,
                height: h,
                bytes: create_rgba_data(w, h).into(),
            };
            let result = process_clipboard_image(&arboard_image, None)
                .unwrap_or_else(|| panic!("Failed to process {w}x{h} image"));
            let loaded = image::load_from_memory(&result.data)
                .unwrap_or_else(|e| panic!("Failed to load processed {w}x{h} image: {e}"));
            assert_eq!((loaded.width(), loaded.height()), (w as u32, h as u32));
        }
    }

    #[test]
    fn test_format_preservation_and_detection() {
        let test_cases = vec![
            (create_simple_png(), "image/png", "test.png"),
            (create_simple_jpeg(), "image/jpeg", "test.jpg"),
            (create_simple_gif(), "image/gif", "test.gif"),
            (create_simple_webp(), "image/webp", "test.webp"),
        ];

        for (data, expected_mime, filename) in test_cases {
            let result = try_preserve_original_format(&data, Some(filename.to_string()));
            if let Some(image_data) = result {
                assert_eq!(image_data.mime_type, expected_mime);
                assert_eq!(image_data.filename, Some(filename.to_string()));
                // Format preservation should keep original data
                assert_eq!(image_data.data, data);
            }
        }
    }

    #[test]
    fn test_unsupported_format_fallback() {
        // Create some random data that doesn't match any supported format
        let unsupported_data = vec![0x50, 0x4B, 0x03, 0x04]; // ZIP signature
        let arboard_image = arboard::ImageData {
            width: 4,
            height: 4,
            bytes: unsupported_data.into(),
        };

        // Should return None since ZIP is not a supported image format
        let result = process_clipboard_image(&arboard_image, None);
        assert!(result.is_none(), "Should reject unsupported format");
    }

    #[test]
    fn test_convert_raw_bitmap_to_png() {
        // Test valid conversion
        let width = 2;
        let height = 2;
        let rgba_data = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];

        let result =
            convert_raw_bitmap_to_png(width, height, rgba_data, Some("test.png".to_string()));
        if let Some(image_data) = result {
            assert_eq!(image_data.mime_type, "image/png");
            assert_eq!(image_data.filename, Some("test.png".to_string()));
            assert!(!image_data.data.is_empty());
        }

        // Test invalid dimensions
        let result = convert_raw_bitmap_to_png(usize::MAX, 1, vec![255, 0, 0, 255], None);
        assert!(result.is_none());
    }
}

// ============================================================================
// CLIPBOARD CONTENT STRUCTURE TESTS
// ============================================================================

#[test]
fn test_clipboard_content_with_images() {
    let image_data = ImageData {
        data: vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
        mime_type: "image/png".to_string(),
        filename: Some("test.png".to_string()),
    };

    let content = ClipboardContent {
        plain_text: "Test text".to_string(),
        html: Some(r##"<img src="test.png">"##.to_string()),
        images: Some(vec![image_data.clone()]),
        paths: None,
    };

    assert!(!content.is_empty());
    assert!(content.images.is_some());
    assert_eq!(content.images.as_ref().unwrap().len(), 1);
    assert_eq!(content.images.as_ref().unwrap()[0].mime_type, "image/png");
}
