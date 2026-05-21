use warpui::{
    elements::{
        Border, Clipped, Container, CornerRadius, Dismiss, Empty, Flex, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
    },
    fonts::Weight,
    platform::Cursor,
    presenter::ChildView,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, ViewHandle,
};

use crate::cloud_object::Space;
use crate::{
    appearance::Appearance, editor::EditorView, server::ids::SyncId, ui_components::blended_colors,
};

use super::{index::DriveIndexAction, DriveObjectType};

const DIALOG_PADDING: f32 = 24.;
const INPUT_MARGIN_TOP: f32 = 16.;
const INPUT_MARGIN_BOTTOM: f32 = 24.;
const INPUT_PADDING_HORIZONTAL: f32 = 16.;
const INPUT_PADDING_VERTICAL: f32 = 10.;
const BORDER_RADIUS_SMALL: f32 = 4.;
const BORDER_RADIUS_LARGE: f32 = 8.;
const BORDER_WIDTH: f32 = 1.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_PADDING: f32 = 12.;
const BUTTON_MARGIN_BETWEEN: f32 = 8.;

const NOTEBOOK_TITLE: &str = "Notebook name";
const FOLDER_TITLE: &str = "Folder name";
const ENV_VAR_COLLECTION_TITLE: &str = "Collection name";
const CREATE_BUTTON_TEXT: &str = "Create";
const CANCEL_BUTTON_TEXT: &str = "Cancel";
const RENAME_BUTTON_TEXT: &str = "Rename";

/// Struct holding necessary information and states for the dialog
/// that opens when creating or updating a folder or notebook.
///
/// This dialog can be opened for a folder or a space. If open_for_folder_id = None, it's a space.
/// If open_for_folder_id = Some, it's a specific folder.
#[derive(Clone)]
pub struct CloudObjectNamingDialog {
    pub title_editor: ViewHandle<EditorView>,
    cancel_mouse_state: MouseStateHandle,
    primary_action_mouse_state: MouseStateHandle,
    pub object_type: Option<DriveObjectType>,
    pub space: Option<Space>,
    is_rename: bool,
    // If the naming dialog is opened for a folder, then we store the open_for_folder_id.
    pub open_for_folder_id: Option<SyncId>,
}

impl CloudObjectNamingDialog {
    pub fn new(title_editor: ViewHandle<EditorView>) -> Self {
        Self {
            title_editor,
            cancel_mouse_state: Default::default(),
            primary_action_mouse_state: Default::default(),
            object_type: Default::default(),
            space: None,
            is_rename: false,
            open_for_folder_id: None,
        }
    }

