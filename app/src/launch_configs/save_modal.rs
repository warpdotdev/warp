use markdown_parser::{
    FormattedText, FormattedTextFragment, FormattedTextInline, FormattedTextLine,
};
use pathfinder_geometry::vector::vec2f;
use serde::{Deserialize, Serialize};
use warp_core::paths::home_relative_path;
use warp_core::ui::theme::Fill;
use warpui::accessibility::{AccessibilityContent, WarpA11yRole};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    Element, Empty, Flex, FormattedTextElement, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, SavePosition, Shrinkable, Stack, Text,
};
use warpui::keymap::FixedBinding;
use warpui::ui_components::button::{Button, ButtonVariant};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::app_state::{get_app_state, AppState};
use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
};
use crate::launch_configs::launch_config::LaunchConfig;
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::user_config::launch_configs_dir;
#[cfg(feature = "local_fs")]
use crate::user_config::{util::file_name_to_human_readable_name, WarpConfig};
use crate::util::bindings::keybinding_name_to_display_string;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;

const MODAL_WIDTH: f32 = 660.;
const SIDE_PADDING: f32 = 16.;
const BUTTON_SIZE: f32 = 24.;
const DOC_LINK_WIDTH: f32 = 120.;
const SAVE_CONFIG_BUTTON_LABEL: &str = "Save Configuration";
const OPEN_FILE_BUTTON_LABEL: &str = "Open YAML File";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        ActionRequest::Action(LaunchConfigSaveAction::Close),
        id!(LaunchConfigSaveModal::ui_name()),
    )]);

    app.register_fixed_bindings([FixedBinding::new(
        "enter",
        ActionRequest::Enter,
        id!(LaunchConfigSaveModal::ui_name()),
    )]);
}

/// A model for tracking the current source of the snapshot taken of the window configuration
///
/// This is necessary due to a quirk in how the UI Framework handles event handlers / callbacks:
///
/// In order to work around Rusts restriction on having two mutable references to the same data,
/// the framework _removes_ a view from the map of all views before calling a handler (it then
/// immediately re-inserts it into the map afterwards). This means then when a handler is being
/// executed in a given View, that View is _not_ in the global map. This means when taking a
/// snapshot of the current application state (window configuration) the view which calls
/// `get_app_state` will not be included in the snapshot (since it's not in the global map).
///
/// Instead, we create a small Model to cache the snapshot source information (window and view id)
/// and subscribe to any changes to that model from here. Then the model update handler is
/// scheduled after the event handler callback completes. This means that the model update handler is
/// called on the Save Modal directly, rather than the Workspace. This is safe because of two reasons.
/// First, the Save Modal cannot be launched from the Save Modal. Second, the Save Modal doesn't need
/// need to be included in the snapshot of the application state (window configuration).
#[derive(PartialEq)]
enum SnapshotTrigger {
    None,
    StartSnapshot,
}

impl Entity for SnapshotTrigger {
    type Event = ();
}

#[derive(Default)]
struct SaveModalMouseStates {
    close_button_state: MouseStateHandle,
    documentation_link_state: MouseStateHandle,
    save_button_state: MouseStateHandle,
    open_file_button_state: MouseStateHandle,
}

/// View that shows up when a user expresses the intent to save their current
/// app state as a launch config.
pub struct LaunchConfigSaveModal {
    editor: ViewHandle<EditorView>,
    mouse_states: SaveModalMouseStates,
    current_app_state: Option<AppState>,
    snapshot_source: ModelHandle<SnapshotTrigger>,
    save_state: SaveState,
    file_name: Option<String>,
    open_modal_keybinding_str: String,
}

