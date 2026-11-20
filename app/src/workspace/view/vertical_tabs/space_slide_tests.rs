use std::time::Duration;

use instant::Instant;

use super::{DURATION, SlideFrom, SpaceSlide, ease_out_cubic};

const WIDTH: f32 = 240.;

fn slide_from(from: SlideFrom) -> (SpaceSlide, Instant) {
    let start = Instant::now();
    (
        SpaceSlide {
            from,
            started_at: start,
        },
        start,
    )
}

#[test]
fn higher_index_slides_in_from_the_right_lower_from_the_left() {
    let now = Instant::now();
    let from = |from_index, to_index| {
        SpaceSlide::between(from_index, to_index, now).map(|slide| slide.from)
    };
    assert_eq!(from(0, 1), Some(SlideFrom::Right));
    assert_eq!(from(0, 3), Some(SlideFrom::Right));
    assert_eq!(from(2, 1), Some(SlideFrom::Left));
    assert_eq!(from(3, 0), Some(SlideFrom::Left));
    assert_eq!(from(1, 1), None);
}

#[test]
fn ease_out_cubic_starts_fast_and_lands_at_one() {
    assert_eq!(ease_out_cubic(0.), 0.);
    assert_eq!(ease_out_cubic(1.), 1.);
    assert_eq!(ease_out_cubic(0.5), 0.875);
    // Out-of-range progress is clamped.
    assert_eq!(ease_out_cubic(-1.), 0.);
    assert_eq!(ease_out_cubic(2.), 1.);
}

#[test]
fn offset_starts_a_full_width_out_on_the_entry_side() {
    let (right, start) = slide_from(SlideFrom::Right);
    assert_eq!(right.offset_at(WIDTH, start), Some(WIDTH));
    let (left, start) = slide_from(SlideFrom::Left);
    assert_eq!(left.offset_at(WIDTH, start), Some(-WIDTH));
}

#[test]
fn offset_shrinks_monotonically_toward_zero() {
    let (slide, start) = slide_from(SlideFrom::Right);
    let offsets = (0..DURATION.as_millis() as u64)
        .step_by(8)
        .map(|ms| {
            slide
                .offset_at(WIDTH, start + Duration::from_millis(ms))
                .expect("slide is still running")
        })
        .collect::<Vec<_>>();
    assert!(offsets.windows(2).all(|pair| pair[1] < pair[0]));
    assert!(offsets.iter().all(|offset| *offset > 0.));

    // Ease-out: well past halfway in distance at half the time.
    let halfway = slide
        .offset_at(WIDTH, start + DURATION / 2)
        .expect("slide is still running");
    assert!((halfway - WIDTH * 0.125).abs() < 0.01);
}

#[test]
fn slide_is_over_after_its_duration() {
    let (slide, start) = slide_from(SlideFrom::Left);
    assert!(
        slide
            .offset_at(WIDTH, start + DURATION - Duration::from_millis(1))
            .is_some()
    );
    assert_eq!(slide.offset_at(WIDTH, start + DURATION), None);
    assert_eq!(slide.offset_at(WIDTH, start + DURATION * 10), None);
}
