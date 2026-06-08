//! Interoperability traits for interaction between the generic rendering/content layers and parent
//! editor layers.

use std::any::Any;
use std::cell::Ref;
use std::ops::Range;

use num_traits::SaturatingSub;
use pathfinder_color::ColorU;
use rangemap::{RangeMap, RangeSet};
use string_offset::CharOffset;
use warpui_core::elements::Border;
use warpui_core::text_layout::PaintStyleOverride;
use warpui_core::{Action, AppContext, Element, TypedActionView, View};

use crate::content::version::BufferVersion;
use crate::render::element::RichTextAction;

/// Interface between a `RichTextElement` and its containing editor view.
pub trait EditorView
where
    // See https://github.com/rust-lang/rust/issues/20671 and the linked blogs/issues.
    // Logically, EditorView requires TypedActionView as a supertrait, but structuring the
    // requirements this way means we don't have to duplicate the bounds in `RichTextElement`.
    Self: Sized + View + TypedActionView<Action = Self::RichTextAction>,
{
    type RichTextAction: RichTextAction<Self> + Action;

    /// Get the backing model for a runnable command block. The implementation may return `None`
    /// if it does not want interactive features for the block.
    fn runnable_command_at<'a>(
        &self,
        block_offset: CharOffset,
        ctx: &'a AppContext,
    ) -> Option<&'a dyn RunnableCommandModel>;

    fn embedded_item_at<'a>(
        &self,
        block_offset: CharOffset,
        ctx: &'a AppContext,
    ) -> Option<&'a dyn EmbeddedItemModel>;

    /// Returns text decorations (e.g., syntax highlighting, underlines) for the given viewport ranges.
    ///
    /// The lifetime `'a` must be tied to both `&'a self` and `&'a AppContext` because:
    /// - `TextDecoration` contains `Ref<'a, RangeMap>` which borrows from cached data
    /// - The cache may be stored in models accessed through `AppContext` (e.g., syntax tree)
    /// - Both `self` and `ctx` must live at least as long as the returned `TextDecoration`
    fn text_decorations<'a>(
        &'a self,
        _viewport_ranges: RangeSet<CharOffset>,
        _content_version: Option<BufferVersion>,
        _ctx: &'a AppContext,
    ) -> TextDecoration<'a> {
        TextDecoration::default()
    }
}

#[derive(Default)]
pub struct TextDecoration<'a> {
    /// Base color map borrowed from cache (e.g., syntax highlighting)
    pub base_color_map: Option<Ref<'a, RangeMap<CharOffset, ColorU>>>,
    /// Override color map for colors that take precedence over base (e.g., search highlights)
    pub override_color_map: Option<RangeMap<CharOffset, ColorU>>,
    pub underline_range: Option<RangeMap<CharOffset, ColorU>>,
}

impl<'a> TextDecoration<'a> {
    /// Get the color decoration for the given charoffset range. The returned range
    /// should be offset-ed from the start of the input range.
    pub fn to_paint_style_override(&self, range: Range<CharOffset>) -> PaintStyleOverride {
        // Merge base and override color maps, with override taking priority
        let mut color = RangeMap::new();

        // First apply base colors
        if let Some(base_map) = &self.base_color_map {
            color.extend(Self::extract_overlapping_ranges(&range, base_map));
        }

        // Then apply override colors (will overwrite base colors in overlapping ranges)
        if let Some(override_map) = &self.override_color_map {
            color.extend(Self::extract_overlapping_ranges(&range, override_map));
        }

        let underline = Self::find_overlapping_ranges(&range, &self.underline_range);
        PaintStyleOverride::default()
            .with_color(color)
            .with_underline(underline)
    }

    fn extract_overlapping_ranges(
        incoming_range: &Range<CharOffset>,
        style_map: &RangeMap<CharOffset, ColorU>,
    ) -> RangeMap<usize, ColorU> {
        style_map
            .overlapping(incoming_range)
            .map(|(range, color)| {
                let start = range.start.saturating_sub(&incoming_range.start).as_usize();
                let end = range
                    .end
                    .min(incoming_range.end)
                    .saturating_sub(&incoming_range.start)
                    .as_usize();
                (start..end, *color)
            })
            .collect()
    }

    fn find_overlapping_ranges(
        incoming_range: &Range<CharOffset>,
        style_map: &Option<RangeMap<CharOffset, ColorU>>,
    ) -> RangeMap<usize, ColorU> {
        match style_map {
            Some(style_map) => Self::extract_overlapping_ranges(incoming_range, style_map),
            None => RangeMap::default(),
        }
    }
}

pub trait EmbeddedItemModel {
    fn render_item_footer(&self, ctx: &AppContext) -> Option<Box<dyn Element>>;

    fn border(&self, app: &AppContext) -> Option<Border>;

    fn render_remove_embedding_button(&self, ctx: &AppContext) -> Option<Box<dyn Element>>;
}

/// API for the logical editor model backing a runnable command block.
pub trait RunnableCommandModel {
    /// Builds the footer component for the command or code block
    /// This can include a button to insert a command into the terminal input.
    fn render_block_footer(&self, editor_is_focused: bool, ctx: &AppContext) -> Box<dyn Element>;

    /// Border for this command block. Models can use this for state-dependent accents like
    /// success/failure/selection colors.
    ///
    /// If `None`, the default code border in the model's `RichTextStyles` is used instead.
    fn border(&self, app: &AppContext) -> Option<Border>;

    fn as_any(&self) -> &dyn Any;
}

/// A navigation key, which could be propagated from an editor to its parent view.
#[derive(Debug, Clone, Copy)]
pub enum NavigationKey {
    Tab,
    ShiftTab,
    Up,
    Down,
    PageUp,
    PageDown,
    Left,
    Right,
}
