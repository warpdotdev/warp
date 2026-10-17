use std::sync::Arc;

use vec1::vec1;
use warp_editor::render::model::LaidOutEmbeddedItem;
use warpui::text_layout::{Line, TextFrame};
use warpui::units::IntoPixels;

use super::LaidOutEmbeddedWorkflow;

/// A single-line `TextFrame` that either does or doesn't report a missing glyph.
fn text_frame(has_missing_glyph: bool) -> Arc<TextFrame> {
    Arc::new(TextFrame::new(
        vec1![Line {
            chars_with_missing_glyphs: if has_missing_glyph {
                vec!['\u{2603}']
            } else {
                vec![]
            },
            ..Default::default()
        }],
        0.,
        Default::default(),
    ))
}

fn workflow(
    title_missing: bool,
    description_missing: Option<bool>,
    command_missing: &[bool],
) -> LaidOutEmbeddedWorkflow {
    LaidOutEmbeddedWorkflow::new(
        text_frame(title_missing),
        description_missing.map(text_frame),
        command_missing.iter().copied().map(text_frame).collect(),
        100.0.into_pixels(),
        false,
    )
}

#[test]
fn test_has_missing_glyphs_false_when_no_frame_reports_one() {
    assert!(!workflow(false, Some(false), &[false, false]).has_missing_glyphs());
}

#[test]
fn test_has_missing_glyphs_false_with_no_description_frame() {
    assert!(!workflow(false, None, &[false]).has_missing_glyphs());
}

#[test]
fn test_has_missing_glyphs_true_when_title_reports_one() {
    assert!(workflow(true, Some(false), &[false]).has_missing_glyphs());
}

#[test]
fn test_has_missing_glyphs_true_when_description_reports_one() {
    assert!(workflow(false, Some(true), &[false]).has_missing_glyphs());
}

#[test]
fn test_has_missing_glyphs_true_when_a_command_frame_reports_one() {
    assert!(workflow(false, Some(false), &[false, true]).has_missing_glyphs());
}
