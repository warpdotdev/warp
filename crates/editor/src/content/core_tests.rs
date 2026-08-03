use markdown_parser::{FormattedImage, FormattedText, FormattedTextLine};

use super::*;

/// A base64 `data:` image whose payload exceeds the asset layer's render limit.
fn oversized_data_uri_image() -> FormattedImage {
    let payload = "A".repeat(asset_cache::MAX_DATA_URI_PAYLOAD_BYTES + 1);
    FormattedImage {
        alt_text: "output".to_string(),
        source: format!("data:image/png;base64,{payload}"),
        title: None,
    }
}

#[test]
fn replace_oversized_data_uri_images_swaps_in_placeholder() {
    // An in-limit image that must be left untouched.
    let small = FormattedImage {
        alt_text: "output".to_string(),
        source: "data:image/png;base64,iVBORw0KGgo=".to_string(),
        title: None,
    };

    let text = FormattedText::new(vec![
        FormattedTextLine::Image(oversized_data_uri_image()),
        FormattedTextLine::Image(small.clone()),
    ]);

    let result = replace_oversized_data_uri_images(text);
    let lines: Vec<_> = result.lines.into_iter().collect();

    // The oversized image becomes a visible placeholder text line ...
    assert!(matches!(&lines[0], FormattedTextLine::Line(_)));
    assert_eq!(
        lines[0].raw_text(),
        format!("{IMAGE_TOO_LARGE_PLACEHOLDER}\n")
    );

    // ... while an in-limit image is left untouched.
    assert_eq!(lines[1], FormattedTextLine::Image(small));
}
