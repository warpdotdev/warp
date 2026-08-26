//! Common utilities for agent SDK commands.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::TryFutureExt;
use inquire::{InquireError, Select};
use warp_cli::agent::Harness;
use warp_cli::environment::{EnvironmentCreateArgs, EnvironmentUpdateArgs};
use warp_cli::scope::ObjectScope;
use warpui::r#async::FutureExt;
use warpui::{AppContext, GetSingletonModelHandle, SingletonEntity as _, UpdateModel};

use crate::ai::agent::conversation::ServerAIConversationMetadata;
use crate::ai::agent_sdk::driver::{AgentDriverError, WARP_DRIVE_SYNC_TIMEOUT};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::llms::{LLMId, LLMPreferences, is_model_allowed_for_scope};
use crate::auth::UserUid;
use crate::auth::auth_state::AuthStateProvider;
use crate::cloud_object::{CloudObject, CloudObjectLookup as _, Owner};
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::AIClient;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::team_workspace_settings::{
    NotATeamMemberError, TeamScopeForCli,
};
use crate::workspaces::user_workspaces::{SoleTeamError, TeamScope as _, UserWorkspaces};

/// How long to wait for workspace metadata to refresh.
pub const WORKSPACE_METADATA_REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

pub fn validate_agent_mode_base_model_id(
    model_id: &str,
    ctx: &AppContext,
) -> anyhow::Result<LLMId> {
    let llm_prefs = LLMPreferences::as_ref(ctx);
    let valid_ids = llm_prefs
        .get_base_llm_choices_for_agent_mode(ctx)
        .map(|info| info.id.clone())
        .collect::<Vec<_>>();

    classify_agent_mode_base_model_id(
        model_id,
        &valid_ids,
        llm_prefs.agent_mode_models_unavailable(),
    )
}

/// Classifies a user-supplied agent-mode model id against the available model
/// list, distinguishing "the model list fetch failed (so the list is empty or
/// stale)" from "the id is genuinely not in a valid list".
fn classify_agent_mode_base_model_id(
    model_id: &str,
    valid_ids: &[LLMId],
    list_unavailable: bool,
) -> anyhow::Result<LLMId> {
    let llm_id: LLMId = model_id.into();
    if valid_ids.contains(&llm_id) {
        Ok(llm_id)
    } else if list_unavailable {
        Err(anyhow::anyhow!(
            "Could not retrieve the agent-mode model list from the server \
             (the request failed or returned no models). Try again later."
        ))
    } else {
        let suggestions = valid_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow::anyhow!(
            "Unknown model id '{model_id}'. Try one of: {suggestions}"
        ))
    }
}

pub(super) fn parse_ambient_task_id(
    run_id: &str,
    error_prefix: &str,
) -> anyhow::Result<AmbientAgentTaskId> {
    run_id
        .parse()
        .map_err(|err| anyhow::anyhow!("{error_prefix} '{run_id}': {err}"))
}

pub(super) fn set_ambient_task_context_from_run_id(
    ctx: &AppContext,
    run_id: &str,
) -> anyhow::Result<AmbientAgentTaskId> {
    let task_id = parse_ambient_task_id(run_id, "Invalid run ID")?;
    ServerApiProvider::handle(ctx)
        .as_ref(ctx)
        .get()
        .set_ambient_agent_task_id(Some(task_id));
    Ok(task_id)
}

pub(super) fn describe_sole_team_error(error: SoleTeamError, ctx: &AppContext) -> anyhow::Error {
    match error {
        SoleTeamError::NoTeam => anyhow::anyhow!("You are not on a team"),
        SoleTeamError::MoreThanOneTeam { team_uids } => anyhow::anyhow!(
            "You are on {} teams. Re-run with one of:\n\n{}",
            team_uids.len(),
            describe_team_choices(&team_uids, ctx)
        ),
    }
}

