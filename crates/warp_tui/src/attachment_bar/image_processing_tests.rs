use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose;
use futures_lite::future::block_on;
use warpui_core::clipboard::{ClipboardContent, ImageData};

use super::{MAX_IMAGE_SIZE_BYTES, parse_image_paths, process_clipboard_content, process_paths};

const ONE_PIXEL_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

#[test]
fn parses_single_and_quoted_image_paths() {
    let cwd = Path::new("/workspace");
    assert_eq!(
        parse_image_paths("image.png", cwd).unwrap(),
        vec![cwd.join("image.png")]
    );
    assert_eq!(
        parse_image_paths("'screenshots/image one.webp'", cwd).unwrap(),
        vec![cwd.join("screenshots/image one.webp")]
    );
}

#[test]
fn parses_multiple_image_paths_in_order() {
    let cwd = Path::new("/workspace");
    assert_eq!(
        parse_image_paths("one.png two.jpg", cwd).unwrap(),
        vec![cwd.join("one.png"), cwd.join("two.jpg")]
    );
}

#[test]
fn rejects_mixed_or_non_image_pastes() {
    let cwd = Path::new("/workspace");
    assert!(parse_image_paths("one.png notes.txt", cwd).is_none());
    assert!(parse_image_paths("ordinary prompt text", cwd).is_none());
}

#[test]
fn processes_valid_images_in_paste_order() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.png");
    let second = directory.path().join("second.png");
    let png = general_purpose::STANDARD.decode(ONE_PIXEL_PNG).unwrap();
    std::fs::write(&first, &png).unwrap();
    std::fs::write(&second, &png).unwrap();

    let images = block_on(process_paths(vec![first, second])).unwrap();

    assert_eq!(
        images
            .iter()
            .map(|image| image.file_name.as_str())
            .collect::<Vec<_>>(),
        ["first.png", "second.png"]
    );
    assert!(images.iter().all(|image| !image.data.is_empty()));
}

#[test]
fn processing_is_all_or_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let valid = directory.path().join("valid.png");
    let invalid = directory.path().join("invalid.png");
    let png = general_purpose::STANDARD.decode(ONE_PIXEL_PNG).unwrap();
    std::fs::write(&valid, png).unwrap();
    std::fs::write(&invalid, b"not an image").unwrap();

    assert!(block_on(process_paths(vec![valid, invalid])).is_err());
}

#[test]
fn rejects_oversized_image_before_reading_it() {
    let directory = tempfile::tempdir().unwrap();
    let oversized = directory.path().join("oversized.png");
    let file = std::fs::File::create(&oversized).unwrap();
    file.set_len(u64::try_from(MAX_IMAGE_SIZE_BYTES).unwrap() + 1)
        .unwrap();

    assert_eq!(
        block_on(process_paths(vec![oversized.clone()])).unwrap_err(),
        format!("Image is too large: {}.", oversized.display())
    );
}

#[test]
fn processes_clipboard_image_content() {
    let png = general_purpose::STANDARD.decode(ONE_PIXEL_PNG).unwrap();
    let content = ClipboardContent {
        images: Some(vec![ImageData {
            data: png,
            mime_type: "image/png".to_owned(),
            filename: Some("clipboard.png".to_owned()),
        }]),
        ..Default::default()
    };

    let context = process_clipboard_content(content).unwrap();

    assert_eq!(context.mime_type, "image/png");
    assert_eq!(context.file_name, "clipboard.png");
    assert!(!context.data.is_empty());
}

#[test]
fn rejects_clipboard_content_without_an_image() {
    assert_eq!(
        process_clipboard_content(ClipboardContent::plain_text("text".to_owned())).unwrap_err(),
        "The clipboard does not contain an image."
    );
}
