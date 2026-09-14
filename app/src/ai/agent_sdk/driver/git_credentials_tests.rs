use warp_graphql::ai::PlatformErrorCode;
use warp_graphql::error::{
    PlatformError as GraphqlPlatformError, UserFacingError, UserFacingErrorInterface,
};
use warp_graphql::platform_error::{
    PlatformErrorInfo, PlatformErrorInfoResponse, PlatformErrorMetadataResponse,
};
use warp_graphql::response_context::ResponseContext;

use super::*;
use crate::server::server_api::ai::TaskGitCredentialsResponse;

#[test]
fn from_user_facing_converts_platform_error_preserving_metadata_and_debug() {
    let error = TaskGitCredentialsError::from_user_facing(UserFacingError {
        error: UserFacingErrorInterface::PlatformError(GraphqlPlatformError {
            message: "GitHub is temporarily unavailable.".to_string(),
            detail: Some("Repository access could not be resolved.".to_string()),
            info: PlatformErrorInfoResponse {
                code: PlatformErrorCode::ResourceUnavailable,
                retryable: true,
                metadata: vec![
                    PlatformErrorMetadataResponse {
                        key: "provider".to_string(),
                        value: "github".to_string(),
                    },
                    PlatformErrorMetadataResponse {
                        key: "resource".to_string(),
                        value: "installation".to_string(),
                    },
                ],
                debug: Some("request-id=dogfood-only".to_string()),
            },
        }),
        response_context: ResponseContext {
            server_version: None,
        },
    });

    match error {
        TaskGitCredentialsError::Platform {
            message,
            detail,
            info,
        } => {
            assert_eq!(message, "GitHub is temporarily unavailable.");
            assert_eq!(info.code, PlatformErrorCode::ResourceUnavailable);
            assert!(info.retryable);
            assert_eq!(
                detail.as_deref(),
                Some("Repository access could not be resolved.")
            );
            assert_eq!(info.metadata["provider"], "github");
            assert_eq!(info.metadata["resource"], "installation");
            assert_eq!(info.debug.as_deref(), Some("request-id=dogfood-only"));
        }
        error => panic!("expected structured platform error, got {error:?}"),
    }
}

fn dependency_error(retryable: bool) -> TaskGitCredentialsError {
    TaskGitCredentialsError::Platform {
        message: "GitHub is temporarily unavailable.".to_string(),
        detail: Some("Repository access could not be resolved.".to_string()),
        info: PlatformErrorInfo {
            code: PlatformErrorCode::ResourceUnavailable,
            retryable,
            metadata: std::collections::BTreeMap::from([
                ("provider".to_string(), "github".to_string()),
                ("resource".to_string(), "installation".to_string()),
            ]),
            debug: None,
        },
    }
}

#[test]
fn is_retryable_treats_retryable_platform_error_as_retryable() {
    assert!(is_retryable(&dependency_error(true)));
}

#[test]
fn is_retryable_treats_non_retryable_platform_error_as_non_retryable() {
    assert!(!is_retryable(&dependency_error(false)));
}

#[test]
fn is_retryable_treats_unstructured_error_as_non_retryable() {
    let error = TaskGitCredentialsError::Unstructured {
        message: "Unable to access task git credentials".to_string(),
    };
    assert!(!is_retryable(&error));
}

#[test]
fn is_retryable_treats_generic_request_error_as_retryable() {
    let error = TaskGitCredentialsError::Request(anyhow::anyhow!("transient request failure"));
    assert!(is_retryable(&error));
}

#[test]
fn is_retryable_treats_missing_isolation_platform_as_non_retryable() {
    let error = TaskGitCredentialsError::Request(anyhow::anyhow!(
        warp_isolation_platform::IsolationPlatformError::NoIsolationPlatformDetected
    ));
    assert!(!is_retryable(&error));
}

#[test]
fn write_gh_hosts_yml_uses_gh_cli_filename() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let gh_config_dir = temp_dir.path().join(".config").join("gh");

    write_gh_hosts_yml(
        &[GitCredential {
            token: "token".to_string(),
            username: Some("octocat".to_string()),
            email: Some("octocat@example.com".to_string()),
            host: "github.com".to_string(),
        }],
        temp_dir.path(),
    )?;

    let hosts_path = gh_config_dir.join(GH_HOSTS_FILENAME);
    assert!(hosts_path.exists());
    assert!(
        !gh_config_dir
            .join(format!("{GH_HOSTS_FILENAME}.tmp"))
            .exists()
    );

    let hosts = std::fs::read_to_string(hosts_path)?;
    assert!(hosts.contains("github.com:"));
    assert!(hosts.contains("    oauth_token: token"));
    assert!(hosts.contains("    git_protocol: https"));
    assert!(hosts.contains("    user: octocat"));

    Ok(())
}

