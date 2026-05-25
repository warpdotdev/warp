//! Common utilities for agent SDK commands.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::TryFutureExt;
use inquire::{InquireError, Select};
use warp_cli::agent::Harness;
use warp_cli::environment::{EnvironmentCreateArgs, EnvironmentUpdateArgs};
use warp_localization::{LocaleId, replace_placeholders};
use warpui::r#async::FutureExt;
use warpui::{AppContext, GetSingletonModelHandle, SingletonEntity as _, UpdateModel};

use crate::ai::agent::conversation::ServerAIConversationMetadata;
use crate::ai::agent_sdk::driver::{AgentDriverError, WARP_DRIVE_SYNC_TIMEOUT};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::auth::auth_state::AuthStateProvider;
use crate::cloud_object::{CloudObject, Owner};
use crate::localization;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::AIClient;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// How long to wait for workspace metadata to refresh.
pub const WORKSPACE_METADATA_REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

fn text(ctx: &AppContext, key: &str) -> String {
    localization::text_for_app(ctx, key)
}

fn text_with_args(ctx: &AppContext, key: &str, args: &[(&str, &str)]) -> String {
    localization::text_for_app_with_args(ctx, key, args)
}

fn default_text(key: &str) -> String {
    localization::text_for_locale(LocaleId::EnUs, key)
}

fn default_text_with_args(key: &str, args: &[(&str, &str)]) -> String {
    replace_placeholders(&default_text(key), args)
        .expect("localized text template arguments must match the catalog")
}

pub fn validate_agent_mode_base_model_id(
    model_id: &str,
    ctx: &AppContext,
) -> anyhow::Result<LLMId> {
    let llm_prefs = LLMPreferences::as_ref(ctx);

    let llm_id: LLMId = model_id.into();
    let valid_ids = llm_prefs
        .get_base_llm_choices_for_agent_mode(ctx)
        .map(|info| info.id.clone())
        .collect::<Vec<_>>();

    if valid_ids.contains(&llm_id) {
        Ok(llm_id)
    } else {
        let suggestions = valid_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow::anyhow!(text_with_args(
            ctx,
            "agent_sdk.common.error.unknown_model_id",
            &[("model_id", model_id), ("suggestions", &suggestions)]
        )))
    }
}

pub(super) fn parse_ambient_task_id(
    run_id: &str,
    error_prefix: &str,
) -> anyhow::Result<AmbientAgentTaskId> {
    run_id.parse::<AmbientAgentTaskId>().map_err(|err| {
        let error = err.to_string();
        anyhow::anyhow!(default_text_with_args(
            "agent_sdk.common.error.invalid_ambient_task_id",
            &[
                ("prefix", error_prefix),
                ("run_id", run_id),
                ("error", &error)
            ],
        ))
    })
}

pub(super) fn set_ambient_task_context_from_run_id(
    ctx: &AppContext,
    run_id: &str,
) -> anyhow::Result<AmbientAgentTaskId> {
    let task_id =
        parse_ambient_task_id(run_id, &text(ctx, "agent_sdk.common.error.invalid_run_id"))?;
    ServerApiProvider::handle(ctx)
        .as_ref(ctx)
        .get()
        .set_ambient_agent_task_id(Some(task_id));
    Ok(task_id)
}

/// Resolve the owner of a new cloud object. This resolution is based on the CLI `--team` and `--personal` flags.
///
/// If `team_flag` is true, attempts to get the current team UID (errors if not on a team).
/// If `user_flag` is true, gets the current user's UID.
/// Otherwise, defaults to team if available, falling back to user.
pub fn resolve_owner(team_flag: bool, user_flag: bool, ctx: &AppContext) -> anyhow::Result<Owner> {
    if team_flag {
        let team_id = UserWorkspaces::as_ref(ctx)
            .current_team_uid()
            .ok_or_else(|| anyhow::anyhow!(text(ctx, "agent_sdk.common.error.user_not_on_team")))?;
        return Ok(Owner::Team { team_uid: team_id });
    }

    if user_flag {
        let user_id = AuthStateProvider::as_ref(ctx)
            .get()
            .user_id()
            .ok_or_else(|| {
                anyhow::anyhow!(text(ctx, "agent_sdk.common.error.user_not_logged_in"))
            })?;
        return Ok(Owner::User { user_uid: user_id });
    }

    // Default: try team first, fall back to user
    if let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() {
        return Ok(Owner::Team { team_uid });
    }

    log::warn!("Tried to default to creating team object, team could not be found.");
    let user_id = AuthStateProvider::as_ref(ctx)
        .get()
        .user_id()
        .ok_or_else(|| anyhow::anyhow!(text(ctx, "agent_sdk.common.error.user_not_logged_in")))?;
    Ok(Owner::User { user_uid: user_id })
}

