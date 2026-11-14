use futures::executor::block_on;

use super::tests::{response, validated_response, with_home};
use super::*;
use crate::server::server_api::ai::MockAIClient;

fn other_response(host: &str) -> RestrictedTaskGitCredentialsOutput {
    RestrictedTaskGitCredentialsOutput {
        github_credential_mode: GitHubCredentialMode::RestrictedInstallation,
        credentials: vec![RestrictedTaskGitCredential {
            host: host.into(),
            token: "other-test-token".into(),
            username: None,
            email: None,
            expires_at: None,
            repository_scope: None,
        }],
        failed_hosts: Vec::new(),
    }
}

#[test]
fn mixed_credentials_have_independent_deadlines_and_discover_absent_providers() -> Result<()> {
    with_home(
        "provider_tests::mixed_credentials_have_independent_deadlines_and_discover_absent_providers",
        &[],
        |_| {
            prepare("task")?;
            let now = Instant::now();
            let wall_now = Utc::now();
            let mut response = response();
            response.credentials[0].expires_at = Some((wall_now + TimeDelta::hours(1)).into());
            response
                .credentials
                .extend(other_response("gitlab.com").credentials);
            apply_refresh("task", Ok(validate_response(response, wall_now, now)?))?;
            with_store("task", |store| {
                assert_eq!(
                    store.provider_refresh_at[&TaskGitCredentialProvider::Github],
                    now + Duration::from_secs(3000)
                );
                assert_eq!(
                    store.provider_refresh_at[&TaskGitCredentialProvider::Gitlab],
                    now + Duration::from_secs(60)
                );
                assert!(due_providers(store, now).is_empty());
                assert_eq!(
                    due_providers(store, now + Duration::from_secs(60)),
                    vec![
                        TaskGitCredentialProvider::Gitlab,
                        TaskGitCredentialProvider::AzureDevops,
                    ]
                );
                assert_eq!(
                    due_providers(store, now + Duration::from_secs(3000)),
                    SUPPORTED_PROVIDERS
                );
                Ok(())
            })
        },
    )
}

#[test]
fn selective_response_cannot_restore_expired_github_credentials() -> Result<()> {
    with_home(
        "provider_tests::selective_response_cannot_restore_expired_github_credentials",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            let expiry = with_store(&owner, |store| Ok(store.expires_at.unwrap()))?;
            expire_credentials(&owner, expiry)?;
            apply_partial_response(
                &owner,
                other_response("gitlab.com"),
                Some(&[TaskGitCredentialProvider::Gitlab]),
            )?;
            assert!(fs::read(directory.join("credentials"))?.is_empty());
            assert!(with_store(&owner, |store| Ok(store.expires_at))?.is_none());
            Ok(())
        },
    )
}

#[test]
fn selective_cleanup_failure_is_fatal_and_keeps_expiry() -> Result<()> {
    with_home(
        "provider_tests::selective_cleanup_failure_is_fatal_and_keeps_expiry",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            let expiry = with_store(&owner, |store| Ok(store.expires_at))?;
            fs::rename(directory.join("credentials"), directory.join("previous"))?;
            fs::create_dir(directory.join("credentials"))?;
            let mut output = other_response("gitlab.com");
            output.github_credential_mode = GitHubCredentialMode::Legacy;
            let error =
                apply_partial_response(&owner, output, Some(&[TaskGitCredentialProvider::Gitlab]))
                    .unwrap_err();
            assert!(is_fatal_cleanup_error(&error));
            assert_eq!(with_store(&owner, |store| Ok(store.expires_at))?, expiry);
            Ok(())
        },
    )
}

#[test]
fn selective_refresh_keeps_unrequested_credential_and_deadline() -> Result<()> {
    with_home(
        "provider_tests::selective_refresh_keeps_unrequested_credential_and_deadline",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            let before = with_store(&owner, |store| {
                Ok((store.expires_at, store.provider_refresh_at.clone()))
            })?;
            let content = fs::read(directory.join("credentials"))?;
            for host in ["gitlab.com", "dev.azure.com"] {
                let provider = provider_for_host(host).unwrap();
                apply_partial_response(&owner, other_response(host), Some(&[provider]))?;
                with_store(&owner, |store| {
                    assert_eq!(store.expires_at, before.0);
                    assert_eq!(
                        store.provider_refresh_at[&TaskGitCredentialProvider::Github],
                        before.1[&TaskGitCredentialProvider::Github]
                    );
                    assert_eq!(fs::read(directory.join("credentials"))?, content);
                    Ok(())
                })?;
            }
            Ok(())
        },
    )
}