/// One `--team=<UID>` line per team, so the flag to copy starts each line and the name that
/// identifies it follows. Sorted by name to keep the list stable across runs.
fn describe_team_choices(team_uids: &[ServerId], ctx: &AppContext) -> String {
    let workspaces = UserWorkspaces::as_ref(ctx);
    let mut choices: Vec<(String, ServerId)> = team_uids
        .iter()
        .map(|uid| {
            let name = workspaces
                .team_from_uid(*uid)
                .map(|team| team.name.clone())
                .unwrap_or_default();
            (name, *uid)
        })
        .collect();
    choices.sort_by_key(|(name, _)| name.to_lowercase());

    choices
        .iter()
        .map(|(name, uid)| format!("  --team={uid}   {name}").trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Why a `--team` invocation could not be pinned to a single team.
#[derive(Debug, thiserror::Error)]
enum TeamResolutionError {
    #[error(transparent)]
    NoSoleTeam(#[from] SoleTeamError),
    #[error(transparent)]
    NotAMember(#[from] NotATeamMemberError),
}

fn describe_team_resolution_error(error: TeamResolutionError, ctx: &AppContext) -> anyhow::Error {
    match error {
        TeamResolutionError::NoSoleTeam(error) => describe_sole_team_error(error, ctx),
        TeamResolutionError::NotAMember(NotATeamMemberError { team_uid }) => {
            anyhow::anyhow!("You are not on team {team_uid}")
        }
    }
}

/// Parses the uid given as `--team=<UID>`, if one was.
fn requested_team_uid(scope: &ObjectScope) -> anyhow::Result<Option<ServerId>> {
    scope
        .requested_team_uid()
        .map(|uid| {
            ServerId::try_from(uid).map_err(|err| anyhow::anyhow!("Invalid --team '{uid}': {err}"))
        })
        .transpose()
}

/// The team a CLI invocation acts as: the one it named, or its sole team when it named none.
///
/// Membership is checked so a mistyped uid fails loudly, rather than resolving to a team whose
/// policy lookups find nothing and being denied everything for a reason the user cannot see.
fn resolve_team_uid(scope: &ObjectScope, ctx: &AppContext) -> anyhow::Result<ServerId> {
    let workspaces = UserWorkspaces::as_ref(ctx);
    let resolved = match requested_team_uid(scope)? {
        Some(team_uid) if workspaces.is_member_of_team(team_uid) => Ok(team_uid),
        Some(team_uid) => Err(NotATeamMemberError { team_uid }.into()),
        None => workspaces.sole_team_uid().map_err(Into::into),
    };
    resolved.map_err(|err| describe_team_resolution_error(err, ctx))
}

/// The team a CLI command's policy reads are scoped to, resolved from the same `--team` the
/// object's owner is resolved from so the two cannot disagree.
fn resolve_team_scope(scope: &ObjectScope, ctx: &AppContext) -> anyhow::Result<TeamScopeForCli> {
    let team_uid = resolve_team_uid(scope, ctx)?;
    UserWorkspaces::as_ref(ctx)
        .team_scope_for_cli(team_uid)
        .map_err(|err| describe_team_resolution_error(err.into(), ctx))
}

/// [`validate_agent_mode_base_model_id`], also rejecting a model `scope`'s team does not let this
/// member use.
///
/// The team is resolved only once the model turns out to be one of the member's own custom
/// endpoints, since that is the only kind a team withholds. Resolving it eagerly would make a
/// multi-team user pass `--team` to name a model no team governs.
pub fn validate_agent_mode_base_model_id_for_scope(
    model_id: &str,
    scope: &ObjectScope,
    ctx: &AppContext,
) -> anyhow::Result<LLMId> {
    let llm_id = validate_agent_mode_base_model_id(model_id, ctx)?;
    let prefs = LLMPreferences::as_ref(ctx);
    let Some(llm) = prefs.custom_llm_info_for_id(&llm_id) else {
        return Ok(llm_id);
    };

    let team_scope = resolve_team_scope(scope, ctx)?;
    if is_model_allowed_for_scope(prefs, llm, &team_scope, ctx) {
        return Ok(llm_id);
    }
    Err(anyhow::anyhow!(
        "Model '{model_id}' is one of your own custom endpoints, which team {} does not allow its \
         members to use.",
        team_scope.team_uid().expect("a CLI scope names a team")
    ))
}

fn current_user_uid(ctx: &AppContext) -> anyhow::Result<UserUid> {
    AuthStateProvider::as_ref(ctx)
        .get()
        .user_id()
        .ok_or_else(|| anyhow::anyhow!("User should be logged in"))
}

/// Resolve the owner of a new cloud object, based on the CLI `--team` and `--personal` flags.
///
/// With neither flag, a user on exactly one team gets a team object and a user on no team gets
/// a personal one. A user on several teams is asked to choose rather than silently handed a
/// personal object.
pub fn resolve_owner(scope: &ObjectScope, ctx: &AppContext) -> anyhow::Result<Owner> {
    if scope.personal {
        return Ok(Owner::User {
            user_uid: current_user_uid(ctx)?,
        });
    }

    if scope.is_team() {
        return Ok(Owner::Team {
            team_uid: resolve_team_uid(scope, ctx)?,
        });
    }

    match UserWorkspaces::as_ref(ctx).sole_team_uid() {
        Ok(team_uid) => Ok(Owner::Team { team_uid }),
        Err(SoleTeamError::NoTeam) => Ok(Owner::User {
            user_uid: current_user_uid(ctx)?,
        }),
        Err(error @ SoleTeamError::MoreThanOneTeam { .. }) => {
            Err(describe_sole_team_error(error, ctx))
        }
    }
}

/// Checks `--team` against the caller's memberships, for commands that leave the owner for the
/// server to resolve.
pub fn validate_team_scope(scope: &ObjectScope, ctx: &AppContext) -> anyhow::Result<()> {
    if !scope.is_team() {
        return Ok(());
    }

    resolve_team_uid(scope, ctx).map(|_| ())
}

/// Refresh workspace metadata before executing an operation.
///
/// This ensures that team state is up-to-date before creating cloud objects or performing
/// other operations that depend on team membership.
pub fn refresh_workspace_metadata<C>(
    ctx: &mut C,
) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<C>
where
    C: GetSingletonModelHandle + UpdateModel,
{
    let refresh_future = TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
        manager
            .refresh_workspace_metadata(ctx)
            .with_timeout(WORKSPACE_METADATA_REFRESH_TIMEOUT)
    });

    async move {
        let _ = refresh_future
            .await
            .map_err(|_| anyhow::anyhow!("Timed out refreshing team metadata"))?;
        Ok(())
    }
}

/// Refresh Warp Drive before executing an operation.
pub fn refresh_warp_drive(
    ctx: &AppContext,
) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<> {
    UpdateManager::as_ref(ctx)
        .initial_load_complete()
        .with_timeout(WARP_DRIVE_SYNC_TIMEOUT)
        .map_err(|_| anyhow::anyhow!("Timed out waiting for Warp Drive to sync"))
}

/// Fetch the conversation's server metadata and validate that its harness matches the caller's
/// `--harness` choice. Returns the metadata on success so the caller can reuse it (e.g. for the
/// server conversation token).
///
/// Called up-front before any task/config-build logic consumes `args.harness`, so a mismatch
/// error surfaces before side effects like task creation. We deliberately do NOT auto-upgrade
/// the harness: `Harness::Oz` default with a Claude conversation id is treated as a mismatch
/// and errors out.
pub(super) async fn fetch_and_validate_conversation_harness(
    ai_client: Arc<dyn AIClient>,
    conversation_id: &str,
    args_harness: Harness,
) -> Result<ServerAIConversationMetadata, AgentDriverError> {
    let metadata = ai_client
        .list_ai_conversation_metadata(Some(vec![conversation_id.to_string()]))
        .await
        .map_err(|e| AgentDriverError::ConversationLoadFailed(format!("{e:#}")))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AgentDriverError::ConversationLoadFailed(format!(
                "conversation {conversation_id} not found or not accessible"
            ))
        })?;

    if metadata.harness != args_harness {
        return Err(AgentDriverError::ConversationHarnessMismatch {
            conversation_id: conversation_id.to_string(),
            expected: Harness::from(metadata.harness).to_string(),
            got: args_harness.to_string(),
        });
    }

    Ok(metadata)
}

