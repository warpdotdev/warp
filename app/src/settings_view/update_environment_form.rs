use super::{
    editor_text_colors,
    settings_page::{render_input_list, InputListItem},
};
use crate::server::server_api::ServerApiProvider;
use crate::{
    ai::ambient_agents::{
        github_auth_url::{self, AuthSource, GithubAuthRedirectTarget},
        telemetry::CloudAgentTelemetryEvent,
    },
    ai::{
        ambient_agents::github_auth_notifier::{GitHubAuthEvent, GitHubAuthNotifier},
        cloud_environments::{AmbientAgentEnvironment, GithubRepo},
    },
    appearance::Appearance,
    editor::{
        EditorOptions, EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
        TextOptions,
    },
    root_view::CreateEnvironmentArg,
    server::ids::SyncId,
    ui_components::{buttons::icon_button, icons::Icon},
    view_components::{
        action_button::{ActionButton, DangerSecondaryTheme, PrimaryTheme},
        render_warning_box, SubmittableTextInput, SubmittableTextInputEvent,
        WarningBoxButtonConfig, WarningBoxConfig,
    },
    workspaces::user_workspaces::UserWorkspaces,
    ChannelState,
};
use instant::{Duration, Instant};
use log::debug;
#[cfg(not(target_family = "wasm"))]
use std::collections::HashMap;
use url::Url;
use warp_core::send_telemetry_from_ctx;
use warp_editor::editor::NavigationKey;
use warp_graphql::queries::user_github_info::UserGithubInfoResult;
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle, ClippedScrollable,
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Empty,
        Expanded, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
        OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition, ScrollTarget,
        ScrollToPositionMode, ScrollbarWidth, SizeConstraintCondition, SizeConstraintSwitch, Stack,
        Text,
    },
    fonts::{Properties, Weight},
    geometry::vector::vec2f,
    keymap::FixedBinding,
    platform::Cursor,
    prelude::Coords,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const SUBMIT_BUTTON_FOCUSED: &str = "SubmitButtonFocused";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            UpdateEnvironmentFormAction::Submit,
            id!(UpdateEnvironmentForm::ui_name()) & id!(SUBMIT_BUTTON_FOCUSED),
        ),
        FixedBinding::new(
            "numpadenter",
            UpdateEnvironmentFormAction::Submit,
            id!(UpdateEnvironmentForm::ui_name()) & id!(SUBMIT_BUTTON_FOCUSED),
        ),
        FixedBinding::new(
            "shift-tab",
            UpdateEnvironmentFormAction::FocusSetupCommandsInput,
            id!(UpdateEnvironmentForm::ui_name()) & id!(SUBMIT_BUTTON_FOCUSED),
        ),
        // Escape behaves like:
        // - close dropdown when open
        // - back/cancel when dropdown is closed
        FixedBinding::new(
            "escape",
            UpdateEnvironmentFormAction::Escape,
            id!(UpdateEnvironmentForm::ui_name()),
        ),
    ]);
}

/// Form data model representing environment values independent of UI state.
#[derive(Clone, Debug, Default)]
pub struct EnvironmentFormValues {
    pub name: String,
    pub description: String,
    pub selected_repos: Vec<GithubRepo>,
    pub docker_image: String,
    pub setup_commands: Vec<String>,
}

impl EnvironmentFormValues {
    /// Converts form values to an AmbientAgentEnvironment for submission.
    pub fn to_ambient_agent_environment(&self) -> AmbientAgentEnvironment {
        let setup_commands: Vec<String> = self
            .setup_commands
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();

        let description = {
            let trimmed = self.description.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };

        AmbientAgentEnvironment::new(
            self.name.trim().to_string(),
            description,
            self.selected_repos.clone(),
            self.docker_image.trim().to_string(),
            setup_commands,
        )
    }

    /// Validates the form values.
    pub fn is_valid(&self) -> bool {
        !self.name.trim().is_empty() && !self.docker_image.trim().is_empty()
    }
}

/// Arguments for initializing the UpdateEnvironmentForm.
/// Contains all data needed for initialization, including initial values for Edit mode.
#[derive(Clone, Debug)]
pub enum EnvironmentFormInitArgs {
    Create,
    Edit {
        env_id: SyncId,
        initial_values: Box<EnvironmentFormValues>,
    },
}

/// Persisted mode for the UpdateEnvironmentForm.
/// Contains only what's needed after initialization (just the env_id for Edit mode).
#[derive(Clone, Debug)]
pub enum EnvironmentFormMode {
    Create,
    Edit { env_id: SyncId },
}

/// Events emitted by UpdateEnvironmentForm.
#[derive(Debug, Clone)]
pub enum UpdateEnvironmentFormEvent {
    Created {
        environment: AmbientAgentEnvironment,
        share_with_team: bool,
    },
    Updated {
        env_id: SyncId,
        environment: AmbientAgentEnvironment,
    },
    DeleteRequested {
        env_id: SyncId,
    },
    Cancelled,
}

/// Actions handled by UpdateEnvironmentForm.
#[derive(Debug, Clone)]
pub enum UpdateEnvironmentFormAction {
    Submit,
    Delete,
    Cancel,
    Escape,
    FocusSetupCommandsInput,

    ToggleShareWithTeam,

    AddRepo,
    RemoveRepo(usize),
    ToggleReposDropdown,
    CloseReposDropdown,
    ToggleRepoSelection(usize),

    RemoveSetupCommand(usize),

    SuggestImage,
    LaunchAgentForSelectedRepos,
    RetryFetchGithubRepos,
    StartGithubAuth,
    OpenUrl(String),
}

/// State for the GitHub repos dropdown.
#[derive(Clone, Default)]
pub struct GithubReposDropdownState {
    pub available_repos: Vec<GithubRepo>,
    pub is_loading: bool,
    pub is_expanded: bool,
    pub auth_url: Option<String>,
    pub auth_fetched_at: Option<Instant>,
    pub load_error_message: Option<String>,
    pub selected_index: Option<usize>,
    app_install_link: Option<String>,
    repo_row_mouse_states: Vec<MouseStateHandle>,
    scroll_state: ClippedScrollStateHandle,
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug)]
enum CachedSuggestImageResult {
    Success {
        image: String,
        needs_custom_image: bool,
        reason: String,
    },
    AuthRequired {
        auth_url: String,
    },
}

#[derive(Clone, Debug)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
enum SuggestImageState {
    Idle,
    Loading {
        key: String,
    },
    Success {
        key: String,
        needs_custom_image: bool,
        reason: String,
    },
    AuthRequired {
        key: String,
        auth_url: String,
    },
    Error {
        key: String,
        message: String,
    },
}

pub struct UpdateEnvironmentForm {
    mode: EnvironmentFormMode,
    form_state: EnvironmentFormValues,
    repos_input: String,
    github_auth_redirect_target: GithubAuthRedirectTarget,

    // Editor views
    name_editor: ViewHandle<EditorView>,
    description_editor: ViewHandle<EditorView>,
    docker_image_editor: ViewHandle<EditorView>,
    repos_input_editor: ViewHandle<EditorView>,

    // Setup commands (multi-input)
    setup_commands_input: ViewHandle<SubmittableTextInput>,
    remove_setup_command_mouse_states: Vec<MouseStateHandle>,

    // Action buttons
    submit_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    back_button_mouse_state: MouseStateHandle,

    // Share-with-team checkbox (Create mode only, when user is on a team)
    share_with_team: bool,
    share_with_team_checkbox_mouse_state: MouseStateHandle,
    share_with_team_label_mouse_state: MouseStateHandle,

    // GitHub repos dropdown
    github_dropdown_state: GithubReposDropdownState,
    dropdown_input_mouse_state: MouseStateHandle,

    // Repo management
    remove_repo_mouse_states: Vec<MouseStateHandle>,
    add_repo_button_mouse_state: MouseStateHandle,
    auth_button_mouse_state: MouseStateHandle,
    retry_fetch_github_repos_mouse_state: MouseStateHandle,
    #[cfg(not(target_family = "wasm"))]
    configure_access_link_mouse_state: MouseStateHandle,
    refresh_repos_button_mouse_state: MouseStateHandle,

    // Suggest image state
    suggest_image_state: SuggestImageState,
    #[cfg(not(target_family = "wasm"))]
    suggest_image_cache: HashMap<String, CachedSuggestImageResult>,
    suggest_image_last_attempt_key: Option<String>,
    suggest_image_request_seq: u64,
    suggest_image_button_mouse_state: MouseStateHandle,
    suggest_image_auth_button_mouse_state: MouseStateHandle,
    suggest_image_launch_agent_button_mouse_state: MouseStateHandle,
    image_link_button_mouse_state: MouseStateHandle,

    /// On the edit page, we keep the suggest-image button disabled until repos have been modified
    /// at least once during the current edit session. Once enabled, it stays enabled even if the
    /// user reverts the repo selection.
    edit_repos_modified: bool,

    /// When true (default), renders the header with back button, title, and submit button.
    /// When false, skips the header and renders the submit button at the bottom-right of the form.
    show_header: bool,

    /// When true, pressing Escape in any editor will emit a Cancelled event.
    /// This should only be enabled for contexts where the form is used as a modal (e.g., first-time setup).
    should_handle_escape_from_editor: bool,

    /// Indicates where the GitHub authorization flow was initiated from.
    /// Affects the redirect URL used after auth completes.
    auth_source: AuthSource,
}

const DESCRIPTION_MAX_CHARS: usize = 240;
const REPOS_PLACEHOLDER_AUTHED: &str = "Enter repos (owner/repo format)";
const REPOS_PLACEHOLDER_UNAUTHED: &str = "Paste repo URL(s)";
const FORM_FIELD_SPACING: f32 = 20.;
const FORM_LABEL_SPACING: f32 = 6.;
const FORM_INPUT_HEIGHT: f32 = 36.;
const FORM_INPUT_HORIZONTAL_PADDING: f32 = 10.;
const AUTH_URL_REFRESH_THRESHOLD: Duration = Duration::from_secs(10 * 60);
const FORM_DESCRIPTION_HEIGHT: f32 = 72.;
const FORM_DESCRIPTION_VERTICAL_PADDING: f32 = 6.;
const CARD_BORDER_WIDTH: f32 = 1.;
const REPO_CHIP_MAX_WIDTH: f32 = 200.;
const DROPDOWN_MAX_WIDTH: f32 = 800.;
const DROPDOWN_MAX_HEIGHT: f32 = 300.;
const REPOS_DROPDOWN_ANCHOR: &str = "repos_dropdown_anchor";
const HEADER_VERTICAL_LAYOUT_THRESHOLD: f32 = 520.;

#[derive(Clone, Copy)]
enum RepoDropdownSelectionDirection {
    Up,
    Down,
}

impl UpdateEnvironmentForm {
    pub fn new(init_args: EnvironmentFormInitArgs, ctx: &mut ViewContext<Self>) -> Self {
        Self::new_impl(init_args, true, ctx)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        init_args: EnvironmentFormInitArgs,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_impl(init_args, false, ctx)
    }

