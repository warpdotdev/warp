use std::path::{Path, PathBuf};

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use ai::project_context::model::ProjectContextModel;
use enum_iterator::Sequence;
use lsp::supported_servers::LSPServerType;
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(not(target_family = "wasm"))]
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, ModelContext, SingletonEntity as _};

use crate::{
    ai::persisted_workspace::PersistedWorkspace,
    settings::CodeSettings,
    terminal::view::init_project::{
        lsp_server_selector::LSPServerInfo, CodebaseIndexingResult, CreateEnvironmentResult,
        InitActionResult, LanguageServersResult, ProjectScopedRulesResult, FILES_TO_CHECK,
        LINKABLE_FILES,
    },
    workspaces::user_workspaces::UserWorkspaces,
};

const INIT_STEP_COUNT: usize = enum_iterator::cardinality::<InitStepKind>();

/// Steps in fixed order - index determines display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Sequence)]
#[repr(usize)]
pub enum InitStepKind {
    Welcome = 0,
    CodebaseContext = 1,
    LanguageServers = 2,
    ProjectScopedRules = 3,
    CreateEnvironment = 4,
}

/// Data needed for views to render ready steps, used to create [`InitStepBlock`]s
#[derive(Clone, Debug)]
pub enum InitStepData {
    CodebaseContext {
        pwd_path: PathBuf,
    },
    LanguageServers {
        servers: Vec<LSPServerInfo>,
        repo_path: PathBuf,
    },
    ProjectScopedRules {
        linkable_files: Vec<PathBuf>,
    },
    CreateEnvironment,
}

/// Status of a step in the /init flow
pub enum InitStepStatus {
    /// Async computation in progress (determining if step is needed)
    Pending,
    /// Ready for user interaction (contains data for view to render)
    Ready(InitStepData),
    /// User initiated action, e.g. AI generating WARP.md
    Running,
    /// Done (accepted, skipped, or auto-completed)
    Completed(InitActionResult),
}

impl InitStepStatus {
    pub fn is_pending(&self) -> bool {
        matches!(self, InitStepStatus::Pending)
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, InitStepStatus::Completed(_))
    }

    pub fn is_running(&self) -> bool {
        matches!(self, InitStepStatus::Running)
    }
}

pub struct InitStep {
    pub kind: InitStepKind,
    pub status: InitStepStatus,
}

impl InitStep {
    fn new_pending(kind: InitStepKind) -> Self {
        Self {
            kind,
            status: InitStepStatus::Pending,
        }
    }

    fn new_ready(kind: InitStepKind, data: InitStepData) -> Self {
        Self {
            kind,
            status: InitStepStatus::Ready(data),
        }
    }

    fn new_completed(kind: InitStepKind, result: InitActionResult) -> Self {
        Self {
            kind,
            status: InitStepStatus::Completed(result),
        }
    }
}

pub struct InitProjectModel {
    /// Fixed-size array of steps. None = step disabled/not applicable.
    /// Index corresponds to InitStepKind discriminant.
    steps: [Option<InitStep>; INIT_STEP_COUNT],
    /// Index of current step shown to user
    current_step_index: usize,
    /// Whether the /init was cancelled
    is_cancelled: bool,
    /// If project already has all setup done before /init started
    is_already_setup: bool,
    /// Root path for this /init session
    #[cfg(feature = "local_fs")]
    root_path: PathBuf,
    path_env_var: Option<String>,
}

impl InitProjectModel {
    /// Create new InitProjectModel
    pub fn new(
        pwd_path: PathBuf,
        path_env_var: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let is_already_setup = !Self::should_have_available_steps(&pwd_path, ctx);

        Self {
            steps: [None, None, None, None, None],
            current_step_index: 0,
            is_cancelled: false,
            is_already_setup,
            #[cfg(feature = "local_fs")]
            root_path: pwd_path,
            path_env_var,
        }
    }

    /// Start the /init flow: compute steps, emit welcome step, progress to next
    pub fn start(&mut self, ctx: &mut ModelContext<Self>) {
        #[cfg(feature = "local_fs")]
        let pwd_path = self.root_path.clone();
        #[cfg(not(feature = "local_fs"))]
        let pwd_path = PathBuf::new();

        // Welcome step is always Completed immediately (no async needed)
        self.set_step(
            InitStepKind::Welcome,
            Some(InitStep::new_completed(
                InitStepKind::Welcome,
                InitActionResult::Welcome,
            )),
        );

        // Start async computations for subsequent steps
        self.compute_codebase_context_step(&pwd_path, ctx);
        if self.path_env_var.is_some() {
            self.compute_language_servers_step(&pwd_path, ctx);
        }
        self.compute_project_scoped_rules_step(&pwd_path, ctx);

        // CreateEnvironment step is always Ready (no async computation)
        self.set_step(
            InitStepKind::CreateEnvironment,
            Some(InitStep::new_ready(
                InitStepKind::CreateEnvironment,
                InitStepData::CreateEnvironment,
            )),
        );

        // Emit welcome step immediately, then progress to next
        ctx.emit(InitProjectModelEvent::InsertStep(InitStepKind::Welcome));
        self.maybe_emit_next_step(ctx);
    }

