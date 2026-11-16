use warp_core::ui::appearance::Appearance;
use warpui_core::Element;
use warpui_core::elements::{
    Align, CrossAxisAlignment, Empty, Flex, MainAxisSize, ParentElement, Shrinkable,
};

use crate::slides::progress_dots;

pub fn onboarding_bottom_nav(
    appearance: &Appearance,
    step_index: usize,
    step_count: usize,
    back_button: Option<Box<dyn Element>>,
    next_button: Option<Box<dyn Element>>,
) -> Box<dyn Element> {
    let dots = progress_dots::progress_dots(step_count, step_index, appearance);

    let back_button = back_button.unwrap_or_else(|| Empty::new().finish());
    let next_button = next_button.unwrap_or_else(|| Empty::new().finish());

    // Equal-weight side slots push Back to the far left and Next to the far
    // right, leaving the natural-width dots centered between them on one row.
    let left = Shrinkable::new(1., Align::new(back_button).left().finish()).finish();
    let right = Shrinkable::new(1., Align::new(next_button).right().finish()).finish();

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(left)
        .with_child(dots)
        .with_child(right)
        .finish()
}
