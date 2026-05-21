//! Components for the notebook header.

use warp_core::features::FeatureFlag;
use warpui::{
    elements::{
        Container, CrossAxisAlignment, Flex, Highlight, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Shrinkable,
    },
    platform::Cursor,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, SingletonEntity,
};

use crate::{
    appearance::Appearance,
    cloud_object::{
        breadcrumbs::ContainingObject,
        model::view::{Editor, EditorState},
    },
    drive::sharing::ContentEditability,
    notebooks::{active_notebook_data::Mode, styles},
    ui_components::{
        breadcrumb::{render_breadcrumbs, BreadcrumbState},
        buttons::{accent_icon_button, icon_button},
        icons::Icon,
    },
    workspaces::user_profiles::UserProfiles,
};

use super::{super::active_notebook_data::ActiveNotebookData, NotebookAction, EDIT_BUTTON_MARGIN};

/// Component to show details about a notebook:
/// * Interactive breadcrumbs for its location within Warp Drive
/// * The current editor of the notebook
/// * Grab-the-baton UI controls
pub struct DetailsBar {
    breadcrumbs: Vec<BreadcrumbState<ContainingObject>>,
    edit_mode_button_mouse_state: MouseStateHandle,
}

impl DetailsBar {
    pub fn new() -> Self {
        Self {
            breadcrumbs: Vec::new(),
            edit_mode_button_mouse_state: Default::default(),
        }
    }

    /// Update the cached breadcrumbs in the notebook header.
    pub fn update_breadcrumbs(&mut self, notebook_data: &ActiveNotebookData, ctx: &AppContext) {
        self.breadcrumbs = notebook_data
            .breadcrumbs(ctx)
            .map(|breadcrumbs| breadcrumbs.into_iter().map(BreadcrumbState::new).collect())
            .unwrap_or_default();
    }

    pub fn render(
        &self,
        notebook_data: &ActiveNotebookData,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        header_row.add_child(
            Shrinkable::new(
                2.,
                render_breadcrumbs(
                    self.breadcrumbs.iter().cloned(),
                    appearance,
                    |ctx, _, breadcrumb| {
                        ctx.dispatch_typed_action(NotebookAction::ViewInWarpDrive(
                            breadcrumb.kind.into_item_id(),
                        ));
                    },
                ),
            )
            .finish(),
        );

        let mut editing_state_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(editor) = notebook_data.current_editor(app) {
            editing_state_row.add_child(
                Shrinkable::new(1., self.render_editor(&editor, appearance, app)).finish(),
            );
        }

        let editability = if FeatureFlag::SharedWithMe.is_enabled() {
            notebook_data.editability(app)
        } else {
            ContentEditability::Editable
        };
        if matches!(
            editability,
            ContentEditability::RequiresLogin | ContentEditability::Editable
        ) {
            editing_state_row.add_child(self.render_mode_toggle(
                notebook_data.mode,
                editability,
                appearance,
                app,
            ));
        }

        header_row.add_child(Shrinkable::new(1., editing_state_row.finish()).finish());

        header_row.finish()
    }

    /// Renders a toggle button for the editing mode.
    fn render_mode_toggle(
        &self,
        mode: Mode,
        editability: ContentEditability,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut edit_button = match mode {
            Mode::View => icon_button(
                appearance,
                Icon::Pencil,
                false,
                self.edit_mode_button_mouse_state.clone(),
            ),
            Mode::Editing => accent_icon_button(
                appearance,
                Icon::Pencil,
                false,
                self.edit_mode_button_mouse_state.clone(),
            ),
        };

        if matches!(editability, ContentEditability::RequiresLogin) {
            let ui_builder = appearance.ui_builder().clone();
            let tooltip = crate::i18n::tr_static(app, "Sign in to edit").to_string();
            edit_button = edit_button
                .with_tooltip(move || ui_builder.tool_tip(tooltip.clone()).build().finish());
        }

        Container::new(
            edit_button
                .build()
                .on_click(move |ctx, _, _| {
                    if editability.can_edit() {
                        ctx.dispatch_typed_action(NotebookAction::ToggleMode)
                    }
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
        )
        .with_margin_left(EDIT_BUTTON_MARGIN)
        .with_margin_right(EDIT_BUTTON_MARGIN)
        .finish()
    }

    /// Renders a label for the current editor.
    fn render_editor(
        &self,
        editor: &Editor,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let base_text_styles = UiComponentStyles {
            font_color: Some(styles::title_text_fill(appearance).into_solid()),
            ..Default::default()
        };
        let theme = appearance.theme();
        match editor.state {
            EditorState::None => appearance
                .ui_builder()
                .span(crate::i18n::tr_static(app, "Viewing"))
                .with_style(base_text_styles)
                .build()
                .finish(),
            EditorState::CurrentUser => appearance
                .ui_builder()
                .span(crate::i18n::tr_static(app, "Editing"))
                .with_style(base_text_styles)
                .build()
                .finish(),
            EditorState::OtherUserActive | EditorState::OtherUserIdle => {
                let editor = editor_display_name(editor.email.as_deref(), app);
                appearance
                    .ui_builder()
                    .span(format!("{editor} is editing"))
                    .with_style(base_text_styles)
                    .with_highlights(
                        (0..editor.chars().count()).collect(),
                        Highlight::new().with_foreground_color(
                            theme.main_text_color(theme.background()).into_solid(),
                        ),
                    )
                    .build()
                    .finish()
            }
        }
    }
}

/// Get the display name for an editor.
fn editor_display_name(email: Option<&str>, app: &AppContext) -> String {
    match email {
        Some(email) => UserProfiles::as_ref(app)
            .displayable_identifier_for_email(email)
            .unwrap_or_else(|| email.to_string()),
        None => "Other user".to_string(),
    }
}

#[cfg(test)]
#[path = "details_bar_tests.rs"]
mod tests;