/// Format an object owner for display in the CLI.
pub fn format_owner(owner: &Owner) -> &'static str {
    // TODO: For potentially-shared objects, consider looking up the particular user/team name.
    match owner {
        Owner::User { .. } => "Personal",
        Owner::Team { .. } => "Team",
    }
}

/// An error resolving an agent option, which we may have prompted the user for.
#[derive(Debug, thiserror::Error)]
pub enum ResolveConfigurationError {
    /// The user canceled the operation, and we should exit.
    #[error("Operation canceled")]
    Canceled,
    #[error("{id} is not a valid {kind} identifier")]
    InvalidId { id: String, kind: &'static str },
    #[error("{kind} {id} not found")]
    ObjectNotFound { id: String, kind: &'static str },
    #[error(transparent)]
    Other(anyhow::Error),
}

#[derive(Clone, Debug, PartialEq)]
pub enum EnvironmentChoice {
    /// The user explicitly chose not to use an environment.
    None,
    /// The user chose a specific environment.
    Environment { id: String, name: String },
}

impl EnvironmentChoice {
    /// Resolve the environment to use when creating an agent integration.
    /// Warp Drive *must* have been synced first.
    pub fn resolve_for_create(
        args: EnvironmentCreateArgs,
        ctx: &AppContext,
    ) -> Result<Self, ResolveConfigurationError> {
        if args.no_environment {
            Ok(EnvironmentChoice::None)
        } else if let Some(id) = args.environment {
            Self::get_by_id(id, ctx)
        } else {
            let all_environments = CloudAmbientAgentEnvironment::get_all(ctx);
            let mut synced_environments: Vec<(ServerId, &CloudAmbientAgentEnvironment)> =
                all_environments
                    .iter()
                    .filter_map(|env| {
                        if let SyncId::ServerId(server_id) = env.sync_id() {
                            Some((server_id, env))
                        } else {
                            None
                        }
                    })
                    .collect();

            synced_environments
                .sort_by_key(|(_, env)| env.model().string_model.name.to_lowercase());

            let environments: Vec<EnvironmentChoice> = synced_environments
                .into_iter()
                .map(|(server_id, env)| EnvironmentChoice::Environment {
                    id: server_id.to_string(),
                    name: env.model().string_model.name.clone(),
                })
                .collect();

            let mut options = vec![EnvironmentChoice::None];
            options.extend(environments);

            // If there are no synced environments, require the user to create one or use --no-environment.
            if options.len() == 1 {
                let cli_name = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());
                return Err(ResolveConfigurationError::Other(anyhow::anyhow!(
                    "No environments are configured for this account.\n\
You can create an environment with `{cli_name} environment create`.\n\
Or, re-run this command with `--no-environment` to not use an environment.\n\
Without an environment, the agent will not be able to access private repositories or create pull requests.",
                )));
            }

            let prompt = "Select an environment to run the agent in (or 'No environment'):";

            let choice = Select::new(prompt, options).prompt();

            match choice {
                Ok(choice) => Ok(choice),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    Err(ResolveConfigurationError::Canceled)
                }
                Err(err) => Err(ResolveConfigurationError::Other(anyhow::anyhow!(
                    "Error selecting environment: {err}"
                ))),
            }
        }
    }