    pub fn close(&mut self, app: &mut AppContext) {
        self.object_type = None;
        self.space = None;
        self.is_rename = false;
        self.open_for_folder_id = None;
        self.title_editor.update(app, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            ctx.notify();
        });
    }

    pub fn open(
        &mut self,
        object_type: DriveObjectType,
        space: Space,
        initial_folder_id: Option<SyncId>,
        is_rename: bool,
        existing_name: Option<String>,
        app: &mut AppContext,
    ) {
        self.object_type = Some(object_type);
        self.space = Some(space);
        self.is_rename = is_rename;
        self.open_for_folder_id = initial_folder_id;

        if let Some(name) = existing_name {
            self.title_editor.update(app, |editor, ctx| {
                editor.set_buffer_text(name.as_str(), ctx)
            })
        }
    }

    pub fn is_open(&self) -> bool {
        self.object_type.is_some()
    }

    // The renaming dialog can either be open for a space or a folder. If it's a space, open_for_folder_id = None.
    pub fn is_open_for_space(&self, space: &Space) -> bool {
        self.is_open() && self.open_for_folder_id.is_none() && (self.space == Some(*space))
    }

    pub fn is_open_for_folder(&self, folder_id: SyncId) -> bool {
        self.is_open() && (self.open_for_folder_id == Some(folder_id))
    }

    /// Returns the KnowledgeIndexAction that's appropriate to the current state of this dialog.
    /// If the dialog is not open or in an invalid state, returns None.
    pub fn current_primary_action(&self) -> Option<DriveIndexAction> {
        match self.open_for_folder_id {
            Some(folder_id) if self.is_rename => Some(DriveIndexAction::RenameFolder { folder_id }),
            _ => {
                let object_type = self.object_type?;
                let space = self.space?;
                Some(DriveIndexAction::CreateObject {
                    object_type,
                    space,
                    initial_folder_id: self.open_for_folder_id,
                })
            }
        }
    }

    pub fn title(&self, app: &AppContext) -> Option<String> {
        self.is_open()
            .then(|| self.title_editor.as_ref(app).buffer_text(app))
    }

    fn render_text_header(
        &self,
        object_type: DriveObjectType,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let title = match object_type {
            DriveObjectType::Notebook { .. } => NOTEBOOK_TITLE,
            DriveObjectType::Folder => FOLDER_TITLE,
            DriveObjectType::EnvVarCollection => ENV_VAR_COLLECTION_TITLE,
            // workflows and ai facts aren't a part of this dialog
            DriveObjectType::Workflow
            | DriveObjectType::AgentModeWorkflow
            | DriveObjectType::AIFact
            | DriveObjectType::AIFactCollection
            | DriveObjectType::MCPServer
            | DriveObjectType::MCPServerCollection => "",
        };

        Text::new_inline(
            title,
            appearance.ui_font_family(),
            appearance.header_font_size(),
        )
        .with_color(
            appearance
                .theme()
                .main_text_color(appearance.theme().surface_1())
                .into(),
        )
        .finish()
    }

    fn render_input(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(Clipped::new(ChildView::new(&self.title_editor).finish()).finish())
            .with_margin_top(INPUT_MARGIN_TOP)
            .with_margin_bottom(INPUT_MARGIN_BOTTOM)
            .with_padding_top(INPUT_PADDING_VERTICAL)
            .with_padding_bottom(INPUT_PADDING_VERTICAL)
            .with_padding_left(INPUT_PADDING_HORIZONTAL)
            .with_padding_right(INPUT_PADDING_HORIZONTAL)
            .with_background(appearance.theme().background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS_SMALL)))
            .with_border(Border::all(BORDER_WIDTH).with_border_fill(appearance.theme().outline()))
            .finish()
    }

    fn render_action_buttons(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let default_button_styles = UiComponentStyles {
            font_size: Some(BUTTON_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().accent_button_color())
                    .into(),
            ),
            font_weight: Some(Weight::Bold),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS_SMALL))),
            border_color: Some(appearance.theme().outline().into()),
            border_width: Some(BORDER_WIDTH),
            padding: Some(Coords::uniform(BUTTON_PADDING)),
            background: Some(appearance.theme().surface_1().into()),
            ..Default::default()
        };

        let primary_button_styles = UiComponentStyles {
            background: Some(appearance.theme().accent_button_color().into()),
            border_color: Some(appearance.theme().accent_button_color().into()),
            ..default_button_styles
        };

        let primary_disabled_styles = UiComponentStyles {
            background: Some(appearance.theme().surface_3().into()),
            border_color: Some(appearance.theme().surface_3().into()),
            font_color: Some(
                appearance
                    .theme()
                    .disabled_text_color(appearance.theme().background())
                    .into(),
            ),
            ..primary_button_styles
        };

        let primary_hovered_and_clicked_styles = UiComponentStyles {
            background: Some(blended_colors::accent_hover(appearance.theme()).into()),
            border_color: Some(blended_colors::accent_hover(appearance.theme()).into()),
            ..primary_button_styles
        };

        let primary_button_text = match self.is_rename {
            true => RENAME_BUTTON_TEXT,
            false => CREATE_BUTTON_TEXT,
        };

        let primary_button_action = self.current_primary_action();

        let mut primary_button = appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Basic,
                self.primary_action_mouse_state.clone(),
                primary_button_styles,
                Some(primary_hovered_and_clicked_styles),
                Some(primary_hovered_and_clicked_styles),
                Some(primary_disabled_styles),
            )
            .with_text_label(primary_button_text.into());

        if let Some(title) = self.title(app) {
            if title.is_empty() || !self.title_editor.as_ref(app).is_dirty(app) {
                primary_button = primary_button.disabled();
            }
        }

        Flex::row()
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(
                        appearance
                            .ui_builder()
                            .button(ButtonVariant::Secondary, self.cancel_mouse_state.clone())
                            .with_style(UiComponentStyles {
                                font_size: Some(BUTTON_FONT_SIZE),
                                font_weight: Some(Weight::Bold),
                                padding: Some(Coords::uniform(BUTTON_PADDING)),
                                ..Default::default()
                            })
                            .with_text_label(CANCEL_BUTTON_TEXT.into())
                            .build()
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    DriveIndexAction::CloseCloudObjectNamingDialog,
                                )
                            })
                            .finish(),
                    )
                    .with_margin_right(BUTTON_MARGIN_BETWEEN)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(
                        primary_button
                            .build()
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                if let Some(primary_action) = primary_button_action.clone() {
                                    ctx.dispatch_typed_action(primary_action)
                                }
                            })
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    pub fn render(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let object_type = self.object_type.unwrap_or(DriveObjectType::Folder);

        if self.space.is_none() {
            return Empty::new().finish();
        }

        let theme = appearance.theme();

        Dismiss::new(
            Container::new(
                Flex::column()
                    .with_child(self.render_text_header(object_type, appearance))
                    .with_child(self.render_input(appearance))
                    .with_child(self.render_action_buttons(appearance, app))
                    .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS_LARGE)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_background(theme.surface_1())
            .with_uniform_padding(DIALOG_PADDING)
            .finish(),
        )
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(DriveIndexAction::CloseCloudObjectNamingDialog)
        })
        .finish()
    }
}
