use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};

use super::*;

#[test]
fn foreground_color_with_minimum_contrast_foreground_already_meets_minimum() {
    assert_eq!(
        ColorU::black(),
        foreground_color_with_minimum_contrast(
            ColorU::black(),
            ColorU::white().into(),
            MinimumAllowedContrast::Text
        )
    )
}

#[test]
fn foreground_color_with_minimum_contrast_foreground_blend_darker() {
    let light_grey = ColorU::from_u32(0xAAAAAAFF);
    let white = ColorU::white();

    // Grey on white should not meet the contrast requirements.
    assert!(!high_enough_contrast(
        light_grey,
        white,
        MinimumAllowedContrast::NonText
    ));

    let result = foreground_color_with_minimum_contrast(
        light_grey,
        white.into(),
        MinimumAllowedContrast::NonText,
    );

    assert_ne!(light_grey, result);
    // The suggested color should meet the contrast requirements.
    assert!(contrast_ratio(result, white) > MinimumAllowedContrast::NonText.get());
}

#[test]
fn foreground_color_with_minimum_contrast_blend_lighter() {
    let minimum_allowed_contrast = MinimumAllowedContrast::NonText;

    let grey = ColorU::from_u32(0x333333FF);
    let black = ColorU::black();

    // Grey on black should not meet the contrast requirements.
    assert!(!high_enough_contrast(
        grey,
        black,
        MinimumAllowedContrast::NonText
    ));

    let suggested_color =
        foreground_color_with_minimum_contrast(grey, black.into(), minimum_allowed_contrast);
    assert_ne!(grey, suggested_color);

    // The suggested color should meet the contrast requirements.
    assert!(contrast_ratio(suggested_color, black) > minimum_allowed_contrast.get());
}

#[test]
fn compute_foreground_color_with_minimum_contrast_already_meets_contrast() {
    let white = ColorU::white();
    let black = ColorU::black();

    // White on black should meet the contrast requirements.
    assert!(high_enough_contrast(
        white,
        black,
        MinimumAllowedContrast::NonText
    ));

    // Since white on black has enough contrast, we shouldn't need to change the color.
    assert_eq!(
        foreground_color_with_minimum_contrast(
            white,
            black.into(),
            MinimumAllowedContrast::NonText,
        ),
        white
    );
}

#[test]
fn compute_foreground_color_with_minimum_contrast_same_color() {
    let black = ColorU::black();

    // black on black should _not_ meet the contrast requirements.
    assert!(!high_enough_contrast(
        black,
        black,
        MinimumAllowedContrast::NonText
    ));

    // Since white on black has enough contrast, we shouldn't need to change the color.
    let suggested_color = foreground_color_with_minimum_contrast(
        black,
        black.into(),
        MinimumAllowedContrast::NonText,
    );

    assert!(high_enough_contrast(
        suggested_color,
        black,
        MinimumAllowedContrast::NonText
    ));
}

/// Test that ensures that a random foreground color against a background color produces
/// a new foreground color that has a minimum contrast after calling
/// `foreground_color_with_minimum_contrast`.
#[test]
fn compute_foreground_color_with_minimum_contrast_random() {
    let minimum_allowed_contrast = MinimumAllowedContrast::NonText;

    for seed in 0..1000 {
        let mut rng = StdRng::seed_from_u64(seed);
        let foreground_color = ColorU::from_u32(rng.r#gen());

        let background_color = ColorU::from_u32(rng.r#gen());

        let suggested_color = foreground_color_with_minimum_contrast(
            foreground_color,
            background_color.into(),
            minimum_allowed_contrast,
        );

        let actual_contrast_ratio = contrast_ratio(suggested_color, background_color);

        let desired_contrast_ratio = minimum_allowed_contrast.get();

        assert!(
            high_enough_contrast(suggested_color, background_color, minimum_allowed_contrast),
            "{foreground_color:?} on {background_color:?} does not have contrast. Expected contrast = {desired_contrast_ratio:?}, actual contrast {actual_contrast_ratio:?}"
        );
    }
}
