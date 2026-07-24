//! [`tui_collapsible`]: a disclosure section — a clickable styled header with
//! a chevron over a lazily-built body that shows only when expanded.
//!
//! This is a plain composition of existing primitives: a [`TuiFlex`] column
//! whose first child is the header (a [`TuiCollapsibleHeader`] — wrapping
//! label spans with a reserved, non-wrapping disclosure chevron pinned to the
//! first row, wrapped in a [`TuiHoverable`] for the click and hover tracking)
//! and whose second child — built and present only when expanded — is the
//! body. State is owned by the caller: `collapsed` and the hover state on
//! `mouse_state` are read at composition time and `on_toggle` fires on a
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
/// one-row slot at the label's laid-out content edge), so it stays visible at
/// narrow widths where the label text wraps onto later rows — unlike appending
/// the chevron to a single `.truncate()`d label, which clips the chevron away
/// once the label no longer fits. At wide widths the chevron sits right after
/// the label, matching the single-line appearance.
struct TuiCollapsibleHeader {
    /// The wrapping label text (header spans without the chevron).
    label: TuiText,
    /// The reserved, non-wrapping disclosure chevron (e.g. `"▸"`).
    chevron: TuiText,
    /// Whether to leave a one-cell gap between the label and chevron.
    ///
    /// At the smallest widths the gap is omitted so the glyph itself remains
    /// visible instead of truncating to a leading spacer.
    chevron_gap: u16,
    /// The label's size retained from the most recent layout, used to place
    /// the chevron during render.
    label_size: Option<TuiSize>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiCollapsibleHeader {
    fn new(label: TuiText, chevron: TuiText) -> Self {
        Self {
            label,
            chevron,
            chevron_gap: 0,
            label_size: None,
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
        self.chevron_gap = chevron_gap;
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
        self.label_size = Some(label_size);

        let width = label_size
            .width
            .saturating_add(chevron_gap)
            .saturating_add(chevron_size.width)
            .min(available);
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
        let Some(label_size) = self.label_size else {
            return;
        };
        // The label paints from the header's origin; the chevron is pinned to
        // the first row, immediately after the label's laid-out content edge.
        self.label.render(origin, surface, ctx);
        let chevron_origin = origin.offset(
            i32::from(label_size.width.saturating_add(self.chevron_gap)),
            0,
        );
        self.chevron.render(chevron_origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

/// Composes a collapsible section: a clickable rich-text header (a wrapping
/// label with a reserved disclosure chevron pinned to the first row) over a
/// body that is built only when `collapsed` is `false`. `on_toggle` runs when
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
