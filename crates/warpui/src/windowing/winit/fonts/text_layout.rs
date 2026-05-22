use cosmic_text::LayoutGlyph;
use pathfinder_geometry::vector::vec2f;

use super::str_index_map::StrIndexMap;
use crate::fonts::FontId;
use crate::text_layout::{Glyph, Run, TextStyle};

/// Helper struct to construct [`Run`]s from a series of shaped glyphs.
pub(super) struct RunBuilder<'a> {
    runs: Vec<Run>,
    font_in_current_run: FontId,
    current_run_style: TextStyle,
    current_run_width: f32,
    glyphs_in_current_run: Vec<Glyph>,
    styles_map: &'a TextStylesMap,
    str_index_map: &'a StrIndexMap,
}

impl<'a> RunBuilder<'a> {
    pub(super) fn new(
        styles_map: &'a TextStylesMap,
        initial_font_id: FontId,
        str_index_map: &'a StrIndexMap,
    ) -> Self {
        Self {
            runs: vec![],
            font_in_current_run: initial_font_id,
            current_run_style: TextStyle::default(),
            current_run_width: 0.0,
            glyphs_in_current_run: vec![],
            styles_map,
            str_index_map,
        }
    }

    /// Reserves space for the provided number of glyphs in the current run.
    pub fn reserve_capacity(&mut self, total: usize) {
        self.glyphs_in_current_run
            .reserve_exact(total.saturating_sub(self.glyphs_in_current_run.capacity()))
    }

    /// Flushes the current style run by appending the current run into the runs list.
    /// NOTE: if there are no glyphs in the run, it is not appended.
    fn flush_current_style_run(&mut self) {
        if !self.glyphs_in_current_run.is_empty() {
            let excess_capacity =
                self.glyphs_in_current_run.capacity() - self.glyphs_in_current_run.len();
            let mut new_glyphs = Vec::with_capacity(excess_capacity);
            std::mem::swap(&mut new_glyphs, &mut self.glyphs_in_current_run);
            self.runs.push(Run {
                font_id: self.font_in_current_run,
                glyphs: new_glyphs,
                styles: self.current_run_style,
                width: self.current_run_width,
            });
        }
    }

    /// Pushes a new laid out glyph into the `RunBuilder`. Internally, `font_id_fn` will be called
    /// to get the `FontId` for the `glyph`.
    pub(super) fn push_glyph<F: FnOnce(&fontdb::ID) -> FontId>(
        &mut self,
        glyph: LayoutGlyph,
        font_id_fn: F,
    ) {
        let font_id = font_id_fn(&glyph.font_id);
        let text_style = self.styles_map.get(glyph.metadata);

        // A run is a series of continuous glyphs that have the same style. We use the combination
        // of font id (which is a proxy of the font properties such as bold or italic) and the
        // `TextStyle` to determine when a new run should be created.
        if font_id != self.font_in_current_run || text_style != self.current_run_style {
            self.flush_current_style_run();

            self.current_run_width = 0.;
            self.current_run_style = text_style;
            self.font_in_current_run = font_id;
        }

        let glyph_char_index = self
            .str_index_map
            .char_index(glyph.start)
            .unwrap_or_else(|| self.str_index_map.num_chars());

        self.glyphs_in_current_run.push(Glyph {
            id: glyph.glyph_id as u32,
            position_along_baseline: vec2f(glyph.x, glyph.y),
            index: glyph_char_index,
            width: glyph.w,
        });

        self.current_run_width += glyph.w;
    }

    /// Returns the final list of [`Run`]s that were computed.
    pub(super) fn build(mut self) -> Vec<Run> {
        self.flush_current_style_run();
        self.runs
    }
}

/// Simple map that maps an index to a [`TextStyle`].
/// [`cosmic_text`] only supports setting a `usize` as metadata, so this struct is used to generate
/// a mapping of an index to its corresponding `TextStyle`.
///
/// Though this is modeled internally as a `Vec`, use a new type to limit the API since some
/// functions on a `Vec` (such as reordering) would break the mapping of index to text style.
pub(super) struct TextStylesMap {
    styles: Vec<TextStyle>,
}

impl TextStylesMap {
    pub(super) fn insert(&mut self, text_style: TextStyle) -> usize {
        let size = self.styles.len();
        self.styles.push(text_style);
        size
    }

    /// Gets the [`TextStyle`] at the given index. If no style is at the index, a default
    /// `TextStyle` is returned.
    pub(super) fn get(&self, index: usize) -> TextStyle {
        self.styles.get(index).copied().unwrap_or_default()
    }

    pub(super) fn new() -> Self {
        Self {
            styles: Default::default(),
        }
    }
}
