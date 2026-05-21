use std::path::PathBuf;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    ChildAnchor, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex,
    FormattedTextElement, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::macros::id;
use warpui::keymap::FixedBinding;
use warpui::keymap::Keystroke;
use warpui::platform::file_picker::FilePickerConfiguration;
use warpui::FocusContext;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, KeystrokeSource, NakedTheme, PrimaryTheme,
};

use super::session_config::{is_git_repo, SessionConfigSelection, SessionType};
use super::session_config_rendering;

pub fn init(app: &mut warpui::AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "enter",
        SessionConfigModalAction::Submit,
        id!(SessionConfigModal::ui_name()),
    )]);
}

const SECTION_GAP: f32 = 16.;

#[derive(Clone, Debug)]
pub enum SessionConfigModalAction {
    SelectSessionType(usize),
    OpenDirectoryPicker,
    DirectorySelected(Result<String, warpui::platform::file_picker::FilePickerError>),
    ToggleWorktree,
    ToggleAutogenerateWorktreeBranchName,
    Submit,
    Dismiss,
}

pub enum SessionConfigModalEvent {
    Completed(SessionConfigSelection),
    Dismissed,
}

pub struct SessionConfigModal {
    session_types: Vec<SessionType>,
    selected_session_type_index: usize,
    selected_directory: PathBuf,
    is_git_repo: bool,
    enable_worktree: bool,
    autogenerate_worktree_branch_name: bool,
    /// When `false`, the session type pill row is hidden and the session type
    /// defaults to Terminal behind the scenes (used when Oz is disabled).
    show_session_type_row: bool,
    session_pill_mouse_states: Vec<MouseStateHandle>,
    directory_button_mouse_state: MouseStateHandle,
    worktree_checkbox_mouse_state: MouseStateHandle,
    autogenerate_worktree_branch_name_checkbox_mouse_state: MouseStateHandle,
    autogenerate_tooltip_mouse_state: MouseStateHandle,
    worktree_tooltip_mouse_state: MouseStateHandle,
    close_button: ViewHandle<ActionButton>,
    submit_button: ViewHandle<ActionButton>,
}

impl SessionConfigModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let session_types = session_config_rendering::visible_session_types(true);

        let close_button = ctx.add_view(|ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(crate::ui_components::icons::Icon::X)
                .with_size(ButtonSize::Small)
                .with_keybinding(
                    KeystrokeSource::Fixed(Keystroke::parse("escape").unwrap_or_default()),
                    ctx,
                )
                .on_click(|ctx| ctx.dispatch_typed_action(SessionConfigModalAction::Dismiss))
        });

        let submit_button = ctx.add_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Get Warping"), PrimaryTheme)
                .with_full_width(true)
                .with_keybinding(
                    KeystrokeSource::Fixed(Keystroke::parse("enter").unwrap_or_default()),
                    ctx,
                )
                .on_click(|ctx| ctx.dispatch_typed_action(SessionConfigModalAction::Submit))
        });

        let pill_mouse_states = session_types
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();

        Self {
            session_types,
            selected_session_type_index: 0,
            selected_directory: home,
            // Filled in by `configure()` before the modal is shown.
            is_git_repo: false,
            enable_worktree: false,
            autogenerate_worktree_branch_name: false,
            show_session_type_row: true,
            session_pill_mouse_states: pill_mouse_states,
            directory_button_mouse_state: MouseStateHandle::default(),
            worktree_checkbox_mouse_state: MouseStateHandle::default(),
            autogenerate_worktree_branch_name_checkbox_mouse_state: MouseStateHandle::default(),
            autogenerate_tooltip_mouse_state: MouseStateHandle::default(),
            worktree_tooltip_mouse_state: MouseStateHandle::default(),
            close_button,
            submit_button,
        }
    }

    /// Reconfigures the visible session types based on whether Oz is available.
    /// Resets the selection to index 0 (the first available type).
    /// When Oz is disabled, hides the session type row entirely and defaults
    /// to Terminal behind the scenes.
    pub fn configure(&mut self, show_oz: bool) {
        self.show_session_type_row = show_oz;
        self.session_types = session_config_rendering::visible_session_types(show_oz);
        self.selected_session_type_index = 0;
        self.session_pill_mouse_states = self
            .session_types
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();
        self.is_git_repo = is_git_repo(&self.selected_directory);
    }

    fn selected_session_type(&self) -> SessionType {
        self.session_types[self.selected_session_type_index]
    }

    fn update_directory(&mut self, path: PathBuf) {
        self.is_git_repo = is_git_repo(&path);
        if !self.is_git_repo {
            self.enable_worktree = false;
            self.autogenerate_worktree_branch_name = false;
        }
        self.selected_directory = path;
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(SessionConfigModalEvent::Completed(SessionConfigSelection {
            session_type: self.selected_session_type(),
            directory: self.selected_directory.clone(),
            enable_worktree: self.enable_worktree,
            autogenerate_worktree_branch_name: self.autogenerate_worktree_branch_name,
        }));
    }

    // ── Rendering ──

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let title = FormattedTextElement::from_str(
            "Create your first tab config",
            appearance.ui_font_family(),
            24.,
        )
        .with_color(blended_colors::text_main(theme, theme.background()))
        .with_weight(Weight::Semibold)
        .finish();

        let subtitle_text = if self.show_session_type_row {
            "Set up a reusable starting point for your tabs. \
             Pick a repo, choose a session type, and optionally attach a worktree. \
             Use it whenever you want to open a new tab with this setup."
        } else {
            "Set up a reusable starting point for your tabs. \
             Pick a repo, optionally attach a worktree, and \
             use it whenever you want to open a new tab with this setup."
        };
        let subtitle =
            FormattedTextElement::from_str(subtitle_text, appearance.ui_font_family(), 14.)
                .with_color(blended_colors::text_sub(theme, theme.background()))
                .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(4.).finish())
            .finish()
    }

    fn render_session_type_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        session_config_rendering::render_session_type_pills(
            &self.session_types,
            self.selected_session_type_index,
            &self.session_pill_mouse_states,
            |i, ctx, _| {
                ctx.dispatch_typed_action(SessionConfigModalAction::SelectSessionType(i));
            },
            appearance,
        )
    }

    fn render_directory_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        session_config_rendering::render_directory_picker(
            &self.selected_directory,
            self.directory_button_mouse_state.clone(),
            |ctx, _| {
                ctx.dispatch_typed_action(SessionConfigModalAction::OpenDirectoryPicker);
            },
            appearance,
        )
    }

    fn render_checkboxes(&self, appearance: &Appearance) -> Box<dyn Element> {
        session_config_rendering::render_worktree_checkbox(
            self.enable_worktree,
            self.is_git_repo,
            self.worktree_checkbox_mouse_state.clone(),
            self.worktree_tooltip_mouse_state.clone(),
            |ctx, _| {
                ctx.dispatch_typed_action(SessionConfigModalAction::ToggleWorktree);
            },
            appearance,
        )
    }

    fn render_autogenerate_worktree_branch_name_checkbox(
        &self,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        session_config_rendering::render_autogenerate_worktree_branch_name_checkbox(
            self.autogenerate_worktree_branch_name,
            self.enable_worktree,
            self.autogenerate_worktree_branch_name_checkbox_mouse_state
                .clone(),
            self.autogenerate_tooltip_mouse_state.clone(),
            |ctx, _| {
                ctx.dispatch_typed_action(
                    SessionConfigModalAction::ToggleAutogenerateWorktreeBranchName,
                );
            },
            appearance,
        )
    }
}

