use super::{TuiSizeConstraintCondition, TuiSizeConstraintSwitch};
use crate::elements::tui::test_support::render_to_lines;
use crate::elements::tui::{TuiElement, TuiSize, TuiText};

#[test]
fn renders_the_default_child_when_no_condition_matches() {
    let switch = TuiSizeConstraintSwitch::new(
        TuiText::new("default").truncate().finish(),
        vec![(
            TuiSizeConstraintCondition::WidthLessThan(8),
            TuiText::new("narrow").truncate().finish(),
        )],
    );

    assert_eq!(
        render_to_lines(switch, TuiSize::new(12, 1)),
        vec!["default     "]
    );
}

#[test]
fn renders_the_first_matching_child() {
    let switch = TuiSizeConstraintSwitch::new(
        TuiText::new("default").truncate().finish(),
        vec![
            (
                TuiSizeConstraintCondition::WidthLessThan(8),
                TuiText::new("narrow").truncate().finish(),
            ),
            (
                TuiSizeConstraintCondition::HeightLessThan(2),
                TuiText::new("short").truncate().finish(),
            ),
        ],
    );

    assert_eq!(render_to_lines(switch, TuiSize::new(6, 1)), vec!["narrow"]);
}

#[test]
fn supports_combined_size_conditions() {
    let switch = TuiSizeConstraintSwitch::new(
        TuiText::new("default").truncate().finish(),
        vec![(
            TuiSizeConstraintCondition::SizeSmallerThan(TuiSize::new(10, 3)),
            TuiText::new("small").truncate().finish(),
        )],
    );

    assert_eq!(
        render_to_lines(switch, TuiSize::new(9, 2)),
        vec!["small    ", "         "]
    );
}