#[test]
fn write_gh_hosts_yml_excludes_gitlab_credentials() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let gh_config_dir = temp_dir.path().join(".config").join("gh");

    write_gh_hosts_yml(
        &[
            GitCredential {
                token: "github-token".to_string(),
                username: Some("octocat".to_string()),
                email: None,
                host: "github.com".to_string(),
            },
            GitCredential {
                token: "gitlab-token".to_string(),
                username: Some("oauth2".to_string()),
                email: None,
                host: "gitlab.com".to_string(),
            },
        ],
        temp_dir.path(),
    )?;

    let hosts = std::fs::read_to_string(gh_config_dir.join(GH_HOSTS_FILENAME))?;
    assert!(hosts.contains("github.com:"));
    assert!(!hosts.contains("gitlab.com:"));
    assert!(!hosts.contains("gitlab-token"));

    Ok(())
}

#[test]
fn write_gh_hosts_yml_skips_gitlab_only_credentials() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;

    write_gh_hosts_yml(
        &[GitCredential {
            token: "gitlab-token".to_string(),
            username: Some("oauth2".to_string()),
            email: None,
            host: "gitlab.com".to_string(),
        }],
        temp_dir.path(),
    )?;

    assert!(!temp_dir.path().join(".config").join("gh").exists());

    Ok(())
}

fn github_credential() -> GitCredential {
    GitCredential {
        token: "github-token".to_string(),
        username: None,
        email: None,
        host: "github.com".to_string(),
    }
}
fn azure_devops_credential(token: &str) -> GitCredential {
    GitCredential {
        token: token.to_string(),
        username: None,
        email: None,
        host: AZURE_DEVOPS_HOST.to_string(),
    }
}

fn gitlab_credential() -> GitCredential {
    GitCredential {
        token: "gitlab-token".to_string(),
        username: Some("oauth2".to_string()),
        email: None,
        host: "gitlab.com".to_string(),
    }
}

#[test]
fn merged_credentials_include_each_provider_host() {
    let content = merge_git_credentials_file_content(
        "",
        &[
            github_credential(),
            gitlab_credential(),
            azure_devops_credential("azure-token"),
        ],
    );

    assert_eq!(
        content,
        "https://x-access-token:github-token@github.com\n\
         https://oauth2:gitlab-token@gitlab.com\n\
         https://x-access-token:azure-token@dev.azure.com\n"
    );
}

#[cfg(unix)]
#[test]
fn azure_cli_wrapper_uses_refreshed_entra_token() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let azure_cli = temp_dir.path().join("real-az");
    std::fs::write(
        &azure_cli,
        "#!/bin/sh\n\
         test \"$AZURE_DEVOPS_EXT_PAT\" = \"$EXPECTED_TOKEN\" && \
         test -z \"${AZURE_DEVOPS_TOKEN+x}\"\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&azure_cli, std::fs::Permissions::from_mode(0o700))?;
    }

    let initial = azure_devops_credential("initial-token");
    write_azure_cli_auth_for_executable(&initial, temp_dir.path(), &azure_cli)?;
    let wrapper = azure_cli_wrapper_path(temp_dir.path());
    let initial_output = BlockingCommand::new(&wrapper)
        .env("EXPECTED_TOKEN", "initial-token")
        .env_remove("AZURE_DEVOPS_EXT_PAT")
        .env_remove("AZURE_DEVOPS_TOKEN")
        .output()?;
    assert!(initial_output.status.success());

    let refreshed = azure_devops_credential("refreshed-token");
    write_azure_cli_auth(&[refreshed], temp_dir.path())?;
    let refreshed_output = BlockingCommand::new(&wrapper)
        .env("EXPECTED_TOKEN", "refreshed-token")
        .env_remove("AZURE_DEVOPS_EXT_PAT")
        .env_remove("AZURE_DEVOPS_TOKEN")
        .output()?;
    assert!(refreshed_output.status.success());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let token_path = azure_devops_auth_dir(temp_dir.path()).join(AZURE_DEVOPS_TOKEN_FILENAME);
        assert_eq!(
            std::fs::metadata(token_path)?.permissions().mode() & 0o777,
            0o600
        );
    }

    Ok(())
}