impl Entity for SessionConfigModal {
    type Event = SessionConfigModalEvent;
}

impl View for SessionConfigModal {
    fn ui_name() -> &'static str {
        "SessionConfigModal"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut form = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_header(appearance));

        if self.show_session_type_row {
            form.add_child(
                Container::new(self.render_session_type_section(appearance))
                    .with_margin_top(SECTION_GAP)
                    .finish(),
            );
        }

        form.add_child(
            Container::new(self.render_directory_section(appearance))
                .with_margin_top(SECTION_GAP)
                .finish(),
        );

        form.add_child(
            Container::new(self.render_checkboxes(appearance))
                .with_margin_top(SECTION_GAP)
                .finish(),
        );
        form.add_child(
            Container::new(self.render_autogenerate_worktree_branch_name_checkbox(appearance))
                .with_margin_top(8.)
                .finish(),
        );

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(form.finish())
            .with_child(
                Container::new(ChildView::new(&self.submit_button).finish())
                    .with_margin_top(32.)
                    .finish(),
            )
            .finish();

        let body = Container::new(content)
            .with_horizontal_padding(32.)
            .with_vertical_padding(40.)
            .finish();

        let mut stack = Stack::new();
        stack.add_child(body);
        stack.add_positioned_overlay_child(
            Container::new(ChildView::new(&self.close_button).finish())
                .with_margin_top(12.)
                .with_margin_right(12.)
                .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        ConstrainedBox::new(stack.finish())
            .with_width(420.)
            .finish()
    }
}

impl TypedActionView for SessionConfigModal {
    type Action = SessionConfigModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SessionConfigModalAction::SelectSessionType(index) => {
                self.selected_session_type_index = *index;
                ctx.notify();
            }
            SessionConfigModalAction::OpenDirectoryPicker => {
                ctx.open_file_picker(
                    |result, ctx| {
                        if let Some(path_result) =
                            result.map(|paths| paths.into_iter().next()).transpose()
                        {
                            ctx.dispatch_typed_action(
                                &SessionConfigModalAction::DirectorySelected(path_result),
                            );
                        }
                    },
                    FilePickerConfiguration::new().folders_only(),
                );
            }
            SessionConfigModalAction::DirectorySelected(result) => match result {
                Ok(path) => {
                    self.update_directory(PathBuf::from(path));
                    ctx.notify();
                }
                Err(err) => {
                    log::warn!("File picker error in session config modal: {err}");
                }
            },
            SessionConfigModalAction::ToggleWorktree => {
                if self.is_git_repo {
                    self.enable_worktree = !self.enable_worktree;
                    if !self.enable_worktree {
                        self.autogenerate_worktree_branch_name = false;
                    }
                    ctx.notify();
                }
            }
            SessionConfigModalAction::ToggleAutogenerateWorktreeBranchName => {
                if self.enable_worktree {
                    self.autogenerate_worktree_branch_name =
                        !self.autogenerate_worktree_branch_name;
                    ctx.notify();
                }
            }
            SessionConfigModalAction::Submit => {
                self.submit(ctx);
            }
            SessionConfigModalAction::Dismiss => {
                ctx.emit(SessionConfigModalEvent::Dismissed);
            }
        }
    }
}
