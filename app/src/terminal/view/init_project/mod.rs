mod lsp_server_selector;
pub mod model;

use crate::ai::agent::icons::{in_progress_icon, yellow_stop_icon};
use crate::ai::blocklist::block::keyboard_navigable_buttons::{
    simple_navigation_button, KeyboardNavigableButtonBuilder, KeyboardNavigableButtons,
};
use crate::ai::blocklist::block::toggleable_items::ToggleableItemsView;
use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::ai::blocklist::inline_action::inline_action_header::HeaderConfig;
use crate::ai::blocklist::inline_action::requested_action::RenderableAction;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::appearance::Appearance;
use crate::code::lsp_telemetry::{LspEnablementSource, LspTelemetryEvent};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{
    AgentModeSetupCodebaseContextActionType, AgentModeSetupCreateEnvironmentActionType,
    AgentModeSetupProjectScopedRulesActionType,
};
use crate::ui_components::icons::Icon;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use crate::TelemetryEvent;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use lsp::supported_servers::LSPServerType;
use lsp_server_selector::{create_lsp_server_selector, LSPServerInfo};
pub use model::{InitProjectModel, InitProjectModelEvent, InitStepKind};
use model::{InitStepData, InitStepStatus};
use std::path::{Path, PathBuf};
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{
        Border, ChildView, Container, CrossAxisAlignment, Empty, Flex, MouseStateHandle,
        ParentElement, Text,
    },
    ui_components::{button::ButtonVariant, components::UiComponent},
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const ONBOARDING_TEXT: &str = "Great - let's begin setting up this project! Would you like to give me permission to index this codebase? It allows me to quickly understand context and provide more targeted solutions when working in this codebase. No code is stored on Warp servers.";
const ALREADY_SETUP_TEXT: &str = "It looks like this project has already been initialized. You can re-generate the AGENTS.md for this codebase by clicking the button below.";
// Native Warp rules file format.
pub const FILES_TO_CHECK: [&str; 2] = ["AGENTS.md", "WARP.md"];
// File formats that can be linked to WARP.md.
pub const LINKABLE_FILES: [&str; 7] = [
    "CLAUDE.md",
    ".cursorrules",
    "AGENT.md",
    "GEMINI.md",
    ".clinerules",
    ".windsurfrules",
    ".github/copilot-instructions.md",
];

/// Result of the codebase context/indexing step
pub enum CodebaseIndexingResult {
    Accepted,
    Skipped,
}

/// Result of the language servers step
pub enum LanguageServersResult {
    Accepted {
        enabled_servers: Vec<LSPServerType>,
        servers_to_install: Vec<LSPServerType>,
    },
    Skipped,
}

/// Result of the create environment step
pub enum CreateEnvironmentResult {
    /// Environment was created
    Created,
    /// User skipped environment creation
    Skipped,
}

/// Result of a completed /init step
pub enum InitActionResult {
    /// Welcome step completed (always auto-completes)
    Welcome,
    CodebaseContext(CodebaseIndexingResult),
    ProjectScopedRules(ProjectScopedRulesResult),
    LanguageServers(LanguageServersResult),
    CreateEnvironment(CreateEnvironmentResult),
}

pub enum ProjectScopedRulesResult {
    LinkedFromExisting(String),
    GenerateNew {
        mouse_state: MouseStateHandle,
        button_disabled: bool,
    },
    AlreadyExists {
        button_disabled: bool,
    },
    Skipped,
}

#[derive(Default)]
struct CodebaseContextMouseStateHandles {
    index_button: MouseStateHandle,
    skip_button: MouseStateHandle,
    view_status_button: MouseStateHandle,
}

struct ProjectRulesMouseStateHandles {
    link_buttons: Vec<MouseStateHandle>,
    generate_button: MouseStateHandle,
    regenerate_button: MouseStateHandle,
    skip_button: MouseStateHandle,
}

#[derive(Default)]
struct LanguageServersMouseStateHandles {
    setup_button: MouseStateHandle,
    skip_button: MouseStateHandle,
}

