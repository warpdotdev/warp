use fuzzy_match::match_indices_case_insensitive;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, EntityId, SingletonEntity};

use crate::ai::execution_profiles::profiles::{AIExecutionProfilesModel, ClientProfileId};
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::terminal::input::inline_menu::{InlineMenuAction, InlineMenuType};
use crate::terminal::input::profiles::search_item::ProfileSearchItem;

#[derive(Clone, Debug)]
pub enum SelectProfileMenuItem {
    Profile { profile_id: ClientProfileId },
    ManageProfiles,
}

impl InlineMenuAction for SelectProfileMenuItem {
    const MENU_TYPE: InlineMenuType = InlineMenuType::ProfileSelector;
}

pub struct ProfileSelectorDataSource {
    terminal_view_id: EntityId,
}

impl ProfileSelectorDataSource {
    pub fn new(terminal_view_id: EntityId) -> Self {
        Self { terminal_view_id }
    }
}

impl SyncDataSource for ProfileSelectorDataSource {
    type Action = SelectProfileMenuItem;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let profiles_model = AIExecutionProfilesModel::as_ref(app);
        let active_profile_id = *profiles_model
            .active_profile(Some(self.terminal_view_id), app)
            .id();
        let query_text = query.text.trim().to_lowercase();
        let mut results = Vec::new();
        if query_text.is_empty() {
            results.push(QueryResult::from(
                ProfileSearchItem::new_manage_profiles_item(),
            ));
        } else if let Some(match_result) =
            match_indices_case_insensitive("manage profiles", &query_text)
        {
            let score = match_result.score;
            results.push(QueryResult::from(
                ProfileSearchItem::new_manage_profiles_item()
                    .with_match_result(match_result)
                    .with_score(OrderedFloat(score as f64)),
            ));
        }

        let mut profiles: Vec<(ClientProfileId, String)> = profiles_model
            .get_all_profile_ids()
            .into_iter()
            .filter_map(|profile_id| {
                let profile_info = profiles_model.get_profile_by_id(profile_id, app)?;
                let profile_name = profile_info.data().display_name();
                Some((profile_id, profile_name))
            })
            .collect();
        profiles.sort_by(|(_, a), (_, b)| a.to_lowercase().cmp(&b.to_lowercase()));

        for (profile_id, profile_name) in profiles {
            if query_text.is_empty() {
                results.push(QueryResult::from(ProfileSearchItem::new_profile_item(
                    profile_id,
                    profile_name,
                    profile_id == active_profile_id,
                )));
                continue;
            }

            if let Some(match_result) =
                match_indices_case_insensitive(&profile_name.to_lowercase(), &query_text)
            {
                let score = match_result.score;
                results.push(QueryResult::from(
                    ProfileSearchItem::new_profile_item(
                        profile_id,
                        profile_name,
                        profile_id == active_profile_id,
                    )
                    .with_match_result(match_result)
                    .with_score(OrderedFloat(score as f64)),
                ));
            }
        }

        Ok(results)
    }
}

impl Entity for ProfileSelectorDataSource {
    type Event = ();
}
