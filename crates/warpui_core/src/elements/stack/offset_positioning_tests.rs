use super::*;
use lazy_static::lazy_static;

lazy_static! {
    static ref OFFSET: Vector2F = vec2f(5., 10.);
    static ref WINDOW_SIZE: Vector2F = vec2f(100., 100.);

    static ref SMALL_CHILD_SIZE: Vector2F = vec2f(10., 10.);
    // Use a size for the child that is sufficiently large to test various bounding behavior.
    static ref CHILD_SIZE: Vector2F = vec2f(75., 75.);
    static ref DEFAULT_SIZE_CONSTRAINT: SizeConstraint = SizeConstraint {
        min: vec2f(0., 0.),
        max: vec2f(50., 50.)
    };
    static ref POSITIONED_ELEMENT_RECT: RectF = RectF::new(vec2f(50., 50.), vec2f(15., 15.));
    static ref SMALL_PARENT_RECT: RectF = RectF::new(vec2f(50., 50.), vec2f(25., 25.));
    static ref PARENT_RECT: RectF = RectF::new(vec2f(25., 25.), vec2f(50., 50.));
    static ref PARENT_ANCHORS: Vec<ParentAnchor> = vec![
        ParentAnchor::TopLeft,
        ParentAnchor::TopMiddle,
        ParentAnchor::TopRight,
        ParentAnchor::MiddleLeft,
        ParentAnchor::MiddleRight,
        ParentAnchor::Center,
        ParentAnchor::BottomLeft,
        ParentAnchor::BottomMiddle,
        ParentAnchor::BottomRight,
    ];
    static ref POSITIONED_ELEMENT_ANCHORS: Vec<PositionedElementAnchor> = vec![
        PositionedElementAnchor::TopLeft,
        PositionedElementAnchor::TopMiddle,
        PositionedElementAnchor::TopRight,
        PositionedElementAnchor::MiddleLeft,
        PositionedElementAnchor::MiddleRight,
        PositionedElementAnchor::Center,
        PositionedElementAnchor::BottomLeft,
        PositionedElementAnchor::BottomMiddle,
        PositionedElementAnchor::BottomRight,
    ];
    static ref CHILD_ANCHORS: Vec<ChildAnchor> = vec![
        ChildAnchor::TopLeft,
        ChildAnchor::TopMiddle,
        ChildAnchor::TopRight,
        ChildAnchor::MiddleLeft,
        ChildAnchor::MiddleRight,
        ChildAnchor::Center,
        ChildAnchor::BottomLeft,
        ChildAnchor::BottomMiddle,
        ChildAnchor::BottomRight,
    ];
}
const SAVE_POSITION_ID: &str = "SAVE_POSITION_ID";

/// Returns the coordinates of the parent's anchor point used to position the child.
fn parent_anchor_point(anchor: ParentAnchor) -> (f32, f32) {
    (
        match anchor {
            ParentAnchor::TopLeft | ParentAnchor::MiddleLeft | ParentAnchor::BottomLeft => {
                PARENT_RECT.min_x()
            }
            ParentAnchor::TopMiddle | ParentAnchor::Center | ParentAnchor::BottomMiddle => {
                PARENT_RECT.center().x()
            }
            ParentAnchor::TopRight | ParentAnchor::MiddleRight | ParentAnchor::BottomRight => {
                PARENT_RECT.max_x()
            }
        } + OFFSET.x(),
        match anchor {
            ParentAnchor::TopLeft | ParentAnchor::TopMiddle | ParentAnchor::TopRight => {
                PARENT_RECT.min_y()
            }
            ParentAnchor::MiddleLeft | ParentAnchor::MiddleRight | ParentAnchor::Center => {
                PARENT_RECT.center().y()
            }
            ParentAnchor::BottomLeft | ParentAnchor::BottomMiddle | ParentAnchor::BottomRight => {
                PARENT_RECT.max_y()
            }
        } + OFFSET.y(),
    )
}