    /// Check if there are any steps that need user action
    pub fn should_have_available_steps(path: &Path, ctx: &warpui::AppContext) -> bool {
        // Note that we consider auto-indexing setting to true to satisfy the codebase context step.
        // This avoids the potential race condition with the banner showing just when we start auto-indexing.
        let has_pending_codebase_context = UserWorkspaces::as_ref(ctx)
            .is_codebase_context_enabled(ctx)
            && CodebaseIndexManager::as_ref(ctx)
                .get_codebase_index_status_for_path(path, ctx)
                .is_none()
            && !*CodeSettings::as_ref(ctx).auto_indexing_enabled;

        let has_pending_project_scoped_rules = ProjectContextModel::as_ref(ctx)
            .find_applicable_project_rules(path)
            .is_none();

        has_pending_codebase_context || has_pending_project_scoped_rules
    }

    pub fn get_step(&self, kind: InitStepKind) -> Option<&InitStep> {
        self.steps
            .get(kind as usize)
            .and_then(|maybe_step| maybe_step.as_ref())
    }

    fn get_step_mut(&mut self, kind: InitStepKind) -> Option<&mut InitStep> {
        self.steps
            .get_mut(kind as usize)
            .and_then(|maybe_step| maybe_step.as_mut())
    }

    fn set_step(&mut self, kind: InitStepKind, step: Option<InitStep>) {
        if let Some(slot) = self.steps.get_mut(kind as usize) {
            *slot = step;
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled
    }

    pub fn is_already_setup(&self) -> bool {
        self.is_already_setup
    }

    pub fn is_completed(&self) -> bool {
        if self.is_cancelled {
            return true;
        }
        // All non-None steps must be Completed
        self.steps.iter().all(|step| {
            step.as_ref()
                .map(|s| s.status.is_completed())
                .unwrap_or(true)
        })
    }

    /// Returns true if /init is still active (not completed and not cancelled)
    pub fn is_active(&self) -> bool {
        !self.is_completed() && !self.is_cancelled
    }

    #[cfg(feature = "local_fs")]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn path_env_var(&self) -> Option<&String> {
        self.path_env_var.as_ref()
    }

    /// Mark a step as completed with the given result
    pub fn mark_step_completed(
        &mut self,
        kind: InitStepKind,
        result: InitActionResult,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(step) = self.get_step_mut(kind) {
            step.status = InitStepStatus::Completed(result);
            ctx.emit(InitProjectModelEvent::StepCompleted(kind));
            self.maybe_emit_next_step(ctx);
        }
    }

    /// Mark a step as Running
    pub fn mark_step_running(&mut self, kind: InitStepKind, ctx: &mut ModelContext<Self>) {
        if let Some(step) = self.get_step_mut(kind) {
            step.status = InitStepStatus::Running;
            ctx.notify();
        }
    }

    /// Disable the regenerate button for project scoped rules (after user clicks it)
    pub fn disable_regenerate_button(&mut self) {
        if let Some(step) = self.get_step_mut(InitStepKind::ProjectScopedRules) {
            match &mut step.status {
                InitStepStatus::Completed(InitActionResult::ProjectScopedRules(
                    ProjectScopedRulesResult::GenerateNew {
                        button_disabled, ..
                    },
                ))
                | InitStepStatus::Completed(InitActionResult::ProjectScopedRules(
                    ProjectScopedRulesResult::AlreadyExists { button_disabled },
                )) => {
                    *button_disabled = true;
                }
                _ => {}
            }
        }
    }

