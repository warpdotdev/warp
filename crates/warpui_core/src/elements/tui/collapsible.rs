//! [`tui_collapsible`]: a disclosure section — a clickable styled header with
//! a chevron over a lazily-built body that shows only when expanded.
//!
//! This is a plain composition of existing primitives: a [`TuiFlex`] column
//! whose first child is the header (a [`TuiCollapsibleHeader`] — wrapping
//! label spans with a reserved, non-wrapping disclosure chevron after the
//! first rendered row, wrapped in a [`TuiHoverable`] for click and hover
//! tracking) and whose second child — built and present only when expanded —
//! is the body. State is owned by the caller: `collapsed` and the hover state
//! on `mouse_state` are read at composition time and `on_toggle` fires on a
//! header click, leaving the caller to flip its own state and re-render.
//!
//! The chevron is reserved as its own non-wrapping element rather than
//! appended to a single truncated label, so at narrow widths the label text
//! wraps onto later rows while the disclosure chevron stays visible on the
//! header's first row — appending it to a `.truncate()`d label clips the
//! chevron away once the label no longer fits.

use super::{
    TuiConstraint, TuiElement, TuiEventContext, TuiFlex, TuiHoverable, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle,
    TuiText,
};
use crate::AppContext;
use crate::elements::MouseStateHandle;

/// Disclosure glyph shown when the section is collapsed.
const CHEVRON_COLLAPSED: &str = "▸";
/// Disclosure glyph shown when the section is expanded.
const CHEVRON_EXPANDED: &str = "▾";

/// Returns the disclosure glyph for a collapsed or expanded section.
fn disclosure_chevron(collapsed: bool) -> &'static str {
    if collapsed {
        CHEVRON_COLLAPSED
    } else {
        CHEVRON_EXPANDED
    }
}

/// A collapsible section header: a wrapping label followed on the header's
/// first row by a reserved, non-wrapping disclosure chevron.
///
/// The chevron is laid out first and its column reserved, then the label wraps
/// into the remaining width. The chevron is pinned to the first row (its own
/// one-row slot after that row's rendered text), so it stays visible at narrow
/// widths where the label text wraps onto later rows — unlike appending the
/// chevron to a single `.truncate()`d label, which clips the chevron away once
/// the label no longer fits. At wide widths the chevron sits right after the
/// label, matching the single-line appearance.
struct TuiCollapsibleHeader {
    /// The wrapping label text (header spans without the chevron).
    label: TuiText,
    /// The reserved, non-wrapping disclosure chevron (e.g. `"▸"`).
    chevron: TuiText,
    /// Horizontal offset of the chevron after the first rendered label row.
    chevron_offset: u16,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiCollapsibleHeader {
    fn new(label: TuiText, chevron: TuiText) -> Self {
        Self {
            label,
            chevron,
            chevron_offset: 0,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for TuiCollapsibleHeader {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let available = constraint.max.width;
        // Lay out the chevron first so its column is reserved; it is a single
        // glyph, so it takes one cell and never wraps onto a later row.
        let chevron_size = self
            .chevron
            .layout(TuiConstraint::loose(constraint.max), ctx, app);
        // Preserve the normal label/chevron spacing whenever there is room for
        // at least one label cell. At widths one and two, omit the gap so the
        // glyph itself remains visible instead of truncating to a spacer or
        // displacing the label entirely.
        let chevron_gap = if available >= chevron_size.width.saturating_add(2) {
            1
        } else {
            0
        };
        // The label wraps into whatever width remains after the chevron's
        // reserved column, so wrapping label text can never push the chevron
        // off the first row.
        let label_max_width = available
            .saturating_sub(chevron_size.width)
            .saturating_sub(chevron_gap);
        let label_constraint = TuiConstraint::new(
            TuiSize::new(0, constraint.min.height),
            TuiSize::new(label_max_width, constraint.max.height),
        );
        let label_size = self.label.layout(label_constraint, ctx, app);

        // Use row one's actual rendered width rather than the label's widest
        // wrapped row, so a longer continuation row cannot right-align the
        // chevron.
        let first_row_width = self.label.first_rendered_line_width(label_max_width);
        let chevron_offset = first_row_width.saturating_add(chevron_gap);
        self.chevron_offset = chevron_offset;
        let chevron_edge = chevron_offset.saturating_add(chevron_size.width);
        let width = label_size.width.max(chevron_edge).min(available);
        let height = label_size.height.max(chevron_size.height);
        let size = TuiSize::new(width, height);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.label.after_layout(ctx, app);
        self.chevron.after_layout(ctx, app);
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        if self.size.is_none() {
            return;
        }
        // The label paints from the header's origin; the chevron is pinned to
        // the first row, immediately after that row's rendered text.
        self.label.render(origin, surface, ctx);
        self.chevron.render(
            origin.offset(i32::from(self.chevron_offset), 0),
            surface,
            ctx,
        );
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

/// Composes a collapsible section: a clickable rich-text header (a wrapping
/// label with a reserved disclosure chevron after its first rendered row) over
/// a body that is built only when `collapsed` is `false`. `on_toggle` runs when
/// the header is clicked. Callers own the header styles, including any
/// hover-dependent styling; hover transitions are recorded on `mouse_state`,
/// which the caller owns so it survives re-renders.
pub fn tui_collapsible(
    collapsed: bool,
    header_spans: impl IntoIterator<Item = (String, TuiStyle)>,
    chevron_style: TuiStyle,
    mouse_state: MouseStateHandle,
    body: impl FnOnce() -> Box<dyn TuiElement>,
    on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
) -> Box<dyn TuiElement> {
    let label = TuiText::from_spans(header_spans);
    let chevron = TuiText::new(disclosure_chevron(collapsed))
        .with_style(chevron_style)
        .truncate();
    let header = TuiCollapsibleHeader::new(label, chevron).finish();
    let header = TuiHoverable::new(mouse_state, header).on_click(on_toggle);

    let mut column = TuiFlex::column().child(header.finish());
    if !collapsed {
        column = column.child(body());
    }
    column.finish()
}

#[cfg(test)]
#[path = "collapsible_tests.rs"]
mod tests;
