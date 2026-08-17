use super::*;
use futures::executor::block_on;
use warp_graphql::ai::PlatformErrorCode;

fn dependency_error(retryable: bool) -> TaskGitCredentialsError {
    TaskGitCredentialsError::Platform {
        message: "External dependency is unavailable.".to_string(),
        code: PlatformErrorCode::ResourceUnavailable,
        retryable,
        detail: Some("GitHub is temporarily unavailable.".to_string()),
        provider: Some("github".to_string()),
        dependency: Some("github_api".to_string()),
    }
}

#[test]
fn fetch_with_retry_retries_request_layer_failures() {
    let mut attempts = 0usize;
    let credentials = block_on(fetch_with_retry(
        "test bootstrap",
        &GIT_CREDENTIALS_BOOTSTRAP_BACKOFF,
        || {
            attempts += 1;
            std::future::ready(if attempts == 1 {
                Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
                    "transient request failure"
                )))
            } else {
                Ok(Vec::new())
            })
        },
        |_| std::future::ready(()),
    ))
    .expect("request-layer failure should be retried");

    assert_eq!(attempts, 2);
    assert!(credentials.is_empty());
}

#[test]
fn fetch_with_retry_succeeds_after_retryable_dependency_failures() {
    let mut attempts = 0usize;
    let mut delays = Vec::new();
    let credentials = block_on(fetch_with_retry(
        "test bootstrap",
        &GIT_CREDENTIALS_BOOTSTRAP_BACKOFF,
        || {
            attempts += 1;
            std::future::ready(if attempts < 3 {
                Err(dependency_error(true))
            } else {
                Ok(vec![GitCredential {
                    token: "secret-token".to_string(),
                    username: None,
                    email: None,
                    host: "github.com".to_string(),
                }])
            })
        },
        |delay| {
            delays.push(delay);
            std::future::ready(())
        },
    ))
    .expect("retryable dependency should eventually succeed");

    assert_eq!(attempts, 3);
    assert_eq!(delays, GIT_CREDENTIALS_BOOTSTRAP_BACKOFF[..2]);
    assert_eq!(credentials.len(), 1);
}

#[test]
fn fetch_with_retry_exhausts_bounded_dependency_attempts() {
    let mut attempts = 0usize;
    let result = block_on(fetch_with_retry(
        "test refresh",
        &GIT_CREDENTIALS_BOOTSTRAP_BACKOFF,
        || {
            attempts += 1;
            std::future::ready(Err(dependency_error(true)))
        },
        |_| std::future::ready(()),
    ));
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("retryable dependency should stop after the bounded attempts"),
    };

    assert!(matches!(
        error,
        TaskGitCredentialsError::Platform {
            retryable: true,
            ..
        }
    ));
    assert_eq!(attempts, GIT_CREDENTIALS_BOOTSTRAP_BACKOFF.len() + 1);
}

#[test]
fn fetch_with_retry_fails_fast_for_non_retryable_user_error() {
    let mut attempts = 0usize;
    let result = block_on(fetch_with_retry(
        "test bootstrap",
        &GIT_CREDENTIALS_BOOTSTRAP_BACKOFF,
        || {
            attempts += 1;
            std::future::ready(Err(TaskGitCredentialsError::Unstructured {
                message: "Unable to access task git credentials".to_string(),
            }))
        },
        |_| std::future::ready(()),
    ));
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("user error should fail fast"),
    };

    assert!(matches!(
        error,
        TaskGitCredentialsError::Unstructured { .. }
    ));
    assert_eq!(attempts, 1);
}

#[test]
fn fetch_with_retry_fails_fast_without_isolation_platform() {
    let mut attempts = 0usize;
    let result = block_on(fetch_with_retry(
        "test bootstrap",
        &GIT_CREDENTIALS_BOOTSTRAP_BACKOFF,
        || {
            attempts += 1;
            std::future::ready(Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
                warp_isolation_platform::IsolationPlatformError::NoIsolationPlatformDetected
            ))))
        },
        |_| std::future::ready(()),
    ));
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("missing isolation platform should fail fast"),
    };

    assert!(matches!(error, TaskGitCredentialsError::Request(_)));
    assert_eq!(attempts, 1);
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

#[test]
fn git_credentials_file_content_includes_each_provider_host() {
    let content = git_credentials_file_content(&[
        GitCredential {
            token: "github-token".to_string(),
            username: None,
            email: None,
            host: "github.com".to_string(),
        },
        GitCredential {
            token: "gitlab-token".to_string(),
            username: Some("oauth2".to_string()),
            email: None,
            host: "gitlab.com".to_string(),
        },
    ]);

    assert_eq!(
        content,
        "https://x-access-token:github-token@github.com\n\
         https://oauth2:gitlab-token@gitlab.com\n"
    );
}

#[test]
fn credential_diagnostics_reports_presence_without_values() {
    let diagnostics = credential_diagnostics(&[GitCredential {
        token: "secret-token".to_string(),
        username: Some("oauth2".to_string()),
        email: Some("user@example.com".to_string()),
        host: "gitlab.com".to_string(),
    }]);

    assert_eq!(
        diagnostics,
        "gitlab.com(token_present=true, username_present=true)"
    );
    assert!(!diagnostics.contains("secret-token"));
    assert!(!diagnostics.contains("oauth2"));
    assert!(!diagnostics.contains("user@example.com"));
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