/// Keeps track of the current lifecycle state of the modal
/// NotSaved => Success(file_name)
/// NotSaved => Failure, ideally this shouldn't happen
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum SaveState {
    /// Succeeds to save (String arg is file_name)
    Success,
    /// Fails to save
    Failure(FailureType),
    /// Not yet saved
    NotSaved,
}

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize, Copy)]
pub enum FailureType {
    FileAlreadyExists,
    Other,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ActionRequest {
    Action(LaunchConfigSaveAction),
    Enter,
}

#[derive(Debug, PartialEq, Clone)]
pub enum LaunchConfigSaveAction {
    Close,
    Save,
    OpenFile,
}

impl LaunchConfigSaveAction {
    pub fn from_state(save_state: &SaveState) -> Self {
        match save_state {
            SaveState::Success => Self::OpenFile,
            SaveState::NotSaved => Self::Save,
            SaveState::Failure(_) => Self::Close,
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum LaunchConfigModalEvent {
    /// Users who have just created a launch config want to verify that the launch config is saved
    /// and test opening the config. When they don't see it, they assume there is a bug.
    /// SuccessfullySavedConfig event addresses it.
    /// It's called when the new config was just saved. Note that when we save the configuration,
    /// it take a moment for the file system to register the change, and us to receive it (as there
    /// is a delay in our watcher). But because we actually have the LaunchConfig in our hands
    /// already, we may as well save it "manually" to the WarpConfig, while waiting for the update
    /// from the file system. This event passes a saved config to the handler to let us do that.
    SuccessfullySavedConfig(LaunchConfig),
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: std::path::PathBuf,
        target: FileTarget,
        line_col: Option<warp_util::path::LineAndColumnArg>,
    },
    Close,
}

impl Entity for LaunchConfigSaveModal {
    type Event = LaunchConfigModalEvent;
}

impl LaunchConfigSaveModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editor = {
            let options = SingleLineEditorOptions {
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            ctx.add_typed_action_view(|ctx| EditorView::single_line(options, ctx))
        };
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let snapshot_source = ctx.add_model(|_| SnapshotTrigger::None);
        ctx.observe(
            &snapshot_source,
            LaunchConfigSaveModal::on_snapshot_source_change,
        );

        LaunchConfigSaveModal {
            editor,
            mouse_states: Default::default(),
            current_app_state: Default::default(),
            snapshot_source,
            save_state: SaveState::NotSaved,
            file_name: None,
            open_modal_keybinding_str: keybinding_name_to_display_string(
                "workspace:toggle_launch_config_palette",
                ctx,
            )
            .unwrap_or_default(),
        }
    }

    pub fn set_snapshot_source(&mut self, ctx: &mut ViewContext<Self>) {
        self.snapshot_source.update(ctx, |snapshot_source, ctx| {
            *snapshot_source = SnapshotTrigger::StartSnapshot;
            ctx.notify();
        })
    }

    fn on_snapshot_source_change(
        &mut self,
        source: ModelHandle<SnapshotTrigger>,
        ctx: &mut ViewContext<Self>,
    ) {
        if *source.as_ref(ctx) == SnapshotTrigger::None {
            return;
        }
        let app_state = get_app_state(ctx);
        self.set_current_app_state(app_state);
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                self.handle_action(&ActionRequest::Enter, ctx);
            }
            EditorEvent::Escape => {
                self.handle_action(&ActionRequest::Action(LaunchConfigSaveAction::Close), ctx)
            }
            _ => ctx.notify(),
        }
    }

    /// Open the saved file if the modal is in the correct state
    #[cfg(feature = "local_fs")]
    fn open_file(&self, ctx: &mut ViewContext<Self>) {
        use crate::util::file::external_editor::EditorSettings;
        use crate::util::openable_file_type::resolve_file_target;

        if let SaveState::Success = &self.save_state {
            if let Some(file_name) = &self.file_name {
                let file_path = launch_configs_dir().join(file_name);
                // Resolve target and emit event - workspace will handle all cases
                let settings = EditorSettings::as_ref(ctx);
                let target = resolve_file_target(&file_path, settings, None);
                ctx.emit(LaunchConfigModalEvent::OpenFileWithTarget {
                    path: file_path,
                    target,
                    line_col: None,
                });
                send_telemetry_from_ctx!(TelemetryEvent::OpenLaunchConfigFile, ctx);
            }
        }
    }