#[cfg(unix)]
#[test]
fn azure_cli_token_write_does_not_follow_predictable_temp_symlink() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let temp_dir = tempfile::tempdir()?;
    let auth_dir = azure_devops_auth_dir(temp_dir.path());
    std::fs::create_dir_all(&auth_dir)?;
    let victim = temp_dir.path().join("victim");
    std::fs::write(&victim, "unchanged")?;
    let predictable_temp_path = auth_dir.join(format!("{AZURE_DEVOPS_TOKEN_FILENAME}.tmp"));
    symlink(&victim, &predictable_temp_path)?;

    let azure_cli = temp_dir.path().join("real-az");
    std::fs::write(&azure_cli, "#!/bin/sh\n")?;
    std::fs::set_permissions(&azure_cli, std::fs::Permissions::from_mode(0o700))?;
    write_azure_cli_auth_for_executable(
        &azure_devops_credential("azure-token"),
        temp_dir.path(),
        &azure_cli,
    )?;

    assert_eq!(std::fs::read_to_string(&victim)?, "unchanged");
    assert!(
        std::fs::symlink_metadata(&predictable_temp_path)?
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(auth_dir.join(AZURE_DEVOPS_TOKEN_FILENAME))?,
        "azure-token"
    );
    Ok(())
}
#[test]
fn azure_cli_wrapper_path_is_injected_without_a_token_env_var() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let credential = azure_devops_credential("token");
    let azure_cli = temp_dir.path().join("real-az");
    std::fs::write(&azure_cli, "")?;
    write_azure_cli_auth_for_executable(&credential, temp_dir.path(), &azure_cli)?;

    let mut env_vars = HashMap::from([(OsString::from("PATH"), OsString::from("/usr/bin"))]);
    prepend_azure_cli_wrapper_to_path_for_home(&mut env_vars, temp_dir.path())?;

    let path = env_vars.get(OsStr::new("PATH")).expect("PATH is set");
    assert_eq!(
        std::env::split_paths(path).next(),
        azure_cli_wrapper_path(temp_dir.path())
            .parent()
            .map(Path::to_path_buf)
    );
    assert!(!env_vars.contains_key(OsStr::new("AZURE_DEVOPS_EXT_PAT")));
    assert!(!env_vars.contains_key(OsStr::new("AZURE_DEVOPS_TOKEN")));
    Ok(())
}

#[test]
fn merged_credentials_replace_only_the_refreshed_host() {
    let existing = "https://x-access-token:stale-github@github.com\n\
                    https://oauth2:stale-gitlab@gitlab.com\n";

    let content = merge_git_credentials_file_content(existing, &[github_credential()]);

    assert!(content.contains("https://x-access-token:github-token@github.com"));
    assert!(!content.contains("stale-github"));
    assert!(content.contains("https://oauth2:stale-gitlab@gitlab.com"));
}

#[test]
fn merged_credentials_preserve_an_unrelated_host() {
    let existing = "https://user:token@git.example.com\n";

    let content = merge_git_credentials_file_content(existing, &[github_credential()]);

    assert_eq!(
        content,
        "https://user:token@git.example.com\n\
         https://x-access-token:github-token@github.com\n"
    );
}

#[test]
fn credential_diagnostics_reports_presence_without_values() {
    let diagnostics = credential_diagnostics(
        &[GitCredential {
            token: "secret-token".to_string(),
            username: Some("oauth2".to_string()),
            email: Some("user@example.com".to_string()),
            host: "gitlab.com".to_string(),
        }],
        &[],
    );

    assert_eq!(
        diagnostics,
        "gitlab.com(refreshed, token_present=true, username_present=true)"
    );
    assert!(!diagnostics.contains("secret-token"));
    assert!(!diagnostics.contains("oauth2"));
    assert!(!diagnostics.contains("user@example.com"));
}

#[test]
fn credential_diagnostics_names_the_stale_host() {
    let diagnostics = credential_diagnostics(&[github_credential()], &["gitlab.com".to_string()]);

    assert!(diagnostics.contains("github.com(refreshed"));
    assert!(diagnostics.contains("gitlab.com(stale"));
}

#[test]
fn repository_identity_selects_the_matching_host() {
    let identities = [
        HostIdentity {
            host: "github.com".to_string(),
            name: "warp-agent[bot]".to_string(),
            email: "bot@users.noreply.github.com".to_string(),
        },
        HostIdentity {
            host: "gitlab.com".to_string(),
            name: "warp-factory-1".to_string(),
            email: "1-warp-factory-1@users.noreply.gitlab.com".to_string(),
        },
    ];

    let matched = select_host_identity(&identities, "gitlab.com").expect("an identity");
    assert_eq!(matched.name, "warp-factory-1");
    assert_eq!(matched.email, "1-warp-factory-1@users.noreply.gitlab.com");
}

