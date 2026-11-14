use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::executor::block_on;
use futures::future::{self, Either};
use warp_graphql::platform_error::PlatformErrorInfo;

use super::tests::{response, validated_response, with_home};
use super::*;
use crate::ai::agent_sdk::standby::{StandbyClock, renew_standby_credentials};
use crate::ai::mcp::{MCPServer, MCPServerExt as _, TemplatableMCPServerManager, TransportType};
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::prewarmed_sandbox::{
    PrewarmedSandboxClient, PrewarmedSandboxConfiguration, PrewarmedSandboxControlStream,
    PrewarmedSandboxPreparation, PrewarmedSandboxState,
};

#[test]
fn expiry_clears_credentials_while_the_original_operation_remains_pending() -> Result<()> {
    with_home(
        "lifecycle_tests::expiry_clears_credentials_while_the_original_operation_remains_pending",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            with_store(&owner, |store| {
                store.expires_at = Some(Instant::now() + Duration::from_millis(1));
                Ok(())
            })?;
            let starts = AtomicUsize::new(0);
            let result = block_on(with_expiry(&owner, async {
                starts.fetch_add(1, Ordering::SeqCst);
                Timer::after(Duration::from_millis(20)).await;
                assert_eq!(
                    fs::read_to_string(directory.join("credentials")).unwrap(),
                    ""
                );
                7
            }))?;
            assert_eq!(result, 7);
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            Ok(())
        },
    )
}

#[test]
fn bootstrap_retries_only_raw_fetch_with_one_prepared_store() -> Result<()> {
    with_home(
        "lifecycle_tests::bootstrap_retries_only_raw_fetch_with_one_prepared_store",
        &[
            ("WARP_WORKLOAD_TOKEN", "workload-test-token"),
            ("WARP_ISOLATION_PLATFORM", "docker"),
        ],
        |home| {
            let directory = home.join(".warp/github-git/task");
            let expected_directory = directory.clone();
            let mut attempts = 0;
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(4)
                .returning(move |task, workload, partial, factory, providers| {
                    assert_eq!(task, "task");
                    assert_eq!(workload, "workload-test-token");
                    assert!(!partial);
                    assert!(providers.is_none());
                    assert!(factory);
                    assert_eq!(
                        fs::read_to_string(expected_directory.join("credentials")).unwrap(),
                        ""
                    );
                    attempts += 1;
                    if attempts < 4 {
                        return Err(TaskGitCredentialsError::Request(
                            HttpStatusError::new(503, String::new()).into(),
                        ));
                    }
                    let mut response = response();
                    response.credentials[0].expires_at =
                        Some((Utc::now() + TimeDelta::hours(1)).into());
                    Ok(response)
                });
            let client: Arc<dyn AIClient> = Arc::new(client);
            block_on(bootstrap_restricted_credentials("task", &client, true))?;
            assert!(!fs::read_to_string(directory.join("credentials"))?.is_empty());
            assert!(!directory.join("gh/hosts.yml").exists());
            close_restricted_credentials("task")?;
            Ok(())
        },
    )
}

#[test]
fn bootstrap_validation_failure_is_not_retried_and_closes_store() -> Result<()> {
    with_home(
        "lifecycle_tests::bootstrap_validation_failure_is_not_retried_and_closes_store",
        &[
            ("WARP_WORKLOAD_TOKEN", "workload-test-token"),
            ("WARP_ISOLATION_PLATFORM", "docker"),
        ],
        |home| {
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(1)
                .returning(|_, _, partial, _, providers| {
                    assert!(!partial);
                    assert!(providers.is_none());
                    let mut response = response();
                    response.credentials[0].repository_scope = None;
                    Ok(response)
                });
            let client: Arc<dyn AIClient> = Arc::new(client);
            assert!(block_on(bootstrap_restricted_credentials("task", &client, false)).is_err());
            assert_eq!(
                fs::read_to_string(home.join(".warp/github-git/task/credentials"))?,
                ""
            );
            assert!(with_store("task", |_| Ok(())).is_err());
            assert!(!home.join(".gitconfig").exists());
            Ok(())
        },
    )
}

