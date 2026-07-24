use itertools::Itertools;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelHandle, SingletonEntity};

use crate::ai::skills::SkillManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::settings::AISettings;
use crate::terminal::input::slash_commands::{
    AcceptSlashCommandOrSavedPrompt, GuiSlashCommandDataSource, InlineItem, SlashCommandDataSource,
    TuiSlashCommandDataSource,
};

pub struct GuiZeroStateDataSource {
    slash_command_data_source: ModelHandle<GuiSlashCommandDataSource>,
}

impl GuiZeroStateDataSource {
    pub fn new(slash_command_data_source: &ModelHandle<GuiSlashCommandDataSource>) -> Self {
        Self {
            slash_command_data_source: slash_command_data_source.clone(),
        }
    }
}

impl Entity for GuiZeroStateDataSource {
    type Event = ();
}

impl SyncDataSource for GuiZeroStateDataSource {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn run_query(
        &self,
        query: &Query,
        app: &warpui::AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if !query.text.is_empty() {
            return Ok(vec![]);
        }

        let source = self.slash_command_data_source.as_ref(app);
        let is_cloud_mode_v2 = source.is_cloud_mode_v2();
        let mut results = source.ordered_zero_state_commands(app);

        if is_cloud_mode_v2
            && FeatureFlag::ListSkills.is_enabled()
            && AISettings::as_ref(app).is_any_ai_enabled(app)
        {
            let cli_agent_providers = source.active_cli_agent_providers(app);
            let active_session = source.active_session().as_ref(app);
            let cwd = active_session.current_working_directory_location(app);
            let skill_manager_handle = SkillManager::handle(app);
            let skill_manager = skill_manager_handle.as_ref(app);
            let skills = skill_manager.get_skills_for_working_directory(cwd.as_ref(), app);

            for mut skill in skills
                .into_iter()
                .sorted_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase()))
            {
                if let Some(providers) = &cli_agent_providers {
                    if !skill_manager.skill_exists_for_any_provider(&skill, providers) {
                        continue;
                    }
                    skill.provider = skill_manager.best_supported_provider(&skill, providers);
                }
                results.push(InlineItem::from_skill(&skill, app));
            }
        }

        if is_cloud_mode_v2 && AISettings::as_ref(app).is_any_ai_enabled(app) {
            let saved_prompts: Vec<_> = CloudModel::as_ref(app)
                .get_all_active_workflows()
                .filter(|cw| cw.model().data.is_agent_mode_workflow())
                .sorted_by(|a, b| {
                    b.model()
                        .data
                        .name()
                        .to_lowercase()
                        .cmp(&a.model().data.name().to_lowercase())
                })
                .collect();
            for saved_prompt in saved_prompts {
                results.push(InlineItem::from_saved_prompt(saved_prompt, app));
            }
        }

        Ok(results
            .into_iter()
            .map(|item| item.with_compact_layout(is_cloud_mode_v2).into())
            .collect())
    }
}

pub struct TuiZeroStateDataSource {
    slash_command_data_source: ModelHandle<TuiSlashCommandDataSource>,
}

impl TuiZeroStateDataSource {
    pub fn new(slash_command_data_source: &ModelHandle<TuiSlashCommandDataSource>) -> Self {
        Self {
            slash_command_data_source: slash_command_data_source.clone(),
        }
    }
}

impl Entity for TuiZeroStateDataSource {
    type Event = ();
}

impl SyncDataSource for TuiZeroStateDataSource {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn run_query(
        &self,
        query: &Query,
        app: &warpui::AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if !query.text.is_empty() {
            return Ok(vec![]);
        }

        Ok(self
            .slash_command_data_source
            .as_ref(app)
            .ordered_zero_state_commands(app)
            .into_iter()
            .map(Into::into)
            .collect())
    }
}