#[test]
fn repository_identity_falls_back_to_the_primary_forge() {
    let identities = [HostIdentity {
        host: "github.com".to_string(),
        name: "warp-agent[bot]".to_string(),
        email: "bot@users.noreply.github.com".to_string(),
    }];

    let matched = select_host_identity(&identities, "gitlab.com").expect("an identity");
    assert_eq!(matched.name, "warp-agent[bot]");

    assert!(select_host_identity(&[], "github.com").is_none());
}

fn init_repo(dir: &std::path::Path) {
    BlockingCommand::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir)
        .output()
        .expect("git init should succeed");
}

#[test]
#[serial_test::serial]
fn global_git_identity_reads_the_actual_global_config() -> Result<()> {
    // #[serial] because this exercises the real `git config --global`
    // invocation, which reads/writes process-wide state (the `--global`
    // config file resolved from HOME) rather than a repo-local temp dir.
    let temp_home = tempfile::tempdir()?;
    let prev_home = std::env::var_os("HOME");
    let prev_git_config_global = std::env::var_os("GIT_CONFIG_GLOBAL");
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HOME", temp_home.path()) };
    // A `GIT_CONFIG_GLOBAL` override in the ambient environment would take
    // priority over HOME and defeat this test's isolation.
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("GIT_CONFIG_GLOBAL") };

    let attempt = || -> Result<()> {
        assert_eq!(
            global_git_identity(),
            None,
            "no global identity should be configured in the fresh temp HOME yet"
        );

        run_git_config("user.name", "Warp");
        run_git_config("user.email", "agent@warp.dev");

        // This is the regression this test guards: `global_git_identity()` must
        // issue `git config --global --get <key>` (an option to the `config`
        // subcommand). Building it as `git --global config --get <key>` instead
        // (`--global` as a top-level git option, which git rejects) would make
        // this call fail silently and always return `None`, permanently
        // defeating the `configure_repository_git_identity_if_unset` fallback
        // logic that depends on a real baseline.
        assert_eq!(
            global_git_identity(),
            Some(("Warp".to_string(), "agent@warp.dev".to_string()))
        );
        Ok(())
    };
    let result = attempt();

    match prev_home {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(home) => unsafe { std::env::set_var("HOME", home) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var("HOME") },
    }
    if let Some(value) = prev_git_config_global {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("GIT_CONFIG_GLOBAL", value) }
    }
    result
}

#[test]
fn repository_identity_is_unchanged_when_it_matches_the_baseline() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_repo(temp_dir.path());
    run_repository_git_config(temp_dir.path(), "user.name", "Warp");
    run_repository_git_config(temp_dir.path(), "user.email", "agent@warp.dev");

    let baseline = Some(("Warp".to_string(), "agent@warp.dev".to_string()));
    assert!(!repository_identity_changed_since(
        temp_dir.path(),
        baseline
    ));
    Ok(())
}

#[test]
fn repository_identity_is_changed_when_a_setup_command_overrides_it() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_repo(temp_dir.path());
    run_repository_git_config(temp_dir.path(), "user.name", "Vercel Bot");
    run_repository_git_config(temp_dir.path(), "user.email", "vercel-bot@example.com");

    let baseline = Some(("Warp".to_string(), "agent@warp.dev".to_string()));
    assert!(repository_identity_changed_since(temp_dir.path(), baseline));
    Ok(())
}

#[test]
fn repository_identity_is_changed_when_baseline_is_none_but_the_repo_has_one() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_repo(temp_dir.path());
    run_repository_git_config(temp_dir.path(), "user.name", "Vercel Bot");
    run_repository_git_config(temp_dir.path(), "user.email", "vercel-bot@example.com");

    assert!(repository_identity_changed_since(temp_dir.path(), None));
    Ok(())
}

#[test]
fn configure_repository_git_identity_if_unset_skips_a_repo_the_customer_already_configured()
-> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_repo(temp_dir.path());
    run_repository_git_config(temp_dir.path(), "user.name", "Vercel Bot");
    run_repository_git_config(temp_dir.path(), "user.email", "vercel-bot@example.com");

    // The repo's identity already differs from `baseline`, so this must return
    // before ever consulting HOST_IDENTITIES (via recorded_identity_for_host) —
    // exercised here with a host that has no recorded identity, to make sure a
    // changed repo is left alone rather than falling through to some default.
    configure_repository_git_identity_if_unset(
        temp_dir.path(),
        "github.com",
        Some(("Warp".to_string(), "agent@warp.dev".to_string())),
    );

    assert_eq!(
        repository_git_identity(temp_dir.path()),
        Some((
            "Vercel Bot".to_string(),
            "vercel-bot@example.com".to_string()
        ))
    );
    Ok(())
}

