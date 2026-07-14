use ai::skills::{SkillProvider, SkillReference, SkillScope};
use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use ordered_float::OrderedFloat;
use warp_core::ui::icons::Icon;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, EntityId, SingletonEntity as _};

use crate::ai::skills::SkillManager;
use crate::terminal::cli_agent_sessions::{CLIAgentInputState, CLIAgentSessionsModel};
pub const LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE: &str =
    "Local skills cannot run on a remote machine. Try forking the conversation locally and running the skill.";

/// Surface-neutral skill selection result shared by GUI and TUI menus.
#[derive(Clone)]
pub struct SelectableSkill {
    pub name: String,
    pub reference: SkillReference,
    pub description: String,
    pub scope: SkillScope,
    pub provider: SkillProvider,
    pub icon_override: Option<Icon>,
    pub name_match_result: Option<FuzzyMatchResult>,
    pub score: OrderedFloat<f64>,
}

/// Returns skills available for selection in the active input surface.
///
/// This owns the shared discovery, CLI-agent provider filtering, bundled-skill
/// policy, fuzzy matching, and ordering used by both frontend adapters.
pub fn query_selectable_skills(
    working_directory: Option<&LocalOrRemotePath>,
    terminal_view_id: EntityId,
    include_bundled: bool,
    query_text: &str,
    app: &AppContext,
) -> Vec<SelectableSkill> {
    let cli_agent_providers = CLIAgentSessionsModel::as_ref(app)
        .session(terminal_view_id)
        .filter(|session| matches!(session.input_state, CLIAgentInputState::Open { .. }))
        .map(|session| session.agent.supported_skill_providers());
    let skill_manager = SkillManager::as_ref(app);
    let query_text = query_text.trim();
    let mut results = skill_manager
        .get_skills_for_working_directory(working_directory, app)
        .into_iter()
        .filter(|skill| {
            if let Some(providers) = &cli_agent_providers {
                skill_manager.skill_exists_for_any_provider(skill, providers)
            } else {
                include_bundled || skill.scope != SkillScope::Bundled
            }
        })
        .filter_map(|mut skill| {
            if let Some(providers) = &cli_agent_providers {
                skill.provider = skill_manager.best_supported_provider(&skill, providers);
            }

            let (name_match_result, score) = if query_text.is_empty() {
                (None, OrderedFloat(f64::MIN))
            } else {
                let match_result = match_indices_case_insensitive(skill.name.as_str(), query_text)?;
                if query_text.len() > 1 && match_result.score < 10 {
                    return None;
                }
                let score = OrderedFloat(match_result.score as f64);
                (Some(match_result), score)
            };

            Some(SelectableSkill {
                name: skill.name,
                reference: skill.reference,
                description: skill.description,
                scope: skill.scope,
                provider: skill.provider,
                icon_override: skill.icon_override,
                name_match_result,
                score,
            })
        })
        .collect::<Vec<_>>();

    // Inline menus render lower-ranked results first and select from the end.
    // Reverse alphabetical tie-breaking puts the alphabetically first skill at
    // the selected end of an unfiltered result list.
    results.sort_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| right.name.to_lowercase().cmp(&left.name.to_lowercase()))
    });
    results
}
