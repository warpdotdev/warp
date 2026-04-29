mod saved_prompts;
mod zero_state;

use ai::skills::SkillProvider;
pub(crate) use saved_prompts::*;
use warp_core::features::FeatureFlag;
pub use zero_state::*;

use std::collections::HashMap;
use std::path::PathBuf;

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::appearance::Appearance;
use warpui::fonts::FamilyId;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::skills::{SkillDescriptor, SkillManager};
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::slash_command_menu::fuzzy_match::SlashCommandFuzzyMatchResult;
use crate::search::slash_command_menu::static_commands::Availability;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::model::session::SessionType;
use warp_core::ui::Icon as WarpIcon;

use super::AcceptSlashCommandOrSavedPrompt;
use crate::{
    ai::blocklist::{
        agent_view::{AgentViewController, AgentViewControllerEvent},
        block::cli_controller::{CLISubagentController, CLISubagentEvent},
    },
    search::{
        slash_command_menu::{
            static_commands::commands::{self, COMMAND_REGISTRY},
            SlashCommandId, StaticCommand,
        },
        SyncDataSource,
    },
    settings::{AISettings, AISettingsChangedEvent, InputSettings, InputSettingsChangedEvent},
    terminal::model::session::active_session::{ActiveSession, ActiveSessionEvent},
    workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent},
};

pub struct DataSourceArgs {
    pub active_session: ModelHandle<ActiveSession>,
    pub agent_view_controller: ModelHandle<AgentViewController>,
    pub cli_subagent_controller: ModelHandle<CLISubagentController>,
    pub terminal_view_id: EntityId,
}

pub struct SlashCommandDataSource {
    active_session: ModelHandle<ActiveSession>,
    agent_view_controller: ModelHandle<AgentViewController>,
    cli_subagent_controller: ModelHandle<CLISubagentController>,
    terminal_view_id: EntityId,
    active_commands_by_id: HashMap<SlashCommandId, StaticCommand>,
    active_repo_root: Option<PathBuf>,
}

