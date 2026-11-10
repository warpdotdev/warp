use computer_use::{Screenshot, Vector2I};

use super::*;

/// Builds the geometry the real actors would report for a full-screen capture.
fn geometry(
    native_width: u32,
    native_height: u32,
    constraints: ImageConstraints,
) -> ScreenGeometry {
    let scale = constraints.scale_factor(native_width, native_height);
    let scaled = |value: u32| (value as f64 * scale).round() as usize;
    let screenshot = Screenshot {
        width: scaled(native_width),
        height: scaled(native_height),
        original_width: native_width as usize,
        original_height: native_height as usize,
        data: Vec::new(),
        mime_type: "image/png".into(),
    };
    ScreenGeometry::from_screenshot(&screenshot, constraints)
}

/// warp-server's `GetScaleFactor`, which the mapping must agree with.
fn server_scale_factor(width: u32, height: u32, constraints: ImageConstraints) -> f64 {
    let long_edge_scale = constraints.max_long_edge_px as f64 / width.max(height) as f64;
    let total_scale = (constraints.max_total_px as f64 / (width as f64 * height as f64)).sqrt();
    long_edge_scale.min(total_scale).min(1.0)
}

#[test]
fn scale_factor_matches_server() {
    for (width, height) in [(2940, 1912), (1920, 1080), (3840, 2160), (1024, 768)] {
        for constraints in [ImageConstraints::ANTHROPIC, ImageConstraints::OPENAI] {
            assert_eq!(
                constraints.scale_factor(width, height),
                server_scale_factor(width, height, constraints),
                "{width}x{height} with {constraints:?}"
            );
        }
    }
}

#[test]
fn downscaled_points_map_back_to_native_pixels() {
    let geometry = geometry(2940, 1912, ImageConstraints::ANTHROPIC);
    let scale = server_scale_factor(2940, 1912, ImageConstraints::ANTHROPIC);
    assert_eq!(
        geometry.to_native(Point { x: 100, y: 200 }).unwrap(),
        Vector2I::new((100.0 / scale) as i32, (200.0 / scale) as i32)
    );
}

#[test]
fn screens_within_constraints_are_not_scaled() {
    let geometry = geometry(1024, 768, ImageConstraints::ANTHROPIC);
    assert_eq!(geometry.image_size(), (1024, 768));
    assert_eq!(
        geometry.to_native(Point { x: 1023, y: 767 }).unwrap(),
        Vector2I::new(1023, 767)
    );
}

#[test]
fn points_outside_the_image_are_rejected() {
    let geometry = geometry(2940, 1912, ImageConstraints::ANTHROPIC);
    let (width, height) = geometry.image_size();
    for point in [
        Point { x: -1, y: 0 },
        Point { x: 0, y: -1 },
        Point {
            x: width as i32,
            y: 0,
        },
        Point {
            x: 0,
            y: height as i32,
        },
    ] {
        let error = geometry.to_native(point).unwrap_err().to_string();
        assert!(error.contains(&format!("outside the {width}x{height} screenshot")));
    }
}

#[test]
fn far_edge_pixels_stay_on_screen() {
    let geometry = geometry(2940, 1912, ImageConstraints::ANTHROPIC);
    let (width, height) = geometry.image_size();
    let native = geometry
        .to_native(Point {
            x: width as i32 - 1,
            y: height as i32 - 1,
        })
        .unwrap();
    assert!((2935..2940).contains(&native.x()), "{native:?}");
    assert!((1907..1912).contains(&native.y()), "{native:?}");
}

#[test]
fn region_corners_may_lie_on_the_far_edges() {
    let geometry = geometry(3840, 2160, ImageConstraints::OPENAI);
    let (width, height) = geometry.image_size();
    let corner = geometry
        .corner_to_native(Point {
            x: width as i32,
            y: height as i32,
        })
        .unwrap();
    assert!((3839..=3840).contains(&corner.x()), "{corner:?}");
    assert!((2159..=2160).contains(&corner.y()), "{corner:?}");
    assert!(
        geometry
            .corner_to_native(Point {
                x: width as i32 + 1,
                y: 0,
            })
            .is_err()
    );
}

#[test]
fn to_model_inverts_to_native() {
    let geometry = geometry(2940, 1912, ImageConstraints::ANTHROPIC);
    for point in [
        Point { x: 0, y: 0 },
        Point { x: 37, y: 512 },
        Point { x: 1000, y: 800 },
    ] {
        let round_trip = geometry.to_model(geometry.to_native(point).unwrap());
        assert!(
            (round_trip.x - point.x).abs() <= 1,
            "{point:?} -> {round_trip:?}"
        );
        assert!(
            (round_trip.y - point.y).abs() <= 1,
            "{point:?} -> {round_trip:?}"
        );
    }
}
