use itertools::Itertools;
use sum_tree::SumTree;
use warpui_core::SizeConstraint;
use warpui_core::geometry::vector::vec2f;
use warpui_core::units::{IntoPixels, Pixels};

use super::ViewportState;
use crate::render::model::RenderState;
use crate::render::model::test_utils::{TEST_STYLES, mock_paragraph};

#[test]
fn test_viewport_offsets() {
    let mut render_state =
        RenderState::new_for_test(TEST_STYLES, 20.0.into_pixels(), 180.0.into_pixels());
    let mut content = SumTree::new();
    content.push(mock_paragraph(10., 100., 1));
    content.push(mock_paragraph(60., 100., 1));
    content.push(mock_paragraph(100., 100., 2));
    content.push(mock_paragraph(30., 100., 3));
    content.push(mock_paragraph(80., 100., 1));
    render_state.set_content(content);

    // Double-check the heights, as later tests rely on them.
    let heights = render_state
        .content
        .borrow()
        .items()
        .iter()
        .map(|item| item.height().as_f32())
        .collect_vec();
    assert_eq!(heights, vec![24., 60., 100., 30., 80., 24.]);

    let content = render_state.content();
    let offsets = content
        .viewport_items(
            180.0.into_pixels(),
            render_state.viewport().width(),
            40.0.into_pixels(),
        )
        .map(|(item, _)| (item.viewport_offset.as_f32(), item.block_offset.as_usize()))
        .collect_vec();

    // Each item should be painted starting at the sum of all previous heights,
    // offset by scroll_top
    assert_eq!(
        offsets,
        vec![
            // The first item is fully above the viewport.
            // The second item is slightly above the viewport.
            (-16., 1),
            // The third is fully within the viewport
            (44., 2),
            // The fourth is slightly past the viewport, and cut off.
            (144., 4),
            // The fifth is slightly past the viewport, and cut off.
            (174., 7)
        ]
    );
}

#[test]
fn test_viewport_item_to_block() {
    let mut render_state =
        RenderState::new_for_test(TEST_STYLES, 100.0.into_pixels(), 160.0.into_pixels());
    let mut content = SumTree::new();
    content.push(mock_paragraph(80., 100., 10));
    content.push(mock_paragraph(20., 100., 3));
    render_state.set_content(content);

    let viewport_items = {
        let content = render_state.content();
        content
            .viewport_items(
                160.0.into_pixels(),
                render_state.viewport().width(),
                0.0.into_pixels(),
            )
            .map(|(item, _)| item)
            .collect_vec()
    };

    let content = render_state.content();
    let block0 = content
        .block_at_offset(viewport_items[0].block_offset())
        .expect("Block should exist");
    assert_eq!(block0.start_char_offset, 0.into());

    let block1 = content
        .block_at_offset(viewport_items[1].block_offset())
        .expect("Block should exist");
    assert_eq!(block1.start_char_offset, 10.into());
    drop(content);

    // Now, invalidate the items by pushing new content. They should detect this and fail.
    let mut new_tree = SumTree::new();
    new_tree.push(mock_paragraph(40., 100., 4));
    new_tree.push_tree(render_state.content.borrow().clone());
    render_state.set_content(new_tree);

    let content = render_state.content();
    // There's still _a_ block at char offset 0, which is what's returned here. This is fine, since
    // the check in ViewportItem::block is a fallback in case of programmer error.
    assert!(
        content
            .block_at_offset(viewport_items[0].block_offset())
            .is_some()
    );
    // However, the next item no longer has a backing block.
    assert!(
        content
            .block_at_offset(viewport_items[1].block_offset())
            .is_none()
    );
}

#[test]
fn test_scroll_bounds() {
    let content_height = 32.0.into_pixels();
    let mut state = ViewportState::new(8.0.into_pixels(), 16.0.into_pixels());

    // Scroll down within the document.
    assert!(state.scroll((-6.0).into_pixels(), content_height));
    assert_eq!(state.scroll_top.as_f32(), 6.);

    // Now, scroll up, but past the beginning of the document. The scroll_top
    // should clamp at 0.
    assert!(state.scroll(100.0.into_pixels(), content_height));
    assert_eq!(state.scroll_top.as_f32(), 0.);

    // Now, scroll back down.
    assert!(state.scroll((-12.0).into_pixels(), content_height));
    assert_eq!(state.scroll_top.as_f32(), 12.);

    // We can keep scrolling, but it's clamped to the last viewport of the document.
    assert!(state.scroll((-100.0).into_pixels(), content_height));
    assert_eq!(state.scroll_top.as_f32(), 16.);

    // If we try to scroll more, it has no effect.
    assert!(!state.scroll((-10.0).into_pixels(), content_height));
    assert_eq!(state.scroll_top.as_f32(), 16.);
}

#[test]
fn test_viewport_width_change() {
    let state = ViewportState::new(100.0.into_pixels(), 500.0.into_pixels());

    // Shrink the viewport by 20 pixels horizontally. It should use the maximum space available,
    // and need layout because of the size change.
    let size_info = state.viewport_size(
        SizeConstraint::new(vec2f(20., 100.), vec2f(80., 500.)),
        vec2f(0., 0.),
        None,
    );
    assert!(size_info.needs_layout);
    assert_eq!(size_info.viewport_size, vec2f(80., 500.));
}

#[test]
fn test_viewport_height_change() {
    let content_height = 300.0.into_pixels();
    let content_width = 100.0.into_pixels();
    let mut state = ViewportState::new(100.0.into_pixels(), 200.0.into_pixels());

    // Scroll the viewport.
    state.scroll(-(50.0.into_pixels()), content_height);
    assert_eq!(state.scroll_top, 50.0.into_pixels());

    // Resize the viewport such that it no longer has a scrollbar.
    state.set_size(vec2f(100., 400.), content_width, content_height);
    assert_eq!(state.scroll_top, Pixels::zero());

    // Resize the viewport to need scrolling again. This won't autoscroll, however.
    state.set_size(vec2f(100., 100.), content_width, content_height);
    assert_eq!(state.scroll_top, Pixels::zero());

    // Scroll, and then shrink the viewport further. This should preserve the scroll position.
    state.scroll(-(50.0.into_pixels()), content_height);
    state.set_size(vec2f(100., 80.), content_width, content_height);
    assert_eq!(state.scroll_top, 50.0.into_pixels());

    // The scroll_top cannot be past the last viewport of content. If the viewport extends such
    // that the scroll position is invalid, but a scrollbar is still needed, we'll scroll up slightly.
    state.set_size(vec2f(100., 260.), content_width, content_height);
    assert_eq!(state.scroll_top, 40.0.into_pixels());
}