/// Returns the coordinates of the `SavePositioned` element's anchor point used to position the
/// child.
fn positioned_element_anchor_point(anchor: PositionedElementAnchor) -> (f32, f32) {
    (
        match anchor {
            PositionedElementAnchor::TopLeft
            | PositionedElementAnchor::MiddleLeft
            | PositionedElementAnchor::BottomLeft => POSITIONED_ELEMENT_RECT.min_x(),
            PositionedElementAnchor::TopMiddle
            | PositionedElementAnchor::Center
            | PositionedElementAnchor::BottomMiddle => POSITIONED_ELEMENT_RECT.center().x(),
            PositionedElementAnchor::TopRight
            | PositionedElementAnchor::MiddleRight
            | PositionedElementAnchor::BottomRight => POSITIONED_ELEMENT_RECT.max_x(),
        } + OFFSET.x(),
        match anchor {
            PositionedElementAnchor::TopLeft
            | PositionedElementAnchor::TopMiddle
            | PositionedElementAnchor::TopRight => POSITIONED_ELEMENT_RECT.min_y(),
            PositionedElementAnchor::MiddleLeft
            | PositionedElementAnchor::MiddleRight
            | PositionedElementAnchor::Center => POSITIONED_ELEMENT_RECT.center().y(),
            PositionedElementAnchor::BottomLeft
            | PositionedElementAnchor::BottomMiddle
            | PositionedElementAnchor::BottomRight => POSITIONED_ELEMENT_RECT.max_y(),
        } + OFFSET.y(),
    )
}

/// Returns the absolute (x, y) position of a child element rendered against a given parent
/// using `PositionedElementOffsetBounds::ParentByPosition` bounds.
fn get_absolute_x_y_position_for_child_element(
    child_size: Vector2F,
    parent_rect: RectF,
    positioned_element_anchor: PositionedElementAnchor,
    child_anchor: ChildAnchor,
) -> Vector2F {
    let offset_positioning = OffsetPositioning::offset_from_save_position_element(
        SAVE_POSITION_ID,
        *OFFSET,
        PositionedElementOffsetBounds::ParentByPosition,
        positioned_element_anchor,
        child_anchor,
    );

    let mut position_cache = PositionCache::new();
    position_cache.start();
    position_cache
        .cache_position_indefinitely(SAVE_POSITION_ID.to_owned(), *POSITIONED_ELEMENT_RECT);
    position_cache.end();

    let child_position_x = offset_positioning
        .x_axis
        .compute_child_position(child_size, parent_rect, *WINDOW_SIZE, &position_cache)
        .expect("Failed to compute child position x.");

    let child_position_y = offset_positioning
        .y_axis
        .compute_child_position(child_size, parent_rect, *WINDOW_SIZE, &position_cache)
        .expect("Failed to compute child position y.");

    vec2f(child_position_x, child_position_y)
}