    /// Cancel the /init flow. This will notify subscribed blocks to rerender.
    pub fn cancel(&mut self, ctx: &mut ModelContext<Self>) {
        self.is_cancelled = true;

        // Mark Ready or Running steps as cancelled/skipped
        for step in self.steps.iter_mut().flatten() {
            if matches!(
                step.status,
                InitStepStatus::Ready(_) | InitStepStatus::Running
            ) {
                let skipped_result = match step.kind {
                    InitStepKind::Welcome => continue,
                    InitStepKind::CodebaseContext => {
                        InitActionResult::CodebaseContext(CodebaseIndexingResult::Skipped)
                    }
                    InitStepKind::LanguageServers => {
                        InitActionResult::LanguageServers(LanguageServersResult::Skipped)
                    }
                    InitStepKind::ProjectScopedRules => {
                        InitActionResult::ProjectScopedRules(ProjectScopedRulesResult::Skipped)
                    }
                    InitStepKind::CreateEnvironment => {
                        InitActionResult::CreateEnvironment(CreateEnvironmentResult::Skipped)
                    }
                };
                step.status = InitStepStatus::Completed(skipped_result);
            }
        }

        ctx.emit(InitProjectModelEvent::Cancelled);
    }

    /// Check if next step should be emitted, emit if ready.
    /// Only emits if current step is Completed (or None).
    fn maybe_emit_next_step(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_cancelled {
            return;
        }

        // Check current step is Completed or None before advancing
        let current_step_completed = self.steps[self.current_step_index]
            .as_ref()
            .map(|s| s.status.is_completed())
            .unwrap_or(true);

        if !current_step_completed {
            return;
        }

        let next_index = self.current_step_index + 1;
        if next_index >= INIT_STEP_COUNT {
            ctx.emit(InitProjectModelEvent::InitCompleted);
            return;
        }

        match &self.steps[next_index] {
            None => {
                // Step disabled/not applicable, skip to next
                self.current_step_index = next_index;
                self.maybe_emit_next_step(ctx);
            }
            Some(step) if !step.status.is_pending() => {
                // Ready to show (or auto-completed)
                self.current_step_index = next_index;
                ctx.emit(InitProjectModelEvent::InsertStep(step.kind));

                // If auto-completed, immediately try next step
                if step.status.is_completed() {
                    self.maybe_emit_next_step(ctx);
                }
            }
            Some(_) => {
                // Still Pending, wait for async to finish
            }
        }
    }

    fn compute_codebase_context_step(&mut self, pwd_path: &Path, ctx: &mut ModelContext<Self>) {
        if !UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx) {
            // Feature disabled, leave as None
            return;
        }

        let codebase_index_manager = CodebaseIndexManager::handle(ctx);
        let is_indexed = codebase_index_manager
            .as_ref(ctx)
            .get_codebase_index_status_for_path(pwd_path, ctx)
            .is_some();