    /// Resolve the environment to use when updating an agent integration. If the user did not
    /// request any changes to the environment, this returns `Ok(None)`.
    /// Warp Drive *must* have been synced first.
    pub fn resolve_for_update(
        args: EnvironmentUpdateArgs,
        ctx: &AppContext,
    ) -> Result<Option<Self>, ResolveConfigurationError> {
        if args.remove_environment {
            Ok(Some(EnvironmentChoice::None))
        } else if let Some(id) = args.environment {
            Self::get_by_id(id, ctx).map(Some)
        } else {
            Ok(None)
        }
    }

    fn get_by_id(id: String, ctx: &AppContext) -> Result<Self, ResolveConfigurationError> {
        let sync_id = SyncId::ServerId(ServerId::try_from(id.as_str()).map_err(|_| {
            ResolveConfigurationError::InvalidId {
                id: id.clone(),
                kind: "environment",
            }
        })?);

        let environment =
            CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx).ok_or_else(|| {
                ResolveConfigurationError::ObjectNotFound {
                    id: id.clone(),
                    kind: "environment",
                }
            })?;

        Ok(EnvironmentChoice::Environment {
            id,
            name: environment.model().string_model.name.clone(),
        })
    }
}

impl fmt::Display for EnvironmentChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvironmentChoice::None => write!(
                f,
                "No environment (agent will not be able to access private repositories or create pull requests)",
            ),
            EnvironmentChoice::Environment { id, name } => write!(f, "{name} ({id})"),
        }
    }
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