#[test]
fn test_offset_from_parent_unbounded() {
    for parent_anchor in PARENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_parent(
                *OFFSET,
                ParentOffsetBounds::Unbounded,
                *parent_anchor,
                *child_anchor,
            );

            let position_cache = PositionCache::new();

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            // The size constraint should be unchanged since there is no bounding behavior.
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

            let (anchor_x, anchor_y) = parent_anchor_point(*parent_anchor);

            // Compute the expected x-axis position of the child relative to the parent's
            // anchor point.
            let expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            let child_position_x = offset_positioning.x_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_x.is_ok());
            assert_eq!(child_position_x.unwrap(), expected_child_x);

            // Compute the expected y-axis position of the child relative to the parent's
            // anchor point.
            let expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            let child_position_y = offset_positioning.y_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_y.is_ok());
            assert_eq!(child_position_y.unwrap(), expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_parent_bound_to_parent_with_position() {
    for parent_anchor in PARENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_parent(
                *OFFSET,
                ParentOffsetBounds::ParentByPosition,
                *parent_anchor,
                *child_anchor,
            );

            let position_cache = PositionCache::new();

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            // The size constraint should be unchanged since the bounding behavior adjusts
            // position.
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

            let (anchor_x, anchor_y) = parent_anchor_point(*parent_anchor);

            // Compute the expected x-axis position of the child relative to the parent's
            // anchor point.
            let mut expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            expected_child_x = expected_child_x
                .min(PARENT_RECT.max_x() - CHILD_SIZE.x())
                .max(PARENT_RECT.min_x());

            let child_position_x = offset_positioning.x_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_x.is_ok());
            assert_eq!(child_position_x.unwrap(), expected_child_x);

            // Compute the expected y-axis position of the child relative to the parent anchor
            // point.
            let mut expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            expected_child_y = expected_child_y
                .min(PARENT_RECT.max_y() - CHILD_SIZE.y())
                .max(PARENT_RECT.min_y());

            let child_position_y = offset_positioning.y_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_y.is_ok());
            assert_eq!(child_position_y.unwrap(), expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_positioned_element_bound_to_anchor() {
    for ratio in 0..10 {
        let normalized_ratio = ratio as f32 / 10.;
        let x_axis = PositioningAxis::relative_to_stack_child(
            SAVE_POSITION_ID,
            PositionedElementOffsetBounds::AnchoredElement,
            OffsetType::Percentage(normalized_ratio),
            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
        );
        let y_axis = PositioningAxis::relative_to_stack_child(
            SAVE_POSITION_ID,
            PositionedElementOffsetBounds::Unbounded,
            OffsetType::Pixel(0.),
            AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
        );

        let offset_positioning = OffsetPositioning::from_axes(x_axis, y_axis);

        let mut position_cache = PositionCache::new();
        position_cache.start();
        position_cache
            .cache_position_indefinitely(SAVE_POSITION_ID.to_owned(), *POSITIONED_ELEMENT_RECT);
        position_cache.end();

        // Use a smaller child size to make sure it's not clipped by the anchored element.
        let expected_child_x = POSITIONED_ELEMENT_RECT.origin_x()
            + (POSITIONED_ELEMENT_RECT.width() - SMALL_CHILD_SIZE.x()) * normalized_ratio;
        let expected_child_y = POSITIONED_ELEMENT_RECT.center().y() - SMALL_CHILD_SIZE.y() / 2.;

        let size_constraint = offset_positioning.size_constraint(
            PARENT_RECT.size(),
            *WINDOW_SIZE,
            *DEFAULT_SIZE_CONSTRAINT,
            &position_cache,
        );
        // The size constraint should be unchanged since there is no bounding behavior.
        assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
        assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

        let child_position_x = offset_positioning.x_axis.compute_child_position(
            *SMALL_CHILD_SIZE,
            *PARENT_RECT,
            *WINDOW_SIZE,
            &position_cache,
        );
        assert!(child_position_x.is_ok());
        assert_eq!(child_position_x.unwrap(), expected_child_x);

        let child_position_y = offset_positioning.y_axis.compute_child_position(
            *SMALL_CHILD_SIZE,
            *PARENT_RECT,
            *WINDOW_SIZE,
            &position_cache,
        );

        assert!(child_position_y.is_ok());
        assert_eq!(child_position_y.unwrap(), expected_child_y);
    }
}

#[test]
fn test_offset_from_parent_bound_to_window_with_position() {
    for parent_anchor in PARENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_parent(
                *OFFSET,
                ParentOffsetBounds::WindowByPosition,
                *parent_anchor,
                *child_anchor,
            );

            let position_cache = PositionCache::new();
            let (anchor_x, anchor_y) = parent_anchor_point(*parent_anchor);

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            // The size constraint should be unchanged since the bounding behavior adjusts
            // position.
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

            // Compute the expected x-axis position of the child relative to the parent's
            // anchor point.
            let mut expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            expected_child_x = expected_child_x
                .min(WINDOW_SIZE.x() - CHILD_SIZE.x())
                .max(0.);

            let child_position_x = offset_positioning.x_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_x.is_ok());
            assert_eq!(child_position_x.unwrap(), expected_child_x);

            // Compute the expected y-axis position of the child relative to the parent anchor
            // point.
            let mut expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            expected_child_y = expected_child_y
                .min(WINDOW_SIZE.y() - CHILD_SIZE.y())
                .max(0.);

            let child_position_y = offset_positioning.y_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );

            assert!(child_position_y.is_ok());
            assert_eq!(child_position_y.unwrap(), expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_parent_bound_to_parent_with_size() {
    for parent_anchor in PARENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_parent(
                *OFFSET,
                ParentOffsetBounds::ParentBySize,
                *parent_anchor,
                *child_anchor,
            );

            let position_cache = PositionCache::new();
            let (anchor_x, anchor_y) = parent_anchor_point(*parent_anchor);

            // Compute the expected size constraint based on the parent's size and expected
            // position of child element within the parent's bounding rect.
            let mut expected_size_constraint_max_width = match child_anchor {
                ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => {
                    PARENT_RECT.max_x() - anchor_x
                }
                ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                    (anchor_x - PARENT_RECT.min_x()).min(PARENT_RECT.max_x() - anchor_x) * 2.
                }
                ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                    anchor_x - PARENT_RECT.min_x()
                }
            };
            expected_size_constraint_max_width =
                expected_size_constraint_max_width.clamp(0., PARENT_RECT.width());

            let mut expected_size_constraint_max_height = match child_anchor {
                ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => {
                    PARENT_RECT.max_y() - anchor_y
                }
                ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                    (anchor_y - PARENT_RECT.min_y()).min(PARENT_RECT.max_y() - anchor_y) * 2.
                }
                ChildAnchor::BottomLeft | ChildAnchor::BottomMiddle | ChildAnchor::BottomRight => {
                    anchor_y - PARENT_RECT.min_y()
                }
            };
            expected_size_constraint_max_height =
                expected_size_constraint_max_height.clamp(0., PARENT_RECT.height());

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(
                size_constraint.max,
                vec2f(
                    expected_size_constraint_max_width,
                    expected_size_constraint_max_height
                )
            );

            // Compute the expected x-axis position of the child relative to the parent's
            // anchor point.
            let mut expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            expected_child_x = expected_child_x.clamp(PARENT_RECT.min_x(), PARENT_RECT.max_x());

            let child_position_x = offset_positioning.x_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_x.is_ok());
            assert_eq!(child_position_x.unwrap(), expected_child_x);

            // Compute the expected y-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            expected_child_y = expected_child_y.clamp(PARENT_RECT.min_y(), PARENT_RECT.max_y());

            let child_position_y = offset_positioning.y_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );

            assert!(child_position_y.is_ok());
            assert_eq!(child_position_y.unwrap(), expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_positioned_element_unbounded() {
    for positioned_element_anchor in POSITIONED_ELEMENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_save_position_element(
                SAVE_POSITION_ID,
                *OFFSET,
                PositionedElementOffsetBounds::Unbounded,
                *positioned_element_anchor,
                *child_anchor,
            );

            let mut position_cache = PositionCache::new();
            position_cache.start();
            position_cache
                .cache_position_indefinitely(SAVE_POSITION_ID.to_owned(), *POSITIONED_ELEMENT_RECT);
            position_cache.end();
            let (anchor_x, anchor_y) = positioned_element_anchor_point(*positioned_element_anchor);

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            // The size constraint should be unchanged since there is no bounding behavior.
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

            // Compute the expected x-axis position of the child relative to the positioned
            // element's anchor point.
            let expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            let child_position_x = offset_positioning.x_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );
            assert!(child_position_x.is_ok());
            assert_eq!(child_position_x.unwrap(), expected_child_x);

            // Compute the expected y-axis position of the child relative to the positioned
            // element's anchor point.
            let expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            let child_position_y = offset_positioning.y_axis.compute_child_position(
                *CHILD_SIZE,
                *PARENT_RECT,
                *WINDOW_SIZE,
                &position_cache,
            );

            assert!(child_position_y.is_ok());
            assert_eq!(child_position_y.unwrap(), expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_positioned_element_bound_to_parent_with_position() {
    for positioned_element_anchor in POSITIONED_ELEMENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_save_position_element(
                SAVE_POSITION_ID,
                *OFFSET,
                PositionedElementOffsetBounds::ParentByPosition,
                *positioned_element_anchor,
                *child_anchor,
            );

            let mut position_cache = PositionCache::new();
            position_cache.start();
            position_cache
                .cache_position_indefinitely(SAVE_POSITION_ID.to_owned(), *POSITIONED_ELEMENT_RECT);
            position_cache.end();
            let (anchor_x, anchor_y) = positioned_element_anchor_point(*positioned_element_anchor);

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            // The size constraint should be unchanged since the bounding behavior adjusts
            // position.
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

            // Compute the expected x-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            expected_child_x = expected_child_x
                .min(PARENT_RECT.max_x() - CHILD_SIZE.x())
                .max(PARENT_RECT.min_x());

            let child_position_x = offset_positioning
                .x_axis
                .compute_child_position(*CHILD_SIZE, *PARENT_RECT, *WINDOW_SIZE, &position_cache)
                .expect("Failed to compute child position x.");
            assert_eq!(child_position_x, expected_child_x);

            // Compute the expected y-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            expected_child_y = expected_child_y
                .min(PARENT_RECT.max_y() - CHILD_SIZE.y())
                .max(PARENT_RECT.min_y());

            let child_position_y = offset_positioning
                .y_axis
                .compute_child_position(*CHILD_SIZE, *PARENT_RECT, *WINDOW_SIZE, &position_cache)
                .expect("Failed to compute child position y.");
            assert_eq!(child_position_y, expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_positioned_element_bound_to_parent_with_position_with_window_overflow() {
    for positioned_element_anchor in POSITIONED_ELEMENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            assert_eq!(
                // positioning a 60x60 rect relatively between (50,50) and (75,75) will result in
                // the element getting pushed up+left until it aligns with its parent's max bounds.
                get_absolute_x_y_position_for_child_element(
                    vec2f(60., 60.),
                    *SMALL_PARENT_RECT,
                    *positioned_element_anchor,
                    *child_anchor
                ),
                vec2f(15.0, 15.0)
            );
        }
    }
}

#[test]
fn test_offset_from_positioned_element_bound_to_parent_with_position_with_double_window_overflow() {
    for positioned_element_anchor in POSITIONED_ELEMENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            assert_eq!(
                // positioning an 80x80 rect relatively between (25,25) and (75,75) will result in
                // the element not being able to align itself either with the lower or upper bounds
                // of the parent - in both cases it would be pushed offscreen.
                // so instead, it centers itself.
                get_absolute_x_y_position_for_child_element(
                    vec2f(80., 80.),
                    *PARENT_RECT,
                    *positioned_element_anchor,
                    *child_anchor
                ),
                vec2f(10.0, 10.0)
            );
        }
    }
}

