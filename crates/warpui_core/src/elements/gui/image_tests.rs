use super::*;

struct TestElement;

impl Element for TestElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        constraint.max
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, _: Vector2F, _: &mut PaintContext, _: &AppContext) {}

    fn size(&self) -> Option<Vector2F> {
        None
    }

    fn origin(&self) -> Option<Point> {
        None
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}

fn test_element() -> Box<dyn Element> {
    Box::new(TestElement)
}

fn test_image() -> Image {
    Image::new(
        AssetSource::Raw {
            id: "test".to_string(),
        },
        CacheOption::BySize,
    )
}

#[test]
fn image_rect_returns_none_for_nan_origin() {
    assert!(
        image_rect(
            vec2f(164.0, 164.0),
            vec2f(f32::NAN, 874.725),
            vec2f(163.75, 163.75),
            false,
            false,
        )
        .is_none()
    );
}

#[test]
fn failed_to_load_prefers_failure_element_when_provided() {
    let image = test_image()
        .before_load(test_element())
        .on_load_failure(test_element());

    assert_eq!(
        image.failed_to_load_backup_element_kind(),
        Some(BackupElementKind::FailedToLoad)
    );
}

#[test]
fn failed_to_load_falls_back_to_before_load_element() {
    let image = test_image().before_load(test_element());

    assert_eq!(
        image.failed_to_load_backup_element_kind(),
        Some(BackupElementKind::BeforeLoad)
    );
}

#[test]
fn loading_image_switches_to_timeout_element_after_timeout() {
    let mut image = test_image()
        .before_load(test_element())
        .on_load_timeout(Duration::from_secs(10), test_element());
    image.clear_load_timeout_started_at();
    let now = Instant::now();

    let (initial_kind, initial_repaint_after) = image.loading_backup_element_kind(now);
    assert_eq!(initial_kind, Some(BackupElementKind::BeforeLoad));
    assert_eq!(initial_repaint_after, Some(Duration::from_secs(10)));

    let (timed_out_kind, timed_out_repaint_after) =
        image.loading_backup_element_kind(now + Duration::from_secs(11));
    assert_eq!(timed_out_kind, Some(BackupElementKind::LoadTimeout));
    assert_eq!(timed_out_repaint_after, None);
}

#[test]
fn loading_timeout_survives_image_rebuild_for_same_source() {
    let mut image = test_image()
        .before_load(test_element())
        .on_load_timeout(Duration::from_secs(10), test_element());
    image.clear_load_timeout_started_at();
    let now = Instant::now();

    let (initial_kind, _initial_repaint_after) = image.loading_backup_element_kind(now);
    assert_eq!(initial_kind, Some(BackupElementKind::BeforeLoad));

    let mut rebuilt_image = test_image()
        .before_load(test_element())
        .on_load_timeout(Duration::from_secs(10), test_element());
    let (timed_out_kind, timed_out_repaint_after) =
        rebuilt_image.loading_backup_element_kind(now + Duration::from_secs(11));
    assert_eq!(timed_out_kind, Some(BackupElementKind::LoadTimeout));
    assert_eq!(timed_out_repaint_after, None);
}

#[test]
fn compute_elapsed_time_ms_uses_started_at_when_set() {
    // This is a regression test for the bug where paint_animated_image would
    // always compute a fresh Instant::now() and ignore self.started_at,
    // causing elapsed_time to be ~0 and animations to freeze on first frame.
    //
    // The fix adds compute_elapsed_time_ms which respects self.started_at.
    // If someone removes the self.started_at check, this test will fail.

    let past_time = Instant::now() - Duration::from_millis(1000);
    let image = test_image().enable_animation_with_start_time(past_time);

    let now = past_time + Duration::from_millis(500);
    let elapsed = image.compute_elapsed_time_ms(now);

    assert_eq!(elapsed, 500, "elapsed time should be 500ms from past_time to now");
    assert!(
        elapsed > 0,
        "elapsed time must be > 0 when started_at is set in the past"
    );
}

#[test]
fn compute_elapsed_time_ms_uses_provided_time_when_started_at_is_none() {
    // When started_at is None (animation not enabled), the provided 'now' time
    // is used as the reference, giving elapsed time of 0 (showing only first frame).

    let image = test_image();
    assert_eq!(
        image.started_at, None,
        "started_at should be None for non-animated setup"
    );

    let now = Instant::now();
    let elapsed = image.compute_elapsed_time_ms(now);

    assert_eq!(elapsed, 0, "elapsed time should be 0 when started_at is None");
}

#[test]
fn compute_elapsed_time_ms_respects_large_time_differences() {
    // Verify that compute_elapsed_time_ms correctly handles larger time spans.
    // This tests that the method actually uses started_at for computation,
    // not recalculating time fresh each time (which would always be ~0).

    let base_time = Instant::now();
    let started_at = base_time - Duration::from_secs(5);
    let image = test_image().enable_animation_with_start_time(started_at);

    let now = started_at + Duration::from_secs(3);
    let elapsed = image.compute_elapsed_time_ms(now);

    assert_eq!(
        elapsed, 3000,
        "elapsed time should be 3000ms (3 seconds) from started_at"
    );

    let now_later = started_at + Duration::from_secs(7);
    let elapsed_later = image.compute_elapsed_time_ms(now_later);

    assert_eq!(
        elapsed_later, 7000,
        "elapsed time should be 7000ms (7 seconds) from started_at"
    );
    assert!(
        elapsed_later > elapsed,
        "elapsed time should increase monotonically"
    );
}
