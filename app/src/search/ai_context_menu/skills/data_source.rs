use fuzzy_match::FuzzyMatchResult;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, Entity, SingletonEntity};

use super::search_item::SkillSearchItem;
use crate::ai::skills::SkillManager;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
#[cfg(not(target_family = "wasm"))]
use crate::workspace::ActiveSession;

const MAX_RESULTS: usize = 50;

pub struct SkillsDataSource;

impl SkillsDataSource {
    pub fn new() -> Self {
        Self
    }
}

impl SyncDataSource for SkillsDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = &query.text;

        // Resolve the current working directory from the active window's session.
        let cwd: Option<LocalOrRemotePath> = {
            #[cfg(not(target_family = "wasm"))]
            {
                app.windows().state().active_window.and_then(|window_id| {
                    ActiveSession::as_ref(app)
                        .working_directory(window_id)
                        .cloned()
                })
            }
            #[cfg(target_family = "wasm")]
            {
                None
            }
        };
        let skills = SkillManager::as_ref(app).get_skills_for_working_directory(cwd.as_ref(), app);

        let mut results: Vec<QueryResult<Self::Action>> = if query_text.is_empty() {
            // Zero state: show all skills with a uniform high score.
            skills
                .into_iter()
                .map(|skill| {
                    QueryResult::from(SkillSearchItem {
                        name: skill.name,
                        description: skill.description,
                        provider: skill.provider,
                        icon_override: skill.icon_override,
                        match_result: FuzzyMatchResult {
                            score: 1000,
                            matched_indices: vec![],
                        },
                    })
                })
                .collect()
        } else {
            // Fuzzy match against skill name.
            skills
                .into_iter()
                .filter_map(|skill| {
                    let match_result =
                        fuzzy_match::match_indices_case_insensitive(&skill.name, query_text)?;
                    // Skip very weak matches once the user has typed more than one character.
                    if query_text.len() > 1 && match_result.score < 10 {
                        return None;
                    }
                    Some(QueryResult::from(SkillSearchItem {
                        name: skill.name,
                        description: skill.description,
                        provider: skill.provider,
                        icon_override: skill.icon_override,
                        match_result,
                    }))
                })
                .collect()
        };

        results.sort_by_key(|r| std::cmp::Reverse(r.score()));
        results.truncate(MAX_RESULTS);

        Ok(results)
    }
}

impl Entity for SkillsDataSource {
    type Event = ();
}
