use std::sync::Arc;

use chrono::Utc;
use warp_cli::agent::OutputFormat;
use warp_cli::runner::UpdateRunnerArgs;
use warp_cli::scope::{ObjectScope, TeamSelection};
use warp_graphql::object::{Space, SpaceType};
use warp_graphql::queries::get_runners::{Runner, RunnerConfig, RunnerOs};
use warpui::App;

use super::{
    RunnerArch, RunnerArchArg, RunnerOsArg, confirm_delete, execute_update, merge_instance_shape,
    resolve_arch, resolve_create_request_scope, resolve_updated_name,
};
use crate::server::ids::ServerId;
use crate::server::server_api::factory::{MockFactoryClient, UpsertedRunner};
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::user_workspaces::{TeamContextForOperation, UserWorkspaces};
use crate::workspaces::workspace::{Workspace, WorkspaceUid};

fn runner(uid: &str, name: &str) -> Runner {
    Runner {
        uid: cynic::Id::new(uid),
        config: RunnerConfig {
            name: name.to_string(),
            description: None,
            setup_commands: None,
            instance_shape: None,
            os: RunnerOs::Linux,
            arch: RunnerArch::X8664,
            mac: None,
            linux: None,
        },
        last_updated: chrono::DateTime::<Utc>::UNIX_EPOCH.into(),
        scope: Space {
            uid: cynic::Id::new("user-1"),
            type_: SpaceType::User,
        },
        creator: None,
        last_editor: None,
    }
}

fn update_args(id: Option<&str>, name: Option<&str>) -> UpdateRunnerArgs {
    UpdateRunnerArgs {
        id: id.map(str::to_string),
        name: name.map(str::to_string),
        description: None,
        setup_command: Vec::new(),
        os: None,
        arch: None,
        docker_image: None,
        macos_version: None,
        vcpus: None,
        memory_gb: None,
        team_selection: TeamSelection { team: None },
    }
}

fn request_team_scope() -> RequestTeamScope {
    RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(
        ServerId::from_string_lossy("team_uid00000000000123"),
    ))
}

#[test]
fn create_without_explicit_team_uses_unscoped_request_for_sole_team_user() {
    App::test((), |app| async move {
        let team = Team {
            uid: ServerId::from(7),
            name: "Sole team".to_string(),
            color: None,
            invite_link: None,
            members: vec![],
            pending_email_invites: vec![],
            invite_link_domain_restrictions: vec![],
            billing_metadata: Default::default(),
            stripe_customer_id: None,
            settings: Default::default(),
            feature_model_choice: Default::default(),
            is_eligible_for_discovery: false,
            has_billing_history: false,
            visibility: TeamVisibility::Open,
        };
        let mut workspace = Workspace::from_local_cache(
            WorkspaceUid::from(ServerId::from(1)),
            "Workspace".to_string(),
            None,
            None,
        );
        workspace.teams = vec![team];
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });
        for scope in [
            ObjectScope {
                team_selection: TeamSelection { team: None },
                personal: false,
            },
            ObjectScope {
                team_selection: TeamSelection { team: None },
                personal: true,
            },
        ] {
            let request_scope = app
                .read(|ctx| resolve_create_request_scope(&scope, ctx))
                .unwrap();

            assert_eq!(request_scope, None);
        }
    });
}