#[test]
fn failed_clearing_is_fatal_and_does_not_discard_expiry() -> Result<()> {
    with_home(
        "lifecycle_tests::failed_clearing_is_fatal_and_does_not_discard_expiry",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            let expiry = with_store(&owner, |store| Ok(store.expires_at))?;
            fs::rename(directory.join("credentials"), directory.join("previous"))?;
            fs::create_dir(directory.join("credentials"))?;
            let error = clear_restricted_credentials(&owner).unwrap_err();
            assert!(is_fatal_cleanup_error(&TaskGitCredentialsError::Request(
                error
            )));
            assert_eq!(with_store(&owner, |store| Ok(store.expires_at))?, expiry);
            let failed = apply_refresh(&owner, Ok(validated_response()?)).unwrap_err();
            assert!(is_fatal_cleanup_error(&failed));
            assert_eq!(with_store(&owner, |store| Ok(store.expires_at))?, expiry);
            Ok(())
        },
    )
}

#[test]
fn initial_mcp_process_environment_uses_active_standby_credentials() -> Result<()> {
    with_home(
        "lifecycle_tests::initial_mcp_process_environment_uses_active_standby_credentials",
        &[],
        |_| {
            let owner = ManagedStoreOwner::Standby {
                sandbox_id: "sandbox".into(),
                generation: "generation".into(),
            };
            let directory = prepare(&owner)?;
            let mut server = MCPServer::from_user_json(r#"{"local":{"command":"test-mcp","env":{"GH_TOKEN":"configured-test-token","GH_CONFIG_DIR":"/unmanaged","GIT_DIR":"/unmanaged","PATH":"/setup/bin","CUSTOM":"keep"}}}"#)?.pop().unwrap();
            TemplatableMCPServerManager::default()
                .apply_runtime_environment(&mut server.transport_type)?;
            let TransportType::CLIServer(server) = server.transport_type else {
                panic!("stdio expected")
            };
            let environment = server
                .static_env_vars
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<HashMap<_, _>>();
            assert_eq!(environment["GH_TOKEN"], "");
            assert_eq!(
                Path::new(environment["GH_CONFIG_DIR"]),
                directory.join("gh")
            );
            assert!(!environment.contains_key("GIT_DIR"));
            assert_eq!(environment["PATH"], "/setup/bin");
            assert_eq!(environment["CUSTOM"], "keep");
            Ok(())
        },
    )
}

struct RenewalClient {
    started: (async_channel::Sender<()>, async_channel::Receiver<()>),
    requests: AtomicUsize,
    cancelled: Arc<AtomicBool>,
    pending: bool,
    renew: bool,
}

struct RecordCancellation(Arc<AtomicBool>);

impl Drop for RecordCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl PrewarmedSandboxClient for RenewalClient {
    async fn fetch_configuration(&self, sandbox_id: &str) -> Result<PrewarmedSandboxConfiguration> {
        assert_eq!(sandbox_id, "sandbox");
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.started.0.send(()).await?;
        if self.pending {
            let _cancelled = RecordCancellation(self.cancelled.clone());
            return future::pending().await;
        }
        if self.renew {
            let mut credentials = response().credentials;
            credentials[0].token = "renewed-standby-token".into();
            credentials[0].expires_at = Some((Utc::now() + TimeDelta::hours(2)).into());
            return Ok(PrewarmedSandboxConfiguration {
                sandbox_id: "sandbox".into(),
                generation: "renewed".into(),
                github_credential_mode: GitHubCredentialMode::RestrictedInstallation,
                git_credentials: credentials,
                expires_at: Utc::now() + TimeDelta::hours(3),
                environment_variables: HashMap::new(),
                managed_secrets: HashMap::new(),
                preparation: PrewarmedSandboxPreparation {
                    harness: "warp".into(),
                    repositories: Vec::new(),
                    deferred_repositories: Vec::new(),
                    setup_commands: Vec::new(),
                    mcp_servers: serde_json::Map::new(),
                    skills: Vec::new(),
                    factory_skill_dirs: Vec::new(),
                    computer_use_enabled: false,
                    providers: None,
                },
            });
        }
        let mut info = PlatformErrorInfo::new(PlatformErrorCode::OperationNotSupported, false);
        info.http_status = Some(410);
        info.metadata
            .insert("reason".into(), "prewarmed_sandbox_unavailable".into());
        Err(TaskGitCredentialsError::Platform {
            message: "unavailable".into(),
            detail: None,
            info: Box::new(info),
        }
        .into())
    }

    async fn report_status(
        &self,
        sandbox_id: &str,
        generation: &str,
        state: PrewarmedSandboxState,
    ) -> Result<()> {
        panic!("unexpected status {sandbox_id} {generation} {state:?}")
    }

    async fn connect_control_stream(
        &self,
        sandbox_id: &str,
        generation: &str,
    ) -> Result<PrewarmedSandboxControlStream> {
        panic!("unexpected control {sandbox_id} {generation}")
    }
}

#[test]
fn actual_standby_renewal_clears_gone_and_cancels_inflight_fetch_before_task_owner() -> Result<()> {
    with_home(
        "lifecycle_tests::actual_standby_renewal_clears_gone_and_cancels_inflight_fetch_before_task_owner",
        &[],
        |_| {
            for (generation, pending, renew) in [
                ("gone", false, false),
                ("pending", true, false),
                ("renewed", false, true),
            ] {
                let owner = ManagedStoreOwner::Standby {
                    sandbox_id: "sandbox".into(),
                    generation: generation.into(),
                };
                let directory = prepare(&owner)?;
                apply_refresh(&owner, Ok(validated_response()?))?;
                let renewed_after = Instant::now() + Duration::from_secs(7100);
                with_store(&owner, |store| {
                    store.refresh_at = Some(Instant::now());
                    Ok(())
                })?;
                let client = RenewalClient {
                    started: async_channel::unbounded(),
                    requests: AtomicUsize::new(0),
                    cancelled: Arc::new(AtomicBool::new(false)),
                    pending,
                    renew,
                };
                let clock = StandbyClock::default();
                block_on(async {
                    let renewal = renew_standby_credentials(
                        &client,
                        &owner,
                        Utc::now() + TimeDelta::hours(2),
                        &clock,
                    )
                    .boxed();
                    match future::select(renewal, client.started.1.recv().boxed()).await {
                        Either::Left((result, _)) => {
                            panic!("renewal must remain pending: {result:?}")
                        }
                        Either::Right((started, renewal)) => {
                            started.unwrap();
                            assert_eq!(
                                fs::read_to_string(directory.join("credentials"))
                                    .unwrap()
                                    .is_empty(),
                                !pending && !renew
                            );
                            if renew {
                                assert!(
                                    fs::read_to_string(directory.join("credentials"))
                                        .unwrap()
                                        .contains("renewed-standby-token")
                                );
                                assert!(
                                    with_store(&owner, |store| Ok(store.expires_at.unwrap()))
                                        .unwrap()
                                        > renewed_after
                                );
                            }
                            drop(renewal);
                        }
                    }
                });
                assert_eq!(client.requests.load(Ordering::SeqCst), 1);
                assert_eq!(client.cancelled.load(Ordering::SeqCst), pending);
                close_restricted_credentials(&owner)?;
                let task = ManagedStoreOwner::Task(format!("task-{generation}"));
                let task_directory = prepare(&task)?;
                apply_refresh(&task, Ok(validated_response()?))?;
                assert!(apply_standby_refresh(&owner, response().credentials).is_err());
                assert!(!fs::read_to_string(task_directory.join("credentials"))?.is_empty());
                close_restricted_credentials(&task)?;
            }
            Ok(())
        },
    )
}