impl SlashCommandDataSource {
    pub fn new(args: DataSourceArgs, ctx: &mut ModelContext<Self>) -> Self {
        let DataSourceArgs {
            active_session,
            agent_view_controller,
            cli_subagent_controller,
            terminal_view_id,
        } = args;
        ctx.subscribe_to_model(&active_session, |me, event, ctx| match event {
            ActiveSessionEvent::UpdatedPwd | ActiveSessionEvent::Bootstrapped => {
                me.recompute_active_commands(ctx);
            }
        });
        ctx.subscribe_to_model(&cli_subagent_controller, |me, event, ctx| {
            if let CLISubagentEvent::SpawnedSubagent { .. }
            | CLISubagentEvent::FinishedSubagent { .. }
            | CLISubagentEvent::UpdatedControl { .. } = event
            {
                me.recompute_active_commands(ctx);
            }
        });
        ctx.subscribe_to_model(&agent_view_controller, |me, event, ctx| match event {
            AgentViewControllerEvent::EnteredAgentView { .. }
            | AgentViewControllerEvent::ExitedAgentView { .. } => {
                me.recompute_active_commands(ctx);
            }
            _ => (),
        });
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::IsAnyAIEnabled { .. }
                    | AISettingsChangedEvent::OrchestrationEnabled { .. }
            ) {
                me.recompute_active_commands(ctx);
            }
        });
        ctx.subscribe_to_model(&InputSettings::handle(ctx), |me, event, ctx| {
            if matches!(
                event,
                InputSettingsChangedEvent::EnableSlashCommandsInTerminal { .. }
            ) {
                me.recompute_active_commands(ctx);
            }
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, event, ctx| {
            if matches!(event, UserWorkspacesEvent::CodebaseContextEnablementChanged) {
                me.recompute_active_commands(ctx);
            }
        });
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |me, event, ctx| {
                if let CLIAgentSessionsModelEvent::InputSessionChanged {
                    terminal_view_id: event_terminal_view_id,
                    ..
                } = event
                {
                    if *event_terminal_view_id == terminal_view_id {
                        me.recompute_active_commands(ctx);
                    }
                }
            },
        );

        let mut me = Self {
            active_session,
            agent_view_controller,
            cli_subagent_controller,
            terminal_view_id,
            active_commands_by_id: Default::default(),
            active_repo_root: None,
        };
        me.recompute_active_commands(ctx);
        me
    }

    /// Slash commands that are available in CLI agent rich input mode.
    /// Add command names here to make them accessible when composing prompts
    /// for a running CLI agent (Claude Code, Codex, etc.).
    const CLI_AGENT_INPUT_ALLOWED_COMMANDS: &[&str] = &["/prompts", "/skills"];

    fn recompute_active_commands(&mut self, ctx: &mut ModelContext<Self>) {
        let is_cli_agent_input = self.is_cli_agent_input_open(ctx);

        let mut session_context = Availability::empty();

        let is_agent_view_active = self.agent_view_controller.as_ref(ctx).is_active();
        if !FeatureFlag::AgentView.is_enabled() {
            // When the AgentView feature flag is disabled, set both view bits so that
            // either view requirement is satisfied (but other requirements like
            // REPOSITORY and LOCAL still apply).
            session_context |= Availability::AGENT_VIEW | Availability::TERMINAL_VIEW;
        } else if is_agent_view_active {
            session_context |= Availability::AGENT_VIEW;
        } else {
            session_context |= Availability::TERMINAL_VIEW;
        }

        if self.active_repo_root.is_some() {
            session_context |= Availability::REPOSITORY;
        }

        let is_local = self
            .active_session
            .as_ref(ctx)
            .session_type(ctx)
            .is_some_and(|st| st == SessionType::Local);
        if is_local {
            session_context |= Availability::LOCAL;
        }

        if !self
            .cli_subagent_controller
            .as_ref(ctx)
            .is_agent_in_control()
        {
            session_context |= Availability::NO_LRC_CONTROL;
        }

        let has_active_conversation = if is_agent_view_active {
            // There is always an active conversation in the agent view.
            true
        } else {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(self.terminal_view_id)
                .is_some()
        };
        if has_active_conversation {
            session_context |= Availability::ACTIVE_CONVERSATION;
        }

        if UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx) {
            session_context |= Availability::CODEBASE_CONTEXT;
        }

        if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
            session_context |= Availability::AI_ENABLED;
        }

        let is_orchestration_enabled = AISettings::as_ref(ctx).is_orchestration_enabled(ctx);

        let old_active_command_count = self.active_commands_by_id.len();
        self.active_commands_by_id = HashMap::from_iter(
            COMMAND_REGISTRY
                .all_commands_by_id()
                .filter(|(_, command)| command.is_active(session_context))
                .filter(|(_, command)| {
                    command.name != commands::ORCHESTRATE_NAME || is_orchestration_enabled
                })
                // The static `/feedback` command is an AI-off fallback for the richer bundled
                // `feedback` skill. Hide it whenever the bundled skill will actually take over,
                // matching the precedence used by `Workspace::send_feedback`.
                .filter(|(_, command)| {
                    command.name != commands::FEEDBACK.name
                        || !crate::workspace::is_feedback_skill_available(ctx)
                })
                // When CLI agent input is open, restrict to the explicit allowlist.
                .filter(|(_, command)| {
                    !is_cli_agent_input
                        || Self::CLI_AGENT_INPUT_ALLOWED_COMMANDS.contains(&command.name)
                })
                .map(|(id, command)| (id, command.clone())),
        );

        // This is an imperfect heuristic, but better than re-firing unnecessarily.
        //
        // If it actually matters, we can update it.
        if self.active_commands_by_id.len() != old_active_command_count {
            ctx.emit(UpdatedActiveCommands);
        }
    }

    /// Update the active repository root for this terminal. Called by the parent when
    /// the terminal navigates into or out of a git repository.
    pub fn set_active_repo_root(
        &mut self,
        repo_root: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.active_repo_root != repo_root {
            self.active_repo_root = repo_root;
            self.recompute_active_commands(ctx);
        }
    }

    pub fn active_commands(&self) -> impl Iterator<Item = (&SlashCommandId, &StaticCommand)> {
        self.active_commands_by_id.iter()
    }

    pub fn is_agent_view_active(&self, ctx: &AppContext) -> bool {
        self.agent_view_controller.as_ref(ctx).is_active()
    }

    /// Returns `true` if the CLI agent rich input is currently open for this terminal.
    pub fn is_cli_agent_input_open(&self, ctx: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id)
    }

    /// Returns the supported skill providers for the active CLI agent, or `None` if
    /// CLI agent input is not open (meaning no filtering should be applied).
    pub fn active_cli_agent_providers(
        &self,
        ctx: &AppContext,
    ) -> Option<&'static [ai::skills::SkillProvider]> {
        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.terminal_view_id)
            .filter(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
            .map(|s| s.agent.supported_skill_providers())
    }
}