    fn new_impl(
        init_args: EnvironmentFormInitArgs,
        fetch_github_repos_on_init: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&Appearance::handle(ctx), |form, _, _, ctx| {
            form.update_editor_text_colors(ctx);
        });
        // Create editors
        let name_editor = Self::create_single_line_editor("Environment name", ctx);
        let description_editor = Self::create_description_editor(ctx);
        let docker_image_editor =
            Self::create_single_line_editor("e.g. python:3.11, node:20-alpine", ctx);
        let repos_input_editor = Self::create_single_line_editor(REPOS_PLACEHOLDER_AUTHED, ctx);

        let setup_commands_input = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx);
            input.set_placeholder_text("e.g. cd my-repo && pip install -r requirements.txt", ctx);
            // Keep this consistent with other form inputs (e.g. repos): caller controls spacing.
            input.set_outer_margins(0., 0., ctx);
            input
        });

        ctx.subscribe_to_view(&setup_commands_input, |me, _, event, ctx| match event {
            SubmittableTextInputEvent::Submit(command) => {
                me.form_state.setup_commands.push(command.clone());
                me.remove_setup_command_mouse_states
                    .push(MouseStateHandle::default());
                ctx.notify();
            }
            SubmittableTextInputEvent::Escape => {
                me.handle_action(&UpdateEnvironmentFormAction::Escape, ctx);
            }
        });

        let setup_commands_input_editor = setup_commands_input.as_ref(ctx).editor().clone();
        setup_commands_input_editor.update(ctx, |editor, _ctx| {
            editor.set_propagate_vertical_navigation_keys(PropagateAndNoOpNavigationKeys::Always);
        });

        ctx.subscribe_to_view(
            &setup_commands_input_editor,
            |me, _, event, ctx| match event {
                crate::editor::Event::Navigate(NavigationKey::Tab) => {
                    if me.form_state.is_valid() {
                        ctx.focus(&me.submit_button);
                    } else {
                        // Wrap around to name (first field)
                        ctx.focus(&me.name_editor);
                    }
                }
                crate::editor::Event::Navigate(NavigationKey::ShiftTab) => {
                    ctx.focus(&me.docker_image_editor);
                }
                crate::editor::Event::Escape => {
                    me.handle_action(&UpdateEnvironmentFormAction::Escape, ctx);
                }
                _ => {}
            },
        );

        // Create buttons
        let submit_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Create", PrimaryTheme)
                .with_icon(Icon::Check)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(UpdateEnvironmentFormAction::Submit);
                })
        });

        let delete_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Delete environment", DangerSecondaryTheme)
                .with_icon(Icon::Trash)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(UpdateEnvironmentFormAction::Delete);
                })
        });

        // Set up editor subscriptions
        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| match event {
            crate::editor::Event::Edited(_) => {
                me.form_state.name = me.name_editor.as_ref(ctx).buffer_text(ctx);
                me.update_button_state(ctx);
            }
            crate::editor::Event::Navigate(NavigationKey::Tab) => {
                ctx.focus(&me.description_editor);
            }
            crate::editor::Event::Navigate(NavigationKey::ShiftTab) => {
                // Wrap around to setup commands (last field)
                ctx.focus(&me.setup_commands_input);
            }
            crate::editor::Event::Escape => {
                me.handle_action(&UpdateEnvironmentFormAction::Escape, ctx);
            }
            _ => {}
        });

        ctx.subscribe_to_view(&description_editor, |me, _, event, ctx| match event {
            crate::editor::Event::Edited(_) => {
                me.form_state.description = me.description_editor.as_ref(ctx).buffer_text(ctx);
                ctx.notify();
            }
            crate::editor::Event::Navigate(NavigationKey::Tab) => {
                ctx.focus(&me.repos_input_editor);
            }
            crate::editor::Event::Navigate(NavigationKey::ShiftTab) => {
                ctx.focus(&me.name_editor);
            }
            crate::editor::Event::Escape => {
                me.handle_action(&UpdateEnvironmentFormAction::Escape, ctx);
            }
            _ => {}
        });

        ctx.subscribe_to_view(&docker_image_editor, |me, _, event, ctx| match event {
            crate::editor::Event::Edited(_) => {
                me.form_state.docker_image = me.docker_image_editor.as_ref(ctx).buffer_text(ctx);
                me.update_button_state(ctx);
            }
            crate::editor::Event::Navigate(NavigationKey::Tab) => {
                ctx.focus(&me.setup_commands_input);
            }
            crate::editor::Event::Navigate(NavigationKey::ShiftTab) => {
                ctx.focus(&me.repos_input_editor);
            }
            crate::editor::Event::Escape => {
                me.handle_action(&UpdateEnvironmentFormAction::Escape, ctx);
            }
            _ => {}
        });

        ctx.subscribe_to_view(&repos_input_editor, |me, _, event, ctx| match event {
            crate::editor::Event::Edited(origin) => {
                me.repos_input = me.repos_input_editor.as_ref(ctx).buffer_text(ctx);

                if origin.is_user() {
                    let was_expanded = me.github_dropdown_state.is_expanded;
                    me.github_dropdown_state.is_expanded = true;
                    if !was_expanded {
                        me.github_dropdown_state.scroll_state = ClippedScrollStateHandle::default();
                    }
                }

                if me.github_dropdown_state.is_expanded {
                    me.ensure_repo_dropdown_selection();
                    me.scroll_repo_dropdown_selection_into_view();
                }
                ctx.notify();
            }
            crate::editor::Event::Blurred => {
                me.clear_invalid_repos_input_on_blur(ctx);
            }
            crate::editor::Event::Enter
            | crate::editor::Event::CmdEnter
            | crate::editor::Event::ShiftEnter
            | crate::editor::Event::AltEnter => {
                me.handle_action(&UpdateEnvironmentFormAction::AddRepo, ctx);
            }
            crate::editor::Event::BackspaceOnEmptyBuffer => {
                if !me.form_state.selected_repos.is_empty() {
                    let last_index = me.form_state.selected_repos.len() - 1;
                    me.form_state.selected_repos.pop();
                    if last_index < me.remove_repo_mouse_states.len() {
                        me.remove_repo_mouse_states.pop();
                    }
                    if me.is_edit_mode() {
                        me.edit_repos_modified = true;
                    }
                    ctx.notify();
                }
            }
            crate::editor::Event::Navigate(NavigationKey::Down) => {
                if me.github_dropdown_state.is_expanded {
                    me.move_repo_dropdown_selection(RepoDropdownSelectionDirection::Down, ctx);
                }
            }
            crate::editor::Event::Navigate(NavigationKey::Up) => {
                if me.github_dropdown_state.is_expanded {
                    me.move_repo_dropdown_selection(RepoDropdownSelectionDirection::Up, ctx);
                }
            }
            crate::editor::Event::Navigate(NavigationKey::Tab) => {
                ctx.focus(&me.docker_image_editor);
            }
            crate::editor::Event::Navigate(NavigationKey::ShiftTab) => {
                ctx.focus(&me.description_editor);
            }
            crate::editor::Event::Escape => {
                me.handle_action(&UpdateEnvironmentFormAction::Escape, ctx);
            }
            _ => {}
        });

        let mode = match &init_args {
            EnvironmentFormInitArgs::Create => EnvironmentFormMode::Create,
            EnvironmentFormInitArgs::Edit { env_id, .. } => {
                EnvironmentFormMode::Edit { env_id: *env_id }
            }
        };

        // Subscribe to GitHubAuthNotifier to refetch repos when auth completes
        ctx.subscribe_to_model(&GitHubAuthNotifier::handle(ctx), |me, _, event, ctx| {
            if matches!(event, GitHubAuthEvent::AuthCompleted) {
                me.fetch_github_repos(ctx);
            }
        });

        let mut form = Self {
            mode,
            form_state: EnvironmentFormValues::default(),
            repos_input: String::new(),
            github_auth_redirect_target: GithubAuthRedirectTarget::SettingsEnvironments,
            name_editor,
            description_editor,
            docker_image_editor,
            repos_input_editor,
            setup_commands_input,
            remove_setup_command_mouse_states: Vec::new(),
            submit_button,
            delete_button,
            back_button_mouse_state: MouseStateHandle::default(),
            share_with_team: false,
            share_with_team_checkbox_mouse_state: MouseStateHandle::default(),
            share_with_team_label_mouse_state: MouseStateHandle::default(),
            github_dropdown_state: GithubReposDropdownState::default(),
            dropdown_input_mouse_state: MouseStateHandle::default(),
            remove_repo_mouse_states: Vec::new(),
            add_repo_button_mouse_state: MouseStateHandle::default(),
            auth_button_mouse_state: MouseStateHandle::default(),
            retry_fetch_github_repos_mouse_state: MouseStateHandle::default(),
            #[cfg(not(target_family = "wasm"))]
            configure_access_link_mouse_state: MouseStateHandle::default(),
            refresh_repos_button_mouse_state: MouseStateHandle::default(),
            suggest_image_state: SuggestImageState::Idle,
            #[cfg(not(target_family = "wasm"))]
            suggest_image_cache: HashMap::new(),
            suggest_image_last_attempt_key: None,
            suggest_image_request_seq: 0,
            suggest_image_button_mouse_state: MouseStateHandle::default(),
            suggest_image_auth_button_mouse_state: MouseStateHandle::default(),
            suggest_image_launch_agent_button_mouse_state: MouseStateHandle::default(),
            image_link_button_mouse_state: MouseStateHandle::default(),
            edit_repos_modified: false,
            show_header: true,
            should_handle_escape_from_editor: false,
            auth_source: AuthSource::default(),
        };

        // Initialize based on init args
        form.apply_mode(&init_args, ctx);
        form.update_button_state(ctx);

        if fetch_github_repos_on_init {
            // Fetch GitHub repos for dropdown
            form.fetch_github_repos(ctx);
        }
        form.update_editor_text_colors(ctx);

        form
    }

    pub fn github_dropdown_state(&self) -> &GithubReposDropdownState {
        &self.github_dropdown_state
    }

    pub fn set_github_auth_redirect_target(&mut self, target: GithubAuthRedirectTarget) {
        self.github_auth_redirect_target = target;
    }

    #[cfg(test)]
    pub(crate) fn github_auth_redirect_target_for_test(&self) -> GithubAuthRedirectTarget {
        self.github_auth_redirect_target
    }

    fn try_close_repos_dropdown(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !self.github_dropdown_state.is_expanded {
            return false;
        }

        self.github_dropdown_state.is_expanded = false;
        self.github_dropdown_state.selected_index = None;
        ctx.notify();
        true
    }

    fn repo_dropdown_row_position_id(index: usize) -> String {
        format!("repos_dropdown_row_{index}")
    }

    fn scroll_repo_dropdown_selection_into_view(&mut self) {
        if !self.github_dropdown_state.is_expanded {
            return;
        }

        let Some(index) = self.github_dropdown_state.selected_index else {
            return;
        };

        self.github_dropdown_state
            .scroll_state
            .scroll_to_position(ScrollTarget {
                position_id: Self::repo_dropdown_row_position_id(index),
                mode: ScrollToPositionMode::FullyIntoView,
            });
    }

    pub fn set_mode(&mut self, init_args: EnvironmentFormInitArgs, ctx: &mut ViewContext<Self>) {
        self.mode = match &init_args {
            EnvironmentFormInitArgs::Create => EnvironmentFormMode::Create,
            EnvironmentFormInitArgs::Edit { env_id, .. } => {
                EnvironmentFormMode::Edit { env_id: *env_id }
            }
        };
        self.apply_mode(&init_args, ctx);
        self.update_button_state(ctx);
        ctx.notify();
    }

    /// Sets whether the header (back button, title, submit button) should be shown.
    /// When `false`, the submit button is rendered at the bottom-right of the form instead.
    pub fn set_show_header(&mut self, show_header: bool, ctx: &mut ViewContext<Self>) {
        self.show_header = show_header;

        // Update button text based on mode when header is hidden
        if !show_header {
            let button_text = match &self.mode {
                EnvironmentFormMode::Create => "Create environment",
                EnvironmentFormMode::Edit { .. } => "Save environment",
            };
            self.submit_button.update(ctx, |button, ctx| {
                button.set_label(button_text, ctx);
            });
        }

        ctx.notify();
    }

    /// Sets whether Escape keypresses in editors should trigger the Cancel action.
    /// This should only be enabled for modal contexts (e.g., first-time setup).
    pub fn set_should_handle_escape_from_editor(&mut self, should_handle: bool) {
        self.should_handle_escape_from_editor = should_handle;
    }

    /// Sets the auth source, which affects the redirect URL used after GitHub auth completes.
    /// When set to `CloudSetup`, the redirect URL will include a source parameter that tells
    /// the URI handler to skip opening the settings page.
    pub fn set_auth_source(&mut self, source: AuthSource) {
        self.auth_source = source;
    }

    /// Focus the Name editor (the first field in the form).
    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
    }

    fn apply_mode(&mut self, init_args: &EnvironmentFormInitArgs, ctx: &mut ViewContext<Self>) {
        match init_args {
            EnvironmentFormInitArgs::Create => {
                // Clear form
                self.form_state = EnvironmentFormValues::default();
                self.share_with_team = UserWorkspaces::as_ref(ctx).current_team_uid().is_some();
                self.name_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                self.description_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                self.docker_image_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                self.repos_input_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                let editor = { self.setup_commands_input.as_ref(ctx).editor().clone() };
                editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                self.remove_repo_mouse_states.clear();
                self.remove_setup_command_mouse_states.clear();
                // Update button text for Create mode
                self.submit_button.update(ctx, |button, ctx| {
                    button.set_label("Create", ctx);
                });
            }
            EnvironmentFormInitArgs::Edit {
                env_id: _,
                initial_values,
            } => {
                self.share_with_team = false;

                // Populate form with initial values
                self.form_state = initial_values.as_ref().clone();
                self.name_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&initial_values.name, ctx);
                });
                self.description_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&initial_values.description, ctx);
                });
                self.docker_image_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&initial_values.docker_image, ctx);
                });
                self.repos_input_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                let editor = { self.setup_commands_input.as_ref(ctx).editor().clone() };
                editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                self.remove_repo_mouse_states = self
                    .form_state
                    .selected_repos
                    .iter()
                    .map(|_| MouseStateHandle::default())
                    .collect();
                self.remove_setup_command_mouse_states = self
                    .form_state
                    .setup_commands
                    .iter()
                    .map(|_| MouseStateHandle::default())
                    .collect();
                // Update button text for Edit mode
                self.submit_button.update(ctx, |button, ctx| {
                    button.set_label("Save", ctx);
                });
            }
        }

        // Reset suggest image state for this session.
        //
        // Note: We intentionally do not set `suggest_image_last_attempt_key` here.
        // That field tracks the last *attempted* suggest-image key, and is used to enforce
        // “repos must change to retry”. Edit-mode gating is handled via `edit_repos_modified`.
        self.suggest_image_state = SuggestImageState::Idle;
        self.suggest_image_last_attempt_key = None;
        self.suggest_image_request_seq = 0;

        // Track that repos haven't been modified yet in this edit session.
        // The suggest button will be disabled until the user modifies repos.
        self.edit_repos_modified = false;
    }

    fn update_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        let is_valid = self.form_state.is_valid();
        self.submit_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_valid, ctx);
        });
    }

    fn is_edit_mode(&self) -> bool {
        matches!(self.mode, EnvironmentFormMode::Edit { .. })
    }

    fn update_repos_input_placeholder(&mut self, ctx: &mut ViewContext<Self>) {
        let placeholder = if self.github_dropdown_state.auth_url.is_some() {
            REPOS_PLACEHOLDER_UNAUTHED
        } else {
            REPOS_PLACEHOLDER_AUTHED
        };
        self.repos_input_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(placeholder, ctx);
        });
    }

    fn selected_repos_as_remote_repo_args(&self) -> Vec<String> {
        self.form_state
            .selected_repos
            .iter()
            .map(|repo| format!("{}/{}", repo.owner.trim(), repo.repo.trim()))
            .collect()
    }

    fn create_single_line_editor(
        placeholder: &'static str,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<EditorView> {
        ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.ui_font_family()),
                    text_colors_override: Some(editor_text_colors(appearance)),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(placeholder, ctx);
            editor
        })
    }

    fn update_editor_text_colors(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let text_colors = editor_text_colors(appearance);

        for editor in [
            &self.name_editor,
            &self.description_editor,
            &self.docker_image_editor,
            &self.repos_input_editor,
        ] {
            let colors = text_colors.clone();
            editor.update(ctx, move |editor, ctx| {
                editor.set_text_colors(colors, ctx);
            });
        }

        let colors = text_colors.clone();
        let editor = self.setup_commands_input.as_ref(ctx).editor().clone();
        editor.update(ctx, move |editor, ctx| {
            editor.set_text_colors(colors, ctx);
        });
    }

    fn create_description_editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.ui_font_family()),
                    text_colors_override: Some(editor_text_colors(appearance)),
                    ..Default::default()
                },
                autogrow: true,
                soft_wrap: true,
                single_line: false,
                max_buffer_len: Some(DESCRIPTION_MAX_CHARS),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::new(options, ctx);
            editor.set_placeholder_text(
                "e.g., this environment is for all front end focused agents",
                ctx,
            );
            editor
        })
    }

    fn parse_repo_input(input: &str) -> Option<(String, String)> {
        use url::Url;
        let trimmed = input.trim().trim_end_matches('/');

        fn parse_owner_repo<'a, I>(mut segments: I) -> Option<(String, String)>
        where
            I: Iterator<Item = &'a str>,
        {
            let owner = segments.next()?.trim();
            let repo = segments.next()?.trim();
            let repo = repo.trim_end_matches(".git");
            if owner.is_empty() || repo.is_empty() {
                return None;
            }
            Some((owner.to_string(), repo.to_string()))
        }

        if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
            return parse_owner_repo(rest.split('/').filter(|p| !p.is_empty()));
        }

        let parsed_url = Url::parse(trimmed).or_else(|_| {
            if trimmed.starts_with("github.com/") || trimmed.starts_with("www.github.com/") {
                Url::parse(&format!("https://{trimmed}"))
            } else {
                Err(url::ParseError::RelativeUrlWithoutBase)
            }
        });

        if let Ok(url) = parsed_url {
            if matches!(url.host_str(), Some("github.com" | "www.github.com")) {
                return parse_owner_repo(url.path_segments()?.filter(|p| !p.is_empty()));
            }
        }

        parse_owner_repo(trimmed.split('/').filter(|p| !p.is_empty()))
    }

    fn parse_repo_inputs(input: &str) -> Vec<(String, String)> {
        // Accept comma- and whitespace-separated entries to support pasting multiple repos at once.
        input
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|part| {
                let trimmed = part.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Self::parse_repo_input(trimmed)
                }
            })
            .collect()
    }

    /// Clears the repos input editor and the mirrored `repos_input` string.
    ///
    /// This is used after successfully adding/toggling a repo, and also for cleanup cases
    /// where we intentionally discard unsubmitted text.
    fn clear_repos_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.repos_input.clear();
        self.repos_input_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
    }

    /// On blur, clear any *unsubmitted* text that doesn't parse as `owner/repo` (or supported URL forms).
    ///
    /// Important: this must not remove already-added repo chips; it only clears the text the user typed.
    fn clear_invalid_repos_input_on_blur(&mut self, ctx: &mut ViewContext<Self>) {
        let input = self.repos_input_editor.as_ref(ctx).buffer_text(ctx);
        if input.trim().is_empty() {
            return;
        }

        if Self::parse_repo_inputs(&input).is_empty() {
            self.clear_repos_input(ctx);
            ctx.notify();
        }
    }

    fn filtered_repo_indices(&self) -> Vec<usize> {
        // Return indices of available repos that match the current input filter.
        let search_text = self.repos_input.to_lowercase();
        self.github_dropdown_state
            .available_repos
            .iter()
            .enumerate()
            .filter(|(_, repo)| {
                if search_text.is_empty() {
                    true
                } else {
                    let repo_str = format!("{}/{}", repo.owner, repo.repo).to_lowercase();
                    repo_str.contains(&search_text)
                }
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn ensure_repo_dropdown_selection(&mut self) {
        // Ensure the dropdown selection points at a visible (filtered) repo.
        let filtered_indices = self.filtered_repo_indices();
        if filtered_indices.is_empty() {
            self.github_dropdown_state.selected_index = None;
            return;
        }

        let selected_index = self.github_dropdown_state.selected_index;
        let is_selected_visible = selected_index
            .and_then(|index| filtered_indices.iter().position(|value| *value == index))
            .is_some();

        if !is_selected_visible {
            self.github_dropdown_state.selected_index = Some(filtered_indices[0]);
        }
    }

    fn move_repo_dropdown_selection(
        &mut self,
        direction: RepoDropdownSelectionDirection,
        ctx: &mut ViewContext<Self>,
    ) {
        // Move the dropdown highlight up/down, wrapping within the filtered list.
        let filtered_indices = self.filtered_repo_indices();
        if filtered_indices.is_empty() {
            self.github_dropdown_state.selected_index = None;
            ctx.notify();
            return;
        }

        let next_index = match (self.github_dropdown_state.selected_index, direction) {
            (None, RepoDropdownSelectionDirection::Down) => filtered_indices[0],
            (None, RepoDropdownSelectionDirection::Up) => {
                *filtered_indices.last().expect("filtered indices non-empty")
            }
            (Some(current), RepoDropdownSelectionDirection::Down) => {
                let current_pos = filtered_indices
                    .iter()
                    .position(|index| *index == current)
                    .unwrap_or(0);
                let next_pos = if current_pos + 1 >= filtered_indices.len() {
                    0
                } else {
                    current_pos + 1
                };
                filtered_indices[next_pos]
            }
            (Some(current), RepoDropdownSelectionDirection::Up) => {
                let current_pos = filtered_indices
                    .iter()
                    .position(|index| *index == current)
                    .unwrap_or(0);
                let next_pos = if current_pos == 0 {
                    filtered_indices.len() - 1
                } else {
                    current_pos - 1
                };
                filtered_indices[next_pos]
            }
        };

        self.github_dropdown_state.selected_index = Some(next_index);
        self.scroll_repo_dropdown_selection_into_view();
        ctx.notify();
    }

    fn toggle_repo_selection_at_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        // Toggle the repo chip for the given available repo index.
        let Some(available_repo) = self.github_dropdown_state.available_repos.get(index) else {
            return;
        };
        let owner = available_repo.owner.clone();
        let repo = available_repo.repo.clone();

        let existing_index = self
            .form_state
            .selected_repos
            .iter()
            .position(|r| r.owner == owner && r.repo == repo);

        if let Some(idx) = existing_index {
            self.form_state.selected_repos.remove(idx);
            self.remove_repo_mouse_states.remove(idx);
        } else {
            self.form_state
                .selected_repos
                .push(GithubRepo::new(owner, repo));
            self.remove_repo_mouse_states
                .push(MouseStateHandle::default());
            self.clear_repos_input(ctx);
        }

        if self.is_edit_mode() {
            self.edit_repos_modified = true;
        }
        ctx.notify();
    }

    pub fn start_github_auth(&mut self, ctx: &mut ViewContext<Self>) {
        if self.github_dropdown_state.is_loading {
            return;
        }
        if self.should_refresh_auth_url() {
            if let Some(elapsed) = self
                .github_dropdown_state
                .auth_fetched_at
                .map(|fetched_at| fetched_at.elapsed())
            {
                debug!(
                    "Refreshing GitHub auth URL after {:.0}s (threshold {:.0}s)",
                    elapsed.as_secs_f64(),
                    AUTH_URL_REFRESH_THRESHOLD.as_secs_f64()
                );
            } else {
                debug!("Refreshing GitHub auth URL (no previous fetch timestamp)");
            }
            self.fetch_github_repos_for_auth(ctx);
        } else {
            self.open_github_auth_url_or_fallback(ctx);
        }
    }

    fn should_refresh_auth_url(&self) -> bool {
        match self.github_dropdown_state.auth_fetched_at {
            Some(fetched_at) => fetched_at.elapsed() >= AUTH_URL_REFRESH_THRESHOLD,
            // No timestamp means the age is unknown — treat as stale to be safe.
            None => self.github_dropdown_state.auth_url.is_some(),
        }
    }

    fn open_github_auth_url_or_fallback(&self, ctx: &mut ViewContext<Self>) {
        let url = self
            .github_dropdown_state
            .auth_url
            .as_deref()
            .map(|auth_url| self.auth_url_with_next(auth_url))
            .unwrap_or_else(|| self.github_connect_fallback_url());
        ctx.open_url(&url);
    }

    fn github_connect_fallback_url(&self) -> String {
        let base_url = format!("{}/oauth/connect/github", ChannelState::server_root_url());
        self.auth_url_with_next(&base_url)
    }

    fn extract_tx_id(auth_url: &str) -> Option<String> {
        let parsed = Url::parse(auth_url).ok()?;
        parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "txId").then(|| value.to_string()))
    }

    fn fetch_github_repos_for_auth(&mut self, ctx: &mut ViewContext<Self>) {
        self.fetch_github_repos_internal(ctx, true);
    }

    /// Fetch GitHub repos for the dropdown.
    pub fn fetch_github_repos(&mut self, ctx: &mut ViewContext<Self>) {
        self.fetch_github_repos_internal(ctx, false);
    }

    fn fetch_github_repos_internal(
        &mut self,
        ctx: &mut ViewContext<Self>,
        open_auth_after_fetch: bool,
    ) {
        self.github_dropdown_state.is_loading = true;
        self.github_dropdown_state.load_error_message = None;
        self.github_dropdown_state.auth_url = None;
        self.github_dropdown_state.auth_fetched_at = None;
        ctx.notify();

        let integrations_client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();

        ctx.spawn(
            async move { integrations_client.get_user_github_info().await },
            move |me, result, ctx| {
                me.github_dropdown_state.is_loading = false;
                let mut should_open_auth = open_auth_after_fetch;

                match result {
                    Ok(UserGithubInfoResult::GithubConnectedOutput(info)) => {
                        me.github_dropdown_state.available_repos = info
                            .installed_repos
                            .into_iter()
                            .map(|r| GithubRepo::new(r.owner, r.repo))
                            .collect();
                        me.github_dropdown_state.repo_row_mouse_states = me
                            .github_dropdown_state
                            .available_repos
                            .iter()
                            .map(|_| MouseStateHandle::default())
                            .collect();
                        me.github_dropdown_state.scroll_state = ClippedScrollStateHandle::default();
                        me.github_dropdown_state.selected_index = None;
                        me.ensure_repo_dropdown_selection();
                        me.scroll_repo_dropdown_selection_into_view();
                        // Store appInstallLink even when authenticated - it's used for "Configure access" link
                        me.github_dropdown_state.app_install_link = Some(info.app_install_link);
                        me.github_dropdown_state.auth_url = None;
                        me.github_dropdown_state.auth_fetched_at = None;
                        should_open_auth = false;
                        me.update_repos_input_placeholder(ctx);
                    }
                    Ok(UserGithubInfoResult::GithubAuthRequiredOutput(auth_info)) => {
                        me.github_dropdown_state.auth_url = Some(auth_info.auth_url);
                        me.github_dropdown_state.auth_fetched_at = Some(Instant::now());
                        me.github_dropdown_state.app_install_link =
                            Some(auth_info.app_install_link);
                        if open_auth_after_fetch {
                            if let Some(auth_url) = me.github_dropdown_state.auth_url.as_deref() {
                                if let Some(tx_id) = Self::extract_tx_id(auth_url) {
                                    debug!("Refetched GitHub auth URL with tx_id={tx_id}");
                                } else {
                                    debug!("Refetched GitHub auth URL (tx_id missing)");
                                }
                            }
                        }
                        me.update_repos_input_placeholder(ctx);
                    }
                    Ok(UserGithubInfoResult::Unknown) => {
                        me.github_dropdown_state.load_error_message =
                            Some("Failed to load GitHub repos".to_string());
                    }
                    Err(e) => {
                        me.github_dropdown_state.load_error_message =
                            Some(format!("Failed to load GitHub repos: {}", e));
                    }
                }

                if should_open_auth {
                    if let Some(auth_url) = me.github_dropdown_state.auth_url.as_deref() {
                        let auth_url = me.auth_url_with_next(auth_url);
                        ctx.open_url(&auth_url);
                    } else if me.github_dropdown_state.available_repos.is_empty() {
                        let fallback_url = me.github_connect_fallback_url();
                        ctx.open_url(&fallback_url);
                    }
                }
                ctx.notify();
            },
        );
    }

    /// Generate a cache key from selected repos for suggest image.
    fn selected_repos_key(&self) -> Option<String> {
        if self.form_state.selected_repos.is_empty() {
            return None;
        }

        let mut repos = self
            .form_state
            .selected_repos
            .iter()
            .map(|r| {
                format!(
                    "{}/{}",
                    r.owner.trim().to_lowercase(),
                    r.repo.trim().to_lowercase()
                )
            })
            .collect::<Vec<_>>();
        repos.sort();
        Some(repos.join("\n"))
    }

    #[cfg(not(target_family = "wasm"))]
    fn apply_cached_suggest_image_result(
        &mut self,
        key: &str,
        cached: CachedSuggestImageResult,
        ctx: &mut ViewContext<Self>,
    ) {
        match cached {
            CachedSuggestImageResult::Success {
                image,
                needs_custom_image,
                reason,
            } => {
                self.apply_suggest_image_success(
                    key.to_string(),
                    image,
                    needs_custom_image,
                    reason,
                    ctx,
                );
            }
            CachedSuggestImageResult::AuthRequired { auth_url } => {
                self.suggest_image_state = SuggestImageState::AuthRequired {
                    key: key.to_string(),
                    auth_url,
                };
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn apply_suggest_image_success(
        &mut self,
        key: String,
        image: String,
        needs_custom_image: bool,
        reason: String,
        ctx: &mut ViewContext<Self>,
    ) {
        // Cache for later
        self.suggest_image_cache.insert(
            key.clone(),
            CachedSuggestImageResult::Success {
                image: image.clone(),
                needs_custom_image,
                reason: reason.clone(),
            },
        );

        // Only update the input if the request key still matches the current repo selection
        if self.selected_repos_key().as_deref() == Some(&key) {
            self.form_state.docker_image = image.clone();
            self.docker_image_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&image, ctx);
            });
            self.update_button_state(ctx);
        }

        self.suggest_image_state = SuggestImageState::Success {
            key,
            needs_custom_image,
            reason,
        };

        send_telemetry_from_ctx!(
            CloudAgentTelemetryEvent::ImageSuggested {
                image,
                needs_custom_image,
            },
            ctx
        );
    }

    #[cfg(not(target_family = "wasm"))]
    fn suggest_image(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(key) = self.selected_repos_key() else {
            return;
        };

        // Don't start a new request if we're already loading for this key
        let is_generating = matches!(&self.suggest_image_state, SuggestImageState::Loading { key: loading_key } if loading_key == &key);
        if is_generating {
            return;
        }

        if self.suggest_image_last_attempt_key.as_deref() == Some(&key) {
            return;
        }

        // Record the attempt immediately to enforce the "repos must change to retry" rule
        self.suggest_image_last_attempt_key = Some(key.clone());

        // If we have a cached result, apply it immediately
        if let Some(cached) = self.suggest_image_cache.get(&key).cloned() {
            self.apply_cached_suggest_image_result(&key, cached, ctx);
            ctx.notify();
            return;
        }

        self.suggest_image_request_seq = self.suggest_image_request_seq.saturating_add(1);
        let request_seq = self.suggest_image_request_seq;
        self.suggest_image_state = SuggestImageState::Loading { key: key.clone() };
        ctx.notify();

        let repos = self
            .form_state
            .selected_repos
            .iter()
            .map(|r| (r.owner.clone(), r.repo.clone()))
            .collect::<Vec<_>>();

        let integrations_client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();

        ctx.spawn(
            async move { integrations_client.suggest_cloud_environment_image(repos).await },
            move |me, result, ctx| {
                if me.suggest_image_request_seq != request_seq {
                    return;
                }

                match result {
                    Ok(result) => match result {
                        warp_graphql::queries::suggest_cloud_environment_image::SuggestCloudEnvironmentImageResult::SuggestCloudEnvironmentImageOutput(output) => {
                            let image = output.image;
                            let needs_custom_image = output.needs_custom_image;
                            let reason = output.reason;
                            me.apply_suggest_image_success(
                                key.clone(),
                                image,
                                needs_custom_image,
                                reason,
                                ctx,
                            );
                        }
                        warp_graphql::queries::suggest_cloud_environment_image::SuggestCloudEnvironmentImageResult::SuggestCloudEnvironmentImageAuthRequiredOutput(output) => {
                            me.suggest_image_cache.insert(
                                key.clone(),
                                CachedSuggestImageResult::AuthRequired {
                                    auth_url: output.auth_url.clone(),
                                },
                            );
                            me.suggest_image_state = SuggestImageState::AuthRequired {
                                key: key.clone(),
                                auth_url: output.auth_url,
                            };
                        }
                        warp_graphql::queries::suggest_cloud_environment_image::SuggestCloudEnvironmentImageResult::UserFacingError(_) => {
                            let error_message = "Failed to suggest a Docker image".to_string();
                            send_telemetry_from_ctx!(
                                CloudAgentTelemetryEvent::ImageSuggestionFailed {
                                    error: error_message.clone(),
                                },
                                ctx
                            );
                            me.suggest_image_state = SuggestImageState::Error {
                                key: key.clone(),
                                message: error_message,
                            };
                        }
                        warp_graphql::queries::suggest_cloud_environment_image::SuggestCloudEnvironmentImageResult::Unknown => {
                            let error_message = "Unknown response from suggestCloudEnvironmentImage".to_string();
                            send_telemetry_from_ctx!(
                                CloudAgentTelemetryEvent::ImageSuggestionFailed {
                                    error: error_message.clone(),
                                },
                                ctx
                            );
                            me.suggest_image_state = SuggestImageState::Error {
                                key: key.clone(),
                                message: error_message,
                            };
                        }
                    },
                    Err(e) => {
                        let error_message = format!("Failed to suggest a Docker image: {}", e);
                        send_telemetry_from_ctx!(
                            CloudAgentTelemetryEvent::ImageSuggestionFailed {
                                error: error_message.clone(),
                            },
                            ctx
                        );
                        me.suggest_image_state = SuggestImageState::Error {
                            key: key.clone(),
                            message: error_message,
                        };
                    }
                }
                ctx.notify();
            },
        );
    }

    #[cfg(target_family = "wasm")]
    fn suggest_image(&mut self, _ctx: &mut ViewContext<Self>) {
        // Not supported on WASM
    }

    fn should_show_share_with_team_checkbox(&self, app: &AppContext) -> bool {
        matches!(self.mode, EnvironmentFormMode::Create)
            && UserWorkspaces::as_ref(app).current_team_uid().is_some()
    }

    fn render_submit_actions(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        button_handle: &ViewHandle<ActionButton>,
    ) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(12.);

        if self.should_show_share_with_team_checkbox(app) {
            let theme = appearance.theme();
            let font_family = appearance.ui_font_family();
            let font_size = appearance.ui_font_size();

            let checkbox = appearance
                .ui_builder()
                .checkbox(self.share_with_team_checkbox_mouse_state.clone(), None)
                .check(self.share_with_team)
                .with_style(UiComponentStyles {
                    margin: Some(Coords::uniform(0.)),
                    ..Default::default()
                })
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(UpdateEnvironmentFormAction::ToggleShareWithTeam);
                })
                .finish();

            let label = Hoverable::new(
                self.share_with_team_label_mouse_state.clone(),
                move |state| {
                    let color = if state.is_mouse_over_element() {
                        theme.active_ui_text_color().with_opacity(200)
                    } else {
                        theme.active_ui_text_color()
                    };

                    Text::new_inline("Share with team", font_family, font_size)
                        .with_color(color.into())
                        .finish()
                },
            )
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(UpdateEnvironmentFormAction::ToggleShareWithTeam);
            })
            .finish();

            row.add_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(checkbox)
                    .with_child(label)
                    .finish(),
            );
        }

        row.add_child(ChildView::new(button_handle).finish());

        row.finish()
    }

    fn render_share_with_team_warning(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !self.should_show_share_with_team_checkbox(app) || self.share_with_team {
            return None;
        }

        Some(render_warning_box(
            WarningBoxConfig::new(
                "Personal environments cannot be used with external integrations or team API keys. For the best experience, use shared environments.",
            )
            .with_width(DROPDOWN_MAX_WIDTH),
            appearance,
        ))
    }

    fn render_back_button_and_title(
        &self,
        title: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let back_button = icon_button(
            appearance,
            Icon::ArrowLeft,
            false,
            self.back_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(UpdateEnvironmentFormAction::Cancel);
        })
        .finish();

        let title_text = Text::new(
            title.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size() * 1.5,
        )
        .with_style(Properties::default().weight(Weight::Bold))
        .with_color(theme.active_ui_text_color().into())
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(back_button)
            .with_child(title_text)
            .finish()
    }

    fn render_header(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let (title, button_handle) = match &self.mode {
            EnvironmentFormMode::Create => ("Create environment", &self.submit_button),
            EnvironmentFormMode::Edit { .. } => ("Edit environment", &self.submit_button),
        };

        let submit_actions = || self.render_submit_actions(appearance, app, button_handle);

        let horizontal_header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(self.render_back_button_and_title(title, appearance))
            .with_child(submit_actions())
            .finish();

        let compact_header = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(8.)
            .with_child(self.render_back_button_and_title(title, appearance))
            .with_child(submit_actions())
            .finish();

        SizeConstraintSwitch::new(
            horizontal_header,
            vec![(
                SizeConstraintCondition::WidthLessThan(HEADER_VERTICAL_LAYOUT_THRESHOLD),
                compact_header,
            )],
        )
        .finish()
    }

    fn render_form_label(
        label: &'static str,
        required: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);

        row.add_child(
            Text::new(
                label,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(theme.active_ui_text_color().into())
            .finish(),
        );

        if required {
            row.add_child(
                Text::new("*", appearance.ui_font_family(), appearance.ui_font_size())
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(theme.accent().into())
                    .finish(),
            );
        }

        row.finish()
    }

    fn render_form_field(
        label: &'static str,
        required: bool,
        helper_text: Option<&'static str>,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        field.add_child(Self::render_form_label(label, required, appearance));

        let editor_container = Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(
                        Clipped::new(
                            Container::new(ChildView::new(editor).finish())
                                .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(FORM_INPUT_HEIGHT)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        field.add_child(editor_container);

        if let Some(helper) = helper_text {
            field.add_child(
                Text::new(
                    helper,
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.85,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .finish(),
            );
        }

        field.finish()
    }

    fn render_setup_commands_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        field.add_child(Self::render_form_label(
            "Setup command(s)",
            false,
            appearance,
        ));

        let items = self
            .form_state
            .setup_commands
            .iter()
            .enumerate()
            .map(|(index, command)| InputListItem {
                item: command.clone(),
                mouse_state_handle: self
                    .remove_setup_command_mouse_states
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
                on_remove_action: UpdateEnvironmentFormAction::RemoveSetupCommand(index),
                is_disabled: false,
                tooltip_mouse_state: None,
            });

        let helper_text = Text::new(
            "Setup commands run independently. Each command runs from the workspace root (/workspace). If a command depends on the previous one, combine them with &&.",
            appearance.ui_font_family(),
            appearance.ui_font_size() * 0.85,
        )
        .soft_wrap(true)
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // We want the helper text close to the input itself (like the helper under the repo input),
        // not separated by the existing command list.
        let input_and_helper = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.)
            .with_child(ChildView::new(&self.setup_commands_input).finish())
            .with_child(helper_text)
            .finish();

        let list_items = render_input_list(None, items, None, appearance);

        let list = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(6.)
            .with_child(input_and_helper)
            .with_child(list_items)
            .finish();

        field.add_child(
            ConstrainedBox::new(Container::new(list).finish())
                .with_max_width(DROPDOWN_MAX_WIDTH)
                .finish(),
        );

        field.finish()
    }

    fn render_description_field(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        field.add_child(
            Text::new(
                "Description",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(theme.active_ui_text_color().into())
            .finish(),
        );

        let editor_container = ConstrainedBox::new(
            Container::new(
                Clipped::new(ChildView::new(&self.description_editor).finish()).finish(),
            )
            .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
            .with_vertical_padding(FORM_DESCRIPTION_VERTICAL_PADDING)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
            .with_background(theme.surface_2())
            .finish(),
        )
        .with_min_height(FORM_DESCRIPTION_HEIGHT)
        .finish();

        field.add_child(editor_container);

        // Character count display
        let char_count = self
            .description_editor
            .as_ref(app)
            .buffer_text(app)
            .chars()
            .count();
        let count_text = format!("{} / {} characters", char_count, DESCRIPTION_MAX_CHARS);
        field.add_child(
            Text::new(
                count_text,
                appearance.ui_font_family(),
                appearance.ui_font_size() * 0.85,
            )
            .with_color(theme.nonactive_ui_text_color().into())
            .finish(),
        );

        field.finish()
    }

    fn render_repos_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Route to appropriate rendering based on dropdown state
        if self.github_dropdown_state.is_loading {
            self.render_repos_field_loading(appearance)
        } else if self.github_dropdown_state.auth_url.is_some() {
            self.render_repos_field_unauthed(appearance)
        } else if self.github_dropdown_state.load_error_message.is_some() {
            self.render_repos_field_error(appearance)
        } else {
            self.render_repos_field_authed(appearance)
        }
    }

    fn render_repos_field_label(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Text::new(
            "Repo(s)",
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into())
        .finish()
    }

    fn render_repos_field_loading(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        field.add_child(self.render_repos_field_label(appearance));

        // Selected repo chips (if any)
        if !self.form_state.selected_repos.is_empty() {
            field.add_child(self.render_selected_repo_chips(appearance));
        }

        // Disabled input with loading placeholder
        let loading_input = Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(
                        Container::new(
                            Text::new(
                                "Loading...",
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(theme.disabled_ui_text_color().into())
                            .finish(),
                        )
                        .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(FORM_INPUT_HEIGHT)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        field.add_child(loading_input);
        field.finish()
    }

    fn render_repos_field_unauthed(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        // Label
        field.add_child(self.render_repos_field_label(appearance));

        // Selected repo chips (if any)
        if !self.form_state.selected_repos.is_empty() {
            field.add_child(self.render_selected_repo_chips(appearance));
        }

        // Input for pasting repo URLs manually
        let editor = Clipped::new(ChildView::new(&self.repos_input_editor).finish()).finish();

        let input_container = Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(
                        Clipped::new(
                            Container::new(editor)
                                .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(FORM_INPUT_HEIGHT)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        let auth_button = Hoverable::new(self.auth_button_mouse_state.clone(), move |state| {
            let bg = if state.is_mouse_over_element() {
                theme.surface_3()
            } else {
                theme.surface_2().with_opacity(45)
            };
            let icon_size = appearance.ui_font_size();

            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_spacing(6.)
                        .with_child(
                            ConstrainedBox::new(
                                Icon::Github
                                    .to_warpui_icon(theme.active_ui_text_color())
                                    .finish(),
                            )
                            .with_width(icon_size)
                            .with_height(icon_size)
                            .finish(),
                        )
                        .with_child(
                            Text::new(
                                "Auth with GitHub",
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .finish(),
                )
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
                .with_background(bg)
                .finish(),
            )
            .with_height(FORM_INPUT_HEIGHT)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(UpdateEnvironmentFormAction::StartGithubAuth);
        })
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(Expanded::new(1., input_container).finish())
            .with_child(auth_button)
            .finish();
        field.add_child(
            ConstrainedBox::new(row)
                .with_max_width(DROPDOWN_MAX_WIDTH)
                .finish(),
        );
        field.finish()
    }

    fn render_repos_field_error(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let message = self
            .github_dropdown_state
            .load_error_message
            .clone()
            .unwrap_or_else(|| "Failed to load GitHub repositories".to_string());

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        // Label
        field.add_child(self.render_repos_field_label(appearance));

        // Selected repo chips (if any)
        if !self.form_state.selected_repos.is_empty() {
            field.add_child(self.render_selected_repo_chips(appearance));
        }

        let error_input = Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(
                        Container::new(
                            Text::new(
                                message,
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(theme.ui_error_color())
                            .finish(),
                        )
                        .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(FORM_INPUT_HEIGHT)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        let retry_button = Hoverable::new(
            self.retry_fetch_github_repos_mouse_state.clone(),
            move |state| {
                let bg = if state.is_mouse_over_element() {
                    theme.surface_3()
                } else {
                    theme.surface_2()
                };
                let icon_size = appearance.ui_font_size();

                ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_main_axis_size(MainAxisSize::Min)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_spacing(6.)
                            .with_child(
                                ConstrainedBox::new(
                                    Icon::Refresh
                                        .to_warpui_icon(theme.active_ui_text_color())
                                        .finish(),
                                )
                                .with_width(icon_size)
                                .with_height(icon_size)
                                .finish(),
                            )
                            .with_child(
                                Text::new(
                                    "Retry",
                                    appearance.ui_font_family(),
                                    appearance.ui_font_size(),
                                )
                                .with_color(theme.active_ui_text_color().into())
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(8.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
                    .with_background(bg)
                    .finish(),
                )
                .with_height(FORM_INPUT_HEIGHT)
                .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(UpdateEnvironmentFormAction::RetryFetchGithubRepos);
        })
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(Expanded::new(1., error_input).finish())
            .with_child(retry_button)
            .finish();

        field.add_child(
            ConstrainedBox::new(row)
                .with_max_width(DROPDOWN_MAX_WIDTH)
                .finish(),
        );

        field.finish()
    }

    fn render_repos_field_authed(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        field.add_child(self.render_repos_field_label(appearance));

        // Build chevron button for toggling dropdown
        // Note: Don't add a click handler here since the entire container is clickable
        let chevron_icon = if self.github_dropdown_state.is_expanded {
            Icon::ChevronUp
        } else {
            Icon::ChevronDown
        };
        let chevron_button = icon_button(
            appearance,
            chevron_icon,
            false,
            self.add_repo_button_mouse_state.clone(),
        )
        .build()
        .finish();

        // Build chips row with editor inline
        let mut chips_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.);

        // Add selected repo chips inline
        for (index, repo) in self.form_state.selected_repos.iter().enumerate() {
            let mouse_state = self
                .remove_repo_mouse_states
                .get(index)
                .cloned()
                .unwrap_or_default();

            let action = UpdateEnvironmentFormAction::RemoveRepo(index);
            let remove_button = icon_button(appearance, Icon::X, false, mouse_state)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(action.clone());
                })
                .finish();

            let chip = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(
                        ConstrainedBox::new(
                            Text::new_inline(
                                repo.to_string(),
                                appearance.ui_font_family(),
                                appearance.ui_font_size() * 0.9,
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .with_max_width(REPO_CHIP_MAX_WIDTH)
                        .finish(),
                    )
                    .with_child(remove_button)
                    .finish(),
            )
            .with_padding_left(8.)
            .with_vertical_padding(4.)
            .with_padding_right(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background(theme.surface_3())
            .finish();

            chips_row.add_child(chip);
        }

        // Add editor after chips (expands to fill remaining space)
        let editor = ChildView::new(&self.repos_input_editor).finish();
        chips_row.add_child(Expanded::new(1., editor).finish());

        let chips_content = chips_row.finish();

        // Main row: [clipped chips + editor area] [chevron]
        let input_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(Expanded::new(1., Clipped::new(chips_content).finish()).finish())
            .with_child(chevron_button)
            .finish();

        // Wrap in container with border and make the entire area clickable to toggle dropdown
        let input_container = SavePosition::new(
            Hoverable::new(self.dropdown_input_mouse_state.clone(), |_| {
                Container::new(
                    ConstrainedBox::new(
                        Flex::column()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_child(
                                Clipped::new(
                                    Container::new(input_content)
                                        .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
                                        .finish(),
                                )
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_height(FORM_INPUT_HEIGHT)
                    .finish(),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
                .with_background(theme.surface_2())
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(UpdateEnvironmentFormAction::ToggleReposDropdown);
            })
            .finish(),
            REPOS_DROPDOWN_ANCHOR,
        )
        .finish();

        // Build the stack with dropdown if expanded
        let mut stack = Stack::new().with_child(input_container);

        if self.github_dropdown_state.is_expanded {
            let dropdown = self.render_repos_dropdown(appearance);
            let dismissible_dropdown = Dismiss::new(dropdown)
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(UpdateEnvironmentFormAction::CloseReposDropdown);
                })
                .finish();
            stack.add_positioned_overlay_child(
                dismissible_dropdown,
                OffsetPositioning::offset_from_save_position_element(
                    REPOS_DROPDOWN_ANCHOR.to_string(),
                    vec2f(0., 4.),
                    PositionedElementOffsetBounds::WindowBySize,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        // Build refresh button (icon only) - placed outside the dropdown in the same column
        let refresh_button = Container::new(
            icon_button(
                appearance,
                Icon::Refresh,
                false,
                self.refresh_repos_button_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                height: Some(FORM_INPUT_HEIGHT),
                width: Some(FORM_INPUT_HEIGHT),
                padding: Some(Coords::uniform(10.)),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(UpdateEnvironmentFormAction::RetryFetchGithubRepos);
            })
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        // Row with dropdown stack and refresh button
        let input_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(4.)
            .with_child(Expanded::new(1., stack.finish()).finish())
            .with_child(refresh_button)
            .finish();

        field.add_child(input_row);

        // Helper text
        field.add_child(self.render_repo_helper_text_row(appearance));
        field.finish()
    }

    fn render_repo_helper_text_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let helper = Text::new(
            "Type owner/repo and press Enter to add, or select from dropdown.",
            appearance.ui_font_family(),
            appearance.ui_font_size() * 0.85,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // Configure access link is only available on non-WASM platforms
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(app_install_link) = &self.github_dropdown_state.app_install_link else {
                return helper;
            };

            // "Missing a repo? Configure access on GitHub" text with link
            let install_link = app_install_link.clone();

            // Build as a row with plain text + link
            let mut text_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.);

            text_row.add_child(helper);
            // Plain text part
            text_row.add_child(
                Text::new(
                    "Missing a repo?",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.85,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .finish(),
            );

            // Link part
            let link = Hoverable::new(
                self.configure_access_link_mouse_state.clone(),
                move |state| {
                    let color = if state.is_mouse_over_element() {
                        theme.accent().with_opacity(180)
                    } else {
                        theme.accent()
                    };
                    Text::new(
                        "Configure access on GitHub",
                        appearance.ui_font_family(),
                        appearance.ui_font_size() * 0.85,
                    )
                    .with_color(color.into())
                    .finish()
                },
            )
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                let url = install_link.clone();
                ctx.dispatch_typed_action(UpdateEnvironmentFormAction::OpenUrl(url));
            })
            .finish();

            text_row.add_child(link);

            text_row.finish()
        }

        #[cfg(target_family = "wasm")]
        {
            helper
        }
    }

    /// Split `haystack` into a sequence of fragments tagged as (text, is_match) for occurrences of `needle`.
    ///
    /// Matches are non-overlapping (as returned by `str::match_indices`). This is used to render dropdown
    /// labels where matching fragments are bolded.
    ///
    /// Note on case-insensitive matching:
    /// We do ASCII-only case-insensitive matching so that the match indices from the searched string are safe
    /// to apply to the original `haystack` (ASCII lowercasing preserves byte lengths and indices).
    fn split_non_overlapping_substring_matches<'a>(
        haystack: &'a str,
        needle: &'a str,
    ) -> Vec<(&'a str, bool)> {
        let needle = needle.trim();
        if needle.is_empty() {
            return vec![(haystack, false)];
        }

        // Only do case-insensitive matching when both strings are ASCII, otherwise slicing `haystack` by
        // indices from a transformed string could panic.
        let (search_haystack, search_needle) = if haystack.is_ascii() && needle.is_ascii() {
            (haystack.to_ascii_lowercase(), needle.to_ascii_lowercase())
        } else {
            (haystack.to_string(), needle.to_string())
        };

        let mut parts = Vec::new();
        let mut cursor = 0usize;

        for (start, _) in search_haystack.match_indices(&search_needle) {
            let end = start + search_needle.len();
            if start > cursor {
                parts.push((&haystack[cursor..start], false));
            }
            parts.push((&haystack[start..end], true));
            cursor = end;
        }

        if cursor < haystack.len() {
            parts.push((&haystack[cursor..], false));
        }

        if parts.is_empty() {
            vec![(haystack, false)]
        } else {
            parts
        }
    }

    /// Render a repo label for the dropdown, bolding any non-overlapping substring matches against the
    /// current query (`self.repos_input`).
    ///
    /// For performance and simplicity, we avoid regexes and only bold literal substring matches.
    fn render_repo_dropdown_label(
        &self,
        repo_label: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let query = self.repos_input.trim();

        if query.is_empty() {
            return Text::new(
                repo_label.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.active_ui_text_color().into())
            .finish();
        }

        let fragments = Self::split_non_overlapping_substring_matches(repo_label, query);
        if fragments.len() == 1 {
            let (text, is_match) = fragments[0];
            let mut label = Text::new(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.active_ui_text_color().into());

            if is_match {
                label = label.with_style(Properties::default().weight(Weight::Bold));
            }

            return label.finish();
        }

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(0.);

        for (text, is_match) in fragments {
            if text.is_empty() {
                continue;
            }

            let mut fragment = Text::new_inline(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.active_ui_text_color().into());

            if is_match {
                fragment = fragment.with_style(Properties::default().weight(Weight::Bold));
            }

            row.add_child(fragment.finish());
        }

        row.finish()
    }

    fn render_repos_dropdown(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Filter repos based on input text
        let search_text = self.repos_input.to_lowercase();
        let filtered_repos: Vec<_> = self
            .github_dropdown_state
            .available_repos
            .iter()
            .enumerate()
            .filter(|(_, repo)| {
                if search_text.is_empty() {
                    true
                } else {
                    let repo_str = format!("{}/{}", repo.owner, repo.repo).to_lowercase();
                    repo_str.contains(&search_text)
                }
            })
            .collect();

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.);

        // If no filtered repos, show message
        if filtered_repos.is_empty() {
            content.add_child(
                Container::new(
                    Text::new(
                        "No repositories found",
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.nonactive_ui_text_color().into())
                    .finish(),
                )
                .with_uniform_padding(12.)
                .finish(),
            );
        } else {
            // Render repo list with checkboxes
            for (index, repo) in filtered_repos {
                let is_selected = self
                    .form_state
                    .selected_repos
                    .iter()
                    .any(|r| r.owner == repo.owner && r.repo == repo.repo);

                let mouse_state = self
                    .github_dropdown_state
                    .repo_row_mouse_states
                    .get(index)
                    .cloned()
                    .unwrap_or_default();

                let repo_label = format!("{}/{}", repo.owner, repo.repo);

                // Build checkbox
                let checkbox = appearance
                    .ui_builder()
                    .checkbox(MouseStateHandle::default(), None)
                    .check(is_selected)
                    .build()
                    .disable()
                    .finish();

                // Render label (bold the matched substring fragments)
                let label = self.render_repo_dropdown_label(&repo_label, appearance);

                // Build row content with checkbox and label
                let row_content = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(checkbox)
                    .with_child(label)
                    .finish();

                let is_selected_row = self.github_dropdown_state.selected_index == Some(index);
                let row = Hoverable::new(mouse_state, move |state| {
                    let bg = if is_selected_row || state.is_mouse_over_element() {
                        theme.surface_3()
                    } else {
                        theme.surface_2()
                    };
                    Container::new(row_content)
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(8.)
                        .with_background(bg)
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(UpdateEnvironmentFormAction::ToggleRepoSelection(
                        index,
                    ));
                })
                .finish();

                let position_id = Self::repo_dropdown_row_position_id(index);
                content.add_child(SavePosition::new(row, &position_id).finish());
            }
        }

        // Use ClippedScrollable for proper scrolling
        let scrollable = ClippedScrollable::vertical(
            self.github_dropdown_state.scroll_state.clone(),
            content.finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_text_color().into(), // scrollbar thumb
            theme.active_ui_text_color().into(),    // scrollbar thumb active
            Fill::None,                             // scrollbar track
        )
        .finish();

        // Constrain height and width
        let dropdown_content = ConstrainedBox::new(scrollable)
            .with_max_height(DROPDOWN_MAX_HEIGHT)
            .with_max_width(DROPDOWN_MAX_WIDTH)
            .finish();

        // Wrap in container with border and background
        Container::new(dropdown_content)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
            .with_background(theme.surface_2())
            .finish()
    }

    fn render_selected_repo_chips(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut chips_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.);

        for (index, repo) in self.form_state.selected_repos.iter().enumerate() {
            let mouse_state = self
                .remove_repo_mouse_states
                .get(index)
                .cloned()
                .unwrap_or_default();

            let action = UpdateEnvironmentFormAction::RemoveRepo(index);
            let remove_button = icon_button(appearance, Icon::X, false, mouse_state)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(action.clone());
                })
                .finish();

            let chip = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(
                        ConstrainedBox::new(
                            Text::new_inline(
                                repo.to_string(),
                                appearance.ui_font_family(),
                                appearance.ui_font_size() * 0.9,
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .with_max_width(REPO_CHIP_MAX_WIDTH)
                        .finish(),
                    )
                    .with_child(remove_button)
                    .finish(),
            )
            .with_padding_left(8.)
            .with_vertical_padding(4.)
            .with_padding_right(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background(theme.surface_3())
            .finish();

            chips_row.add_child(chip);
        }

        chips_row.finish()
    }

    fn auth_url_with_next(&self, base_auth_url: &str) -> String {
        match (self.github_auth_redirect_target, self.auth_source) {
            (GithubAuthRedirectTarget::SettingsEnvironments, AuthSource::Settings) => {
                github_auth_url::settings_environments_auth_url_with_next(base_auth_url)
            }
            (GithubAuthRedirectTarget::FocusCloudMode, AuthSource::CloudSetup) => {
                github_auth_url::cloud_setup_auth_url_with_next(base_auth_url)
            }
            (target, auth_source) => {
                github_auth_url::auth_url_with_next(base_auth_url, target, auth_source)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn build_auth_url_with_next(
        base_auth_url: &str,
        target: GithubAuthRedirectTarget,
        scheme: &str,
    ) -> String {
        github_auth_url::build_auth_url_with_next(
            base_auth_url,
            target,
            scheme,
            AuthSource::Settings,
        )
    }

    /// Parses a Docker image reference and returns the Docker Hub URL if it looks like a Docker Hub image.
    ///
    /// Recognizes:
    /// - Official images like `python:3.11` (no owner, single segment) → `https://hub.docker.com/_/python`
    /// - `owner/repo` or `owner/repo:tag` → `https://hub.docker.com/r/owner/repo`
    /// - `docker.io/owner/repo` or `docker.io/owner/repo:tag`
    /// - `docker.io/library/python` (explicit library prefix for official images)
    /// - `index.docker.io/owner/repo` or `index.docker.io/owner/repo:tag`
    ///
    /// Does NOT recognize (returns None):
    /// - Other registries like `ghcr.io/owner/repo`
    fn parse_docker_hub_url(image_ref: &str) -> Option<String> {
        let trimmed = image_ref.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Remove tag or digest suffix for URL construction.
        let image_without_tag = trimmed
            .split('@')
            .next()
            .unwrap_or(trimmed)
            .split(':')
            .next()
            .unwrap_or(trimmed);

        // Check for explicit docker.io or index.docker.io prefix
        let stripped = image_without_tag
            .strip_prefix("docker.io/")
            .or_else(|| image_without_tag.strip_prefix("index.docker.io/"));

        let path = if let Some(path) = stripped {
            // Explicit Docker Hub reference
            path
        } else {
            // Check if it looks like a bare image reference (no registry prefix)
            let parts: Vec<&str> = image_without_tag.split('/').collect();
            match parts.len() {
                // Single segment = official image (e.g. "python", "node")
                1 => {
                    let name = parts[0];
                    if name.is_empty() {
                        return None;
                    }
                    return Some(format!("https://hub.docker.com/_/{name}"));
                }
                // Two segments = owner/repo; if the first part contains a dot, it's a registry hostname
                2 => {
                    if parts[0].contains('.') {
                        return None;
                    }
                    image_without_tag
                }
                _ => return None,
            }
        };

        // Handle explicit "library/" prefix for official images (e.g. docker.io/library/python)
        if let Some(official_name) = path.strip_prefix("library/") {
            if !official_name.is_empty() && !official_name.contains('/') {
                return Some(format!("https://hub.docker.com/_/{official_name}"));
            }
        }

        // Validate the path has owner/repo format
        let path_parts: Vec<&str> = path.split('/').collect();
        if path_parts.len() != 2 {
            return None;
        }

        let owner = path_parts[0];
        let repo = path_parts[1];

        if owner.is_empty() || repo.is_empty() {
            return None;
        }

        Some(format!("https://hub.docker.com/r/{owner}/{repo}"))
    }

    fn render_image_link_button(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let docker_hub_url = Self::parse_docker_hub_url(&self.form_state.docker_image)?;
        let theme = appearance.theme();

        let action = UpdateEnvironmentFormAction::OpenUrl(docker_hub_url.clone());

        let button = Container::new(
            icon_button(
                appearance,
                Icon::Docker,
                false,
                self.image_link_button_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                height: Some(FORM_INPUT_HEIGHT),
                width: Some(FORM_INPUT_HEIGHT),
                padding: Some(Coords::uniform(10.)),
                ..Default::default()
            })
            .with_tooltip({
                let ui_builder = appearance.ui_builder().clone();
                move || {
                    ui_builder
                        .tool_tip(format!("Open image at {docker_hub_url}"))
                        .build()
                        .finish()
                }
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            })
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        Some(button)
    }
    fn render_docker_image_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut field = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(FORM_LABEL_SPACING);

        // Label (without suggest button)
        field.add_child(Self::render_form_label(
            "Docker image reference",
            true,
            appearance,
        ));

        // Docker image input
        let editor_container = Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(
                        Clipped::new(
                            Container::new(ChildView::new(&self.docker_image_editor).finish())
                                .with_horizontal_padding(FORM_INPUT_HORIZONTAL_PADDING)
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(FORM_INPUT_HEIGHT)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_2())
        .finish();

        // Row with editor, optional Docker Hub link button, and suggest button.
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.);

        row.add_child(Expanded::new(1., editor_container).finish());

        // Add image link button if the image looks like a Docker Hub image.
        if let Some(link_button) = self.render_image_link_button(appearance) {
            row.add_child(link_button);
        }

        row.add_child(self.render_docker_image_suggest_button(appearance));

        let row = row.finish();

        field.add_child(
            ConstrainedBox::new(row)
                .with_width(DROPDOWN_MAX_WIDTH)
                .finish(),
        );

        // Suggest image callout (if applicable) - shown below the input
        if let Some(callout) = self.render_suggest_image_callout(appearance) {
            field.add_child(callout);
        }

        field.finish()
    }

    fn can_suggest_image_for_current_repos(&self) -> bool {
        if self.form_state.selected_repos.is_empty() {
            return false;
        }

        if matches!(&self.suggest_image_state, SuggestImageState::Loading { .. }) {
            return false;
        }

        // In Edit mode, disable suggest-image until repos have been modified at least once.
        if self.is_edit_mode() && !self.edit_repos_modified {
            return false;
        }

        // Enforce “repos must change to retry” after a suggest-image attempt.
        let repos_unchanged = self
            .selected_repos_key()
            .and_then(|key| {
                self.suggest_image_last_attempt_key
                    .as_ref()
                    .map(|last_key| key == *last_key)
            })
            .unwrap_or(false);
        if repos_unchanged {
            return false;
        }

        true
    }

    fn render_docker_image_suggest_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let is_loading = matches!(&self.suggest_image_state, SuggestImageState::Loading { .. });
        let is_disabled = !self.can_suggest_image_for_current_repos();

        let button_text = if is_loading {
            "Generating…"
        } else {
            "Suggest image"
        };

        let tooltip_text = "Warp will suggest a Docker image based on your selected repositories.";

        let button = Hoverable::new(
            self.suggest_image_button_mouse_state.clone(),
            move |state| {
                let bg = if !is_disabled {
                    if state.is_mouse_over_element() {
                        theme.surface_3()
                    } else {
                        theme.surface_2()
                    }
                } else {
                    theme.surface_2()
                };

                let text_fill = if is_disabled {
                    theme.disabled_ui_text_color()
                } else {
                    theme.active_ui_text_color()
                };

                let icon_size = appearance.ui_font_size();
                let icon = ConstrainedBox::new(Icon::Lightbulb.to_warpui_icon(text_fill).finish())
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish();

                let text = Text::new(
                    button_text,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(text_fill.into())
                .finish();

                let button_content = Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_spacing(6.)
                        .with_child(icon)
                        .with_child(text)
                        .finish(),
                )
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_border(Border::all(CARD_BORDER_WIDTH).with_border_fill(theme.outline()))
                .with_background(bg)
                .finish();

                let button = ConstrainedBox::new(button_content)
                    .with_height(FORM_INPUT_HEIGHT)
                    .finish();

                let mut stack = Stack::new().with_child(button);

                if state.is_hovered() {
                    let tooltip = ConstrainedBox::new(
                        appearance
                            .ui_builder()
                            .tool_tip(tooltip_text.to_string())
                            .build()
                            .finish(),
                    )
                    .with_max_width(420.)
                    .finish();

                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 6.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        ),
                    );
                }

                stack.finish()
            },
        );

        if is_disabled {
            return button.finish();
        }

        button
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(UpdateEnvironmentFormAction::SuggestImage);
            })
            .finish()
    }

    fn render_suggest_image_callout(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let current_key = self.selected_repos_key();

        // Only show callouts if they match the current repo selection
        match (&self.suggest_image_state, current_key.as_ref()) {
            (SuggestImageState::Idle, _) => None,
            (SuggestImageState::Loading { .. }, _) => None,
            (
                SuggestImageState::Success {
                    key,
                    needs_custom_image: true,
                    reason,
                },
                Some(current_key),
            ) if key == current_key => {
                Some(self.render_suggest_image_callout_with_action(reason, appearance))
            }
            (SuggestImageState::AuthRequired { key, auth_url }, Some(current_key))
                if key == current_key =>
            {
                let auth_url_with_next = self.auth_url_with_next(auth_url);
                let action = UpdateEnvironmentFormAction::OpenUrl(auth_url_with_next);
                let button = WarningBoxButtonConfig::new(
                    "Authenticate",
                    self.suggest_image_auth_button_mouse_state.clone(),
                    move |ctx| {
                        ctx.dispatch_typed_action(action.clone());
                    },
                );
                Some(render_warning_box(
                    WarningBoxConfig::new(
                        "You need to grant access to your GitHub repos to suggest a Docker image",
                    )
                    .with_width(DROPDOWN_MAX_WIDTH)
                    .with_button(button),
                    appearance,
                ))
            }
            (SuggestImageState::Error { key, message }, Some(current_key))
                if key == current_key =>
            {
                Some(render_warning_box(
                    WarningBoxConfig::new(message).with_width(DROPDOWN_MAX_WIDTH),
                    appearance,
                ))
            }
            _ => None,
        }
    }

    fn render_suggest_image_callout_with_action(
        &self,
        reason: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let action = UpdateEnvironmentFormAction::LaunchAgentForSelectedRepos;
        let button = WarningBoxButtonConfig::new(
            "Launch agent",
            self.suggest_image_launch_agent_button_mouse_state.clone(),
            move |ctx| {
                ctx.dispatch_typed_action(action.clone());
            },
        );

        render_warning_box(
            WarningBoxConfig::new(
                "We couldn't find a good match. We recommend using a custom Docker image for these repos.",
            )
            .with_description(reason)
            .with_icon(Icon::AlertTriangle)
            .with_width(DROPDOWN_MAX_WIDTH)
            .with_button(button),
            appearance,
        )
    }
}

impl Entity for UpdateEnvironmentForm {
    type Event = UpdateEnvironmentFormEvent;
}

impl TypedActionView for UpdateEnvironmentForm {
    type Action = UpdateEnvironmentFormAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            UpdateEnvironmentFormAction::Submit => {
                if !self.form_state.is_valid() {
                    return;
                }

                let environment = self.form_state.to_ambient_agent_environment();
                match &self.mode {
                    EnvironmentFormMode::Create => {
                        send_telemetry_from_ctx!(CloudAgentTelemetryEvent::EnvironmentCreated, ctx);
                        ctx.emit(UpdateEnvironmentFormEvent::Created {
                            environment,
                            share_with_team: self.share_with_team,
                        });
                    }
                    EnvironmentFormMode::Edit { env_id } => {
                        send_telemetry_from_ctx!(
                            CloudAgentTelemetryEvent::EnvironmentUpdated {
                                environment_id: env_id.into_server(),
                            },
                            ctx
                        );
                        ctx.emit(UpdateEnvironmentFormEvent::Updated {
                            env_id: *env_id,
                            environment,
                        });
                    }
                }
            }
            UpdateEnvironmentFormAction::Delete => {
                if let EnvironmentFormMode::Edit { env_id } = &self.mode {
                    send_telemetry_from_ctx!(
                        CloudAgentTelemetryEvent::EnvironmentDeleted {
                            environment_id: env_id.into_server(),
                        },
                        ctx
                    );
                    ctx.emit(UpdateEnvironmentFormEvent::DeleteRequested { env_id: *env_id });
                }
            }
            UpdateEnvironmentFormAction::Cancel => {
                ctx.emit(UpdateEnvironmentFormEvent::Cancelled);
            }
            UpdateEnvironmentFormAction::ToggleShareWithTeam => {
                if matches!(self.mode, EnvironmentFormMode::Create)
                    && UserWorkspaces::as_ref(ctx).current_team_uid().is_some()
                {
                    self.share_with_team = !self.share_with_team;
                    ctx.notify();
                }
            }
            UpdateEnvironmentFormAction::Escape => {
                if !self.try_close_repos_dropdown(ctx) {
                    ctx.emit(UpdateEnvironmentFormEvent::Cancelled);
                }
            }
            UpdateEnvironmentFormAction::FocusSetupCommandsInput => {
                ctx.focus(&self.setup_commands_input);
            }
            UpdateEnvironmentFormAction::AddRepo => {
                let repo_input = self.repos_input_editor.as_ref(ctx).buffer_text(ctx);
                let repo_input = if repo_input.trim().is_empty() {
                    self.repos_input.clone()
                } else {
                    repo_input
                };
                let parsed_repos = Self::parse_repo_inputs(&repo_input);
                let has_custom_repo = parsed_repos.iter().any(|(owner, repo)| {
                    !self
                        .github_dropdown_state
                        .available_repos
                        .iter()
                        .any(|available| available.owner == *owner && available.repo == *repo)
                });

                if self.github_dropdown_state.is_expanded {
                    if let Some(selected_index) = self.github_dropdown_state.selected_index {
                        if parsed_repos.is_empty() || !has_custom_repo {
                            self.toggle_repo_selection_at_index(selected_index, ctx);
                            return;
                        }
                    }
                }

                if parsed_repos.is_empty() {
                    return;
                }

                for (owner, repo) in parsed_repos {
                    let already_selected = self
                        .form_state
                        .selected_repos
                        .iter()
                        .any(|r| r.owner == owner && r.repo == repo);
                    if !already_selected {
                        self.form_state
                            .selected_repos
                            .push(GithubRepo::new(owner, repo));
                        self.remove_repo_mouse_states
                            .push(MouseStateHandle::default());
                    }
                }

                // Mark repos as modified in Edit mode to enable suggest button
                if self.is_edit_mode() {
                    self.edit_repos_modified = true;
                }

                // Clear the input
                self.clear_repos_input(ctx);
                ctx.notify();
            }
            UpdateEnvironmentFormAction::RemoveRepo(index) => {
                if *index < self.form_state.selected_repos.len() {
                    self.form_state.selected_repos.remove(*index);
                    self.remove_repo_mouse_states.remove(*index);

                    // Mark repos as modified in Edit mode to enable suggest button
                    if self.is_edit_mode() {
                        self.edit_repos_modified = true;
                    }
                    ctx.notify();
                }
            }
            UpdateEnvironmentFormAction::ToggleReposDropdown => {
                self.github_dropdown_state.is_expanded = !self.github_dropdown_state.is_expanded;
                ctx.focus(&self.repos_input_editor);

                if self.github_dropdown_state.is_expanded {
                    self.github_dropdown_state.scroll_state = ClippedScrollStateHandle::default();
                    self.ensure_repo_dropdown_selection();
                    self.scroll_repo_dropdown_selection_into_view();
                } else {
                    self.github_dropdown_state.selected_index = None;
                }

                ctx.notify();
            }
            UpdateEnvironmentFormAction::CloseReposDropdown => {
                self.github_dropdown_state.is_expanded = false;
                self.github_dropdown_state.selected_index = None;
                ctx.notify();
            }
            UpdateEnvironmentFormAction::ToggleRepoSelection(index) => {
                self.toggle_repo_selection_at_index(*index, ctx);
            }
            UpdateEnvironmentFormAction::RemoveSetupCommand(index) => {
                if *index < self.form_state.setup_commands.len() {
                    self.form_state.setup_commands.remove(*index);
                }
                if *index < self.remove_setup_command_mouse_states.len() {
                    self.remove_setup_command_mouse_states.remove(*index);
                }
                ctx.notify();
            }
            UpdateEnvironmentFormAction::SuggestImage => {
                self.suggest_image(ctx);
            }
            UpdateEnvironmentFormAction::LaunchAgentForSelectedRepos => {
                send_telemetry_from_ctx!(
                    CloudAgentTelemetryEvent::LaunchedAgentFromEnvironmentForm,
                    ctx
                );

                let repos = self.selected_repos_as_remote_repo_args();
                if repos.is_empty() {
                    return;
                }

                let arg = CreateEnvironmentArg { repos };

                let window_id = ctx.window_id();
                let primary_window_and_view = ctx
                    .root_view_id(window_id)
                    .map(|view_id| (window_id, view_id));

                if let Some((primary_window_id, root_view_id)) = primary_window_and_view {
                    ctx.dispatch_action(
                        primary_window_id,
                        &[root_view_id],
                        "root_view:create_environment_in_existing_window_and_run",
                        &arg,
                        log::Level::Info,
                    );
                } else {
                    ctx.dispatch_global_action("root_view:create_environment_and_run", arg);
                }

                ctx.notify();
            }
            UpdateEnvironmentFormAction::RetryFetchGithubRepos => {
                self.fetch_github_repos(ctx);
            }
            UpdateEnvironmentFormAction::StartGithubAuth => {
                send_telemetry_from_ctx!(
                    CloudAgentTelemetryEvent::GitHubAuthFromEnvironmentForm,
                    ctx
                );
                self.start_github_auth(ctx);
            }
            UpdateEnvironmentFormAction::OpenUrl(url) => {
                ctx.open_url(url);
            }
        }
    }

    fn action_accessibility_contents(
        &mut self,
        _action: &Self::Action,
        _ctx: &mut ViewContext<Self>,
    ) -> warpui::accessibility::ActionAccessibilityContent {
        warpui::accessibility::ActionAccessibilityContent::default()
    }
}

