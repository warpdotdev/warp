use warp_core::ui::appearance::Appearance;
use warpui_core::elements::{
    Align, ClippedScrollStateHandle, ClippedScrollable, Container, CrossAxisAlignment, Flex,
    MainAxisSize, ParentElement, ScrollbarWidth, Shrinkable,
};
use warpui_core::Element;

pub fn onboarding_slide_content(
    children: Vec<Box<dyn Element>>,
    bottom_nav: Box<dyn Element>,
    scroll_state: ClippedScrollStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    const PADDING: f32 = 64.;

    // Build the content column with its natural (minimum) height.
    let mut content_column = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    for child in children {
        content_column = content_column.with_child(child);
    }

    // Apply right padding inside the scrollable so the scrollbar sits at the
    // outer edge of the slide rather than overlapping the content.
    let padded_content = Container::new(content_column.finish())
        .with_padding_right(PADDING)
        .finish();

    // Wrap the content in Align so it is centered within the visible area
    // when there is enough space.
    let centered_content = Align::new(padded_content).finish();

    let theme = appearance.theme();

    // Create a scrollable content area using vertical_centered so the child's
    // min-height matches the visible height (enabling Align to center).
    let scrollable = ClippedScrollable::vertical_centered(
        scroll_state,
        centered_content,
        ScrollbarWidth::Auto,
        theme.disabled_text_color(theme.background()).into(),
        theme.main_text_color(theme.background()).into(),
        theme.background().into(),
    )
    .with_overlayed_scrollbar()
    .finish();

    // Outer layout: scrollable content takes remaining space, bottom nav
    // is always visible at the bottom.
    let outer = Flex::column()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(Shrinkable::new(1., scrollable).finish())
        .with_child(
            Container::new(bottom_nav)
                .with_margin_top(24.)
                .with_padding_right(PADDING)
                .finish(),
        )
        .finish();

    // Left/top/bottom padding on the outer container; right padding is handled
    // inside the scrollable and bottom nav so the scrollbar stays at the edge.
    Container::new(outer)
        .with_padding_top(PADDING)
        .with_padding_bottom(PADDING)
        .with_padding_left(PADDING)
        .finish()
}
