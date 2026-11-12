use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use command::Stdio;
use warp_cli::agent::{Harness, HarnessTransport};
use warp_graphql::queries::task_restricted_git_credentials::RestrictedTaskGitCredential;

use super::*;
use crate::ai::agent_sdk::driver::{AgentDriverError, AgentDriverOptions, split_run_options};
use crate::server::server_api::ai::MockAIClient;

fn temporary_error() -> TaskGitCredentialsError {
    TaskGitCredentialsError::Request(
        GraphQLError::HttpError {
            status: http::StatusCode::SERVICE_UNAVAILABLE,
            body: String::new(),
        }
        .into(),
    )
}

fn live_response() -> RestrictedTaskGitCredentialsOutput {
    let mut response = response();
    response.credentials[0].expires_at = Some((Utc::now() + TimeDelta::hours(1)).into());
    response
}

#[test]
fn bootstrap_recovers_without_preparing_the_store_again() -> Result<()> {
    with_home(
        "bootstrap_recovers_without_preparing_the_store_again",
        &[
            ("WARP_ISOLATION_PLATFORM", "docker"),
            ("WARP_WORKLOAD_TOKEN", "test-workload-token"),
        ],
        |_| {
            let mut client = MockAIClient::new();
            let mut sequence = mockall::Sequence::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(1)
                .in_sequence(&mut sequence)
                .withf(|task, _, partial, factory| task == "task" && !partial && *factory)
                .returning(|_, _, _, _| Err(temporary_error()));
            client
                .expect_get_restricted_task_git_credentials()
                .times(1)
                .in_sequence(&mut sequence)
                .withf(|task, _, partial, factory| task == "task" && !partial && *factory)
                .returning(|_, _, _, _| Ok(live_response()));
            let client: Arc<dyn AIClient> = Arc::new(client);

            futures::executor::block_on(bootstrap_restricted_credentials("task", &client, true))?;

            assert_eq!(
                fs::read_to_string(task_dir("task")?.join("credentials"))?,
                "https://x-access-token:installation-test-token@github.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn renewed_token_is_not_cleared_at_previous_expiry() -> Result<()> {
    with_home(
        "renewed_token_is_not_cleared_at_previous_expiry",
        &[],
        |_| {
            let directory = prepare("task")?;
            let received_at = Instant::now();
            apply_refresh(
                "task",
                Ok(validate_response(response(), now(), received_at)?),
            )?;
            apply_refresh(
                "task",
                Ok(validate_response(
                    response(),
                    now(),
                    received_at + Duration::from_secs(600),
                )?),
            )?;

            assert_eq!(
                expire_credentials("task", received_at + Duration::from_secs(3600))?,
                Duration::from_secs(600)
            );

            assert_eq!(
                fs::read_to_string(directory.join("credentials"))?,
                "https://x-access-token:installation-test-token@github.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn explicit_platform_denial_overrides_retryable_flag() {
    let error = TaskGitCredentialsError::Platform {
        message: "Access denied".into(),
        detail: None,
        info: Box::new(warp_graphql::platform_error::PlatformErrorInfo::new(
            PlatformErrorCode::NotAuthorized,
            true,
        )),
    };

    assert!(!is_temporary_failure(&error));
}

#[test]
fn retryable_dependency_failure_is_temporary() {
    let error = TaskGitCredentialsError::Platform {
        message: "Dependency unavailable".into(),
        detail: None,
        info: Box::new(warp_graphql::platform_error::PlatformErrorInfo::new(
            PlatformErrorCode::ResourceUnavailable,
            true,
        )),
    };

    assert!(is_temporary_failure(&error));
}

#[test]
fn bootstrap_stops_after_bounded_temporary_failures() -> Result<()> {
    with_home(
        "bootstrap_stops_after_bounded_temporary_failures",
        &[
            ("WARP_ISOLATION_PLATFORM", "docker"),
            ("WARP_WORKLOAD_TOKEN", "test-workload-token"),
        ],
        |_| {
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(4)
                .returning(|_, _, _, _| Err(temporary_error()));
            let client: Arc<dyn AIClient> = Arc::new(client);

            assert!(
                futures::executor::block_on(bootstrap_restricted_credentials(
                    "task", &client, false
                ))
                .is_err()
            );

            assert_eq!(
                fs::read_to_string(task_dir("task")?.join("credentials"))?,
                ""
            );
            Ok(())
        },
    )
}

#[test]
fn bootstrap_does_not_install_partial_credentials() -> Result<()> {
    with_home(
        "bootstrap_does_not_install_partial_credentials",
        &[
            ("WARP_ISOLATION_PLATFORM", "docker"),
            ("WARP_WORKLOAD_TOKEN", "test-workload-token"),
        ],
        |_| {
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(1)
                .withf(|_, _, partial, _| !partial)
                .returning(|_, _, _, _| {
                    let mut response = live_response();
                    response.failed_hosts.push("gitlab.com".into());
                    Ok(response)
                });
            let client: Arc<dyn AIClient> = Arc::new(client);

            assert!(
                futures::executor::block_on(bootstrap_restricted_credentials(
                    "task", &client, false
                ))
                .is_err()
            );

            assert_eq!(
                fs::read_to_string(task_dir("task")?.join("credentials"))?,
                ""
            );
            Ok(())
        },
    )
}

#[test]
fn failed_gitlab_refresh_still_updates_github() -> Result<()> {
    with_home(
        "failed_gitlab_refresh_still_updates_github",
        &[
            ("WARP_ISOLATION_PLATFORM", "docker"),
            ("WARP_WORKLOAD_TOKEN", "test-workload-token"),
        ],
        |home| {
            let directory = prepare("task")?;
            fs::write(
                home.join(".git-credentials"),
                "https://oauth2:retained-token@gitlab.com\n",
            )?;
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(1)
                .withf(|task, _, partial, factory| task == "task" && *partial && *factory)
                .returning(|_, _, _, _| {
                    let mut response = live_response();
                    response.failed_hosts.push("gitlab.com".into());
                    Ok(response)
                });
            let client: Arc<dyn AIClient> = Arc::new(client);

            assert!(
                futures::executor::block_on(refresh_restricted_credentials("task", &client, true))
                    .is_err()
            );

            assert_eq!(
                fs::read_to_string(directory.join("credentials"))?,
                "https://x-access-token:installation-test-token@github.com\n"
            );
            assert_eq!(
                fs::read_to_string(home.join(".git-credentials"))?,
                "https://oauth2:retained-token@gitlab.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn unknown_github_failure_clears_github_and_updates_gitlab() -> Result<()> {
    with_home(
        "unknown_github_failure_clears_github_and_updates_gitlab",
        &[],
        |home| {
            let directory = prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;
            let mut response = response();
            response.credentials = vec![other_forge_credential("gitlab.com")];
            response.failed_hosts.push("github.com".into());

            assert!(
                apply_refresh(
                    "task",
                    Ok(validate_response(response, now(), Instant::now())?)
                )
                .is_err()
            );

            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            assert_eq!(
                fs::read_to_string(home.join(".git-credentials"))?,
                "https://x-access-token:other-forge-test-token@gitlab.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn denied_refresh_clears_github() -> Result<()> {
    with_home("denied_refresh_clears_github", &[], |_| {
        let directory = prepare("task")?;
        apply_refresh("task", Ok(validated_response()?))?;
        let error = TaskGitCredentialsError::Request(
            GraphQLError::HttpError {
                status: http::StatusCode::FORBIDDEN,
                body: String::new(),
            }
            .into(),
        );

        assert!(apply_refresh("task", Err(error)).is_err());

        assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
        Ok(())
    })
}

#[test]
fn temporary_refresh_failure_preserves_token_until_original_expiry() -> Result<()> {
    with_home(
        "temporary_refresh_failure_preserves_token_until_original_expiry",
        &[],
        |_| {
            let directory = prepare("task")?;
            let received_at = Instant::now();
            apply_refresh(
                "task",
                Ok(validate_response(response(), now(), received_at)?),
            )?;
            assert!(apply_refresh("task", Err(temporary_error())).is_err());

            assert_eq!(
                expire_credentials("task", received_at + Duration::from_secs(3599))?,
                Duration::from_secs(1)
            );
            assert_eq!(
                fs::read_to_string(directory.join("credentials"))?,
                "https://x-access-token:installation-test-token@github.com\n"
            );
            expire_credentials("task", received_at + Duration::from_secs(3600))?;

            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            Ok(())
        },
    )
}

fn with_home(
    test_name: &str,
    env: &[(&str, &str)],
    test: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if let Some(home) = std::env::var_os("WARP_TEST_RESTRICTED_GIT_HOME") {
        return test(Path::new(&home));
    }
    let home = tempfile::tempdir()?;
    let mut command = Command::new(std::env::current_exe()?);
    #[cfg(windows)]
    command.creation_flags(0);
    command
        .args([
            "--exact",
            &format!("ai::agent_sdk::driver::git_credentials::restricted::tests::{test_name}"),
            "--nocapture",
        ])
        .current_dir(home.path())
        .env("WARP_TEST_RESTRICTED_GIT_HOME", home.path())
        .env("HOME", home.path())
        .env("GH_CONFIG_DIR", home.path().join("gh"))
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.path().join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("WARP_GITHUB_CREDENTIAL_MODE");
    for name in AUTH_ENV {
        command.env_remove(name);
    }
    command.envs(env.iter().copied());
    let output = command.output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}

fn response() -> RestrictedTaskGitCredentialsOutput {
    RestrictedTaskGitCredentialsOutput {
        github_credential_mode: GitHubCredentialMode::RestrictedInstallation,
        failed_hosts: Vec::new(),
        credentials: vec![RestrictedTaskGitCredential {
            host: "github.com".into(),
            token: "installation-test-token".into(),
            username: Some("test-bot[bot]".into()),
            email: Some("test-bot@users.noreply.github.com".into()),
            expires_at: Some(DateTime::from_timestamp(1_800_003_600, 0).unwrap().into()),
            repository_scope: Some("example/repo".into()),
        }],
    }
}

fn validated_response() -> Result<ValidatedCredentials> {
    validate_response(response(), now(), Instant::now())
}

fn other_forge_credential(host: &str) -> RestrictedTaskGitCredential {
    RestrictedTaskGitCredential {
        host: host.into(),
        token: "other-forge-test-token".into(),
        username: None,
        email: None,
        expires_at: None,
        repository_scope: None,
    }
}

#[test]
fn github_refresh_is_scheduled_ten_minutes_before_expiry() -> Result<()> {
    let received_at = Instant::now();

    let validated = validate_response(response(), now(), received_at)?;

    assert_eq!(
        validated.refresh_at - received_at,
        Duration::from_secs(3000)
    );
    Ok(())
}

#[test]
fn shorter_lived_github_credentials_refresh_earlier() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response.credentials[0].expires_at =
        Some(DateTime::from_timestamp(1_800_001_800, 0).unwrap().into());

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(
        validated.refresh_at - received_at,
        Duration::from_secs(1200)
    );
    Ok(())
}

#[test]
fn longer_lived_github_credentials_still_refresh_within_fifty_minutes() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response.credentials[0].expires_at =
        Some(DateTime::from_timestamp(1_800_007_200, 0).unwrap().into());

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(
        validated.refresh_at - received_at,
        Duration::from_secs(3000)
    );
    Ok(())
}

#[test]
fn credentials_inside_the_refresh_buffer_do_not_cause_a_busy_loop() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response.credentials[0].expires_at =
        Some(DateTime::from_timestamp(1_800_000_360, 0).unwrap().into());

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(validated.refresh_at - received_at, Duration::from_secs(60));
    Ok(())
}

#[test]
fn mixed_gitlab_credentials_keep_the_existing_refresh_interval() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response
        .credentials
        .push(other_forge_credential("gitlab.com"));

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(validated.refresh_at - received_at, Duration::from_secs(60));
    Ok(())
}

#[test]
fn mixed_azure_credentials_keep_the_existing_refresh_interval() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response
        .credentials
        .insert(0, other_forge_credential("dev.azure.com"));

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(validated.refresh_at - received_at, Duration::from_secs(60));
    Ok(())
}

#[test]
fn responses_without_github_keep_the_existing_refresh_interval() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response.credentials = vec![other_forge_credential("gitlab.com")];

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(validated.refresh_at - received_at, Duration::from_secs(60));
    Ok(())
}

#[test]
fn empty_responses_keep_the_existing_refresh_interval() -> Result<()> {
    let received_at = Instant::now();
    let mut response = response();
    response.credentials.clear();

    let validated = validate_response(response, now(), received_at)?;

    assert_eq!(validated.refresh_at - received_at, Duration::from_secs(60));
    Ok(())
}

#[test]
fn setup_time_counts_toward_the_refresh_deadline() -> Result<()> {
    with_home("setup_time_counts_toward_the_refresh_deadline", &[], |_| {
        prepare("task")?;
        let received_at = Instant::now();
        let credentials = validate_response(response(), now(), received_at)?;

        apply_refresh("task", Ok(credentials))?;

        assert_eq!(
            refresh_delay("task", received_at + Duration::from_secs(600)),
            Duration::from_secs(2400)
        );
        Ok(())
    })
}

#[test]
fn overdue_credentials_refresh_without_another_interval() -> Result<()> {
    with_home(
        "overdue_credentials_refresh_without_another_interval",
        &[],
        |_| {
            prepare("task")?;
            let received_at = Instant::now();
            let credentials = validate_response(response(), now(), received_at)?;

            apply_refresh("task", Ok(credentials))?;

            assert_eq!(
                refresh_delay("task", received_at + Duration::from_secs(3600)),
                Duration::ZERO
            );
            Ok(())
        },
    )
}

#[test]
fn missing_expiry_is_not_accepted() {
    let mut response = response();
    response.credentials[0].expires_at = None;

    assert!(validate_response(response, now(), Instant::now()).is_err());
}

#[test]
fn five_minutes_of_remaining_validity_is_not_accepted() {
    let mut response = response();
    response.credentials[0].expires_at =
        Some(DateTime::from_timestamp(1_800_000_300, 0).unwrap().into());

    assert!(validate_response(response, now(), Instant::now()).is_err());
}

#[test]
fn successful_refresh_after_failure_restores_expiry_schedule() -> Result<()> {
    with_home(
        "successful_refresh_after_failure_restores_expiry_schedule",
        &[],
        |_| {
            prepare("task")?;
            let error = TaskGitCredentialsError::Request(anyhow::anyhow!("test failure"));
            assert!(apply_refresh("task", Err(error)).is_err());
            let received_at = Instant::now();

            apply_refresh(
                "task",
                Ok(validate_response(response(), now(), received_at)?),
            )?;

            assert_eq!(
                refresh_delay("task", received_at),
                Duration::from_secs(3000)
            );
            Ok(())
        },
    )
}

#[test]
fn failed_credential_write_does_not_retain_a_long_refresh_deadline() -> Result<()> {
    with_home(
        "failed_credential_write_does_not_retain_a_long_refresh_deadline",
        &[],
        |_| {
            let directory = prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;
            fs::rename(
                directory.join("credentials"),
                directory.join("previous-credentials"),
            )?;
            fs::create_dir(directory.join("credentials"))?;

            assert!(apply_refresh("task", Ok(validated_response()?)).is_err());

            assert_eq!(
                refresh_delay("task", Instant::now()),
                Duration::from_secs(60)
            );
            Ok(())
        },
    )
}

#[test]
fn unknown_mode_uses_legacy_credentials() -> Result<()> {
    with_home(
        "unknown_mode_uses_legacy_credentials",
        &[("WARP_GITHUB_CREDENTIAL_MODE", "future_mode")],
        |_| {
            assert!(!restricted_github_mode()?);
            Ok(())
        },
    )
}

#[test]
fn restricted_mode_uses_restricted_credentials() -> Result<()> {
    with_home(
        "restricted_mode_uses_restricted_credentials",
        &[("WARP_GITHUB_CREDENTIAL_MODE", "restricted_installation")],
        |_| {
            assert!(restricted_github_mode()?);
            Ok(())
        },
    )
}

#[test]
fn absent_mode_uses_legacy_credentials() -> Result<()> {
    with_home("absent_mode_uses_legacy_credentials", &[], |_| {
        assert!(!restricted_github_mode()?);
        Ok(())
    })
}

#[test]
fn legacy_response_is_not_accepted_in_restricted_mode() {
    let mut response = response();
    response.github_credential_mode = GitHubCredentialMode::Legacy;
    assert!(validate_response(response, now(), Instant::now()).is_err());
}

#[test]
fn expiring_installation_token_is_not_installed() {
    let mut response = response();
    response.credentials[0].expires_at = Some(now().into());
    assert!(validate_response(response, now(), Instant::now()).is_err());
}

#[test]
fn installation_scope_is_required() {
    let mut response = response();
    response.credentials[0].repository_scope = None;
    assert!(validate_response(response, now(), Instant::now()).is_err());
}

#[test]
fn contradictory_host_results_are_not_accepted() {
    let mut response = response();
    response.failed_hosts.push("github.com".into());
    assert!(validate_response(response, now(), Instant::now()).is_err());
}

#[test]
fn refresh_does_not_change_fixed_bot_identity() -> Result<()> {
    with_home("refresh_does_not_change_fixed_bot_identity", &[], |_| {
        prepare("task")?;
        git_config(&["user.name", "fixed-bot[bot]"])?;
        git_config(&["user.email", "fixed-bot@example.com"])?;

        apply_refresh("task", Ok(validated_response()?))?;

        let output = Command::new("git")
            .args(["config", "--global", "user.name"])
            .output()?;
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout)?, "fixed-bot[bot]\n");
        Ok(())
    })
}

#[test]
fn github_refresh_failure_preserves_other_forge_store() -> Result<()> {
    with_home(
        "github_refresh_failure_preserves_other_forge_store",
        &[],
        |home| {
            prepare("task")?;
            let mut credentials = validated_response()?;
            credentials.credentials.push(GitCredential {
                host: "gitlab.com".into(),
                token: "gitlab-test-token".into(),
                username: Some("oauth2".into()),
                email: None,
            });
            apply_refresh("task", Ok(credentials))?;

            assert!(
                apply_refresh(
                    "task",
                    Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
                        "test failure"
                    )))
                )
                .is_err()
            );
            assert_eq!(
                fs::read_to_string(home.join(".git-credentials"))?,
                "https://oauth2:gitlab-test-token@gitlab.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn third_party_run_options_preserve_restricted_github_environment() -> Result<()> {
    with_home(
        "third_party_run_options_preserve_restricted_github_environment",
        &[("WARP_GITHUB_CREDENTIAL_MODE", "restricted_installation")],
        |home| {
            for harness in [
                Harness::Claude,
                Harness::Codex,
                Harness::Gemini,
                Harness::Pi,
                Harness::OpenCode,
            ] {
                for transport in [HarnessTransport::Pty, HarnessTransport::Acp] {
                    let (_, config) = split_run_options(AgentDriverOptions {
                        working_dir: home.to_path_buf(),
                        task_id: Some("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()),
                        selected_harness: harness,
                        harness_transport: transport,
                        environment_variables: HashMap::from([
                            ("GH_TOKEN".to_owned(), "task-provided-test-token".to_owned()),
                            ("RUN_SETTING".to_owned(), "keep".to_owned()),
                        ]),
                        ..Default::default()
                    })
                    .unwrap();

                    assert_eq!(config.resolved_env_vars[OsStr::new("GH_TOKEN")], "");
                    assert_eq!(config.resolved_env_vars[OsStr::new("GITHUB_TOKEN")], "");
                    assert_eq!(config.resolved_env_vars[OsStr::new("RUN_SETTING")], "keep");
                    assert_eq!(
                        config.resolved_env_vars[OsStr::new("WARP_GITHUB_CREDENTIAL_MODE")],
                        "restricted_installation"
                    );
                    assert_eq!(
                        config.resolved_env_vars[OsStr::new("GH_CONFIG_DIR")],
                        home.join(".warp")
                            .join("github-git")
                            .join("550e8400-e29b-41d4-a716-446655440000")
                            .join("gh")
                            .into_os_string()
                    );
                }
            }
            Ok(())
        },
    )
}

#[test]
fn third_party_run_options_reject_git_configuration_overrides() -> Result<()> {
    with_home(
        "third_party_run_options_reject_git_configuration_overrides",
        &[("WARP_GITHUB_CREDENTIAL_MODE", "restricted_installation")],
        |home| {
            let result = split_run_options(AgentDriverOptions {
                working_dir: home.to_path_buf(),
                task_id: Some("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()),
                selected_harness: Harness::Claude,
                environment_variables: HashMap::from([(
                    "GIT_CONFIG_COUNT".to_owned(),
                    "1".to_owned(),
                )]),
                ..Default::default()
            });
            assert!(matches!(
                result,
                Err(AgentDriverError::ConfigBuildFailed(_))
            ));
            Ok(())
        },
    )
}

#[test]
fn cleared_github_helper_does_not_use_legacy_store() -> Result<()> {
    with_home(
        "cleared_github_helper_does_not_use_legacy_store",
        &[],
        |home| {
            let directory = prepare("task")?;
            install_helper(&directory, &[])?;
            apply_refresh("task", Ok(validated_response()?))?;
            fs::write(
                home.join(".git-credentials"),
                "https://legacy:legacy-test-token@github.com\n",
            )?;
            clear_restricted_credentials("task")?;

            let mut command = Command::new("git")
                .args(["credential", "fill"])
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_ASKPASS", "")
                .env("SSH_ASKPASS", "")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            command
                .stdin
                .take()
                .unwrap()
                .write_all(b"protocol=https\nhost=github.com\n\n")?;
            let output = command.wait_with_output()?;

            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert_eq!(
                fs::read_to_string(home.join(".git-credentials"))?,
                "https://legacy:legacy-test-token@github.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn existing_personal_store_is_not_changed() -> Result<()> {
    with_home("existing_personal_store_is_not_changed", &[], |home| {
        let path = home.join(".git-credentials");
        fs::write(&path, "personal-test-fixture")?;
        assert!(prepare("task").is_err());
        assert_eq!(fs::read_to_string(path)?, "personal-test-fixture");
        Ok(())
    })
}

#[test]
fn inherited_github_token_requires_fresh_sandbox() -> Result<()> {
    with_home(
        "inherited_github_token_requires_fresh_sandbox",
        &[("GH_TOKEN", "personal-test-fixture")],
        |home| {
            assert!(prepare("task").is_err());
            assert!(!home.join(".warp/github-git/task").exists());
            Ok(())
        },
    )
}

#[test]
fn restricted_store_does_not_write_gh_credentials() -> Result<()> {
    with_home(
        "restricted_store_does_not_write_gh_credentials",
        &[],
        |home| {
            let directory = prepare("task")?;
            install_helper(&directory, &[])?;
            apply_refresh("task", Ok(validated_response()?))?;

            assert_eq!(
                fs::read_to_string(directory.join("credentials"))?,
                "https://x-access-token:installation-test-token@github.com\n"
            );
            assert!(!home.join(".git-credentials").exists());
            assert!(!gh_config_dir(home).join("hosts.yml").exists());
            #[cfg(unix)]
            {
                assert_eq!(
                    fs::metadata(directory.join("credentials"))?
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            Ok(())
        },
    )
}

#[test]
fn unknown_refresh_failure_clears_managed_github_store() -> Result<()> {
    with_home(
        "unknown_refresh_failure_clears_managed_github_store",
        &[],
        |_| {
            let directory = prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;

            let error = TaskGitCredentialsError::Request(anyhow::anyhow!("unknown test failure"));
            assert!(apply_refresh("task", Err(error)).is_err());
            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            assert_eq!(
                refresh_delay("task", Instant::now()),
                Duration::from_secs(60)
            );
            Ok(())
        },
    )
}

#[test]
fn missing_github_credential_clears_managed_store() -> Result<()> {
    with_home(
        "missing_github_credential_clears_managed_store",
        &[],
        |_| {
            let directory = prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;

            let received_at = Instant::now();
            let mut response = response();
            response.credentials.clear();
            apply_refresh("task", Ok(validate_response(response, now(), received_at)?))?;
            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            assert_eq!(refresh_delay("task", received_at), Duration::from_secs(60));
            Ok(())
        },
    )
}

#[test]
fn completed_run_cannot_restore_credentials() -> Result<()> {
    with_home("completed_run_cannot_restore_credentials", &[], |_| {
        let directory = prepare("task")?;
        apply_refresh("task", Ok(validated_response()?))?;
        close_restricted_credentials("task")?;

        assert!(apply_refresh("task", Ok(validated_response()?)).is_err());
        assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
        assert_eq!(
            refresh_delay("task", Instant::now()),
            Duration::from_secs(60)
        );
        Ok(())
    })
}

#[test]
fn another_task_cannot_replace_or_clear_managed_credentials() -> Result<()> {
    with_home(
        "another_task_cannot_replace_or_clear_managed_credentials",
        &[],
        |_| {
            let directory = prepare("task")?;
            let received_at = Instant::now();
            apply_refresh(
                "task",
                Ok(validate_response(response(), now(), received_at)?),
            )?;
            close_restricted_credentials("other-task")?;

            assert!(prepare("other-task").is_err());
            assert!(apply_refresh("other-task", Ok(validated_response()?)).is_err());
            assert_eq!(
                refresh_delay("other-task", received_at),
                Duration::from_secs(60)
            );
            assert_eq!(
                refresh_delay("task", received_at),
                Duration::from_secs(3000)
            );
            assert_eq!(
                fs::read_to_string(directory.join("credentials"))?,
                "https://x-access-token:installation-test-token@github.com\n"
            );
            Ok(())
        },
    )
}

#[test]
fn child_environment_removes_github_authentication() -> Result<()> {
    with_home(
        "child_environment_removes_github_authentication",
        &[],
        |home| {
            let env = restricted_child_environment("task")?;
            assert_eq!(env.get(OsStr::new("GH_TOKEN")), Some(&OsString::new()));
            assert_eq!(env.get(OsStr::new("GITHUB_TOKEN")), Some(&OsString::new()));
            assert_eq!(
                env.get(OsStr::new("GH_CONFIG_DIR")),
                Some(
                    &home
                        .join(".warp")
                        .join("github-git")
                        .join("task")
                        .join("gh")
                        .into_os_string()
                )
            );
            Ok(())
        },
    )
}
