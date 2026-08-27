use super::*;

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
    let content =
        merge_git_credentials_file_content("", &[github_credential(), gitlab_credential()]);

    assert_eq!(
        content,
        "https://x-access-token:github-token@github.com\n\
         https://oauth2:gitlab-token@gitlab.com\n"
    );
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
