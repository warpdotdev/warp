use warpui::elements::{Border, ChildView, Container};
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewHandle};

use crate::ai::blocklist::inline_action::code_diff_view::CodeDiffView;
use crate::ai::blocklist::inline_action::inline_action_header::INLINE_ACTION_HORIZONTAL_PADDING;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;

/// A lightweight wrapper view that renders a [`CodeDiffView`] with the same
/// container styling (border, padding, background) that the AI block applies
/// to inline-banner passive code diffs. This allows out-of-band code diff
/// views to be inserted as standalone rich content without an AI block.
pub struct PassiveCodeDiff {
    pub diff_view: ViewHandle<CodeDiffView>,
}

impl Entity for PassiveCodeDiff {
    type Event = ();
}

impl View for PassiveCodeDiff {
    fn ui_name() -> &'static str {
        "PassiveCodeDiff"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Match the AI block's inline-banner wrapper styling exactly
        // (see render_requested_edits_output_message in output.rs).
        Container::new(ChildView::new(&self.diff_view).finish())
            .with_border(Border::all(1.).with_border_fill(theme.surface_2()))
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_background_color(blended_colors::fg_overlay_2(theme).into())
            .finish()
    }
}
