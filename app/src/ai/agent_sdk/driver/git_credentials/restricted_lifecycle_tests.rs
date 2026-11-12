use std::sync::atomic::{AtomicUsize, Ordering};

use futures::executor::block_on;

use super::tests::{response, validated_response, with_home};
use super::*;
use crate::ai::mcp::{MCPServer, MCPServerExt as _, TemplatableMCPServerManager, TransportType};
use crate::server::server_api::ai::MockAIClient;

#[test]
fn expiry_clears_credentials_while_the_original_operation_remains_pending() -> Result<()> {
    with_home(
        "lifecycle_tests::expiry_clears_credentials_while_the_original_operation_remains_pending",
        &[],
        |_| {
            let directory = prepare("task")?;
            let task_id = "task";
            apply_refresh(task_id, Ok(validated_response()?))?;
            with_store(task_id, |store| {
                store.expires_at = Some(Instant::now() + Duration::from_millis(1));
                Ok(())
            })?;
            let starts = AtomicUsize::new(0);
            let result = block_on(with_expiry(task_id, async {
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
                .returning(move |task, workload, partial, factory| {
                    assert_eq!(task, "task");
                    assert_eq!(workload, "workload-test-token");
                    assert!(!partial);
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
                .returning(|_, _, partial, _| {
                    assert!(!partial);
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
            let task_id = "task";
            apply_refresh(task_id, Ok(validated_response()?))?;
            let expiry = with_store(task_id, |store| Ok(store.expires_at))?;
            fs::rename(directory.join("credentials"), directory.join("previous"))?;
            fs::create_dir(directory.join("credentials"))?;
            let error = clear_restricted_credentials(task_id).unwrap_err();
            assert!(is_fatal_cleanup_error(&TaskGitCredentialsError::Request(
                error
            )));
            assert_eq!(with_store(task_id, |store| Ok(store.expires_at))?, expiry);
            let failed = apply_refresh(task_id, Ok(validated_response()?)).unwrap_err();
            assert!(is_fatal_cleanup_error(&failed));
            assert_eq!(with_store(task_id, |store| Ok(store.expires_at))?, expiry);
            Ok(())
        },
    )
}

#[test]
fn initial_mcp_process_environment_uses_active_task_credentials() -> Result<()> {
    with_home(
        "lifecycle_tests::initial_mcp_process_environment_uses_active_task_credentials",
        &[],
        |_| {
            let task_id = "task";
            let directory = prepare(task_id)?;
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
