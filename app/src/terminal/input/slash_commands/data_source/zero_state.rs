use itertools::Itertools;
use warpui::{Entity, ModelHandle};

use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::slash_command_menu::static_commands::commands;
use crate::search::SyncDataSource;
use crate::terminal::input::slash_commands::{
    AcceptSlashCommandOrSavedPrompt, InlineItem, SlashCommandDataSource,
};

pub struct ZeroStateDataSource {
    slash_command_data_source: ModelHandle<SlashCommandDataSource>,
}

impl ZeroStateDataSource {
    pub fn new(slash_command_data_source: &ModelHandle<SlashCommandDataSource>) -> Self {
        Self {
            slash_command_data_source: slash_command_data_source.clone(),
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
                    InlineItem::from_slash_command(active_command_id, active_command, app).into(),
                );
            }
        }

        for prioritized_command in prioritized_commands {
            if let Some((id, command)) = active_prioritized_commands
                .iter()
                .find(|(_, active_command)| active_command.name == prioritized_command.name)
            {
                results.push(InlineItem::from_slash_command(id, command, app).into());
            }
        }

        Ok(results)
    }
}