impl Default for ProjectRulesMouseStateHandles {
    fn default() -> Self {
        let linkable_file_count = LINKABLE_FILES.len();
        Self {
            link_buttons: (0..linkable_file_count)
                .map(|_| MouseStateHandle::default())
                .collect(),
            generate_button: Default::default(),
            regenerate_button: Default::default(),
            skip_button: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum InitProjectBlockAction {
    IndexCodebase(PathBuf),
    SetupLanguageServers {
        server_info: Vec<LSPServerInfo>,
        repo_path: PathBuf,
    },
    SkipLanguageServers,
    SkipIndex,
    LinkFromExisting(PathBuf),
    GenerateRules,
    RegenerateRules,
    SkipRules,
    ViewCodebaseContextStatus,
    StartCreateEnvironment,
    SkipCreateEnvironment,
}

#[derive(Default)]
struct CreateEnvironmentMouseStateHandles {
    create_button: MouseStateHandle,
    skip_button: MouseStateHandle,
}

enum StepState {
    Welcome,
    CodebaseContext {
        mouse_states: CodebaseContextMouseStateHandles,
        keyboard_nav_buttons: Option<ViewHandle<KeyboardNavigableButtons>>,
    },
    LanguageServersSingle {
        mouse_states: LanguageServersMouseStateHandles,
        keyboard_nav_buttons: Option<ViewHandle<KeyboardNavigableButtons>>,
    },
    LanguageServersMultiple {
        skip_mouse_state: MouseStateHandle,
        enable_mouse_state: MouseStateHandle,
        lsp_selector: Option<ViewHandle<ToggleableItemsView<LSPServerInfo>>>,
    },
    ProjectRules {
        mouse_states: ProjectRulesMouseStateHandles,
        keyboard_nav_buttons: Option<ViewHandle<KeyboardNavigableButtons>>,
    },
    CreateEnvironment {
        mouse_states: CreateEnvironmentMouseStateHandles,
        keyboard_nav_buttons: Option<ViewHandle<KeyboardNavigableButtons>>,
    },
}

/// View for a single /init step. Renders based on step kind and reads status from model.
pub struct InitStepBlock {
    model: ModelHandle<InitProjectModel>,
    state: StepState,
}

impl InitStepBlock {
    pub fn new(
        step_kind: InitStepKind,
        model: ModelHandle<InitProjectModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Subscribe to model for rerenders
        ctx.subscribe_to_model(&model, move |_, _, event, ctx| match event {
            InitProjectModelEvent::StepCompleted(kind) if *kind == step_kind => {
                ctx.notify();
            }
            InitProjectModelEvent::Cancelled => {
                ctx.notify();
            }
            // ProjectScopedRules block needs to rerender when init completes to show regenerate button
            InitProjectModelEvent::InitCompleted
                if step_kind == InitStepKind::ProjectScopedRules =>
            {
                ctx.notify();
            }
            _ => {}
        });

        let state = match step_kind {
            InitStepKind::Welcome => StepState::Welcome,
            InitStepKind::CodebaseContext => StepState::CodebaseContext {
                mouse_states: CodebaseContextMouseStateHandles::default(),
                keyboard_nav_buttons: None,
            },
            InitStepKind::LanguageServers => {
                // Determine single vs multiple from model data
                let is_multiple = model
                    .as_ref(ctx)
                    .get_step(InitStepKind::LanguageServers)
                    .and_then(|step| {
                        if let InitStepStatus::Ready(InitStepData::LanguageServers {
                            servers,
                            ..
                        }) = &step.status
                        {
                            Some(servers.len() > 1)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(false);

                if is_multiple {
                    StepState::LanguageServersMultiple {
                        skip_mouse_state: MouseStateHandle::default(),
                        enable_mouse_state: MouseStateHandle::default(),
                        lsp_selector: None,
                    }
                } else {
                    StepState::LanguageServersSingle {
                        mouse_states: LanguageServersMouseStateHandles::default(),
                        keyboard_nav_buttons: None,
                    }
                }
            }
            InitStepKind::ProjectScopedRules => StepState::ProjectRules {
                mouse_states: ProjectRulesMouseStateHandles::default(),
                keyboard_nav_buttons: None,
            },
            InitStepKind::CreateEnvironment => StepState::CreateEnvironment {
                mouse_states: CreateEnvironmentMouseStateHandles::default(),
                keyboard_nav_buttons: None,
            },
        };

        let mut new_block = Self { model, state };

        // Create keyboard nav buttons or LSP selector based on step kind and status
        new_block.create_interactive_views(ctx);

        new_block
    }

    fn create_interactive_views(&mut self, ctx: &mut ViewContext<Self>) {
        let step = self.model.as_ref(ctx).get_step(self.step_kind());
        let Some(step) = step else { return };

        match (&step.status, &mut self.state) {
            (
                InitStepStatus::Ready(InitStepData::CodebaseContext { pwd_path }),
                StepState::CodebaseContext {
                    mouse_states,
                    keyboard_nav_buttons,
                },
            ) => {
                let buttons = Self::create_codebase_context_buttons(pwd_path, mouse_states);
                *keyboard_nav_buttons =
                    Some(ctx.add_typed_action_view(|_| KeyboardNavigableButtons::new(buttons)));
            }
            (
                InitStepStatus::Ready(InitStepData::LanguageServers { servers, repo_path }),
                StepState::LanguageServersSingle {
                    mouse_states,
                    keyboard_nav_buttons,
                },
            ) if servers.len() == 1 => {
                let buttons = Self::create_single_lsp_buttons(&servers[0], repo_path, mouse_states);
                *keyboard_nav_buttons =
                    Some(ctx.add_typed_action_view(|_| KeyboardNavigableButtons::new(buttons)));
            }
            (
                InitStepStatus::Ready(InitStepData::LanguageServers { servers, repo_path }),
                StepState::LanguageServersMultiple { lsp_selector, .. },
            ) if servers.len() > 1 => {
                *lsp_selector = Some(create_lsp_server_selector(
                    servers.clone(),
                    repo_path.clone(),
                    ctx,
                ));
            }
            (
                InitStepStatus::Ready(InitStepData::ProjectScopedRules { linkable_files }),
                StepState::ProjectRules {
                    mouse_states,
                    keyboard_nav_buttons,
                },
            ) => {
                let buttons = Self::create_project_rules_buttons(linkable_files, mouse_states);
                *keyboard_nav_buttons =
                    Some(ctx.add_typed_action_view(|_| KeyboardNavigableButtons::new(buttons)));
            }
            (
                InitStepStatus::Ready(InitStepData::CreateEnvironment),
                StepState::CreateEnvironment {
                    mouse_states,
                    keyboard_nav_buttons,
                },
            ) => {
                let buttons = Self::create_environment_buttons(mouse_states);
                *keyboard_nav_buttons =
                    Some(ctx.add_typed_action_view(|_| KeyboardNavigableButtons::new(buttons)));
            }
            _ => {}
        }
    }

    pub fn try_steal_focus(&self, ctx: &mut ViewContext<Self>) {
        match &self.state {
            StepState::CodebaseContext {
                keyboard_nav_buttons: Some(buttons),
                ..
            }
            | StepState::LanguageServersSingle {
                keyboard_nav_buttons: Some(buttons),
                ..
            }
            | StepState::ProjectRules {
                keyboard_nav_buttons: Some(buttons),
                ..
            }
            | StepState::CreateEnvironment {
                keyboard_nav_buttons: Some(buttons),
                ..
            } => ctx.focus(buttons),
            StepState::LanguageServersMultiple {
                lsp_selector: Some(selector),
                ..
            } => ctx.focus(selector),
            _ => {}
        }
    }

    pub fn step_kind(&self) -> InitStepKind {
        match &self.state {
            StepState::Welcome => InitStepKind::Welcome,
            StepState::CodebaseContext { .. } => InitStepKind::CodebaseContext,
            StepState::LanguageServersSingle { .. } | StepState::LanguageServersMultiple { .. } => {
                InitStepKind::LanguageServers
            }
            StepState::ProjectRules { .. } => InitStepKind::ProjectScopedRules,
            StepState::CreateEnvironment { .. } => InitStepKind::CreateEnvironment,
        }
    }

    fn create_single_lsp_buttons(
        server_info: &LSPServerInfo,
        repo_path: &Path,
        mouse_states: &LanguageServersMouseStateHandles,
    ) -> Vec<KeyboardNavigableButtonBuilder> {
        let button_text = if server_info.is_installed {
            format!("Enable {} support", server_info.server_type.language_name())
        } else {
            format!(
                "Install and enable {}",
                server_info.server_type.language_name()
            )
        };

        vec![
            simple_navigation_button(
                button_text,
                mouse_states.setup_button.clone(),
                InitProjectBlockAction::SetupLanguageServers {
                    server_info: vec![server_info.clone()],
                    repo_path: repo_path.to_path_buf(),
                },
                false,
            ),
            simple_navigation_button(
                "Skip for now.".to_string(),
                mouse_states.skip_button.clone(),
                InitProjectBlockAction::SkipLanguageServers,
                false,
            ),
        ]
    }

    fn create_codebase_context_buttons(
        pwd_path: &Path,
        mouse_states: &CodebaseContextMouseStateHandles,
    ) -> Vec<KeyboardNavigableButtonBuilder> {
        vec![
            simple_navigation_button(
                "Yes, index this codebase.".to_string(),
                mouse_states.index_button.clone(),
                InitProjectBlockAction::IndexCodebase(pwd_path.to_path_buf()),
                false,
            ),
            simple_navigation_button(
                "Skip for now.".to_string(),
                mouse_states.skip_button.clone(),
                InitProjectBlockAction::SkipIndex,
                false,
            ),
        ]
    }

    fn create_project_rules_buttons(
        linkable_files: &[PathBuf],
        mouse_states: &ProjectRulesMouseStateHandles,
    ) -> Vec<KeyboardNavigableButtonBuilder> {
        let mut buttons = Vec::new();

        for (i, linkable_file) in LINKABLE_FILES.iter().enumerate() {
            if let Some(path) = linkable_files.iter().find(|p| p.ends_with(linkable_file)) {
                buttons.push(simple_navigation_button(
                    format!("Link existing {linkable_file} to my AGENTS.md file"),
                    mouse_states.link_buttons[i].clone(),
                    InitProjectBlockAction::LinkFromExisting(path.clone()),
                    false,
                ));
            }
        }

        buttons.push(simple_navigation_button(
            "Generate AGENTS.md file".to_string(),
            mouse_states.generate_button.clone(),
            InitProjectBlockAction::GenerateRules,
            false,
        ));
        buttons.push(simple_navigation_button(
            "Skip AGENTS.md generation for now".to_string(),
            mouse_states.skip_button.clone(),
            InitProjectBlockAction::SkipRules,
            false,
        ));

        buttons
    }

    fn create_environment_buttons(
        mouse_states: &CreateEnvironmentMouseStateHandles,
    ) -> Vec<KeyboardNavigableButtonBuilder> {
        vec![
            simple_navigation_button(
                "Create an environment".to_string(),
                mouse_states.create_button.clone(),
                InitProjectBlockAction::StartCreateEnvironment,
                false,
            ),
            simple_navigation_button(
                "Skip for now".to_string(),
                mouse_states.skip_button.clone(),
                InitProjectBlockAction::SkipCreateEnvironment,
                false,
            ),
        ]
    }

    /// Renders a "ready" state block with keyboard-navigable buttons and a header prompt.
    fn render_ready_with_buttons(
        action_view: &ViewHandle<KeyboardNavigableButtons>,
        header_text: impl Into<std::borrow::Cow<'static, str>>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        RenderableAction::new_with_element(ChildView::new(action_view).finish(), app)
            .with_header(
                HeaderConfig::new(header_text, app)
                    .with_icon(yellow_stop_icon(appearance))
                    .with_soft_wrap_title(),
            )
            .with_background_color(appearance.theme().surface_1().into_solid())
            .with_content_item_spacing()
            .render(app)
            .finish()
    }

    /// Renders a success completion state with check icon.
    fn render_success_completion(text: &str, app: &AppContext) -> Box<dyn Element> {
        RenderableAction::new(text, app)
            .with_icon(Icon::Check.to_warpui_icon(Fill::success()).finish())
            .with_content_item_spacing()
            .render(app)
            .finish()
    }

    /// Renders a skipped/cancelled completion state with X icon.
    fn render_skipped_completion(text: &str, app: &AppContext) -> Box<dyn Element> {
        RenderableAction::new(text, app)
            .with_icon(Icon::X.to_warpui_icon(Fill::error()).finish())
            .with_content_item_spacing()
            .render(app)
            .finish()
    }

    /// Creates a regenerate AGENTS.md button.
    fn regenerate_button(
        mouse_state: &MouseStateHandle,
        disabled: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Outlined, mouse_state.clone())
            .with_text_label(crate::i18n::tr_static(app, "Re-generate AGENTS.md file").to_string());
        if disabled {
            button = button.disabled();
        }
        button
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(InitProjectBlockAction::RegenerateRules);
            })
            .finish()
    }

    #[cfg(feature = "local_fs")]
    async fn create_symlink_to_agents_md(
        source_path: &Path,
        project_root: &Path,
    ) -> Result<PathBuf, std::io::Error> {
        let agents_md_path = project_root.join("AGENTS.md");

        // Check if AGENTS.md already exists
        if agents_md_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "AGENTS.md already exists",
            ));
        }

        // Create relative path from AGENTS.md location to source file
        let relative_path = source_path.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Source file has no filename",
            )
        })?;

        // Create the symlink
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(relative_path, &agents_md_path)?;
        }

        #[cfg(windows)]
        {
            // On Windows, use junction for directories or symlink for files
            std::os::windows::fs::symlink_file(relative_path, &agents_md_path)?;
        }

        Ok(agents_md_path)
    }

    fn render_welcome(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let is_already_setup = self.model.as_ref(app).is_already_setup();

        let display_text = if !is_already_setup {
            ONBOARDING_TEXT
        } else {
            ALREADY_SETUP_TEXT
        };

        let text = Text::new(
            display_text,
            appearance.ai_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .soft_wrap(true)
        .finish()
        .with_content_item_spacing()
        .finish();

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(text);

        Container::new(content.finish())
            .with_padding_top(16.)
            .with_border(Border::top(1.).with_border_fill(appearance.theme().outline()))
            .finish()
    }

    fn render_codebase_context(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let step = self
            .model
            .as_ref(app)
            .get_step(InitStepKind::CodebaseContext);

        let Some(step) = step else {
            return Empty::new().finish();
        };

        match &step.status {
            InitStepStatus::Pending => {
                // Should not happen for codebase context (computed sync)
                Empty::new().finish()
            }
            InitStepStatus::Ready(_) => {
                let StepState::CodebaseContext {
                    keyboard_nav_buttons: Some(action_view),
                    ..
                } = &self.state
                else {
                    return Empty::new().finish();
                };

                RenderableAction::new_with_element(
                    Container::new(ChildView::new(action_view).finish())
                        .with_background(appearance.theme().surface_1())
                        .finish(),
                    app,
                )
                .with_header(
                    HeaderConfig::new(
                        "Would you like the Agent to index this codebase? This will lead to more efficient and tailored help.",
                        app,
                    )
                    .with_icon(yellow_stop_icon(appearance))
                    .with_soft_wrap_title(),
                )
                .with_background_color(appearance.theme().surface_1().into_solid())
                .with_content_item_spacing()
                .render(app)
                .finish()
            }
            InitStepStatus::Running => {
                // Codebase context doesn't have a "running" state
                Empty::new().finish()
            }
            InitStepStatus::Completed(result) => {
                self.render_completed_codebase_context(result, app)
            }
        }
    }

    fn render_completed_codebase_context(
        &self,
        result: &InitActionResult,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let InitActionResult::CodebaseContext(indexing_result) = result else {
            return Empty::new().finish();
        };

        let StepState::CodebaseContext { mouse_states, .. } = &self.state else {
            return Empty::new().finish();
        };

        match indexing_result {
            CodebaseIndexingResult::Accepted => {
                RenderableAction::new("Codebase index started", app)
                    .with_icon(Icon::Check.to_warpui_icon(Fill::success()).finish())
                    .with_action_button(
                        Appearance::as_ref(app)
                            .ui_builder()
                            .button(
                                ButtonVariant::Outlined,
                                mouse_states.view_status_button.clone(),
                            )
                            .with_text_label(
                                crate::i18n::tr_static(app, "View index status").to_string(),
                            )
                            .build()
                            .on_click(|ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    InitProjectBlockAction::ViewCodebaseContextStatus,
                                );
                            })
                            .finish(),
                    )
                    .with_content_item_spacing()
                    .render(app)
                    .finish()
            }
            CodebaseIndexingResult::Skipped => {
                Self::render_skipped_completion("Codebase index cancelled", app)
            }
        }
    }

    fn render_language_servers(&self, app: &AppContext) -> Box<dyn Element> {
        let step = self
            .model
            .as_ref(app)
            .get_step(InitStepKind::LanguageServers);

        let Some(step) = step else {
            return Empty::new().finish();
        };

        match &step.status {
            InitStepStatus::Pending => {
                // Still loading LSP detection
                Empty::new().finish()
            }
            InitStepStatus::Ready(InitStepData::LanguageServers { servers, repo_path }) => {
                if servers.len() == 1 {
                    self.render_single_lsp_ready(&servers[0], app)
                } else {
                    self.render_multiple_lsp_ready(repo_path, app)
                }
            }
            InitStepStatus::Ready(_) => Empty::new().finish(),
            InitStepStatus::Running => Empty::new().finish(),
            InitStepStatus::Completed(result) => {
                self.render_completed_language_servers(result, app)
            }
        }
    }

    fn render_single_lsp_ready(
        &self,
        server_info: &LSPServerInfo,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let StepState::LanguageServersSingle {
            keyboard_nav_buttons: Some(action_view),
            ..
        } = &self.state
        else {
            return Empty::new().finish();
        };
        Self::render_ready_with_buttons(
            action_view,
            format!(
                "Enable {} support for this codebase? This will give you smarter code navigation, inline error checking, and more.",
                server_info.server_type.language_name()
            ),
            app,
        )
    }

    fn render_multiple_lsp_ready(&self, repo_path: &Path, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let StepState::LanguageServersMultiple {
            skip_mouse_state,
            enable_mouse_state,
            lsp_selector: Some(action_view),
        } = &self.state
        else {
            return Empty::new().finish();
        };

        lsp_server_selector::render_lsp_selector_block(
            action_view,
            repo_path,
            skip_mouse_state,
            enable_mouse_state,
            appearance,
            app,
        )
        .with_content_item_spacing()
        .finish()
    }

    fn render_completed_language_servers(
        &self,
        result: &InitActionResult,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let InitActionResult::LanguageServers(lsp_result) = result else {
            return Empty::new().finish();
        };

        match lsp_result {
            LanguageServersResult::Accepted {
                enabled_servers,
                servers_to_install,
            } => {
                let label = if !servers_to_install.is_empty() {
                    "Started installation for language support".to_string()
                } else if enabled_servers.len() == 1 {
                    format!(
                        "{} language support enabled",
                        enabled_servers[0].language_name()
                    )
                } else {
                    "Language support enabled".to_string()
                };
                Self::render_success_completion(&label, app)
            }
            LanguageServersResult::Skipped => {
                Self::render_skipped_completion("Language support skipped", app)
            }
        }
    }

    fn render_project_rules(&self, app: &AppContext) -> Box<dyn Element> {
        let step = self
            .model
            .as_ref(app)
            .get_step(InitStepKind::ProjectScopedRules);

        let Some(step) = step else {
            return Empty::new().finish();
        };

        match &step.status {
            InitStepStatus::Pending => Empty::new().finish(),
            InitStepStatus::Ready(_) => {
                let StepState::ProjectRules {
                    keyboard_nav_buttons: Some(action_view),
                    ..
                } = &self.state
                else {
                    return Empty::new().finish();
                };
                Self::render_ready_with_buttons(
                    action_view,
                    "Would you like to create an AGENTS.md file? Warp can create one for you with project specific rules, context, and conventions inferred from your codebase. The agent will use this context as it codes.",
                    app,
                )
            }
            InitStepStatus::Running => {
                // AI is generating AGENTS.md - show in-progress state
                let appearance = Appearance::as_ref(app);
                RenderableAction::new("Generating AGENTS.md...", app)
                    .with_icon(in_progress_icon(appearance).finish())
                    .with_content_item_spacing()
                    .render(app)
                    .finish()
            }
            InitStepStatus::Completed(result) => self.render_completed_project_rules(result, app),
        }
    }

    fn render_create_environment(&self, app: &AppContext) -> Box<dyn Element> {
        let step = self
            .model
            .as_ref(app)
            .get_step(InitStepKind::CreateEnvironment);

        let Some(step) = step else {
            return Empty::new().finish();
        };

        match &step.status {
            InitStepStatus::Pending => Empty::new().finish(),
            InitStepStatus::Ready(_) => {
                let StepState::CreateEnvironment {
                    keyboard_nav_buttons: Some(action_view),
                    ..
                } = &self.state
                else {
                    return Empty::new().finish();
                };
                Self::render_ready_with_buttons(
                    action_view,
                    "Would you like to create an environment for this project so you can run cloud agents in it? The agent will guide you through choosing GitHub repos, configuring a Docker image, and specifying startup commands.",
                    app,
                )
            }
            InitStepStatus::Running => {
                let appearance = Appearance::as_ref(app);
                RenderableAction::new("Creating environment...", app)
                    .with_icon(in_progress_icon(appearance).finish())
                    .with_content_item_spacing()
                    .render(app)
                    .finish()
            }
            InitStepStatus::Completed(result) => {
                self.render_completed_create_environment(result, app)
            }
        }
    }

    fn render_completed_create_environment(
        &self,
        result: &InitActionResult,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let InitActionResult::CreateEnvironment(env_result) = result else {
            return Empty::new().finish();
        };

        match env_result {
            CreateEnvironmentResult::Created => {
                Self::render_success_completion("Environment created", app)
            }
            CreateEnvironmentResult::Skipped => {
                Self::render_skipped_completion("Environment creation skipped", app)
            }
        }
    }

    fn render_completed_project_rules(
        &self,
        result: &InitActionResult,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let InitActionResult::ProjectScopedRules(rules_result) = result else {
            return Empty::new().finish();
        };

        let StepState::ProjectRules { mouse_states, .. } = &self.state else {
            return Empty::new().finish();
        };

        let appearance = Appearance::as_ref(app);

        let init_completed = self.model.as_ref(app).is_completed();
        match rules_result {
            ProjectScopedRulesResult::LinkedFromExisting(path) => {
                Self::render_success_completion(&format!("Project rules linked from {path}"), app)
            }
            ProjectScopedRulesResult::GenerateNew {
                button_disabled, ..
            } => {
                let mut action = RenderableAction::new("Project rules configured", app)
                    .with_icon(Icon::Check.to_warpui_icon(Fill::success()).finish());
                if init_completed {
                    action = action.with_action_button(Self::regenerate_button(
                        &mouse_states.regenerate_button,
                        *button_disabled,
                        appearance,
                        app,
                    ));
                }
                action.with_content_item_spacing().render(app).finish()
            }
            ProjectScopedRulesResult::AlreadyExists { button_disabled } => {
                let mut action = RenderableAction::new("Project rules already configured", app)
                    .with_icon(Icon::Check.to_warpui_icon(Fill::success()).finish());
                if init_completed {
                    action = action.with_action_button(Self::regenerate_button(
                        &mouse_states.regenerate_button,
                        *button_disabled,
                        appearance,
                        app,
                    ));
                }
                action.with_content_item_spacing().render(app).finish()
            }
            ProjectScopedRulesResult::Skipped => {
                Self::render_skipped_completion("Project rules skipped", app)
            }
        }
    }

    fn spawn_server_installation(
        server_type: LSPServerType,
        repo_root: PathBuf,
        path_env_var: Option<String>,
        model: ModelHandle<InitProjectModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        let executor = lsp::CommandBuilder::new(path_env_var);
        let http_client =
            crate::server::server_api::ServerApiProvider::as_ref(ctx).get_http_client();

        ctx.spawn(
            async move {
                let candidate = server_type.candidate(http_client);
                let metadata = candidate.fetch_latest_server_metadata().await?;
                candidate.install(metadata, &executor).await?;
                Ok::<_, anyhow::Error>(())
            },
            move |_me, result, ctx| match result {
                Ok(()) => {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerInstallCompleted {
                            server_type: server_type.binary_name().to_string(),
                            success: true,
                        },
                        ctx
                    );

                    PersistedWorkspace::handle(ctx).update(ctx, |workspace, _| {
                        workspace.enable_lsp_server_for_path(&repo_root, server_type);
                    });

                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::success(format!(
                                "{} installed and enabled successfully.",
                                server_type.binary_name()
                            )),
                            window_id,
                            ctx,
                        );
                    });

                    model.update(ctx, |_, ctx| {
                        ctx.emit(InitProjectModelEvent::LanguageServerInstalledAndEnabled);
                    });
                }
                Err(e) => {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerInstallCompleted {
                            server_type: server_type.binary_name().to_string(),
                            success: false,
                        },
                        ctx
                    );

                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!(
                                "Failed to install {}: {e}",
                                server_type.binary_name()
                            )),
                            window_id,
                            ctx,
                        );
                    });
                }
            },
        );
    }
}

