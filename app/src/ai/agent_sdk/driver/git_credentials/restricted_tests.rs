use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use command::Stdio;
use warp_cli::agent::{Harness, HarnessTransport};
use warp_graphql::queries::task_restricted_git_credentials::RestrictedTaskGitCredential;

use super::*;
use crate::ai::agent_sdk::driver::git_credentials::gh_config_dir;
use crate::ai::agent_sdk::driver::{AgentDriverError, AgentDriverOptions, split_run_options};

pub(super) fn with_home(
    test_name: &str,
    env: &[(&str, &str)],
    test: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if let Some(home) = std::env::var_os("WARP_TEST_RESTRICTED_GIT_HOME") {
        return test(Path::new(&home));
    }
    let home = tempfile::tempdir()?;
    let test_name = if test_name.contains("::") {
        test_name.to_owned()
    } else {
        format!("tests::{test_name}")
    };
    let mut command = Command::new(std::env::current_exe()?);
    #[cfg(windows)]
    command.creation_flags(0);
    command
        .args([
            "--exact",
            &format!("ai::agent_sdk::driver::git_credentials::restricted::{test_name}"),
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
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}

pub(super) fn response() -> RestrictedTaskGitCredentialsOutput {
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

pub(super) fn validated_response() -> Result<ValidatedCredentials> {
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

            let delay = refresh_delay("task", Instant::now());
            assert!(delay <= Duration::from_secs(60) && delay > Duration::ZERO);
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
fn partial_response_is_not_accepted() {
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
        assert!(prepare("task").is_ok());
        assert_eq!(fs::read_to_string(path)?, "personal-test-fixture");
        Ok(())
    })
}

#[test]
fn inherited_github_token_is_cleared_from_child_environment() -> Result<()> {
    with_home(
        "inherited_github_token_is_cleared_from_child_environment",
        &[("GH_TOKEN", "personal-test-fixture")],
        |home| {
            assert!(prepare("task").is_ok());
            assert!(home.join(".warp/github-git/task").exists());
            assert_eq!(
                restricted_child_environment("task")?[OsStr::new("GH_TOKEN")],
                ""
            );
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
fn unstructured_failed_refresh_clears_managed_github_store() -> Result<()> {
    with_home(
        "unstructured_failed_refresh_clears_managed_github_store",
        &[],
        |_| {
            let directory = prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;

            let error = TaskGitCredentialsError::Request(anyhow::anyhow!("transient test failure"));
            assert!(apply_refresh("task", Err(error)).is_err());
            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            let delay = refresh_delay("task", Instant::now());
            assert!(delay <= Duration::from_secs(60) && delay > Duration::ZERO);
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

#[test]
fn ambient_helpers_and_credentials_do_not_block_managed_setup() -> Result<()> {
    with_home(
        "ambient_helpers_and_credentials_do_not_block_managed_setup",
        &[
            ("GH_TOKEN", "ambient-test-token"),
            ("GIT_CONFIG_COUNT", "0"),
        ],
        |home| {
            fs::write(home.join(".netrc"), "unrelated fixture")?;
            fs::write(home.join(".git-credentials"), "unrelated fixture")?;
            git_config(&["--add", "credential.helper", "osxkeychain"])?;
            git_config(&["--add", "credential.helper", "store"])?;
            let directory = prepare("task")?;
            install_helper(&directory, &[])?;
            assert_eq!(
                fs::read_to_string(home.join(".netrc"))?,
                "unrelated fixture"
            );
            assert_eq!(
                fs::read_to_string(home.join(".git-credentials"))?,
                "unrelated fixture"
            );
            assert_eq!(
                restricted_child_environment("task")?[OsStr::new("GH_TOKEN")],
                ""
            );
            let output = Command::new("git")
                .args(["config", "--file"])
                .arg(home.join(".gitconfig"))
                .args(["--get-all", "credential.helper"])
                .output()?;
            assert_eq!(
                String::from_utf8(output.stdout)?.trim(),
                READ_ONLY_CREDENTIAL_HELPER
            );
            Ok(())
        },
    )
}

#[test]
fn config_count_allows_only_absent_empty_or_exact_zero() {
    for (value, accepted) in [
        (None, true),
        (Some(""), true),
        (Some("0"), true),
        (Some("00"), false),
        (Some("1"), false),
        (Some(" 0"), false),
    ] {
        assert_eq!(valid_config_count(value.map(OsStr::new)), accepted);
        let env = value
            .map(|value| HashMap::from([("GIT_CONFIG_COUNT".into(), value.into())]))
            .unwrap_or_default();
        assert_eq!(
            validate_restricted_child_environment(&env).is_ok(),
            accepted
        );
    }
}

#[test]
fn only_positive_transient_causes_allow_retention_and_denial_takes_priority() {
    for (status, expected) in [
        (401, false),
        (403, false),
        (404, false),
        (408, true),
        (429, true),
        (503, true),
    ] {
        let error =
            TaskGitCredentialsError::Request(HttpStatusError::new(status, String::new()).into());
        assert_eq!(is_transient_credential_error(&error), expected);
    }
    assert!(!is_transient_credential_error(
        &TaskGitCredentialsError::Request(anyhow::anyhow!("connection reset"))
    ));
    let denial = anyhow::Error::new(HttpStatusError::new(403, String::new()))
        .context(HttpStatusError::new(503, String::new()));
    assert!(!is_transient_request_error(&denial));
}

#[test]
fn transient_failure_keeps_only_unexpired_credentials_without_extending_expiry() -> Result<()> {
    with_home(
        "transient_failure_keeps_only_unexpired_credentials_without_extending_expiry",
        &[],
        |_| {
            let directory = prepare("task")?;
            let received_at = Instant::now();
            let response = validate_response(response(), now(), received_at)?;
            let expiry = response.expires_at.unwrap();
            apply_refresh("task", Ok(response))?;
            let transient =
                TaskGitCredentialsError::Request(HttpStatusError::new(503, String::new()).into());
            assert!(apply_refresh("task", Err(transient)).is_err());
            assert!(!fs::read_to_string(directory.join("credentials"))?.is_empty());
            assert_eq!(
                with_store("task", |store| Ok(store.expires_at))?,
                Some(expiry)
            );
            credential_fetch_failed("task", true, expiry)?;
            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            Ok(())
        },
    )
}

#[test]
fn expiry_clears_token_without_changing_the_refresh_schedule() -> Result<()> {
    with_home(
        "expiry_clears_token_without_changing_the_refresh_schedule",
        &[],
        |_| {
            let directory = prepare("task")?;
            let received_at = Instant::now();
            apply_refresh(
                "task",
                Ok(validate_response(response(), now(), received_at)?),
            )?;
            let task_id = "task";
            schedule_refresh_retry(task_id, received_at + Duration::from_secs(3590))?;
            expire_credentials(task_id, received_at + Duration::from_secs(3600))?;
            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            assert_eq!(
                refresh_delay(task_id, received_at + Duration::from_secs(3600)),
                Duration::from_secs(50)
            );
            Ok(())
        },
    )
}

#[test]
fn partial_refresh_keeps_valid_github_when_other_forge_write_fails() -> Result<()> {
    with_home(
        "partial_refresh_keeps_valid_github_when_other_forge_write_fails",
        &[],
        |home| {
            let directory = prepare("task")?;
            fs::create_dir(home.join(".git-credentials.tmp"))?;
            let mut response = response();
            response.credentials[0].expires_at = Some((Utc::now() + TimeDelta::hours(1)).into());
            response
                .credentials
                .push(other_forge_credential("gitlab.com"));
            assert!(apply_partial_response("task", response).is_err());
            assert!(!fs::read_to_string(directory.join("credentials"))?.is_empty());
            Ok(())
        },
    )
}

#[test]
fn partial_refresh_clears_failed_github_and_updates_other_forge() -> Result<()> {
    with_home(
        "partial_refresh_clears_failed_github_and_updates_other_forge",
        &[],
        |home| {
            let directory = prepare("task")?;
            apply_refresh("task", Ok(validated_response()?))?;
            let response = RestrictedTaskGitCredentialsOutput {
                github_credential_mode: GitHubCredentialMode::RestrictedInstallation,
                credentials: vec![other_forge_credential("gitlab.com")],
                failed_hosts: vec!["github.com".into()],
            };
            assert!(apply_partial_response("task", response).is_err());
            assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
            assert!(fs::read_to_string(home.join(".git-credentials"))?.contains("gitlab.com"));
            Ok(())
        },
    )
}

#[test]
fn store_guard_closes_credentials_on_early_exit() -> Result<()> {
    with_home("store_guard_closes_credentials_on_early_exit", &[], |_| {
        let directory = prepare("task")?;
        {
            let _guard = CredentialStoreGuard::new("task".into());
            apply_refresh("task", Ok(validated_response()?))?;
        }
        assert_eq!(fs::read_to_string(directory.join("credentials"))?, "");
        assert!(apply_refresh("task", Ok(validated_response()?)).is_err());
        Ok(())
    })
}
