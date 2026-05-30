use itertools::Itertools;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelHandle, SingletonEntity};

use crate::ai::skills::SkillManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::slash_command_menu::static_commands::commands;
use crate::search::SyncDataSource;
use crate::settings::AISettings;
use crate::terminal::input::slash_commands::{
    AcceptSlashCommandOrSavedPrompt, InlineItem, SlashCommandDataSource,
};

pub struct ZeroStateDataSource {
    slash_command_data_source: ModelHandle<SlashCommandDataSource>,
    is_cloud_mode_v2: bool,
}

impl ZeroStateDataSource {
    pub fn new(
        slash_command_data_source: &ModelHandle<SlashCommandDataSource>,
        is_cloud_mode_v2: bool,
    ) -> Self {
        Self {
            slash_command_data_source: slash_command_data_source.clone(),
            is_cloud_mode_v2,
        }
    }
}

impl Entity for ZeroStateDataSource {
    type Event = ();
}

impl SyncDataSource for ZeroStateDataSource {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn run_query(
        &self,
        query: &Query,
        app: &warpui::AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if !query.text.is_empty() {
            return Ok(vec![]);
        }

        // This is kind of a convoluted way to explicitly order these commands after all others.
        //
        // DataSource implementations must return highest priority items last (results sorted in
        // ascending order of priority).
        //
        // The results construction below basically orders all active commands, sorted
        // alphabetically, except for the commands in this vec, which are explicitly appended
        // to all the other alphabetically sorted commands, in this order.
        let prioritized_commands = vec![
            &*commands::CREATE_ENVIRONMENT,
            &*commands::EDIT,
            &commands::CONVERSATIONS,
            &commands::PROMPTS,
            &*commands::PLAN,
            &commands::AGENT,
        ];

        let mut active_prioritized_commands = vec![];
        let mut results = vec![];

        for (active_command_id, active_command) in self
            .slash_command_data_source
            .as_ref(app)
            .active_commands()
            .sorted_by_key(|(_, command)| std::cmp::Reverse(&command.name))
        {
            if prioritized_commands
                .iter()
                .any(|prioritized_command| prioritized_command.name == active_command.name)
            {
                active_prioritized_commands.push((active_command_id, active_command));
            } else {
                results.push(
                    InlineItem::from_slash_command(active_command_id, active_command, app)
                        .with_compact_layout(self.is_cloud_mode_v2)
                        .into(),
                );
            }
        }

        for prioritized_command in prioritized_commands {
            if let Some((id, command)) = active_prioritized_commands
                .iter()
                .find(|(_, active_command)| active_command.name == prioritized_command.name)
            {
                results.push(
                    InlineItem::from_slash_command(id, command, app)
                        .with_compact_layout(self.is_cloud_mode_v2)
                        .into(),
                );
            }
        }

        if self.is_cloud_mode_v2
            && FeatureFlag::ListSkills.is_enabled()
            && AISettings::as_ref(app).is_any_ai_enabled(app)
        {
            let slash_command_data_source = self.slash_command_data_source.as_ref(app);
            let cli_agent_providers = slash_command_data_source.active_cli_agent_providers(app);
            let cwd = slash_command_data_source
                .active_session_for_v2_zero_state()
                .as_ref(app)
                .current_working_directory();
            let cwd_path = cwd.as_ref().map(std::path::Path::new);
            let skill_manager_handle = SkillManager::handle(app);
            let skill_manager = skill_manager_handle.as_ref(app);
            let skills = skill_manager.get_skills_for_working_directory(cwd_path, app);

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
                results.push(
                    InlineItem::from_skill(&skill, app)
                        .with_compact_layout(self.is_cloud_mode_v2)
                        .into(),
                );
            }
        }

        if self.is_cloud_mode_v2 && AISettings::as_ref(app).is_any_ai_enabled(app) {
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
                results.push(
                    InlineItem::from_saved_prompt(saved_prompt, app)
                        .with_compact_layout(self.is_cloud_mode_v2)
                        .into(),
                );
            }
        }

        Ok(results)
    }
}
