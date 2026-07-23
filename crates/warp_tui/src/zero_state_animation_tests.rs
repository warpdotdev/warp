use super::{cell_hash, ivy_luminance, noise2, portrait_luminance};

// ---------------------------------------------------------------------------
// portrait_luminance: output range
// ---------------------------------------------------------------------------

#[test]
fn portrait_luminance_always_in_range() {
    // Sample a dense grid of normalised coordinates and raw integer coords.
    for row in -30..=30_i32 {
        for col in -30..=30_i32 {
            let nr = row as f64 / 30.0;
            let nc = col as f64 / 30.0;
            let lum = portrait_luminance(nr, nc, row, col);
            assert!(
                lum <= 9,
                "portrait_luminance({nr:.2}, {nc:.2}, {row}, {col}) = {lum} > 9"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Portrait anatomical regions
// ---------------------------------------------------------------------------

#[test]
fn face_centre_is_skin() {
    // The centre of the panel (nr=0, nc=0) is well inside the face oval and
    // should render as skin luminance (5 or 6).
    let lum = portrait_luminance(0.0, 0.0, 0, 0);
    assert!(
        (5..=6).contains(&lum),
        "face centre should be skin (5–6), got {lum}"
    );
}

#[test]
fn ivy_corners_are_dim() {
    // The four extreme corners of the panel are background ivy (luminance 1–3).
    for (nr, nc, row, col) in [
        (-0.95f64, -0.95f64, -28i32, -28i32),
        (-0.95, 0.95, -28, 28),
        (0.95, -0.95, 28, -28),
        (0.95, 0.95, 28, 28),
    ] {
        let lum = portrait_luminance(nr, nc, row, col);
        assert!(
            (1..=3).contains(&lum),
            "corner ({nr:.2}, {nc:.2}) should be ivy (1–3), got {lum}"
        );
    }
}

#[test]
fn face_brighter_than_background() {
    let face_lum = portrait_luminance(0.0, 0.0, 0, 0);
    let bg_lum = portrait_luminance(-0.90, -0.90, -27, -27);
    assert!(
        face_lum > bg_lum,
        "face centre ({face_lum}) should be brighter than ivy background ({bg_lum})"
    );
}

#[test]
fn hair_region_is_bright() {
    // Just above the face oval at the top-centre: this is the hair zone.
    // nr ≈ −0.90, nc = 0 → face_dist_sq ≈ (−0.90/0.80)² = 1.266 < 1.45, nr < 0.20.
    let lum = portrait_luminance(-0.90, 0.0, -27, 0);
    assert!(
        (7..=8).contains(&lum),
        "hair region (top centre) should be 7–8 (#/@), got {lum}"
    );
}

#[test]
fn suit_sides_are_dark() {
    // nr > 0.55, non-tie position: suit jacket → luminance 0–1.
    for (nc, col) in [(-0.50f64, -15i32), (0.50, 15)] {
        let lum = portrait_luminance(0.70, nc, 21, col);
        assert!(
            lum <= 1,
            "suit (non-tie) at (0.70, {nc:.2}) should be 0–1, got {lum}"
        );
    }
}

#[test]
fn tie_has_mid_luminance() {
    // Tie (nc ≈ 0, nr > 0.55): luminance 3–4 ('-' or '=').
    let lum = portrait_luminance(0.70, 0.0, 21, 0);
    assert!(
        (3..=4).contains(&lum),
        "tie region should be 3–4, got {lum}"
    );
}

#[test]
fn glasses_frame_is_bright_seven() {
    // Nose bridge (nc=0) is inside the glasses band (nr ≈ −0.19) but outside
    // both lens ovals → should return exactly 7 ('#').
    let lum = portrait_luminance(-0.19, 0.0, -6, 0);
    assert_eq!(lum, 7, "glasses frame/bridge should be 7 ('#'), got {lum}");
}

#[test]
fn glasses_lens_interior_is_darker_than_frame() {
    // Left lens centre: nc ≈ −0.18, nr ≈ −0.185.
    // left_lc = (−0.18 + 0.18) / 0.14 = 0, lens_rc = 0 → left_dist = 0 < 1 → inside lens.
    let lens_lum = portrait_luminance(-0.185, -0.18, -6, -5);
    let frame_lum = portrait_luminance(-0.19, 0.0, -6, 0); // = 7
    assert!(
        lens_lum < frame_lum,
        "lens interior ({lens_lum}) should be dimmer than frame ({frame_lum})"
    );
    assert!(
        (3..=4).contains(&lens_lum),
        "lens interior should be 3–4, got {lens_lum}"
    );
}

#[test]
fn glasses_frame_and_lens_are_inside_face_oval() {
    // Confirm the glasses band is inside the face oval at nc = 0.
    // face_dist_sq = (0/0.45)^2 + (−0.19/0.80)^2 = 0.0564 < 1.0
    let fc = 0.0_f64 / 0.45;
    let fr = -0.19_f64 / 0.80;
    let face_dist_sq = fc * fc + fr * fr;
    assert!(
        face_dist_sq < 1.0,
        "glasses coordinates should be inside face oval, got face_dist_sq = {face_dist_sq:.4}"
    );
}

// ---------------------------------------------------------------------------
// ivy_luminance
// ---------------------------------------------------------------------------

#[test]
fn ivy_luminance_always_in_one_to_three() {
    for row in -20..=20_i32 {
        for col in -20..=20_i32 {
            let lum = ivy_luminance(row, col);
            assert!(
                (1..=3).contains(&lum),
                "ivy_luminance({row}, {col}) = {lum}, expected 1–3"
            );
        }
    }
}

#[test]
fn ivy_luminance_varies_spatially() {
    // The texture should not be flat: at least two distinct values in an 8×8 sample.
    let vals: std::collections::HashSet<u8> = (0..8_i32)
        .flat_map(|r| (0..8_i32).map(move |c| ivy_luminance(r, c)))
        .collect();
    assert!(
        vals.len() >= 2,
        "ivy texture should have at least 2 distinct luminance values"
    );
}

// ---------------------------------------------------------------------------
// noise2
// ---------------------------------------------------------------------------

#[test]
fn noise2_returns_zero_or_one() {
    for row in -10..=10_i32 {
        for col in -10..=10_i32 {
            let n = noise2(row, col);
            assert!(n <= 1, "noise2({row}, {col}) = {n}, expected 0 or 1");
        }
    }
}

// ---------------------------------------------------------------------------
// cell_hash
// ---------------------------------------------------------------------------

#[test]
fn cell_hash_is_deterministic() {
    assert_eq!(cell_hash(7, 13, 99), cell_hash(7, 13, 99));
}

#[test]
fn cell_hash_varies_with_each_argument() {
    let base = cell_hash(5, 10, 42);
    assert_ne!(cell_hash(6, 10, 42), base, "hash should differ when row changes");
    assert_ne!(cell_hash(5, 11, 42), base, "hash should differ when col changes");
    assert_ne!(cell_hash(5, 10, 43), base, "hash should differ when frame changes");
}

#[test]
fn cell_hash_flicker_rate_near_four_percent() {
    // Approximately 1/25 (4%) of cells should trigger the flicker condition.
    let sample = 1_000u64;
    let count = (0..sample)
        .filter(|&i| cell_hash(i, i % 7, i % 13) % 25 == 0)
        .count();
    // Allow a wide margin (factor of 3×) around the expected 40 hits.
    assert!(
        (13..120).contains(&count),
        "flicker rate {count}/{sample} far from expected ~40"
    );
}

#[test]
fn cell_hash_accent_rate_near_one_point_five_percent() {
    // Approximately 1/67 (~1.5%) of bright cells should get the accent colour.
    // Sample using the condition: lum >= 6 (held fixed) AND h % 67 == 1.
    let sample = 1_000u64;
    let count = (0..sample)
        .filter(|&i| cell_hash(i, i.wrapping_mul(3), i % 17) % 67 == 1)
        .count();
    // Expect roughly 15 hits; allow a 4× margin.
    assert!(
        (4..60).contains(&count),
        "accent rate {count}/{sample} far from expected ~15"
    );
}
