use std::ops::Range;

use ai::diff_validation::DiffType;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::elements::ScrollbarWidth;
use warpui::{AppContext, View, ViewContext, ViewHandle};

use super::editor::scroll::ScrollWheelBehavior;
use super::editor::view::CodeEditorView;
use super::editor::NavBarBehavior;
use crate::editor::InteractionState;

/// Whether a view is displayed in a full pane or embedded in another view, like the blocklist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DisplayMode {
    /// The code diff view takes up its own pane.
    FullPane,
    /// The code diff view is embedded inside an AI block.
    Embedded { max_height: f32 },
    /// The code diff view is its own element in the blocklist,
    /// instead of being nested inside an existing block.
    InlineBanner {
        max_height: f32,
        is_expanded: bool,
        is_dismissed: bool,
    },
}

impl DisplayMode {
    pub fn with_embedded(max_height: f32) -> Self {
        DisplayMode::Embedded { max_height }
    }

    pub fn with_inline_banner(max_height: f32) -> Self {
        DisplayMode::InlineBanner {
            max_height,
            is_expanded: false,
            is_dismissed: false,
        }
    }

    pub fn max_height(&self) -> Option<f32> {
        match self {
            DisplayMode::FullPane => None,
            DisplayMode::Embedded { max_height } => Some(*max_height),
            DisplayMode::InlineBanner { max_height, .. } => Some(*max_height),
        }
    }

    pub(crate) fn scroll_wheel_behavior(&self) -> ScrollWheelBehavior {
        match self {
            DisplayMode::InlineBanner {
                is_expanded: false, ..
            } => ScrollWheelBehavior::NeverHandle,
            _ => ScrollWheelBehavior::AlwaysHandle,
        }
    }

    pub(crate) fn scrollbar_appearance(&self) -> ScrollableAppearance {
        match self {
            DisplayMode::InlineBanner {
                is_expanded: false, ..
            } => ScrollableAppearance::new(ScrollbarWidth::None, true),
            _ => ScrollableAppearance::new(ScrollbarWidth::Auto, false),
        }
    }

    pub(crate) fn vertical_expansion_behavior(&self) -> VerticalExpansionBehavior {
        match self {
            DisplayMode::FullPane => VerticalExpansionBehavior::FillMaxHeight,
            DisplayMode::Embedded { .. } => VerticalExpansionBehavior::GrowToMaxHeight,
            DisplayMode::InlineBanner { .. } => VerticalExpansionBehavior::GrowToMaxHeight,
        }
    }

    pub(crate) fn interaction_state(&self, is_delete: bool) -> InteractionState {
        if is_delete {
            return InteractionState::Selectable;
        }
        match self {
            DisplayMode::FullPane => InteractionState::Editable,
            DisplayMode::Embedded { .. } => InteractionState::Selectable,
            DisplayMode::InlineBanner { .. } => InteractionState::Selectable,
        }
    }

    pub(crate) fn show_nav_bar(&self) -> bool {
        !matches!(self, DisplayMode::InlineBanner { .. })
    }

    pub fn title(&self) -> Option<&str> {
        match self {
            DisplayMode::InlineBanner { .. } => Some("Suggested fixes based on your last command:"),
            _ => None,
        }
    }

    pub fn is_full_pane(&self) -> bool {
        matches!(self, DisplayMode::FullPane)
    }

    pub fn is_embedded(&self) -> bool {
        matches!(self, DisplayMode::Embedded { .. })
    }

    pub fn is_inline_banner(&self) -> bool {
        matches!(self, DisplayMode::InlineBanner { .. })
    }
}

/// A shared trait for views that display an inline diff.
/// Implemented by both `LocalCodeEditorView` (for native file-backed diffs)
/// and `InlineDiffView` (for mocked/WASM diffs).
pub trait DiffViewer
where
    Self: Sized + View,
{
    fn editor(&self) -> &ViewHandle<CodeEditorView>;
    fn diff(&self) -> Option<&DiffType>;

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn was_edited(&self) -> bool {
        false
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn changed_lines(&self, ctx: &AppContext) -> Vec<Range<usize>> {
        self.editor().as_ref(ctx).changed_lines(ctx)
    }

    fn set_display_mode(&self, mode: DisplayMode, ctx: &mut ViewContext<Self>) {
        let is_delete = matches!(self.diff(), Some(DiffType::Delete { .. }));
        self.editor().update(ctx, |editor, ctx| {
            editor.set_scroll_wheel_behavior(mode.scroll_wheel_behavior());
            editor.set_vertical_expansion_behavior(mode.vertical_expansion_behavior(), ctx);
            editor.set_vertical_scrollbar_appearance(mode.scrollbar_appearance());
            editor.set_horizontal_scrollbar_appearance(mode.scrollbar_appearance());
            editor.set_interaction_state(mode.interaction_state(is_delete), ctx);
            editor.set_show_nav_bar(mode.show_nav_bar());
            editor.set_nav_bar_behavior(NavBarBehavior::NotClosable, ctx);
        });
    }

    fn navigate_next_diff_hunk(&self, ctx: &mut ViewContext<Self>) {
        self.editor()
            .update(ctx, |editor, ctx| editor.navigate_next_diff_hunk(ctx));
    }

    fn navigate_previous_diff_hunk(&self, ctx: &mut ViewContext<Self>) {
        self.editor()
            .update(ctx, |editor, ctx| editor.navigate_previous_diff_hunk(ctx));
    }

    fn reject_diff(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn restore_diff_base(&mut self, _ctx: &mut ViewContext<Self>) -> Result<(), String> {
        Ok(())
    }
}
