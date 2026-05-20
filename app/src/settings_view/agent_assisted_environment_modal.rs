#[cfg_attr(target_family = "wasm", allow(unused_imports))]
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use pathfinder_color::ColorU;
use warp_core::{
    features::FeatureFlag, paths::home_relative_path, ui::theme::color::internal_colors,
};
use warpui::{
    elements::{
        Align, Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
        Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Empty, Expanded, Flex,
        MainAxisSize, MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Text,
    },
    fonts::{Properties, Weight},
    platform::{file_picker::FilePickerError, FilePickerConfiguration},
    r#async::{SpawnedFutureHandle, Timer},
    ui_components::components::UiComponent,
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    modal::MODAL_BACKDROP_OPACITY,
    themes::theme::Blend,
    ui_components::{
        buttons::icon_button,
        dialog::{dialog_styles, Dialog},
        icons::Icon,
    },
    view_components::{
        action_button::{ActionButton, ButtonSize, PrimaryTheme, SecondaryTheme},
        DismissibleToast,
    },
    workspace::ToastStack,
};

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use git2::Repository as GitRepository;

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;

#[cfg(all(
    feature = "local_fs",
    not(target_family = "wasm"),
    not(any(test, feature = "integration_tests"))
))]
use ai::index::full_source_code_embedding::manager::CodebaseIndexManagerEvent;

const DIALOG_WIDTH: f32 = 600.;
const AVAILABLE_LIST_MAX_HEIGHT: f32 = 260.;

const REPO_ROW_HORIZONTAL_PADDING: f32 = 10.;
const REPO_ROW_VERTICAL_PADDING: f32 = 8.;
const REPO_ROW_CORNER_RADIUS: f32 = 6.;

#[derive(Debug, Clone)]
pub enum AgentAssistedEnvironmentModalEvent {
    Cancelled,
    Confirmed { repo_paths: Vec<String> },
}

#[derive(Debug, Clone)]
pub enum AgentAssistedEnvironmentModalAction {
    Cancel,
    Confirm,
    AddRepo(usize),
    RemoveRepo(usize),
    OpenDirectoryPicker,
    DirectoryPicked(Result<PathBuf, FilePickerError>),
}

#[derive(Clone, Debug)]
struct RepoEntry {
    name: String,
    path: PathBuf,
}

pub struct AgentAssistedEnvironmentModal {
    visible: bool,

    available_repos: Vec<RepoEntry>,
    selected_repo_paths: Vec<PathBuf>,

    available_row_mouse_states: Vec<MouseStateHandle>,
    selected_row_mouse_states: Vec<MouseStateHandle>,
    close_button_mouse_state: MouseStateHandle,
    available_scroll_state: ClippedScrollStateHandle,

    available_repos_loading: bool,
    available_repos_loading_timeout: Option<SpawnedFutureHandle>,

    add_repo_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    create_button: ViewHandle<ActionButton>,
}

impl AgentAssistedEnvironmentModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let add_repo_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Add repo", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        AgentAssistedEnvironmentModalAction::OpenDirectoryPicker,
                    );
                })
        });

        let cancel_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Cancel", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AgentAssistedEnvironmentModalAction::Cancel);
            })
        });

        let create_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Create environment", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AgentAssistedEnvironmentModalAction::Confirm);
            })
        });

        let me = Self {
            visible: false,
            available_repos: Vec::new(),
            selected_repo_paths: Vec::new(),
            available_row_mouse_states: Vec::new(),
            selected_row_mouse_states: Vec::new(),
            close_button_mouse_state: MouseStateHandle::default(),
            available_scroll_state: ClippedScrollStateHandle::default(),
            available_repos_loading: false,
            available_repos_loading_timeout: None,
            add_repo_button,
            cancel_button,
            create_button,
        };

        #[cfg(all(
            feature = "local_fs",
            not(target_family = "wasm"),
            not(any(test, feature = "integration_tests"))
        ))]
        {
            let index_manager = CodebaseIndexManager::handle(ctx);
            ctx.subscribe_to_model(&index_manager, |me, _, event, ctx| {
                if !me.visible {
                    return;
                }

                match event {
                    CodebaseIndexManagerEvent::SyncStateUpdated
                    | CodebaseIndexManagerEvent::NewIndexCreated
                    | CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata { .. }
                    | CodebaseIndexManagerEvent::IndexMetadataUpdated { .. } => {
                        me.refresh_available_repos(ctx);
                        if me.available_repos.is_empty() {
                            me.maybe_start_available_repos_loading(ctx);
                        } else {
                            me.stop_available_repos_loading();
                        }
                        ctx.notify();
                    }
                    _ => {}
                }
            });
        }

        me.update_create_button_disabled_state(ctx);
        me
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = true;
        self.selected_repo_paths.clear();
        self.selected_row_mouse_states.clear();

        self.stop_available_repos_loading();
        self.refresh_available_repos(ctx);
        self.maybe_start_available_repos_loading(ctx);

        self.update_create_button_disabled_state(ctx);
        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        self.stop_available_repos_loading();
        ctx.notify();
    }

    fn update_create_button_disabled_state(&self, ctx: &mut ViewContext<Self>) {
        let disabled = self.selected_repo_paths.is_empty();
        self.create_button.update(ctx, |button, ctx| {
            button.set_disabled(disabled, ctx);
        });
    }

    fn refresh_available_repos(&mut self, ctx: &mut ViewContext<Self>) {
        self.available_repos = available_indexed_repos(ctx);
        self.available_row_mouse_states = self
            .available_repos
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();
    }

    fn maybe_start_available_repos_loading(&mut self, ctx: &mut ViewContext<Self>) {
        if !cfg!(all(feature = "local_fs", not(target_family = "wasm"))) {
            return;
        }

        if !self.visible || !self.available_repos.is_empty() || self.available_repos_loading {
            return;
        }

        self.available_repos_loading = true;
        if let Some(handle) = self.available_repos_loading_timeout.take() {
            handle.abort();
        }

        self.available_repos_loading_timeout = Some(ctx.spawn_abortable(
            Timer::after(Duration::from_millis(750)),
            |me, _, ctx| {
                me.available_repos_loading_timeout = None;
                if !me.visible {
                    return;
                }

                me.refresh_available_repos(ctx);

                // If repos are still empty after a brief wait, stop showing the loading state so we
                // can surface the empty-state message.
                if me.available_repos.is_empty() {
                    me.available_repos_loading = false;
                } else {
                    me.stop_available_repos_loading();
                }

                ctx.notify();
            },
            |_, _| {},
        ));
    }

    fn stop_available_repos_loading(&mut self) {
        self.available_repos_loading = false;
        if let Some(handle) = self.available_repos_loading_timeout.take() {
            handle.abort();
        }
    }

    fn is_selected(&self, path: &PathBuf) -> bool {
        self.selected_repo_paths.iter().any(|p| p == path)
    }

    fn add_repo_path(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        if self.is_selected(&path) {
            return;
        }

        self.selected_repo_paths.push(path);
        self.selected_row_mouse_states
            .push(MouseStateHandle::default());
        self.update_create_button_disabled_state(ctx);
        ctx.notify();
    }

    fn add_repo(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(entry) = self.available_repos.get(index) else {
            return;
        };

        self.add_repo_path(entry.path.clone(), ctx);
    }

    fn remove_repo(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index >= self.selected_repo_paths.len() {
            return;
        }

        self.selected_repo_paths.remove(index);
        self.selected_row_mouse_states.remove(index);
        self.update_create_button_disabled_state(ctx);
        ctx.notify();
    }

    fn render_section_title(&self, title: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Text::new(
            title.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into())
        .finish()
    }

    fn render_repo_info(name: String, path: String, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(
                Text::new(name, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
            )
            .with_child(
                Text::new(
                    path,
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.9,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .soft_wrap(true)
                .finish(),
            )
            .finish()
    }

    fn render_selected_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(8.);

        col.add_child(self.render_section_title("Selected repos", appearance));

        if self.selected_repo_paths.is_empty() {
            col.add_child(
                Text::new(
                    "No repos selected yet",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.95,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .finish(),
            );

            return col.finish();
        }

        for (idx, repo_path) in self.selected_repo_paths.iter().enumerate() {
            let mouse_state = self
                .selected_row_mouse_states
                .get(idx)
                .cloned()
                .unwrap_or_default();

            let name = repo_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("(unknown)")
                .to_string();

            let path_text = home_relative_path(repo_path);

            let remove_action = AgentAssistedEnvironmentModalAction::RemoveRepo(idx);
            let remove_button = icon_button(appearance, Icon::X, false, mouse_state)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(remove_action.clone());
                })
                .finish();

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Expanded::new(1., Self::render_repo_info(name, path_text, appearance)).finish(),
                )
                .with_child(remove_button)
                .finish();

            col.add_child(
                Container::new(row)
                    .with_horizontal_padding(REPO_ROW_HORIZONTAL_PADDING)
                    .with_vertical_padding(REPO_ROW_VERTICAL_PADDING)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        REPO_ROW_CORNER_RADIUS,
                    )))
                    .with_background(
                        theme
                            .surface_2()
                            .blend(&internal_colors::accent_overlay_1(theme)),
                    )
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .finish(),
            );
        }

        col.finish()
    }

    fn render_available_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(8.);

        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Expanded::new(
                    1.,
                    self.render_section_title("Available indexed repos", appearance),
                )
                .finish(),
            )
            .with_child(
                if cfg!(all(feature = "local_fs", not(target_family = "wasm"))) {
                    Container::new(ChildView::new(&self.add_repo_button).finish())
                        .with_margin_left(8.)
                        .finish()
                } else {
                    Empty::new().finish()
                },
            )
            .finish();

        col.add_child(header);

        if self.available_repos.is_empty() {
            let text = if cfg!(all(feature = "local_fs", not(target_family = "wasm"))) {
                if self.available_repos_loading {
                    "Loading locally indexed repos…"
                } else {
                    "No locally indexed repos found yet. Index a repo, then try again."
                }
            } else {
                "Local repo selection is unavailable in this build."
            };

            col.add_child(
                Text::new(
                    text,
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.95,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .soft_wrap(true)
                .finish(),
            );

            return col.finish();
        }

        let mut list = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.);

        let mut has_any_available = false;
        for (idx, entry) in self.available_repos.iter().enumerate() {
            if self.is_selected(&entry.path) {
                continue;
            }
            has_any_available = true;

            let mouse_state = self
                .available_row_mouse_states
                .get(idx)
                .cloned()
                .unwrap_or_default();

            let add_action = AgentAssistedEnvironmentModalAction::AddRepo(idx);
            let add_button = icon_button(appearance, Icon::Plus, false, mouse_state)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(add_action.clone());
                })
                .finish();

            let name = entry.name.clone();
            let path_text = home_relative_path(&entry.path);

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Expanded::new(1., Self::render_repo_info(name, path_text, appearance)).finish(),
                )
                .with_child(add_button)
                .finish();

            list.add_child(
                Container::new(row)
                    .with_horizontal_padding(REPO_ROW_HORIZONTAL_PADDING)
                    .with_vertical_padding(REPO_ROW_VERTICAL_PADDING)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        REPO_ROW_CORNER_RADIUS,
                    )))
                    .with_background(theme.surface_2())
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .finish(),
            );
        }

        if !has_any_available {
            col.add_child(
                Text::new(
                    "All locally indexed repos are already selected.",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.95,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .soft_wrap(true)
                .finish(),
            );

            return col.finish();
        }

        // NOTE: `Scrollable` reserves horizontal space for its scrollbar gutter by default,
        // which makes the list content slightly narrower than non-scrollable rows above.
        // Overlay the scrollbar so it doesn't affect layout width.
        let scrollable = ClippedScrollable::vertical(
            self.available_scroll_state.clone(),
            list.finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .with_padding_start(0.)
        .with_padding_end(0.)
        .finish();

        col.add_child(
            ConstrainedBox::new(scrollable)
                .with_max_height(AVAILABLE_LIST_MAX_HEIGHT)
                .finish(),
        );

        col.finish()
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    fn show_not_a_repo_toast(&self, selected_path: &Path, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let path = home_relative_path(selected_path);
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast =
                DismissibleToast::error(format!("Selected folder is not a Git repository: {path}"))
                    .with_object_id("agent_assisted_env_add_repo_not_git_repo".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    fn show_file_picker_error_toast(&self, error: &FilePickerError, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(format!("{error}"));
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    fn handle_directory_picked(&mut self, selected_path: PathBuf, ctx: &mut ViewContext<Self>) {
        let selected_path = dunce::canonicalize(&selected_path).unwrap_or(selected_path);

        // `discover` accepts subdirectories; we normalize to the repo working tree root.
        match GitRepository::discover(&selected_path)
            .ok()
            .and_then(|repo| repo.workdir().map(|workdir| workdir.to_path_buf()))
        {
            Some(repo_root) => {
                self.add_repo_path(repo_root, ctx);
            }
            None => {
                self.show_not_a_repo_toast(&selected_path, ctx);
            }
        }
    }

    fn open_directory_picker(&mut self, ctx: &mut ViewContext<Self>) {
        if !cfg!(all(feature = "local_fs", not(target_family = "wasm"))) {
            return;
        }

        let window_id = ctx.window_id();
        let view_id = ctx.view_id();

        ctx.open_file_picker(
            move |paths_result, ctx| {
                let result = paths_result.and_then(|paths| {
                    paths.into_iter().next().map(PathBuf::from).ok_or_else(|| {
                        FilePickerError::DialogFailed("No directory selected".to_string())
                    })
                });

                ctx.dispatch_typed_action_for_view(
                    window_id,
                    view_id,
                    &AgentAssistedEnvironmentModalAction::DirectoryPicked(result),
                );
            },
            FilePickerConfiguration::new().folders_only(),
        );

        ctx.notify();
    }

    fn render_dialog(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let description = if FeatureFlag::FullSourceCodeEmbedding.is_enabled() {
            "Select locally indexed repos to provide context for the environment creation agent."
        } else {
            "Select repos to provide context for the environment creation agent."
        }
        .to_string();

        let close_button = icon_button(
            appearance,
            Icon::X,
            false,
            self.close_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AgentAssistedEnvironmentModalAction::Cancel);
        })
        .finish();

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(16.)
            .with_child(self.render_selected_section(appearance))
            .with_child(self.render_available_section(appearance))
            .finish();

        let dialog = Dialog::new(
            "Select repos for your environment".to_string(),
            Some(description),
            dialog_styles(appearance),
        )
        .with_close_button(close_button)
        .with_child(content)
        .with_separator()
        .with_bottom_row_child(ChildView::new(&self.cancel_button).finish())
        .with_bottom_row_child(
            Container::new(ChildView::new(&self.create_button).finish())
                .with_margin_left(12.)
                .finish(),
        )
        .with_width(DIALOG_WIDTH)
        .build();

        let dialog = Dismiss::new(dialog.finish())
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(AgentAssistedEnvironmentModalAction::Cancel);
            })
            .finish();

        Container::new(Align::new(dialog).finish())
            .with_background_color(ColorU::new(0, 0, 0, MODAL_BACKDROP_OPACITY))
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl Entity for AgentAssistedEnvironmentModal {
    type Event = AgentAssistedEnvironmentModalEvent;
}

impl TypedActionView for AgentAssistedEnvironmentModal {
    type Action = AgentAssistedEnvironmentModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentAssistedEnvironmentModalAction::Cancel => {
                ctx.emit(AgentAssistedEnvironmentModalEvent::Cancelled);
            }
            AgentAssistedEnvironmentModalAction::Confirm => {
                if self.selected_repo_paths.is_empty() {
                    return;
                }

                let repo_paths = self
                    .selected_repo_paths
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();

                ctx.emit(AgentAssistedEnvironmentModalEvent::Confirmed { repo_paths });
            }
            AgentAssistedEnvironmentModalAction::AddRepo(index) => {
                self.add_repo(*index, ctx);
            }
            AgentAssistedEnvironmentModalAction::RemoveRepo(index) => {
                self.remove_repo(*index, ctx);
            }
            AgentAssistedEnvironmentModalAction::OpenDirectoryPicker => {
                self.open_directory_picker(ctx);
            }
            AgentAssistedEnvironmentModalAction::DirectoryPicked(result) => match result {
                Ok(path) => {
                    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
                    {
                        self.handle_directory_picked(path.clone(), ctx);
                    }

                    #[cfg(any(not(feature = "local_fs"), target_family = "wasm"))]
                    {
                        self.add_repo_path(path.clone(), ctx);
                    }
                }
                Err(error) => {
                    self.show_file_picker_error_toast(error, ctx);
                }
            },
        }
    }
}

impl View for AgentAssistedEnvironmentModal {
    fn ui_name() -> &'static str {
        "AgentAssistedEnvironmentModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.visible {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        self.render_dialog(appearance, app)
    }
}

fn available_indexed_repos(app: &AppContext) -> Vec<RepoEntry> {
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    {
        let mut repos: Vec<RepoEntry> = CodebaseIndexManager::as_ref(app)
            .get_codebase_index_statuses(app)
            .filter_map(|(root, status)| {
                status.has_synced_version().then(|| {
                    let name = root
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| root.to_string_lossy().into_owned());
                    RepoEntry {
                        name,
                        path: root.clone(),
                    }
                })
            })
            .collect();

        repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        repos
    }

    #[cfg(any(not(feature = "local_fs"), target_family = "wasm"))]
    {
        let _ = app;
        Vec::new()
    }
}

#[cfg(test)]
#[path = "agent_assisted_environment_modal_tests.rs"]
mod tests;
