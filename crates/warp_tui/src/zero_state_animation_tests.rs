use super::{
    StarfieldState, WARP_INITIAL, WARP_TARGET, star_count_for_max_r, warp_at, xorshift_f64,
    xorshift64,
};

// ---------------------------------------------------------------------------
// xorshift64 PRNG
// ---------------------------------------------------------------------------

#[test]
fn xorshift64_is_deterministic() {
    let mut s = 0xDEAD_BEEF_CAFE_1234_u64;
    let a = xorshift64(&mut s);
    let mut s2 = 0xDEAD_BEEF_CAFE_1234_u64;
    let b = xorshift64(&mut s2);
    assert_eq!(a, b);
}

#[test]
fn xorshift64_produces_different_values_sequentially() {
    let mut s = 12345_u64;
    let a = xorshift64(&mut s);
    let b = xorshift64(&mut s);
    assert_ne!(a, b, "sequential xorshift64 calls should differ");
}

#[test]
fn xorshift_f64_stays_in_unit_interval() {
    let mut s = 0xABCD_EF01_u64;
    for _ in 0..10_000 {
        let v = xorshift_f64(&mut s);
        assert!((0.0..1.0).contains(&v), "xorshift_f64 out of [0,1): {v}");
    }
}

// ---------------------------------------------------------------------------
// Warp spring (analytical solution)
// ---------------------------------------------------------------------------

#[test]
fn warp_at_zero_equals_initial() {
    // At t=0 the spring has not moved yet.
    let w = warp_at(0.0);
    assert!(
        (w - WARP_INITIAL).abs() < 1e-9,
        "warp_at(0) = {w}, expected {WARP_INITIAL}"
    );
}

#[test]
fn warp_at_large_t_converges_to_target() {
    // After 30 seconds the spring is essentially settled.
    let w = warp_at(30.0);
    assert!(
        (w - WARP_TARGET).abs() < 1e-4,
        "warp_at(30) = {w}, should be ≈ {WARP_TARGET}"
    );
}

#[test]
fn warp_increases_monotonically_through_ramp() {
    // Sample at 0.5s intervals; warp should be strictly increasing during
    // the ramp phase (first ~5 seconds).
    let mut prev = warp_at(0.0);
    for i in 1..10 {
        let t = i as f64 * 0.5;
        let w = warp_at(t);
        assert!(
            w > prev,
            "warp should increase: warp_at({t}) = {w} ≤ prev {prev}"
        );
        prev = w;
    }
}

#[test]
fn warp_stays_above_initial_and_below_ceiling() {
    for i in 0..1000 {
        let t = i as f64 * 0.05;
        let w = warp_at(t);
        assert!(
            ((WARP_INITIAL - 0.01)..=(WARP_TARGET + 0.1)).contains(&w),
            "warp_at({t}) = {w} out of allowed range"
        );
    }
}

// ---------------------------------------------------------------------------
// StarfieldState
// ---------------------------------------------------------------------------

#[test]
fn starfield_state_initializes_with_correct_star_count() {
    let sf = StarfieldState::new();
    assert_eq!(sf.stars.len(), super::NUM_STARS);
}

#[test]
fn starfield_state_stars_start_near_centre() {
    let sf = StarfieldState::new();
    let init_max_r = 62.5_f64;
    for (i, star) in sf.stars.iter().enumerate() {
        assert!(
            star.r <= init_max_r * 0.15 + 1e-9,
            "star[{i}].r = {} exceeds init range",
            star.r
        );
    }
}

#[test]
fn starfield_state_star_speeds_in_range() {
    let sf = StarfieldState::new();
    for (i, star) in sf.stars.iter().enumerate() {
        assert!(
            star.speed >= 0.5 && star.speed <= 1.5,
            "star[{i}].speed = {} out of [0.5, 1.5]",
            star.speed
        );
    }
}

#[test]
fn starfield_simulate_advances_stars_outward() {
    let mut sf = StarfieldState::new();
    let initial_r: Vec<f64> = sf.stars.iter().map(|s| s.r).collect();
    sf.simulate_to(1.0); // 1 real second → ~60 sim steps
    let advanced = sf
        .stars
        .iter()
        .zip(&initial_r)
        .filter(|(star, r0)| star.r > **r0)
        .count();
    // At least 90% of stars should have moved outward.
    assert!(
        advanced > super::NUM_STARS * 9 / 10,
        "only {advanced}/{} stars moved outward",
        super::NUM_STARS
    );
}

#[test]
fn starfield_stars_reset_when_they_leave_screen() {
    let mut sf = StarfieldState::new();
    // Override max_r to something tiny so stars leave immediately.
    let tiny_max_r = 5.0;
    sf.set_dimensions(tiny_max_r, star_count_for_max_r(tiny_max_r));
    sf.simulate_to(5.0); // 5 seconds
    // All stars should be back near centre (r < tiny_max_r).
    for (i, star) in sf.stars.iter().enumerate() {
        assert!(
            star.r < tiny_max_r,
            "star[{i}].r = {} should be < max_r = {tiny_max_r}",
            star.r,
        );
    }
}

#[test]
fn starfield_simulate_is_deterministic_for_same_duration() {
    // Two independent state machines simulated to the same target should agree.
    let mut sf1 = StarfieldState::new();
    let mut sf2 = StarfieldState::new();
    sf1.simulate_to(2.0);
    sf2.simulate_to(2.0);
    for (i, (s1, s2)) in sf1.stars.iter().zip(sf2.stars.iter()).enumerate() {
        assert!(
            (s1.r - s2.r).abs() < 1e-9,
            "star[{i}].r diverged: {:.6} vs {:.6}",
            s1.r,
            s2.r
        );
    }
}
