use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use crate::presenter::PositionCache;

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