#[test]
fn unique_credentials_drop_identical_duplicate_hosts() {
    let unique = unique_credentials_by_host(&[github_credential(), github_credential()]).unwrap();

    assert_eq!(unique.len(), 1);
    assert_eq!(unique[0].host, "github.com");
    assert_eq!(unique[0].token, "github-token");
}

#[test]
fn unique_credentials_reject_conflicting_duplicate_hosts() {
    let mut conflicting = github_credential();
    conflicting.token = "other-github-token".to_string();

    let error = unique_credentials_by_host(&[github_credential(), conflicting]).unwrap_err();
    assert!(error.to_string().contains("github.com"));
}

#[test]
fn bootstrap_rejects_a_one_host_failure() {
    let error = credentials_for_bootstrap(TaskGitCredentialsResponse {
        credentials: vec![github_credential()],
        failed_hosts: vec!["gitlab.com".to_string()],
    })
    .unwrap_err();

    assert!(error.to_string().contains("gitlab.com"));
    assert!(error.to_string().contains("all-or-nothing"));
}

#[test]
fn bootstrap_accepts_complete_multi_host_credentials() {
    let credentials = credentials_for_bootstrap(TaskGitCredentialsResponse {
        credentials: vec![github_credential(), gitlab_credential()],
        failed_hosts: vec![],
    })
    .unwrap();

    assert_eq!(credentials.len(), 2);
}

#[test]
fn write_glab_config_uses_glab_cli_filename() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let glab_config_dir = temp_dir.path().join(".config").join("glab-cli");

    write_glab_config(
        &[GitCredential {
            token: "gitlab-token".to_string(),
            username: Some("oauth2".to_string()),
            email: Some("user@example.com".to_string()),
            host: "gitlab.com".to_string(),
        }],
        temp_dir.path(),
    )?;

    let config_path = glab_config_dir.join(GLAB_CONFIG_FILENAME);
    assert!(config_path.exists());
    assert!(
        !glab_config_dir
            .join(format!("{GLAB_CONFIG_FILENAME}.tmp"))
            .exists()
    );

    let config = std::fs::read_to_string(config_path)?;
    assert!(config.contains("hosts:"));
    assert!(config.contains("    gitlab.com:"));
    assert!(config.contains("        token: gitlab-token"));
    assert!(config.contains("        git_protocol: https"));
    assert!(config.contains("        api_protocol: https"));

    Ok(())
}

#[test]
fn write_glab_config_excludes_github_credentials() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let glab_config_dir = temp_dir.path().join(".config").join("glab-cli");

    write_glab_config(
        &[
            GitCredential {
                token: "github-token".to_string(),
                username: Some("octocat".to_string()),
                email: None,
                host: "github.com".to_string(),
            },
            GitCredential {
                token: "gitlab-token".to_string(),
                username: Some("oauth2".to_string()),
                email: None,
                host: "gitlab.com".to_string(),
            },
        ],
        temp_dir.path(),
    )?;

    let config = std::fs::read_to_string(glab_config_dir.join(GLAB_CONFIG_FILENAME))?;
    assert!(config.contains("gitlab.com:"));
    assert!(!config.contains("github.com:"));
    assert!(!config.contains("github-token"));

    Ok(())
}

#[test]
fn write_glab_config_skips_github_only_credentials() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;

    write_glab_config(
        &[GitCredential {
            token: "github-token".to_string(),
            username: Some("octocat".to_string()),
            email: None,
            host: "github.com".to_string(),
        }],
        temp_dir.path(),
    )?;

    assert!(!temp_dir.path().join(".config").join("glab-cli").exists());

    Ok(())
}

#[test]
fn refreshed_credentials_return_err_when_the_local_write_fails() {
    let mut conflicting = github_credential();
    conflicting.token = "other-github-token".to_string();

    let error = apply_refreshed_credentials(TaskGitCredentialsResponse {
        credentials: vec![github_credential(), conflicting],
        failed_hosts: vec![],
    })
    .unwrap_err();

    let message = format!("{error:#}");
    assert!(message.contains("Failed to write refreshed git credentials"));
    assert!(message.contains("github.com"));
}
