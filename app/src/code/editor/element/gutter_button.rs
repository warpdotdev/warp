use crate::localization;
use crate::view_components::action_button::{
    ActionButtonTheme, DisabledSecondaryTheme, SecondaryTheme,
};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::contrast::MinimumAllowedContrast;
use warp_core::ui::color::ContrastingColor;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::MouseState;
use warpui::AppContext;

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

/// A button rendered within the gutter of the editor.
pub(super) trait GutterButton {
    /// The icon color for the gutter.
    fn icon_color(&self, mouse_state: &MouseState, appearance: &Appearance) -> Fill {
        let button_background = self.background_color(mouse_state, appearance);

        let is_hovered = mouse_state.is_hovered();
        let color = if self.is_enabled() {
            SecondaryTheme.text_color(is_hovered, Some(button_background), appearance)
        } else {
            DisabledSecondaryTheme.text_color(is_hovered, Some(button_background), appearance)
        };

        let contrast_shifted_color = color.on_background(
            button_background.into_solid(),
            MinimumAllowedContrast::NonText,
        );
        contrast_shifted_color.into()
    }

    /// The background color of the button.
    fn background_color(&self, mouse_state: &MouseState, appearance: &Appearance) -> Fill {
        if self.is_enabled() {
            if mouse_state.is_hovered() {
                Fill::Solid(internal_colors::neutral_3(appearance.theme()))
            } else {
                Fill::Solid(internal_colors::neutral_1(appearance.theme()))
            }
        } else {
            Fill::Solid(internal_colors::neutral_1(appearance.theme()))
        }
    }

    /// Whether the button is currently enabled. If false, the button is rendered in a disabled
    /// state.
    fn is_enabled(&self) -> bool;

    /// The tooltip text displayed when the button is hovered.
    fn tooltip_text(&self, app: &AppContext) -> Option<String>;

    /// The icon of the button.
    fn icon(&self) -> Icon;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AddAsContextButton {
    is_enabled: bool,
}

impl AddAsContextButton {
    pub fn new(is_enabled: bool) -> Self {
        Self { is_enabled }
    }
}

impl GutterButton for AddAsContextButton {
    fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    fn tooltip_text(&self, app: &AppContext) -> Option<String> {
        if self.is_enabled {
            Some(text(app, "code.gutter.tooltip.add_diff_hunk_as_context"))
        } else {
            Some(text(app, "code.gutter.tooltip.save_changes_to_attach"))
        }
    }

    fn icon(&self) -> Icon {
        Icon::Paperclip
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RevertHunkButton {
    is_enabled: bool,
}

impl RevertHunkButton {
    pub fn new(is_enabled: bool) -> Self {
        Self { is_enabled }
    }
}

impl GutterButton for RevertHunkButton {
    fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    fn tooltip_text(&self, app: &AppContext) -> Option<String> {
        if self.is_enabled {
            Some(text(app, "code.gutter.tooltip.revert_diff_hunk"))
        } else {
            Some(text(app, "code.gutter.tooltip.save_changes_to_revert"))
        }
    }

    fn icon(&self) -> Icon {
        Icon::ReverseLeft
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[allow(dead_code)]
pub enum CommentButton {
    #[default]
    CreateNewComment,
    Disabled,
    AddedComment,
    EditorOpenedToCreateNewComment,
    EditorOpenedToUpdateComment,
}

impl GutterButton for CommentButton {
    fn background_color(&self, mouse_state: &MouseState, appearance: &Appearance) -> Fill {
        match self {
            CommentButton::CreateNewComment => {
                if mouse_state.is_hovered() {
                    Fill::Solid(internal_colors::neutral_3(appearance.theme()))
                } else {
                    Fill::Solid(internal_colors::neutral_1(appearance.theme()))
                }
            }
            CommentButton::EditorOpenedToCreateNewComment => {
                Fill::Solid(internal_colors::neutral_3(appearance.theme()))
            }
            CommentButton::Disabled => Fill::Solid(internal_colors::neutral_1(appearance.theme())),
            CommentButton::AddedComment | CommentButton::EditorOpenedToUpdateComment => {
                internal_colors::accent(appearance.theme())
            }
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(
            self,
            CommentButton::AddedComment
                | CommentButton::CreateNewComment
                | CommentButton::EditorOpenedToCreateNewComment
        )
    }

    fn tooltip_text(&self, app: &AppContext) -> Option<String> {
        match self {
            CommentButton::CreateNewComment => Some(text(app, "code.gutter.tooltip.add_comment")),
            CommentButton::Disabled => {
                Some(text(app, "code.gutter.tooltip.save_changes_to_add_comment"))
            }
            CommentButton::AddedComment => {
                Some(text(app, "code.gutter.tooltip.show_saved_comment"))
            }
            CommentButton::EditorOpenedToCreateNewComment
            | CommentButton::EditorOpenedToUpdateComment => None,
        }
    }

    fn icon(&self) -> Icon {
        match self {
            CommentButton::CreateNewComment
            | CommentButton::Disabled
            | CommentButton::EditorOpenedToCreateNewComment => Icon::MessagePlusSquare,
            CommentButton::AddedComment | CommentButton::EditorOpenedToUpdateComment => {
                Icon::MessageText
            }
        }
    }
}
