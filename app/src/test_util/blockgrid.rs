use itertools::Itertools;
use unicode_width::UnicodeWidthChar;

use crate::terminal::SizeInfo;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::model::ansi::{self, Handler};
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::cell::Flags;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::grid::grid_handler::PerformResetGridChecks;
use crate::terminal::model::index::{VisiblePoint, VisibleRow};

const MAX_SCROLL_LIMIT: usize = 1000;

/// Constructs a blockgrid from its contents as a string.
///
/// A `\n` will break line and `\r\n` will break line without wrapping.
///
/// This function will set `max_cursor` in the grid based on the position of
/// the last character. Some features rely on the max cursor appearing at the
/// end of a grid on a newline (e.g. block filtering), so when writing tests
/// you may need to end the string with `\r\n`, even if there's no content
/// after it.
///
/// # Example
/// The line `mock_blockgrid("hello\n:)\r\nearth!")` will create a blockgrid with the following cells:
/// ```
/// // [h][e][l][l][o][ ] <- WRAPLINE flag set
/// // [:][)][ ][ ][ ][ ]
/// // [e][a][r][t][h][!]
/// ```
///
pub fn mock_blockgrid(content: &str) -> BlockGrid {
    let rows: Vec<&str> = content.split('\n').collect();
    let num_cols = rows
        .iter()
        .map(|row| {
            let sum: usize = row
                .chars()
                .filter(|c| *c != '\r')
                // All characters in our mock blockgrid should have a minimum character width of one
                // because any character that is rendered in the blockgrid will occupy at least one
                // grapheme. This is true for the null-character '\0' as well.
                .map(|c| usize::max(c.width().unwrap_or(1), 1))
                .sum();
            sum
        })
        .collect_vec();
    let max_num_cols = num_cols.iter().cloned().max().unwrap_or(0);

    // Create terminal with the appropriate dimensions.
    let size = SizeInfo::new_without_font_metrics(rows.len(), max_num_cols);

    let mut blockgrid = BlockGrid::new(
        size,
        MAX_SCROLL_LIMIT,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    blockgrid.start();

    // Fill blockgrid with content.
    for (row, text) in rows.iter().enumerate() {
        if !text.ends_with('\r') && row + 1 != rows.len() {
            blockgrid.grid_storage_mut()[row][max_num_cols - 1]
                .flags_mut()
                .insert(Flags::WRAPLINE);
        }

        let mut index = 0;
        for c in text.chars().take_while(|c| *c != '\r') {
            blockgrid.grid_storage_mut()[row][index].c = c;

            // All characters in our mock blockgrid should have a minimum character width of one
            // because any character that is rendered in the blockgrid will occupy at least one
            // grapheme. This is true for the null-character '\0' as well.
            let width = usize::max(c.width().unwrap_or(1), 1);
            if width == 2 {
                blockgrid.grid_storage_mut()[row][index]
                    .flags_mut()
                    .insert(Flags::WIDE_CHAR);
                blockgrid.grid_storage_mut()[row][index + 1]
                    .flags_mut()
                    .insert(Flags::WIDE_CHAR_SPACER);
            }

            index += width;
        }
    }
    if !rows.is_empty() {
        let total_cols = blockgrid.grid_handler().columns();
        blockgrid.grid_handler_mut().update_cursor(|cursor| {
            cursor.point = VisiblePoint {
                row: VisibleRow(rows.len() - 1),
                col: num_cols[rows.len() - 1].saturating_sub(1),
            };
            // If we are at the end of the line, we need to wrap the input on the next
            // usage of the cursor for writing!
            if num_cols[rows.len() - 1].saturating_sub(1) == total_cols - 1 {
                cursor.input_needs_wrap = true;
            }
        });
        blockgrid.grid_storage_mut().update_max_cursor();
    }

    blockgrid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    blockgrid
}
