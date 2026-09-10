//! Tests for the display-scoped window crop geometry. Pure arithmetic, so they
//! run on every host regardless of which capture substrate is available.

use super::{CaptureCrop, PointRect, window_crop_in_capture_space};

fn rect(x: f64, y: f64, width: f64, height: f64) -> PointRect {
    PointRect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn scales_window_points_into_retina_capture_pixels() {
    let crop = window_crop_in_capture_space(
        rect(100.0, 50.0, 400.0, 300.0),
        (0.0, 0.0),
        2.0,
        (2560, 1600),
    )
    .expect("a contained window resolves");

    assert_eq!(
        crop,
        CaptureCrop {
            x: 200,
            y: 100,
            width: 800,
            height: 600,
        }
    );
    assert_eq!(crop.filter_arg(), "crop=800:600:200:100");
}

#[test]
fn maps_points_one_to_one_on_a_non_retina_display() {
    let crop = window_crop_in_capture_space(
        rect(10.0, 20.0, 640.0, 480.0),
        (0.0, 0.0),
        1.0,
        (1920, 1080),
    )
    .expect("a contained window resolves");

    assert_eq!(
        crop,
        CaptureCrop {
            x: 10,
            y: 20,
            width: 640,
            height: 480,
        }
    );
}

#[test]
fn subtracts_a_non_zero_display_origin() {
    // A secondary-positioned main display: window points are global, the crop is
    // relative to the captured frame.
    let crop = window_crop_in_capture_space(
        rect(1540.0, 320.0, 500.0, 400.0),
        (1440.0, 300.0),
        2.0,
        (2560, 1600),
    )
    .expect("a contained window resolves");

    assert_eq!(
        crop,
        CaptureCrop {
            x: 200,
            y: 40,
            width: 1000,
            height: 800,
        }
    );
}

#[test]
fn rounds_offsets_and_sizes_down_to_even_pixels() {
    // 10.5 pt * 1x -> 11 px offset and 401 px width, neither of which is a
    // chroma sample boundary.
    let crop = window_crop_in_capture_space(
        rect(10.5, 20.5, 400.5, 300.5),
        (0.0, 0.0),
        1.0,
        (1920, 1080),
    )
    .expect("a contained window resolves");

    assert_eq!(crop.x % 2, 0);
    assert_eq!(crop.y % 2, 0);
    assert_eq!(crop.width % 2, 0);
    assert_eq!(crop.height % 2, 0);
    assert!(u64::from(crop.x) + u64::from(crop.width) <= 1920);
    assert!(u64::from(crop.y) + u64::from(crop.height) <= 1080);
}

#[test]
fn clamps_a_rounding_overshoot_at_the_frame_edge() {
    // The right edge converts to 1920.6 px, one rounding step past the frame.
    let crop = window_crop_in_capture_space(
        rect(1420.0, 0.0, 500.6, 540.0),
        (0.0, 0.0),
        1.0,
        (1920, 1080),
    )
    .expect("a rounding overshoot is clamped, not rejected");

    assert_eq!(crop.x, 1420);
    assert_eq!(crop.x + crop.width, 1920);
}

#[test]
fn rejects_a_window_that_extends_past_the_frame() {
    let error = window_crop_in_capture_space(
        rect(1800.0, 0.0, 400.0, 540.0),
        (0.0, 0.0),
        1.0,
        (1920, 1080),
    )
    .expect_err("a window hanging off the display is not croppable");

    assert!(error.contains("capture frame"), "{error}");
}

#[test]
fn rejects_a_window_starting_before_the_frame() {
    let error = window_crop_in_capture_space(
        rect(-40.0, 0.0, 400.0, 540.0),
        (0.0, 0.0),
        1.0,
        (1920, 1080),
    )
    .expect_err("a window starting off the display is not croppable");

    assert!(error.contains("capture frame"), "{error}");
}

#[test]
fn rejects_a_zero_sized_window() {
    for window in [
        rect(0.0, 0.0, 0.0, 100.0),
        rect(0.0, 0.0, 100.0, 0.0),
        rect(0.0, 0.0, -100.0, 100.0),
    ] {
        let error = window_crop_in_capture_space(window, (0.0, 0.0), 2.0, (1920, 1080))
            .expect_err("a non-positive window is not croppable");
        assert!(error.contains("usable rectangle"), "{error}");
    }
}

#[test]
fn rejects_a_window_too_small_to_encode() {
    // 0.6 pt at 1x rounds to a single pixel, below the even-size floor.
    let error =
        window_crop_in_capture_space(rect(10.0, 10.0, 0.6, 0.6), (0.0, 0.0), 1.0, (1920, 1080))
            .expect_err("a sub-2px window is not encodable");

    assert!(error.contains("too small to encode"), "{error}");
}

#[test]
fn rejects_an_unusable_backing_scale() {
    for scale in [0.0, -2.0, f64::NAN] {
        let error = window_crop_in_capture_space(
            rect(0.0, 0.0, 100.0, 100.0),
            (0.0, 0.0),
            scale,
            (1920, 1080),
        )
        .expect_err("an unknown backing scale is not croppable");
        assert!(error.contains("backing scale"), "{error}");
    }
}

#[test]
fn rejects_an_empty_capture_frame() {
    let error = window_crop_in_capture_space(rect(0.0, 0.0, 100.0, 100.0), (0.0, 0.0), 2.0, (0, 0))
        .expect_err("an empty frame is not croppable");

    assert!(error.contains("capture frame"), "{error}");
}
