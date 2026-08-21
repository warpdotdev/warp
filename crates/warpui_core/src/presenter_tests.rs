use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use crate::presenter::{COMMITTED_POSITION_SCENE_BUILD_LIFETIME, PositionCache};

/// The most invalidate/layout/paint passes `AppContext::build_scene` runs for a single
/// presented redraw.
const MAX_SCENE_BUILDS_PER_REDRAW: u64 = 3;

/// The presented-redraw margin [`COMMITTED_POSITION_SCENE_BUILD_LIFETIME`] is budgeted to
/// guarantee even when every redraw costs the worst-case number of scene builds.
const INTENDED_REDRAW_MARGIN: u64 = 600;

fn rect(size: f32) -> RectF {
    RectF::new(Vector2F::zero(), Vector2F::new(size, size))
}

/// Simulates one scene build: the per-build reset followed by one namespace in which every
/// painted element re-caches its position.
fn build_scene(position_cache: &mut PositionCache, painted_position_ids: &[&str]) {
    position_cache.clear_single_frame_positions();
    position_cache.start();
    for position_id in painted_position_ids {
        position_cache.cache_position_indefinitely((*position_id).to_string(), rect(100.0));
    }
    position_cache.end();
}

/// Simulates one presented redraw that costs the worst-case number of scene builds.
fn worst_case_redraw(position_cache: &mut PositionCache, painted_position_ids: &[&str]) {
    for _ in 0..MAX_SCENE_BUILDS_PER_REDRAW {
        build_scene(position_cache, painted_position_ids);
    }
}

#[test]
fn test_position_cache_caching() {
    let mut position_cache = PositionCache::new();
    position_cache.start();

    position_cache.cache_position_indefinitely(
        "position_1".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(100.0, 100.0)),
    );
    position_cache.cache_position_for_one_frame(
        "position_2".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(50.0, 50.0)),
    );

    position_cache.start();
    position_cache.cache_position_indefinitely(
        "position_1".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(25.0, 25.0)),
    );
    position_cache.cache_position_indefinitely(
        "position_2".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(10.0, 10.0)),
    );
    position_cache.cache_position_for_one_frame(
        "position_3".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(5.0, 5.0)),
    );
    assert_eq!(position_cache.get_position("position_1"), None);

    position_cache.end();
    assert_eq!(
        position_cache.get_position("position_1"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(25.0, 25.0)))
    );
    assert_eq!(
        position_cache.get_position("position_2"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(10.0, 10.0)))
    );
    assert_eq!(
        position_cache.get_position("position_3"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(5.0, 5.0)))
    );

    position_cache.end();
    assert_eq!(
        position_cache.get_position("position_1"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(100.0, 100.0)))
    );
    assert_eq!(
        position_cache.get_position("position_2"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(50.0, 50.0)))
    );
    assert_eq!(
        position_cache.get_position("position_3"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(5.0, 5.0)))
    );

    position_cache.clear_single_frame_positions();
    assert_eq!(
        position_cache.get_position("position_1"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(100.0, 100.0)))
    );
    assert_eq!(
        position_cache.get_position("position_2"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(50.0, 50.0)))
    );
    assert_eq!(position_cache.get_position("position_3"), None);

    position_cache.clear_position("position_1");
    assert_eq!(position_cache.get_position("position_1"), None);
}

#[test]
fn test_committed_positions_survive_a_brief_gap_in_painting() {
    let mut position_cache = PositionCache::new();
    build_scene(&mut position_cache, &["transiently_hidden"]);

    for _ in 0..(COMMITTED_POSITION_SCENE_BUILD_LIFETIME - 1) {
        build_scene(&mut position_cache, &[]);
    }

    assert_eq!(
        position_cache.get_position("transiently_hidden"),
        Some(rect(100.0))
    );
}

#[test]
fn test_committed_positions_expire_once_their_element_stops_painting() {
    let mut position_cache = PositionCache::new();

    for scene_build in 0..=COMMITTED_POSITION_SCENE_BUILD_LIFETIME {
        let painted: &[&str] = if scene_build == 0 {
            &["painted_every_scene_build", "painted_once"]
        } else {
            &["painted_every_scene_build"]
        };
        build_scene(&mut position_cache, painted);
    }

    assert_eq!(
        position_cache.get_position("painted_every_scene_build"),
        Some(rect(100.0))
    );
    assert_eq!(position_cache.get_position("painted_once"), None);
    assert_eq!(position_cache.committed_position_count(), 1);
}

/// The expiry clock ticks per scene build, and a single presented redraw can cost up to
/// three of them, so the lifetime must be budgeted in scene builds for the redraw margin to
/// hold in the worst case.
#[test]
fn test_committed_positions_survive_the_worst_case_redraw_budget() {
    let mut position_cache = PositionCache::new();
    let redraw_margin = COMMITTED_POSITION_SCENE_BUILD_LIFETIME / MAX_SCENE_BUILDS_PER_REDRAW;
    assert!(
        redraw_margin >= INTENDED_REDRAW_MARGIN,
        "lifetime of {COMMITTED_POSITION_SCENE_BUILD_LIFETIME} scene builds only guarantees \
         {redraw_margin} redraws, short of the intended {INTENDED_REDRAW_MARGIN}"
    );

    worst_case_redraw(&mut position_cache, &["stopped_painting"]);
    for _ in 1..redraw_margin {
        worst_case_redraw(&mut position_cache, &[]);
    }

    assert_eq!(
        position_cache.get_position("stopped_painting"),
        Some(rect(100.0))
    );

    for _ in 0..redraw_margin {
        worst_case_redraw(&mut position_cache, &[]);
    }

    assert_eq!(position_cache.get_position("stopped_painting"), None);
    assert_eq!(position_cache.committed_position_count(), 0);
}