        if is_indexed {
            // Already indexed, mark as completed
            self.set_step(
                InitStepKind::CodebaseContext,
                Some(InitStep::new_completed(
                    InitStepKind::CodebaseContext,
                    InitActionResult::CodebaseContext(CodebaseIndexingResult::Accepted),
                )),
            );
        } else {
            // Ready for user interaction
            self.set_step(
                InitStepKind::CodebaseContext,
                Some(InitStep::new_ready(
                    InitStepKind::CodebaseContext,
                    InitStepData::CodebaseContext {
                        pwd_path: pwd_path.to_path_buf(),
                    },
                )),
            );
        }
    }

    fn compute_language_servers_step(&mut self, pwd_path: &Path, ctx: &mut ModelContext<Self>) {
        // Start as Pending
        self.set_step(
            InitStepKind::LanguageServers,
            Some(InitStep::new_pending(InitStepKind::LanguageServers)),
        );

        let pwd_path = pwd_path.to_path_buf();
        #[cfg(not(target_family = "wasm"))]
        let repo_root = DetectedRepositories::as_ref(ctx)
            .get_root_for_path(&LocalOrRemotePath::Local(pwd_path.clone()))
            .and_then(|r| r.to_local_path().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| pwd_path.clone());
        #[cfg(target_family = "wasm")]
        let repo_root = pwd_path.clone();
        let repo_root_for_callback = repo_root.clone();
        let executor = lsp::CommandBuilder::new(self.path_env_var.clone());
        let http_client =
            crate::server::server_api::ServerApiProvider::as_ref(ctx).get_http_client();

        ctx.spawn(
            async move {
                let mut relevant_servers = Vec::new();
                for server_type in LSPServerType::all() {
                    let candidate = server_type.candidate(http_client.clone());
                    let should_suggest = candidate
                        .should_suggest_for_repo(&repo_root, &executor)
                        .await;

                    if should_suggest {
                        let is_installed = candidate.is_installed(&executor).await;

                        relevant_servers.push(LSPServerInfo {
                            server_type,
                            is_installed,
                        });
                    }
                }
                relevant_servers
            },
            move |me, relevant_servers, ctx| {
                let repo_root = repo_root_for_callback;

                if relevant_servers.is_empty() {
                    // No relevant servers, mark step as None (skip it)
                    me.set_step(InitStepKind::LanguageServers, None);
                    me.maybe_emit_next_step(ctx);
                    return;
                }

                // Check if already enabled and filter
                let enabled_server_types: Vec<LSPServerType> = {
                    let enabled_lsp_servers =
                        PersistedWorkspace::as_ref(ctx).enabled_lsp_servers(&pwd_path);

                    enabled_lsp_servers
                        .map(|servers| servers.collect())
                        .unwrap_or_default()
                };

                let filtered_servers: Vec<LSPServerInfo> = relevant_servers
                    .into_iter()
                    .filter(|info| !enabled_server_types.contains(&info.server_type))
                    .collect();

                if filtered_servers.is_empty() {
                    // All relevant servers already enabled, auto-complete
                    me.set_step(
                        InitStepKind::LanguageServers,
                        Some(InitStep::new_completed(
                            InitStepKind::LanguageServers,
                            InitActionResult::LanguageServers(LanguageServersResult::Accepted {
                                enabled_servers: enabled_server_types,
                                servers_to_install: Vec::new(),
                            }),
                        )),
                    );
                } else {
                    // Ready for user interaction
                    me.set_step(
                        InitStepKind::LanguageServers,
                        Some(InitStep::new_ready(
                            InitStepKind::LanguageServers,
                            InitStepData::LanguageServers {
                                servers: filtered_servers,
                                repo_path: repo_root,
                            },
                        )),
                    );
                }

                me.maybe_emit_next_step(ctx);
            },
        );
    }

    fn compute_project_scoped_rules_step(&mut self, pwd_path: &Path, ctx: &mut ModelContext<Self>) {
        // Start as Pending
        self.set_step(
            InitStepKind::ProjectScopedRules,
            Some(InitStep::new_pending(InitStepKind::ProjectScopedRules)),
        );

        let path_to_check: Vec<PathBuf> = FILES_TO_CHECK
            .iter()
            .chain(LINKABLE_FILES.iter())
            .map(|name| pwd_path.join(name))
            .collect();

        ctx.spawn(
            async move {
                let mut exists = vec![];
                for path in path_to_check {
                    if path.exists() {
                        exists.push(path);
                    }
                }
                exists
            },
            move |me, existing_files, ctx| {
                let has_agents_md = existing_files.iter().any(|p| {
                    p.file_name()
                        .map(|n| {
                            let name = n.to_string_lossy().to_lowercase();
                            name == "agents.md" || name == "warp.md"
                        })
                        .unwrap_or(false)
                });

                if has_agents_md {
                    // Already has AGENTS.md or WARP.md, mark as completed
                    me.set_step(
                        InitStepKind::ProjectScopedRules,
                        Some(InitStep::new_completed(
                            InitStepKind::ProjectScopedRules,
                            InitActionResult::ProjectScopedRules(
                                ProjectScopedRulesResult::AlreadyExists {
                                    button_disabled: false,
                                },
                            ),
                        )),
                    );
                } else {
                    // Collect linkable files
                    let linkable_files: Vec<PathBuf> = existing_files
                        .into_iter()
                        .filter(|p| {
                            p.file_name()
                                .map(|n| {
                                    let name = n.to_string_lossy();
                                    LINKABLE_FILES.iter().any(|lf| name.ends_with(lf))
                                })
                                .unwrap_or(false)
                        })
                        .collect();

                    me.set_step(
                        InitStepKind::ProjectScopedRules,
                        Some(InitStep::new_ready(
                            InitStepKind::ProjectScopedRules,
                            InitStepData::ProjectScopedRules { linkable_files },
                        )),
                    );
                }

                me.maybe_emit_next_step(ctx);
            },
        );
    }
}

/// Events emitted by InitProjectModel
#[derive(Debug, Clone)]
pub enum InitProjectModelEvent {
    /// Insert a new step block into the terminal view
    InsertStep(InitStepKind),
    /// A step was completed
    StepCompleted(InitStepKind),
    /// The /init flow was cancelled
    Cancelled,
    /// All steps completed
    InitCompleted,
    /// Trigger AGENTS.md generation slash command
    GenerateProjectRules,
    /// Trigger AGENTS.md regeneration
    RegenerateProjectRules,
    /// View codebase context status
    ViewCodebaseContextStatus,
    /// Language server installed and enabled
    LanguageServerInstalledAndEnabled,
    /// Trigger create environment slash command
    CreateEnvironment,
    /// Cloud environment was created
    EnvironmentCreated,
}

impl Entity for InitProjectModel {
    type Event = InitProjectModelEvent;
}
