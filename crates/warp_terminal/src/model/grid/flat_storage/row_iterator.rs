use std::rc::Rc;

use crate::model::grid::{
    cell::{Cell, Flags},
    row::Row,
};

use super::{grapheme::Grapheme, style::BgAndStyle, EndOfPromptMarker, FlatStorage};

/// An iterator over [`Row`]s in the grid.
///
/// The item type is [`Rc<Row>`] so that we can minimize the number of heap
/// allocations performed during iteration.  If the `Rc<Row>` returned by a
/// call to `next()` is always dropped before the next call, only one [`Row`]
/// will be allocated for the entire lifetime of the [`RowIterator`].
pub struct RowIterator<'s> {
    /// A reference to the backing grid storage.
    storage: &'s FlatStorage,
    /// The index of the next row to return.
    row_index: usize,
    /// The [`Row`] that we will return to the caller.
    row: Rc<Row>,
    /// A template for what empty cells in the row should look like.
    template: Cell,
}

impl<'s> RowIterator<'s> {
    /// Constructs a new [`RowIterator`] that starts at the given row index.
    pub fn new(storage: &'s FlatStorage, start_row: usize) -> Self {
        Self {
            storage,
            row_index: start_row,
            row: Row::new(storage.columns).into(),
            template: Cell::default(),
        }
    }
}

impl Iterator for RowIterator<'_> {
    type Item = Rc<Row>;

    fn next(&mut self) -> Option<Self::Item> {
        let start_offset = self
            .storage
            .index
            .content_range_for_row(self.row_index)?
            .start;

        let mut fg_color_iter = self.storage.fg_color_map.iter_from(start_offset);
        let mut bg_and_style_iter = self.storage.bg_and_style_map.iter_from(start_offset);

        let row = Rc::make_mut(&mut self.row);
        row.reset(&self.template);

        let mut current_offset = start_offset;
        for grapheme_info in self.storage.index.grapheme_infos_for_row(self.row_index)? {
            let content = {
                let start = current_offset;
                let end = start + grapheme_info.utf8_bytes.get() as usize;
                &self.storage.content()[start..end]
            };
            let grapheme = Grapheme::new_from_str_and_info(content, grapheme_info);

            if grapheme.starts_new_row() {
                break;
            }

            // We must advance content offset-based iterators before any uses
            // of the continue keyword to ensure those iterators are in sync
            // with our content offset position.
            //
            // TODO(vorporeal): Figure out a cleaner way to handle advancing the
            // iterator by grapheme byte length.  My initial implementation advanced
            // the iterator once per grapheme instead of once per byte, which was
            // incorrect (but easy to get wrong).  This works, but I wonder if the
            // iterator returned by `AttributeMap` shouldn't actually implement
            // `Iterator` and should provide its own `next(&Grapheme)` function.
            let fg = next_attribute(&mut fg_color_iter, &grapheme);
            let BgAndStyle {
                bg,
                flags,
                underline_color,
            } = next_attribute(&mut bg_and_style_iter, &grapheme);

            let cell_width = grapheme.cell_width();
            if cell_width == 0 {
                current_offset += grapheme.len();
                continue;
            }

            // The next cell to fill is the first untouched one.  This allows
            // us to cleanly handle wide chars, which modify multiple cells
            // in the row.
            let idx = row.occ;
            let Some(cell) = row.get_mut(idx) else {
                log::warn!(
                    "Tried to mutate cell past the end of a row in RowIterator::next!\n\
                            \tidx: {idx}\n\
                            \tlen: {}\n\
                            \tgrapheme runs: {:?}",
                    row.len(),
                    self.storage.index.grapheme_runs_for_row(self.row_index)?
                );
                panic!("Tried to mutate cell past the end of a row in RowIterator::next!")
            };

            let mut chars = grapheme.chars();
            // SAFETY: Grapheme::new() asserts that the grapheme is non-empty.
            cell.c = chars.next().unwrap();
            // Add any remaining chars in the grapheme to the cell as zero-width
            // characters.  We suppress `Cell::push_zerowidth`'s
            // long-grapheme warning on this path: we're replaying chars
            // from an already-stored grapheme that was capped when it was
            // first seen on the ANSI-input path, so a warning here would
            // be redundant and would fire every time a row is
            // rematerialized (e.g. on scroll or resize).
            chars.for_each(|c| cell.push_zerowidth(c, /* log_long_grapheme_warnings */ false));

            cell.fg = fg;
            cell.bg = bg;
            cell.flags = flags;
            if let Some(underline_color) = underline_color {
                cell.set_underline_color(underline_color);
            }

            match self.storage.end_of_prompt_marker {
                Some(EndOfPromptMarker {
                    offset,
                    has_extra_trailing_newline,
                }) if offset == current_offset => {
                    cell.mark_end_of_prompt(has_extra_trailing_newline);
                }
                _ => {}
            }

            // If the grapheme takes up two cells, mark the following cell as
            // a spacer.
            if cell_width == 2 {
                row[idx].flags.insert(Flags::WIDE_CHAR);
                row[idx + 1].flags.insert(Flags::WIDE_CHAR_SPACER);
            }

            current_offset += grapheme.len();
        }

        let entry = self
            .storage
            .index
            .get_entry(self.row_index)
            .expect("should not fail to get entry for row");
        if !entry.has_trailing_newline {
            row.last_mut().unwrap().flags_mut().insert(Flags::WRAPLINE);
        }
        if entry.ends_with_leading_wide_char_spacer {
            row.last_mut()
                .unwrap()
                .flags_mut()
                .insert(Flags::LEADING_WIDE_CHAR_SPACER);
        }

        self.row_index += 1;
        Some(self.row.clone())
    }
}

/// Returns the next value for the given attribute iterator, given the current
/// grapheme.
///
/// This must be used instead of [`Iterator::next`] in order to handle
/// multi-byte graphemes properly.
fn next_attribute<T>(iter: &mut impl Iterator<Item = T>, grapheme: &Grapheme) -> T {
    iter.nth(grapheme.len().as_usize() - 1)
        .expect("should never fail to provide value")
}