#[tokio::test]
async fn name_update_scopes_discovery_but_not_uid_mutation() {
    let team_scope = request_team_scope();
    let mut factory = MockFactoryClient::new();
    factory
        .expect_get_runners()
        .withf(move |sort_by, actual_scope| sort_by.is_none() && *actual_scope == Some(team_scope))
        .once()
        .return_once(|_, _| Ok(vec![runner("runner-1", "runner-name")]));
    factory
        .expect_upsert_runner()
        .withf(|input, team_scope| {
            input.uid.as_ref().map(cynic::Id::inner) == Some("runner-1") && team_scope.is_none()
        })
        .once()
        .return_once(|_, _| {
            Ok(UpsertedRunner {
                runner: runner("runner-1", "runner-name"),
                is_update: true,
            })
        });

    execute_update(
        Arc::new(factory),
        update_args(None, Some("runner-name")),
        Some(team_scope),
        OutputFormat::Text,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn name_update_without_team_uses_unscoped_discovery() {
    let mut factory = MockFactoryClient::new();
    factory
        .expect_get_runners()
        .withf(|sort_by, team_scope| sort_by.is_none() && team_scope.is_none())
        .once()
        .return_once(|_, _| Ok(vec![runner("runner-1", "runner-name")]));
    factory
        .expect_upsert_runner()
        .withf(|input, team_scope| {
            input.uid.as_ref().map(cynic::Id::inner) == Some("runner-1") && team_scope.is_none()
        })
        .once()
        .return_once(|_, _| {
            Ok(UpsertedRunner {
                runner: runner("runner-1", "runner-name"),
                is_update: true,
            })
        });

    execute_update(
        Arc::new(factory),
        update_args(None, Some("runner-name")),
        None,
        OutputFormat::Text,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn uid_update_uses_resource_authoritative_lookup_and_mutation() {
    let mut factory = MockFactoryClient::new();
    factory
        .expect_get_runners()
        .withf(|sort_by, team_scope| sort_by.is_none() && team_scope.is_none())
        .once()
        .return_once(|_, _| Ok(vec![runner("runner-1", "runner-name")]));
    factory
        .expect_upsert_runner()
        .withf(|input, team_scope| {
            input.uid.as_ref().map(cynic::Id::inner) == Some("runner-1") && team_scope.is_none()
        })
        .once()
        .return_once(|_, _| {
            Ok(UpsertedRunner {
                runner: runner("runner-1", "runner-name"),
                is_update: true,
            })
        });

    execute_update(
        Arc::new(factory),
        update_args(Some("runner-1"), None),
        None,
        OutputFormat::Text,
    )
    .await
    .unwrap();
}

#[test]
fn confirm_delete_refuses_non_interactive_without_force() {
    // In non-interactive mode, refusal must surface as an error so the caller
    // exits non-zero instead of treating a skipped delete as a success.
    let err = confirm_delete("runner-123", false).expect_err("non-interactive refusal is an error");
    let msg = err.to_string();
    assert!(msg.contains("non-interactive"), "got: {msg}");
    assert!(msg.contains("runner-123"), "got: {msg}");
}

#[test]
fn resolve_arch_auto_maps_to_os_default() {
    assert!(matches!(
        resolve_arch(RunnerArchArg::Auto, RunnerOsArg::Linux),
        RunnerArch::X8664
    ));
    assert!(matches!(
        resolve_arch(RunnerArchArg::Auto, RunnerOsArg::Macos),
        RunnerArch::Aarch64
    ));
}

#[test]
fn resolve_arch_explicit_is_preserved_regardless_of_os() {
    assert!(matches!(
        resolve_arch(RunnerArchArg::X8664, RunnerOsArg::Macos),
        RunnerArch::X8664
    ));
    assert!(matches!(
        resolve_arch(RunnerArchArg::Aarch64, RunnerOsArg::Linux),
        RunnerArch::Aarch64
    ));
}

#[test]
fn merge_instance_shape_updates_dimensions_independently() {
    // Neither specified: preserve the existing shape.
    assert_eq!(
        merge_instance_shape(None, None, Some((2, 4))).unwrap(),
        Some((2, 4))
    );
    // Only vCPUs: keep existing memory.
    assert_eq!(
        merge_instance_shape(Some(8), None, Some((2, 4))).unwrap(),
        Some((8, 4))
    );
    // Only memory: keep existing vCPUs.
    assert_eq!(
        merge_instance_shape(None, Some(16), Some((2, 4))).unwrap(),
        Some((2, 16))
    );
    // Both specified: use both.
    assert_eq!(
        merge_instance_shape(Some(8), Some(16), Some((2, 4))).unwrap(),
        Some((8, 16))
    );
    // No existing shape and nothing set: no shape.
    assert_eq!(merge_instance_shape(None, None, None).unwrap(), None);
}

#[test]
fn merge_instance_shape_errors_on_partial_shape_without_existing() {
    assert!(merge_instance_shape(Some(8), None, None).is_err());
    assert!(merge_instance_shape(None, Some(16), None).is_err());
}

#[test]
fn resolve_updated_name_renames_only_with_uid() {
    // UID + --name renames the runner.
    assert_eq!(resolve_updated_name(true, Some("new"), "old"), "new");
    // UID without --name keeps the existing name.
    assert_eq!(resolve_updated_name(true, None, "old"), "old");
    // No UID: --name is the selector, so the name is unchanged.
    assert_eq!(resolve_updated_name(false, Some("old"), "old"), "old");
}
