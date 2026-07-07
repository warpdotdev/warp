use super::*;
use crate::util::color::OPAQUE;

// TODO(CORE-3626): figure out why the colors returned on Windows are slightly different.
#[test]
#[cfg(all(not(target_family = "wasm"), not(windows)))]
fn top_colors_jellyfish_test() {
    let jellyfish_bg_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "assets",
        "async",
        "jpg",
        "jellyfish_bg.jpg",
    ]
    .iter()
    .collect();

    let colors = top_colors_for_image(jellyfish_bg_path)
        .expect("should be able to get colors from jellyfish bg");

    assert_eq!(colors[0], ColorU::new(14, 13, 30, OPAQUE));
    assert_eq!(colors[1], ColorU::new(23, 22, 55, OPAQUE));
    assert_eq!(colors[2], ColorU::new(94, 38, 70, OPAQUE));
    assert_eq!(colors[3], ColorU::new(52, 68, 91, OPAQUE));
    assert_eq!(colors[4], ColorU::new(112, 118, 129, OPAQUE));
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn top_colors_invalid_image_test() {
    let invalid_image_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "assets",
        "async",
        "jpg",
        "this_doesnt_exist.jpg",
    ]
    .iter()
    .collect();

    let colors = top_colors_for_image(invalid_image_path);
    assert!(colors.is_err());
}

#[test]
fn accent_colors_contrast_test() {
    let foreground = ColorU::white();
    let background = ColorU::black();
    let accent_options = [
        ColorU::new(255, 0, 0, OPAQUE),
        ColorU::new(100, 0, 0, OPAQUE),
        ColorU::new(10, 0, 0, OPAQUE),
    ];
    assert_eq!(
        accent_options[1],
        pick_accent_color_from_options(&[background, foreground], &accent_options)
    );
}