impl SyncDataSource for SlashCommandDataSource {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn run_query(
        &self,
        query: &Query,
        app: &warpui::AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if query.text.is_empty() {
            return Ok(vec![]);
        }

        let query_text = query.text.trim().to_lowercase();

        let mut results = Vec::new();

        /// Multiplier to ensure static commands always appear at the top of the match results.
        const SCORE_MULTIPLIER: OrderedFloat<f64> = OrderedFloat(1000.0);

        for (id, command) in self.active_commands_by_id.iter() {
            if let Some(fuzzy_result) = SlashCommandFuzzyMatchResult::try_match(
                &query_text,
                command.name,
                None, // Don't match on description for slash commands.
            ) {
                let score = fuzzy_result.score();

                // Only include results with score > 25 once the user has started typing a query and is past the first character
                if query_text.len() > 1 && score <= 25.0 {
                    continue;
                }

                // Boost prefix matches so that closer matches (e.g. "new" → "/new")
                // rank above longer fuzzy matches (e.g. "new" → "/create-new-project").
                let prefix_boost = prefix_match_bonus(&query_text, command.name);

                results.push(QueryResult::from(
                    InlineItem::from_slash_command(id, command, app)
                        .with_name_match_result(fuzzy_result.name_match_result)
                        .with_description_match_result(fuzzy_result.description_match_result)
                        .with_score(
                            OrderedFloat(score) * SCORE_MULTIPLIER
                                + OrderedFloat(prefix_boost) * SCORE_MULTIPLIER
                                // Boost commands with shorter names, if match result is otherwise
                                // equal.
                                + OrderedFloat(1. / command.name.len() as f64),
                        ),
                ));
            }
        }

        // Also search skills — when CLI agent input is open, filter to natively supported providers.
        // Skills are invoked by the agent, so they're hidden entirely when AI is globally off.
        if FeatureFlag::ListSkills.is_enabled() && AISettings::as_ref(app).is_any_ai_enabled(app) {
            let cli_agent_providers = self.active_cli_agent_providers(app);
            let cwd = self.active_session.as_ref(app).current_working_directory();
            let cwd_path = cwd.as_ref().map(std::path::Path::new);
            let skills = SkillManager::handle(app)
                .as_ref(app)
                .get_skills_for_working_directory(cwd_path, app);

            let skill_manager = SkillManager::as_ref(app);
            for mut skill in skills {
                // In CLI agent input mode, only show skills that exist in a supported
                // provider folder. We check all paths (not just the deduplicated
                // provider) because deduplication may have picked a higher-priority
                // provider even when the skill also exists in the CLI agent's folder.
                if let Some(providers) = &cli_agent_providers {
                    if !skill_manager.skill_exists_for_any_provider(&skill, providers) {
                        continue;
                    }
                    // Re-map the provider to the best supported one so the icon
                    // reflects the active CLI agent's native provider.
                    skill.provider = skill_manager.best_supported_provider(&skill, providers);
                }
                if let Some(fuzzy_result) = SlashCommandFuzzyMatchResult::try_match(
                    &query_text,
                    &skill.name,
                    Some(&skill.description),
                ) {
                    let score = fuzzy_result.score();

                    // Only include results with score > 25 once the user has started typing a query
                    if query_text.len() > 1 && score <= 25.0 {
                        continue;
                    }

                    let prefix_boost = prefix_match_bonus(&query_text, &skill.name);

                    results.push(QueryResult::from(
                        InlineItem::from_skill(&skill, app)
                            .with_name_match_result(fuzzy_result.name_match_result)
                            .with_description_match_result(fuzzy_result.description_match_result)
                            .with_score(
                                OrderedFloat(score) * SCORE_MULTIPLIER
                                    + OrderedFloat(prefix_boost) * SCORE_MULTIPLIER
                                    + OrderedFloat(1. / skill.name.len() as f64),
                            ),
                    ));
                }
            }
        }

        Ok(results)
    }
}