#[test]
fn test_offset_from_positioned_element_bound_to_window_with_position() {
    for positioned_element_anchor in POSITIONED_ELEMENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_save_position_element(
                SAVE_POSITION_ID,
                *OFFSET,
                PositionedElementOffsetBounds::WindowByPosition,
                *positioned_element_anchor,
                *child_anchor,
            );

            let mut position_cache = PositionCache::new();
            position_cache.start();
            position_cache
                .cache_position_indefinitely(SAVE_POSITION_ID.to_owned(), *POSITIONED_ELEMENT_RECT);
            position_cache.end();
            let (anchor_x, anchor_y) = positioned_element_anchor_point(*positioned_element_anchor);

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );
            // The size constraint should be unchanged since the bounding behavior adjusts
            // position.
            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(size_constraint.max, DEFAULT_SIZE_CONSTRAINT.max);

            // Compute the expected x-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            expected_child_x = expected_child_x
                .min(WINDOW_SIZE.x() - CHILD_SIZE.x())
                .max(0.);

            let child_position_x = offset_positioning
                .x_axis
                .compute_child_position(
                    *CHILD_SIZE,
                    *POSITIONED_ELEMENT_RECT,
                    *WINDOW_SIZE,
                    &position_cache,
                )
                .expect("Failed to compute child position x.");
            assert_eq!(child_position_x, expected_child_x);

            // Compute the expected y-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            expected_child_y = expected_child_y
                .min(WINDOW_SIZE.y() - CHILD_SIZE.y())
                .max(0.);

            let child_position_y = offset_positioning
                .y_axis
                .compute_child_position(
                    *CHILD_SIZE,
                    *POSITIONED_ELEMENT_RECT,
                    *WINDOW_SIZE,
                    &position_cache,
                )
                .expect("Failed to compute child position y.");
            assert_eq!(child_position_y, expected_child_y);
        }
    }
}