impl View for UpdateEnvironmentForm {
    fn ui_name() -> &'static str {
        "UpdateEnvironmentForm"
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if self.submit_button.is_focused(app) {
            context.set.insert(SUBMIT_BUTTON_FOCUSED);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut page = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(FORM_FIELD_SPACING);

        // Header row with back button, title, and action button (only when show_header is true)
        if self.show_header {
            page.add_child(self.render_header(appearance, app));
        }
        if let Some(warning) = self.render_share_with_team_warning(appearance, app) {
            page.add_child(warning);
        }

        // Form fields
        page.add_child(Self::render_form_field(
            "Name",
            true,
            None,
            &self.name_editor,
            appearance,
        ));

        page.add_child(self.render_description_field(appearance, app));
        page.add_child(self.render_repos_field(appearance));
        page.add_child(self.render_docker_image_field(appearance));
        page.add_child(self.render_setup_commands_field(appearance));

        // Footer row with buttons (only when header is hidden)
        if !self.show_header {
            let mut footer_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

            // Delete button on the left for edit mode
            if matches!(&self.mode, EnvironmentFormMode::Edit { .. }) {
                footer_row.add_child(ChildView::new(&self.delete_button).finish());
            } else {
                // Empty spacer for create mode to push submit button to the right
                footer_row.add_child(Empty::new().finish());
            }

            // Submit actions on the right
            footer_row.add_child(self.render_submit_actions(appearance, app, &self.submit_button));

            page.add_child(footer_row.finish());
        } else if matches!(&self.mode, EnvironmentFormMode::Edit { .. }) {
            // Delete button row when header is shown
            page.add_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(ChildView::new(&self.delete_button).finish())
                    .finish(),
            );
        }

        page.finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus(ctx);
        }
    }
}

#[cfg(test)]
#[path = "update_environment_form_tests.rs"]
mod tests;