/// Computes a bonus score for slash command matches where the query is a prefix
/// of the command name. This ensures closer matches (e.g., "new" → "/new") rank
/// above longer fuzzy matches (e.g., "new" → "/figma-create-new-file").
///
/// Returns a value in `[0.0, 100.0]` based on the query's coverage of the name.
/// An exact match yields the maximum bonus of 100; partial prefix matches yield
/// a proportionally smaller bonus.
fn prefix_match_bonus(query: &str, name: &str) -> f64 {
    let name_lower = name.to_lowercase();
    let name_stripped = name_lower.strip_prefix('/').unwrap_or(&name_lower);
    if name_stripped.starts_with(query) {
        // coverage = 1.0 for exact match, smaller for partial prefix match.
        let coverage = query.len() as f64 / name_stripped.len() as f64;
        coverage * 100.0
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UpdatedActiveCommands;

impl Entity for SlashCommandDataSource {
    type Event = UpdatedActiveCommands;
}

#[derive(Debug, Clone)]
pub struct InlineItem {
    pub action: AcceptSlashCommandOrSavedPrompt,
    pub icon_path: &'static str,
    pub name: String,
    pub description: Option<String>,
    pub font_family: FamilyId,
    pub name_match_result: Option<FuzzyMatchResult>,
    pub description_match_result: Option<FuzzyMatchResult>,
    pub score: OrderedFloat<f64>,
}

impl InlineItem {
    fn from_slash_command(
        command_id: &SlashCommandId,
        command: &StaticCommand,
        app: &AppContext,
    ) -> Self {
        let appearance = Appearance::as_ref(app);
        Self {
            action: AcceptSlashCommandOrSavedPrompt::SlashCommand { id: *command_id },
            icon_path: command.icon_path,
            name: command.name.to_owned(),
            description: Some(command.description.to_owned()),
            font_family: appearance.monospace_font_family(),
            name_match_result: None,
            description_match_result: None,
            score: OrderedFloat(f64::MIN),
        }
    }

    pub(super) fn from_skill(skill: &SkillDescriptor, app: &AppContext) -> Self {
        let appearance = Appearance::handle(app).as_ref(app);
        // Use icon_override if set (e.g. Figma skills), otherwise derive from provider.
        let icon = if let Some(override_icon) = skill.icon_override {
            override_icon
        } else {
            match skill.provider {
                SkillProvider::Warp => WarpIcon::Warp,
                SkillProvider::Claude => WarpIcon::ClaudeLogo,
                SkillProvider::Codex => WarpIcon::OpenAILogo,
                SkillProvider::Gemini => WarpIcon::GeminiLogo,
                SkillProvider::Droid => WarpIcon::DroidLogo,
                SkillProvider::OpenCode => WarpIcon::OpenCodeLogo,
                _ => WarpIcon::Warp,
            }
        };

        Self {
            action: AcceptSlashCommandOrSavedPrompt::Skill {
                reference: skill.reference.clone(),
                name: skill.name.clone(),
            },
            icon_path: icon.into(),
            name: format!("/{}", &skill.name),
            description: Some(skill.description.clone()),
            font_family: appearance.monospace_font_family(),
            name_match_result: None,
            description_match_result: None,
            score: OrderedFloat(f64::MIN),
        }
    }

    fn with_name_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.name_match_result = result;
        self
    }

    fn with_description_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.description_match_result = result;
        self
    }

    fn with_score(mut self, score: OrderedFloat<f64>) -> Self {
        self.score = score;
        self
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