/// Refresh workspace metadata before executing an operation.
///
/// This ensures that team state is up-to-date before creating cloud objects or performing
/// other operations that depend on team membership.
pub fn refresh_workspace_metadata<C>(
    ctx: &mut C,
) -> impl Future<Output = anyhow::Result<()>> + Send + 'static
where
    C: GetSingletonModelHandle + UpdateModel,
{
    let timeout_message = default_text("agent_sdk.common.error.workspace_metadata_timeout");
    let refresh_future = TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
        manager
            .refresh_workspace_metadata(ctx)
            .with_timeout(WORKSPACE_METADATA_REFRESH_TIMEOUT)
    });

    async move {
        let _ = refresh_future
            .await
            .map_err(|_| anyhow::anyhow!(timeout_message))?;
        Ok(())
    }
}

/// Refresh Warp Drive before executing an operation.
pub fn refresh_warp_drive(
    ctx: &AppContext,
) -> impl Future<Output = anyhow::Result<()>> + Send + 'static {
    let timeout_message = text(ctx, "agent_sdk.common.error.warp_drive_sync_timeout");
    UpdateManager::as_ref(ctx)
        .initial_load_complete()
        .with_timeout(WARP_DRIVE_SYNC_TIMEOUT)
        .map_err(move |_| anyhow::anyhow!(timeout_message))
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
            AgentDriverError::ConversationLoadFailed(default_text_with_args(
                "agent_sdk.common.error.conversation_not_found_or_not_accessible",
                &[("conversation_id", conversation_id)],
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

pub fn format_owner_for_app(owner: &Owner, ctx: &AppContext) -> String {
    let key = match owner {
        Owner::User { .. } => "agent_sdk.common.owner.personal",
        Owner::Team { .. } => "agent_sdk.common.owner.team",
    };
    text(ctx, key)
}

/// An error resolving an agent option, which we may have prompted the user for.
#[derive(Debug)]
pub enum ResolveConfigurationError {
    /// The user canceled the operation, and we should exit.
    Canceled,
    InvalidId {
        id: String,
        kind: &'static str,
    },
    ObjectNotFound {
        id: String,
        kind: &'static str,
    },
    Other(anyhow::Error),
}

impl fmt::Display for ResolveConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            ResolveConfigurationError::Canceled => {
                default_text("agent_sdk.common.error.operation_canceled")
            }
            ResolveConfigurationError::InvalidId { id, kind } => default_text_with_args(
                "agent_sdk.common.error.invalid_id",
                &[("id", id), ("kind", kind)],
            ),
            ResolveConfigurationError::ObjectNotFound { id, kind } => default_text_with_args(
                "agent_sdk.common.error.object_not_found",
                &[("id", id), ("kind", kind)],
            ),
            ResolveConfigurationError::Other(err) => err.to_string(),
        };
        f.write_str(&message)
    }
}

impl Error for ResolveConfigurationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ResolveConfigurationError::Other(err) => Some(err.as_ref()),
            _ => None,
        }
    }
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
                    text_with_args(
                        ctx,
                        "agent_sdk.common.environment.none_configured",
                        &[("cli_name", &cli_name)]
                    )
                )));
            }

            let prompt = text(ctx, "agent_sdk.common.environment.select_prompt");

            let choice = Select::new(&prompt, options).prompt();

            match choice {
                Ok(choice) => Ok(choice),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    Err(ResolveConfigurationError::Canceled)
                }
                Err(err) => Err(ResolveConfigurationError::Other(anyhow::anyhow!(
                    text_with_args(
                        ctx,
                        "agent_sdk.common.environment.select_error",
                        &[("error", &err.to_string())]
                    )
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
mod tests {
    use super::{default_text, parse_ambient_task_id};

    #[test]
    fn parse_ambient_task_id_accepts_valid_ids() {
        let prefix = default_text("agent_sdk.common.error.invalid_run_id");
        let task_id =
            parse_ambient_task_id("550e8400-e29b-41d4-a716-446655440000", &prefix).unwrap();

        assert_eq!(task_id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn parse_ambient_task_id_preserves_error_prefix() {
        let prefix = default_text("agent_sdk.common.error.invalid_run_id");
        let err = parse_ambient_task_id("not-a-run-id", &prefix).unwrap_err();

        assert!(
            err.to_string()
                .contains(&format!("{prefix} 'not-a-run-id'"))
        );
    }
}