#[test]
fn test_offset_from_positioned_element_bound_to_window_with_size() {
    for positioned_element_anchor in POSITIONED_ELEMENT_ANCHORS.iter() {
        for child_anchor in CHILD_ANCHORS.iter() {
            let offset_positioning = OffsetPositioning::offset_from_save_position_element(
                SAVE_POSITION_ID,
                *OFFSET,
                PositionedElementOffsetBounds::WindowBySize,
                *positioned_element_anchor,
                *child_anchor,
            );

            let mut position_cache = PositionCache::new();
            position_cache.start();
            position_cache
                .cache_position_indefinitely(SAVE_POSITION_ID.to_owned(), *POSITIONED_ELEMENT_RECT);
            position_cache.end();
            let (anchor_x, anchor_y) = positioned_element_anchor_point(*positioned_element_anchor);

            // Compute the expected size constraint based on the window bounds and expected
            // position of the child element.
            let expected_size_constraint_max_width = match child_anchor {
                ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => {
                    WINDOW_SIZE.x() - anchor_x
                }
                ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                    anchor_x.min(WINDOW_SIZE.x() - anchor_x) * 2.
                }
                ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                    anchor_x
                }
            };
            let expected_size_constraint_max_height = match child_anchor {
                ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => {
                    WINDOW_SIZE.y() - anchor_y
                }
                ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                    anchor_y.min(WINDOW_SIZE.y() - anchor_y) * 2.
                }
                ChildAnchor::BottomLeft | ChildAnchor::BottomMiddle | ChildAnchor::BottomRight => {
                    anchor_y
                }
            };

            let size_constraint = offset_positioning.size_constraint(
                PARENT_RECT.size(),
                *WINDOW_SIZE,
                *DEFAULT_SIZE_CONSTRAINT,
                &position_cache,
            );

            assert_eq!(size_constraint.min, DEFAULT_SIZE_CONSTRAINT.min);
            assert_eq!(
                size_constraint.max,
                vec2f(
                    expected_size_constraint_max_width,
                    expected_size_constraint_max_height
                )
            );

            // Compute the expected x-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_x = anchor_x
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::MiddleLeft | ChildAnchor::BottomLeft => 0.,
                    ChildAnchor::TopMiddle | ChildAnchor::Center | ChildAnchor::BottomMiddle => {
                        CHILD_SIZE.x() / 2.
                    }
                    ChildAnchor::TopRight | ChildAnchor::MiddleRight | ChildAnchor::BottomRight => {
                        CHILD_SIZE.x()
                    }
                };
            expected_child_x = expected_child_x.clamp(0., WINDOW_SIZE.x());

            let child_position_x = offset_positioning
                .x_axis
                .compute_child_position(
                    *CHILD_SIZE,
                    *POSITIONED_ELEMENT_RECT,
                    *WINDOW_SIZE,
                    &position_cache,
                )
                .expect("Failed to compute child position x.");
            assert_eq!(child_position_x, expected_child_x);

            // Compute the expected y-axis position of the child relative to the positioned
            // element's anchor point.
            let mut expected_child_y = anchor_y
                - match child_anchor {
                    ChildAnchor::TopLeft | ChildAnchor::TopMiddle | ChildAnchor::TopRight => 0.,
                    ChildAnchor::MiddleLeft | ChildAnchor::MiddleRight | ChildAnchor::Center => {
                        CHILD_SIZE.y() / 2.
                    }
                    ChildAnchor::BottomLeft
                    | ChildAnchor::BottomMiddle
                    | ChildAnchor::BottomRight => CHILD_SIZE.y(),
                };
            expected_child_y = expected_child_y.clamp(0., WINDOW_SIZE.y());

            let child_position_y = offset_positioning
                .y_axis
                .compute_child_position(
                    *CHILD_SIZE,
                    *POSITIONED_ELEMENT_RECT,
                    *WINDOW_SIZE,
                    &position_cache,
                )
                .expect("Failed to compute child position y.");
            assert_eq!(child_position_y, expected_child_y);
        }
    }
}