impl Entity for InitStepBlock {
    type Event = ();
}

impl View for InitStepBlock {
    fn ui_name() -> &'static str {
        "InitStepBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        match self.step_kind() {
            InitStepKind::Welcome => self.render_welcome(app),
            InitStepKind::CodebaseContext => self.render_codebase_context(app),
            InitStepKind::LanguageServers => self.render_language_servers(app),
            InitStepKind::ProjectScopedRules => self.render_project_rules(app),
            InitStepKind::CreateEnvironment => self.render_create_environment(app),
        }
    }
}

impl TypedActionView for InitStepBlock {
    type Action = InitProjectBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            InitProjectBlockAction::IndexCodebase(directory) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupCodebaseContextAction {
                        action: AgentModeSetupCodebaseContextActionType::IndexCodebase,
                    },
                    ctx
                );
                CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.index_directory(directory.clone(), ctx);
                });
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::CodebaseContext,
                        InitActionResult::CodebaseContext(CodebaseIndexingResult::Accepted),
                        ctx,
                    );
                });
            }
            InitProjectBlockAction::SkipIndex => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupCodebaseContextAction {
                        action: AgentModeSetupCodebaseContextActionType::SkipIndexing,
                    },
                    ctx
                );
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::CodebaseContext,
                        InitActionResult::CodebaseContext(CodebaseIndexingResult::Skipped),
                        ctx,
                    );
                });
            }
            InitProjectBlockAction::SetupLanguageServers {
                server_info,
                repo_path,
            } => {
                let repo_root = repo_path.clone();
                let mut enabled_servers = Vec::new();
                let mut servers_to_install = Vec::new();

                // Separate installed servers from those needing installation
                for info in server_info {
                    if info.is_installed {
                        PersistedWorkspace::handle(ctx).update(ctx, |workspace, _| {
                            workspace.enable_lsp_server_for_path(&repo_root, info.server_type);
                        });
                        enabled_servers.push(info.server_type);
                    } else {
                        servers_to_install.push(info.server_type);
                    }
                }

                // Send telemetry for each enabled server
                for server_type in enabled_servers.iter().chain(servers_to_install.iter()) {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerEnabled {
                            server_type: server_type.binary_name().to_string(),
                            source: LspEnablementSource::InitFlow,
                            needed_install: !enabled_servers.contains(server_type),
                        },
                        ctx
                    );
                }

                // Spawn installation tasks for uninstalled servers
                let model = self.model.clone();
                let path_env_var = self.model.as_ref(ctx).path_env_var().cloned();
                for server_type in &servers_to_install {
                    Self::spawn_server_installation(
                        *server_type,
                        repo_root.clone(),
                        path_env_var.clone(),
                        model.clone(),
                        ctx,
                    );
                }

                // Show toast for servers being installed in background
                if !servers_to_install.is_empty() {
                    let window_id = ctx.window_id();
                    let server_names: Vec<_> =
                        servers_to_install.iter().map(|s| s.binary_name()).collect();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::default(format!(
                                "Installing {} in background...",
                                server_names.join(", ")
                            )),
                            window_id,
                            ctx,
                        );
                    });
                }

                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::LanguageServers,
                        InitActionResult::LanguageServers(
                            if enabled_servers.is_empty() && servers_to_install.is_empty() {
                                LanguageServersResult::Skipped
                            } else {
                                LanguageServersResult::Accepted {
                                    enabled_servers,
                                    servers_to_install,
                                }
                            },
                        ),
                        ctx,
                    );
                });
            }
            InitProjectBlockAction::SkipLanguageServers => {
                send_telemetry_from_ctx!(LspTelemetryEvent::ServerEnablementSkipped, ctx);
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::LanguageServers,
                        InitActionResult::LanguageServers(LanguageServersResult::Skipped),
                        ctx,
                    );
                });
            }
            InitProjectBlockAction::LinkFromExisting(path) => {
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_string();
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupProjectScopedRulesAction {
                        action: AgentModeSetupProjectScopedRulesActionType::LinkFromExisting(
                            file_name,
                        ),
                    },
                    ctx
                );

                // Create symlink in background
                #[cfg(feature = "local_fs")]
                {
                    let path_clone = path.clone();
                    let root_path = self.model.as_ref(ctx).root_path().to_path_buf();
                    ctx.spawn(
                        async move { Self::create_symlink_to_agents_md(&path_clone, &root_path).await },
                        |_me, result, _ctx| {
                            if let Err(e) = result {
                                log::error!("Failed to create symlink to AGENTS.md: {e}");
                            }
                        },
                    );
                }

                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::ProjectScopedRules,
                        InitActionResult::ProjectScopedRules(
                            ProjectScopedRulesResult::LinkedFromExisting(
                                path.to_string_lossy().to_string(),
                            ),
                        ),
                        ctx,
                    );
                });
            }
            InitProjectBlockAction::GenerateRules => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupProjectScopedRulesAction {
                        action: AgentModeSetupProjectScopedRulesActionType::GenerateWarpMd,
                    },
                    ctx
                );
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_running(InitStepKind::ProjectScopedRules, ctx);
                    ctx.emit(InitProjectModelEvent::GenerateProjectRules);
                });
            }
            InitProjectBlockAction::RegenerateRules => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupProjectScopedRulesAction {
                        action: AgentModeSetupProjectScopedRulesActionType::RegenerateWarpMd,
                    },
                    ctx
                );
                self.model.update(ctx, |model, ctx| {
                    model.disable_regenerate_button();
                    ctx.emit(InitProjectModelEvent::RegenerateProjectRules);
                });
            }
            InitProjectBlockAction::SkipRules => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupProjectScopedRulesAction {
                        action: AgentModeSetupProjectScopedRulesActionType::SkipRules,
                    },
                    ctx
                );
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::ProjectScopedRules,
                        InitActionResult::ProjectScopedRules(ProjectScopedRulesResult::Skipped),
                        ctx,
                    );
                });
            }
            InitProjectBlockAction::ViewCodebaseContextStatus => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupCodebaseContextAction {
                        action: AgentModeSetupCodebaseContextActionType::ViewIndexStatus,
                    },
                    ctx
                );
                self.model.update(ctx, |_, ctx| {
                    ctx.emit(InitProjectModelEvent::ViewCodebaseContextStatus);
                });
            }
            InitProjectBlockAction::StartCreateEnvironment => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupCreateEnvironmentAction {
                        action: AgentModeSetupCreateEnvironmentActionType::CreateEnvironment,
                    },
                    ctx
                );
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_running(InitStepKind::CreateEnvironment, ctx);
                    ctx.emit(InitProjectModelEvent::CreateEnvironment);
                });
            }
            InitProjectBlockAction::SkipCreateEnvironment => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeSetupCreateEnvironmentAction {
                        action: AgentModeSetupCreateEnvironmentActionType::SkipEnvironment,
                    },
                    ctx
                );
                self.model.update(ctx, |model, ctx| {
                    model.mark_step_completed(
                        InitStepKind::CreateEnvironment,
                        InitActionResult::CreateEnvironment(CreateEnvironmentResult::Skipped),
                        ctx,
                    );
                });
            }
        }
        ctx.notify();
    }
}