#[test]
fn selected_failures_retry_without_clearing_unrequested_github() -> Result<()> {
    with_home(
        "provider_tests::selected_failures_retry_without_clearing_unrequested_github",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            let deadline = with_store(&owner, |store| {
                Ok(store.provider_refresh_at[&TaskGitCredentialProvider::Github])
            })?;
            let mut response = other_response("gitlab.com");
            response.credentials.clear();
            response.failed_hosts = vec!["gitlab.com".into(), "dev.azure.com".into()];
            let selected = [
                TaskGitCredentialProvider::Gitlab,
                TaskGitCredentialProvider::AzureDevops,
            ];
            assert!(apply_partial_response(&owner, response, Some(&selected)).is_err());
            with_store(&owner, |store| {
                let retry = store.provider_refresh_at[&TaskGitCredentialProvider::Gitlab];
                assert_eq!(
                    store.provider_refresh_at[&TaskGitCredentialProvider::Github],
                    deadline
                );
                assert_eq!(due_providers(store, retry), selected);
                assert!(!fs::read(directory.join("credentials"))?.is_empty());
                Ok(())
            })
        },
    )
}

#[test]
fn empty_selection_result_preserves_other_schedules_and_full_absence_clears_github() -> Result<()> {
    with_home(
        "provider_tests::empty_selection_result_preserves_other_schedules_and_full_absence_clears_github",
        &[],
        |_| {
            let directory = prepare("task")?;
            let owner = ManagedStoreOwner::from("task");
            apply_refresh(&owner, Ok(validated_response()?))?;
            let before = with_store(&owner, |store| Ok(store.provider_refresh_at.clone()))?;
            let mut empty = other_response("gitlab.com");
            empty.credentials.clear();
            apply_partial_response(&owner, empty, Some(&[TaskGitCredentialProvider::Gitlab]))?;
            assert_eq!(
                with_store(&owner, |store| Ok(store.provider_refresh_at.clone()))?,
                before
            );
            assert!(!fs::read(directory.join("credentials"))?.is_empty());
            let mut empty = other_response("gitlab.com");
            empty.credentials.clear();
            apply_partial_response(&owner, empty, None)?;
            assert!(fs::read(directory.join("credentials"))?.is_empty());
            assert!(refresh_delay(&owner, Instant::now()) > Duration::from_secs(50));
            Ok(())
        },
    )
}

#[test]
fn selective_request_uses_due_providers_only() -> Result<()> {
    with_home(
        "provider_tests::selective_request_uses_due_providers_only",
        &[
            ("WARP_WORKLOAD_TOKEN", "test-workload"),
            ("WARP_ISOLATION_PLATFORM", "docker"),
        ],
        |_| {
            prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;
            with_store("task", |store| {
                store
                    .provider_refresh_at
                    .insert(TaskGitCredentialProvider::Gitlab, Instant::now());
                Ok(())
            })?;
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(1)
                .returning(|task, token, partial, factory, providers, _| {
                    assert_eq!(task, "task");
                    assert_eq!(token, "test-workload");
                    assert!(partial && factory);
                    assert_eq!(
                        providers,
                        Some(vec![
                            TaskGitCredentialProvider::Gitlab,
                            TaskGitCredentialProvider::AzureDevops
                        ])
                    );
                    Ok(other_response("gitlab.com"))
                });
            let client: Arc<dyn AIClient> = Arc::new(client);
            block_on(refresh_restricted_credentials("task", &client, true))?;
            block_on(refresh_restricted_credentials("task", &client, true))?;
            Ok(())
        },
    )
}

#[test]
fn schema_fallback_retains_restricted_contract_and_reports_full_selection() -> Result<()> {
    with_home(
        "provider_tests::schema_fallback_retains_restricted_contract_and_reports_full_selection",
        &[
            ("WARP_WORKLOAD_TOKEN", "test-workload"),
            ("WARP_ISOLATION_PLATFORM", "docker"),
        ],
        |_| {
            prepare("task")?;
            let mut calls = 0;
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(2)
                .returning(move |task, token, partial, factory, providers, _| {
                    assert_eq!(task, "task");
                    assert_eq!(token, "test-workload");
                    assert!(partial && factory);
                    calls += 1;
                    if calls == 1 {
                        assert_eq!(providers, Some(vec![TaskGitCredentialProvider::Gitlab]));
                        Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
                            "Field \"providers\" is not defined by type \"TaskGitCredentialsInput\""
                        )))
                    } else {
                        assert!(providers.is_none());
                        Ok(response())
                    }
                });
            let client: Arc<dyn AIClient> = Arc::new(client);
            let (output, providers) = block_on(fetch(
                "task",
                &client,
                true,
                true,
                Some(vec![TaskGitCredentialProvider::Gitlab]),
            ))?;
            assert!(providers.is_none());
            assert_eq!(
                output.github_credential_mode,
                GitHubCredentialMode::RestrictedInstallation
            );
            assert_eq!(output.credentials[0].host, GITHUB_HOST);
            Ok(())
        },
    )
}

