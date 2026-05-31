use warp_graphql::queries::get_user::UserOutput as GqlUserOutput;

use super::user::User;
use super::UserUid;
use crate::convert_to_server_experiment;
use crate::server::experiments::ServerExperiment;

/// Intermediate app model state converted from a user response returned by the auth client.
pub(crate) struct UserProperties {
    pub(crate) user: User,
    pub(crate) server_experiments: Vec<ServerExperiment>,
    pub(crate) llms: crate::ai::llms::ModelsByFeature,
}

impl From<GqlUserOutput> for UserProperties {
    fn from(user_output: GqlUserOutput) -> Self {
        let principal_type = user_output
            .principal_type
            .map(|pt| pt.into())
            .unwrap_or_default();
        let user_properties = user_output.user;

        let is_on_work_domain = user_properties.is_on_work_domain;
        let is_onboarded = user_properties.is_onboarded;
        let global_skills = user_properties.global_skills;

        let linked_at = user_properties
            .anonymous_user_info
            .as_ref()
            .and_then(|info| info.linked_at);

        let anonymous_user_type = user_properties
            .anonymous_user_info
            .as_ref()
            .map(|info| info.anonymous_user_type.clone());
        let personal_object_limits = user_properties
            .anonymous_user_info
            .and_then(|info| info.personal_object_limits.clone());
        let user_profile = user_properties.profile;
        let local_id = UserUid::new(user_profile.uid.as_str());
        let needs_sso_link = user_profile.needs_sso_link;

        let server_experiments: Vec<ServerExperiment> = user_properties
            .experiments
            .and_then(|experiments| convert_to_server_experiment!(experiments))
            .unwrap_or_default();

        let llms = user_properties.llms.try_into().unwrap_or_default();

        let user = User {
            is_onboarded,
            local_id,
            metadata: user_profile.into(),
            needs_sso_link,
            anonymous_user_type: anonymous_user_type.and_then(|t| t.try_into().ok()),
            is_on_work_domain,
            linked_at,
            personal_object_limits: personal_object_limits.and_then(|t| t.try_into().ok()),
            principal_type,
            global_skills,
        };

        UserProperties {
            user,
            server_experiments,
            llms,
        }
    }
}