    #[cfg(feature = "local_fs")]
    fn try_save_launch_config(&mut self, ctx: &mut ViewContext<Self>) {
        let file_name_candidate = self.editor.as_ref(ctx).buffer_text(ctx);
        let launch_config_name = file_name_to_human_readable_name(&file_name_candidate);
        if let Some(app_state) = &self.current_app_state {
            let launch_config = LaunchConfig::from_snapshot(launch_config_name, app_state);
            match WarpConfig::save_new_launch_config(file_name_candidate, launch_config.clone()) {
                Ok(file_name) => {
                    self.saved_successfully(file_name, ctx);
                    ctx.emit(LaunchConfigModalEvent::SuccessfullySavedConfig(
                        launch_config,
                    ));
                }
                Err(e) => {
                    log::warn!("Failed to save current session as template. error: {e}");
                    if e.to_string().contains("File already exists") {
                        self.failed_save(FailureType::FileAlreadyExists, ctx);
                    } else {
                        self.failed_save(FailureType::Other, ctx);
                    }
                }
            }
        } else {
            self.failed_save(FailureType::Other, ctx);
        }
        ctx.notify();
    }

    /// Clears the editor
    pub fn reset_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text("launch_config.yaml", ctx);
        });
    }

    /// Closes the modal
    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset_editor(ctx); // clears editor and resets placeholder
        self.current_app_state = None;
        self.save_state = SaveState::NotSaved;
        ctx.notify();
        ctx.emit(LaunchConfigModalEvent::Close);
    }

    /// Sets the app state to get all the sessions to put in the yaml on save
    fn set_current_app_state(&mut self, app_state: AppState) {
        self.current_app_state = Some(app_state);
    }

    fn save_modal_button(
        &self,
        appearance: &Appearance,
        button_text: String,
        mouse_state: MouseStateHandle,
        disabled: bool,
    ) -> Button {
        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, mouse_state)
            .with_centered_text_label(button_text)
            .with_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 10.,
                    bottom: 10.,
                    left: 20.,
                    right: 20.,
                }),
                ..Default::default()
            });

        if disabled {
            button.disabled()
        } else {
            button
        }
    }

    fn render_save_config_button(
        &self,
        appearance: &Appearance,
        disabled: bool,
    ) -> Box<dyn Element> {
        self.save_modal_button(
            appearance,
            SAVE_CONFIG_BUTTON_LABEL.to_owned(),
            self.mouse_states.save_button_state.clone(),
            disabled,
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ActionRequest::Action(LaunchConfigSaveAction::Save));
        })
        .finish()
    }

    fn render_open_file_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.save_modal_button(
            appearance,
            OPEN_FILE_BUTTON_LABEL.to_owned(),
            self.mouse_states.open_file_button_state.clone(),
            false,
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ActionRequest::Action(LaunchConfigSaveAction::OpenFile));
        })
        .finish()
    }

    /// Helper function to add padding to a button
    fn add_button_padding(container: Container) -> Container {
        container
            .with_padding_top(20.)
            .with_padding_bottom(20.)
            .with_padding_left(SIDE_PADDING)
            .with_padding_right(SIDE_PADDING)
    }

    /// Renders the button based on the state the modal is in
    fn render_button_row(
        &self,
        appearance: &Appearance,
        save_config_disabled: bool,
    ) -> Box<dyn Element> {
        match LaunchConfigSaveAction::from_state(&self.save_state) {
            LaunchConfigSaveAction::OpenFile => {
                Self::add_button_padding(Container::new(self.render_open_file_button(appearance)))
                    .finish()
            }
            LaunchConfigSaveAction::Save => Self::add_button_padding(Container::new(
                self.render_save_config_button(appearance, save_config_disabled),
            ))
            .finish(),
            LaunchConfigSaveAction::Close => Empty::new().finish(),
        }
    }

    /// Renders a generic text block in a span
    fn render_text_block(&self, appearance: &Appearance, text: String) -> Container {
        Container::new(
            appearance
                .ui_builder()
                .span(text)
                .with_soft_wrap()
                .build()
                .finish(),
        )
        .with_padding_left(SIDE_PADDING)
        .with_padding_right(SIDE_PADDING)
    }

    fn render_formatted_text_line(
        &self,
        appearance: &Appearance,
        formatted_text_fragments: FormattedTextInline,
    ) -> Container {
        let formatted_text =
            FormattedText::new([FormattedTextLine::Line(formatted_text_fragments)]);
        Container::new(
            FormattedTextElement::new(
                formatted_text,
                appearance.ui_font_size(),
                appearance.ui_font_family(),
                appearance.monospace_font_family(),
                appearance.theme().active_ui_text_color().into(),
                Default::default(),
            )
            .finish(),
        )
        .with_padding_left(SIDE_PADDING)
        .with_padding_right(SIDE_PADDING)
    }

    /// Renders the title of the modal
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let header = Flex::row()
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Text::new_inline(
                            "Save Current Configuration",
                            appearance.header_font_family(),
                            appearance.header_font_size(),
                        )
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .close_button(BUTTON_SIZE, self.mouse_states.close_button_state.clone())
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ActionRequest::Action(
                            LaunchConfigSaveAction::Close,
                        ))
                    })
                    .finish(),
            )
            .finish();

        Container::new(header)
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
            .with_padding_left(SIDE_PADDING)
            .with_padding_right(SIDE_PADDING)
            .with_padding_top(SIDE_PADDING)
            .with_padding_bottom(8.)
            .finish()
    }

    pub fn saved_successfully(&mut self, file_name: String, ctx: &mut ViewContext<Self>) {
        self.set_save_state(SaveState::Success, Some(file_name));
        send_telemetry_from_ctx!(
            TelemetryEvent::SaveLaunchConfig {
                state: SaveState::Success,
            },
            ctx
        );
        ctx.notify();
    }

    pub fn failed_save(&mut self, failure_type: FailureType, ctx: &mut ViewContext<Self>) {
        self.set_save_state(SaveState::Failure(failure_type), None);
        send_telemetry_from_ctx!(
            TelemetryEvent::SaveLaunchConfig {
                state: SaveState::Failure(failure_type)
            },
            ctx
        );
        ctx.notify();
    }

    /// Renders the editor portion of the modal
    fn render_editor(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let editor = self.editor.as_ref(app);
        let height = editor.line_height(app.font_cache(), appearance);
        let editor =
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.editor).finish()).finish())
                .with_height(height)
                .finish();
        Container::new(editor)
            .with_uniform_padding(SIDE_PADDING)
            // TODO theme should be agnostic of different UI elements / features
            .with_background(appearance.theme().background())
            .finish()
    }

    /// Renders the entire modal
    fn render_modal(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Title of modal
        let header = Flex::column().with_child(
            ConstrainedBox::new(self.render_header(appearance))
                .with_max_height(60.)
                .finish(),
        );

        let link_to_docs = Container::new(
            ConstrainedBox::new(
                appearance
                    .ui_builder()
                    .link(
                        "Link to Documentation".to_string(),
                        Some(
                            "https://docs.warp.dev/terminal/sessions/launch-configurations"
                                .to_string(),
                        ),
                        None,
                        self.mouse_states.documentation_link_state.clone(),
                    )
                    .soft_wrap(false)
                    .build()
                    .finish(),
            )
            .with_width(DOC_LINK_WIDTH)
            .finish(),
        )
        .with_uniform_padding(SIDE_PADDING)
        .finish();

        let info = match &self.save_state {
            SaveState::Success => header
                .with_child(
                    self.render_formatted_text_line(appearance, vec![
                        FormattedTextFragment::plain_text("Saved successfully to "),
                        FormattedTextFragment::inline_code(self.file_name.clone().unwrap_or_default()),
                        FormattedTextFragment::plain_text(".")
                    ])
                    .with_padding_bottom(24.)
                    .finish(),
                )
                .with_child(link_to_docs),
            SaveState::Failure(failure_type) => header.with_child(
                self.render_text_block(
                    appearance,
                    match failure_type {
                        FailureType::FileAlreadyExists => {
                            "Failed to save. A launch configuration with the same name already exists.".to_string()
                        }
                        FailureType::Other => "An issue was encountered while saving.".to_string(),
                    },
                )
                .with_padding_bottom(24.)
                .finish(),
            ),
            SaveState::NotSaved => {
                let mut text = "This will save your current configuration of windows, tabs \
                and panes to a file so you can easily open it again".to_string();
                if self.open_modal_keybinding_str.is_empty() {
                    text.push('.');
                } else {
                    text.push_str(&format!(" with {}.", self.open_modal_keybinding_str));
                }
                header
                    .with_child(
                        self.render_formatted_text_line(appearance, vec![
                            FormattedTextFragment::plain_text(text)
                        ]).finish()
                    )
                    .with_child(
                        self.render_formatted_text_line(appearance, vec![
                            FormattedTextFragment::plain_text("\nThe YAML file is saved to "),
                            FormattedTextFragment::inline_code(home_relative_path(&launch_configs_dir())),
                            FormattedTextFragment::plain_text("."),
                        ])
                        .with_padding_bottom(24.)
                        .finish(),
                    )
                    .with_child(link_to_docs)
                    .with_child(self.render_editor(app))
            }
        };

        ConstrainedBox::new(
            Container::new(
                info.with_child(
                    self.render_button_row(appearance, self.editor.as_ref(app).is_empty(app)),
                )
                .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
            .with_uniform_margin(35.)
            .with_background(appearance.theme().surface_2())
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish()
    }

    fn set_save_state(&mut self, save_state: SaveState, file_name: Option<String>) {
        self.save_state = save_state;
        if let Some(file_name) = file_name {
            self.file_name = Some(file_name);
        }
    }
}

impl View for LaunchConfigSaveModal {
    fn ui_name() -> &'static str {
        "LaunchConfigSaveModal"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // renders the modal in a stack
        let mut stack = Stack::new();
        stack.add_positioned_child(
            SavePosition::new(self.render_modal(app), "save_modal:modal").finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }

    fn accessibility_contents(&self, _ctx: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "Save Config Modal",
            "Type the name of the file to which you want to save your
            current configuration of windows, tabs, and panes. Use enter to save the
            launch configuration, esc to quit the save configuration modal.",
            WarpA11yRole::PopoverRole,
        ))
    }
}

impl TypedActionView for LaunchConfigSaveModal {
    type Action = ActionRequest;

    fn handle_action(&mut self, action: &ActionRequest, ctx: &mut ViewContext<Self>) {
        // TODO(vorporeal): We should figure out a better way to handle the
        // interactions with the filesystem here, whether it's compiling out
        // the save modal more completely or doing something else.  Perhaps
        // this will become moot when we put launch configs in Warp Drive.
        let action = match action {
            ActionRequest::Action(action) => action.clone(),
            ActionRequest::Enter => LaunchConfigSaveAction::from_state(&self.save_state),
        };
        match action {
            LaunchConfigSaveAction::Close => self.close(ctx),
            LaunchConfigSaveAction::Save =>
            {
                #[cfg(feature = "local_fs")]
                if !self.editor.as_ref(ctx).is_empty(ctx) {
                    self.try_save_launch_config(ctx);
                }
            }
            LaunchConfigSaveAction::OpenFile => {
                #[cfg(feature = "local_fs")]
                self.open_file(ctx);
                self.close(ctx);
            }
        }
    }
}
