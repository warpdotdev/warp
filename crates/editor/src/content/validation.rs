use pathfinder_color::ColorU;
use sum_tree::SumTree;

use super::text::{BufferBlockStyle, BufferSummary, BufferText, MarkerDir};
use crate::content::cursor::BufferSumTree;
use crate::content::text::{BlockLineBreakBehavior, BlockType, ColorMarker};

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;

/// Validates a [`SumTree`] of content, panicking if it is not valid.
pub fn validate_content(content: &SumTree<BufferText>) {
    let mut cursor = content.cursor::<(), BufferSummary>();
    cursor.descend_to_first_item(content, |_| true);

    let mut active_block_style: Option<BlockType> = None;
    let mut active_color: Option<ColorU> = None;

    while let Some(item) = cursor.item() {
        let start_summary = cursor.start();
        let char_offset = start_summary.text.chars;
        match item {
            BufferText::Text { .. } => {
                assert!(
                    !matches!(active_block_style, Some(BlockType::Item(_))),
                    "{char_offset}: Found character, but active block item does not decorate text\nBuffer: {}",
                    content.debug()
                );
            }
            BufferText::Marker { marker_type, dir } => {
                assert!(
                    !matches!(active_block_style, Some(BlockType::Item(_))),
                    "{char_offset}: Found style marker, but active block item does not decorate text\nBuffer: {}",
                    content.debug()
                );

                let style_depth = start_summary.style_summary().style_counter(marker_type);
                match dir {
                    MarkerDir::Start => {
                        assert!(
                            style_depth == 0,
                            "{char_offset}: Found {marker_type:?} start marker, but style was already active\nBuffer: {}",
                            content.debug()
                        );
                    }
                    MarkerDir::End => {
                        assert!(
                            style_depth == 1,
                            "{char_offset}: Found {marker_type:?} end marker, but style was not active\nBuffer: {}",
                            content.debug()
                        );
                    }
                }
            }
            BufferText::Newline => match active_block_style.clone() {
                Some(BlockType::Text(block_style)) => {
                    assert!(
                        block_style.line_break_behavior() == BlockLineBreakBehavior::NewLine,
                        "{char_offset}: Found newline, but active block style only supports single line\nBuffer: {}",
                        content.debug()
                    );
                }
                Some(BlockType::Item(_)) => {
                    panic!(
                        "{char_offset}: Found newline, but active block item does not decorate text\nBuffer: {}",
                        content.debug()
                    );
                }
                None => (),
            },
            BufferText::BlockItem { item_type } => {
                active_block_style = Some(BlockType::Item(item_type.clone()));
            }
            BufferText::BlockMarker { marker_type } => {
                if Some(BlockType::Text(BufferBlockStyle::PlainText)) == active_block_style {
                    assert!(
                        marker_type != &BufferBlockStyle::PlainText,
                        "{char_offset}: Found plain text marker when the active style is plain text\nBuffer: {}",
                        content.debug()
                    );
                }

                active_block_style = Some(BlockType::Text(marker_type.clone()));
            }
            BufferText::Color(color_marker) => {
                assert!(
                    matches!(
                        active_block_style,
                        Some(BlockType::Text(BufferBlockStyle::CodeBlock { .. }))
                    ),
                    "{char_offset}: Found syntax color marker, but active block item is not a code block\nBuffer: {}",
                    content.debug()
                );

                match color_marker {
                    ColorMarker::Start(color) => {
                        assert!(
                            active_color.is_none(),
                            "{char_offset}: Found a starting syntax color marker when there is already an active syntax color\nBuffer: {}",
                            content.debug()
                        );
                        active_color = Some(*color);
                    }
                    ColorMarker::End => {
                        assert!(
                            active_color.is_some(),
                            "{char_offset}: Found an ending syntax color marker when there is no active syntax color\nBuffer: {}",
                            content.debug()
                        );
                        active_color = None;
                    }
                }
            }
            BufferText::Placeholder { .. } | BufferText::Link(_) => {
                assert!(
                    !matches!(active_block_style, Some(BlockType::Item(_))),
                    "{char_offset}: Found link/placeholder, but active block item does not decorate text\nBuffer: {}",
                    content.debug()
                );
            }
        }

        assert!(
            active_block_style.is_some(),
            "{char_offset}: Buffer doesn't have an active block style.\nBuffer: {}",
            content.debug()
        );
        cursor.next();
    }

    // Buffers must end as plain text.
    assert!(
        active_block_style
            .clone()
            .is_some_and(|style| style == BlockType::Text(BufferBlockStyle::PlainText)),
        "Buffer ends as {active_block_style:?}, not plain text.\nBuffer: {}",
        content.debug()
    );
}