#[test]
fn unrelated_errors_do_not_trigger_provider_fallback() -> Result<()> {
    with_home(
        "provider_tests::unrelated_errors_do_not_trigger_provider_fallback",
        &[
            ("WARP_WORKLOAD_TOKEN", "test-workload"),
            ("WARP_ISOLATION_PLATFORM", "docker"),
        ],
        |_| {
            prepare("task")?;
            for message in [
                "Field \"acceptsPartialRefresh\" is not defined by type \"TaskGitCredentialsInput\"",
                "providers request timed out",
                "TaskGitCredentialsInput providers authorization denied",
            ] {
                let mut client = MockAIClient::new();
                client
                    .expect_get_restricted_task_git_credentials()
                    .times(1)
                    .returning(move |_, _, _, _, _, _| {
                        Err(TaskGitCredentialsError::Request(anyhow::anyhow!(message)))
                    });
                let client: Arc<dyn AIClient> = Arc::new(client);
                assert!(
                    block_on(fetch(
                        "task",
                        &client,
                        true,
                        false,
                        Some(vec![TaskGitCredentialProvider::Gitlab])
                    ))
                    .is_err()
                );
            }
            Ok(())
        },
    )
}

#[test]
fn malformed_selective_responses_fail_closed() -> Result<()> {
    with_home(
        "provider_tests::malformed_selective_responses_fail_closed",
        &[],
        |_| {
            for (index, mut output) in [
                response(),
                other_response("gitlab.com"),
                other_response("gitlab.com"),
            ]
            .into_iter()
            .enumerate()
            {
                let owner = ManagedStoreOwner::Task(format!("task-{index}"));
                let directory = prepare(&owner)?;
                apply_refresh(&owner, Ok(validated_response()?))?;
                if index == 1 {
                    output.github_credential_mode = GitHubCredentialMode::Legacy;
                } else if index == 2 {
                    output.failed_hosts.push("gitlab.com".into());
                }
                assert!(
                    apply_partial_response(
                        &owner,
                        output,
                        Some(&[TaskGitCredentialProvider::Gitlab])
                    )
                    .is_err()
                );
                assert!(fs::read(directory.join("credentials"))?.is_empty());
                close_restricted_credentials(&owner)?;
            }
            Ok(())
        },
    )
}

#[test]
fn stale_owner_cannot_write_other_provider_credentials() -> Result<()> {
    with_home(
        "provider_tests::stale_owner_cannot_write_other_provider_credentials",
        &[],
        |home| {
            prepare("current")?;
            let result = apply_partial_response(
                &ManagedStoreOwner::from("stale"),
                other_response("gitlab.com"),
                Some(&[TaskGitCredentialProvider::Gitlab]),
            );
            assert!(result.is_err());
            assert!(!home.join(".git-credentials").exists());
            Ok(())
        },
    )
}

#[test]
fn retry_schedule_does_not_postpone_successful_provider() -> Result<()> {
    with_home(
        "provider_tests::retry_schedule_does_not_postpone_successful_provider",
        &[],
        |_| {
            prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;
            let now = Instant::now();
            let future_deadline = now + Duration::from_secs(20);
            with_store("task", |store| {
                store
                    .provider_refresh_at
                    .insert(TaskGitCredentialProvider::Github, future_deadline);
                store
                    .provider_refresh_at
                    .insert(TaskGitCredentialProvider::Gitlab, now);
                Ok(())
            })?;
            schedule_refresh_retry(&ManagedStoreOwner::from("task"), now)?;
            assert_eq!(refresh_delay("task", now), Duration::from_secs(20));
            with_store("task", |store| {
                assert_eq!(
                    store.provider_refresh_at[&TaskGitCredentialProvider::Github],
                    future_deadline
                );
                assert_eq!(
                    store.provider_refresh_at[&TaskGitCredentialProvider::Gitlab],
                    now + Duration::from_secs(60)
                );
                Ok(())
            })
        },
    )
}
