use command::Stdio;
use futures::executor::block_on;
use warp_cli::agent::Harness;
use warpui::App;

use super::tests::{response, validated_response, with_home};
use super::*;
use crate::ai::agent_sdk::AgentDriverRunner;
use crate::ai::agent_sdk::driver::{AgentDriverOptions, split_run_options};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AgentConfigSnapshot, MockAIClient};
use crate::workspaces::user_workspaces::HeadlessTeamScope;

#[test]
fn local_git_uses_run_store_and_preserves_developer_files() -> Result<()> {
    with_home(
        "local_tests::local_git_uses_run_store_and_preserves_developer_files",
        &[],
        |home| {
            let fixtures = [
                (
                    ".gitconfig",
                    "[user]\nname = developer\nemail = developer@example.com\n[credential]\nhelper = store\n",
                ),
                (
                    ".git-credentials",
                    "https://personal:personal-test-token@github.com\n",
                ),
                (
                    ".netrc",
                    "machine unrelated.example.com login personal password personal-test-token\n",
                ),
            ];
            for (name, content) in fixtures {
                fs::write(home.join(name), content)?;
            }
            let repository = home.join("repository");
            fs::create_dir(&repository)?;
            ensure!(
                Command::new("git")
                    .args(["init", "--quiet"])
                    .arg(&repository)
                    .status()?
                    .success(),
                "git init failed"
            );
            let repo_config = repository.join(".git/config");
            fs::write(
                &repo_config,
                "[core]\nrepositoryformatversion = 0\n[user]\nname = repository-user\nemail = repository@example.com\n[credential \"https://github.com\"]\nhelper = store\n",
            )?;
            let original_repo_config = fs::read(&repo_config)?;
            let owner = ManagedStoreOwner::from("local-task");
            let directory = prepare_local(&owner, "7".into(), &repository)?;
            let mut credentials = validated_response()?;
            credentials.credentials.push(GitCredential {
                host: "gitlab.com".into(),
                token: "gitlab-test-token".into(),
                username: Some("oauth2".into()),
                email: None,
            });
            install(&owner, &credentials, true)?;
            let environment = restricted_child_environment(&owner)?;
            let identity = Command::new("git")
                .args(["var", "GIT_AUTHOR_IDENT"])
                .current_dir(&repository)
                .envs(&environment)
                .output()?;
            ensure!(identity.status.success(), "identity lookup failed");
            assert!(
                String::from_utf8(identity.stdout)?
                    .starts_with("test-bot[bot] <test-bot@users.noreply.github.com>")
            );
            let fill = |host: &str| -> Result<_> {
                let mut child = Command::new("git")
                    .args(["credential", "fill"])
                    .current_dir(&repository)
                    .envs(&environment)
                    .env("GIT_ASKPASS", "")
                    .env("SSH_ASKPASS", "")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(format!("protocol=https\nhost={host}\n\n").as_bytes())?;
                Ok(child.wait_with_output()?)
            };
            let github = fill("github.com")?;
            assert!(github.status.success());
            assert!(String::from_utf8(github.stdout)?.contains("password=installation-test-token"));
            let gitlab = fill("gitlab.com")?;
            assert!(gitlab.status.success());
            assert!(String::from_utf8(gitlab.stdout)?.contains("password=gitlab-test-token"));
            let expiry = with_store(&owner, |store| Ok(store.expires_at.unwrap()))?;
            expire_credentials(&owner, expiry)?;
            let denied = fill("github.com")?;
            assert!(!denied.status.success());
            assert!(denied.stdout.is_empty());
            close_restricted_credentials("other-task")?;
            assert!(directory.exists());
            close_restricted_credentials(&owner)?;
            assert!(!directory.exists());
            for (name, content) in fixtures {
                assert_eq!(fs::read(home.join(name))?, content.as_bytes());
            }
            assert_eq!(fs::read(repo_config)?, original_repo_config);
            ensure!(
                Command::new("git")
                    .args([
                        "config",
                        "http.extraHeader",
                        "Authorization: unsafe-test-fixture"
                    ])
                    .current_dir(&repository)
                    .status()?
                    .success(),
                "git config failed"
            );
            assert!(prepare_local(&owner, "7".into(), &repository).is_err());
            Ok(())
        },
    )
}

#[test]
fn local_git_rejects_effective_netrc_authentication_without_modifying_it() -> Result<()> {
    with_home(
        "local_tests::local_git_rejects_effective_netrc_authentication_without_modifying_it",
        &[],
        |home| {
            let content = "machine github.com login personal password personal-test-token\n";
            let path = home.join(".netrc");
            fs::write(&path, content)?;
            assert!(
                prepare_local(&ManagedStoreOwner::from("local-task"), "7".into(), home).is_err()
            );
            assert_eq!(fs::read_to_string(path)?, content);
            Ok(())
        },
    )
}

#[test]
fn concurrent_local_processes_keep_stores_independent() -> Result<()> {
    if let Ok(index) = std::env::var("WARP_TEST_LOCAL_STORE_PROCESS") {
        let home = home_dir()?;
        let owner = ManagedStoreOwner::from(format!("task-{index}").as_str());
        let directory = prepare_local(&owner, "7".into(), &home)?;
        let mut credentials = validated_response()?;
        credentials.credentials[0].username = Some(format!("bot-{index}"));
        install(&owner, &credentials, true)?;
        let environment = restricted_child_environment(&owner)?;
        assert_eq!(
            environment[OsStr::new("GIT_AUTHOR_NAME")],
            format!("bot-{index}").as_str()
        );
        let config = directory.join("gitconfig");
        fs::write(
            home.join(format!("ready-{index}")),
            config.to_string_lossy().as_bytes(),
        )?;
        let wait_for = |path: &Path| -> Result<()> {
            for _ in 0..500 {
                if path.exists() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            bail!("Local process barrier timed out")
        };
        wait_for(&home.join(if index == "0" { "ready-1" } else { "ready-0" }))?;
        let other = PathBuf::from(fs::read_to_string(home.join(if index == "0" {
            "ready-1"
        } else {
            "ready-0"
        }))?);
        assert_ne!(config, other);
        if index == "0" {
            let expiry = with_store(&owner, |store| Ok(store.expires_at.unwrap()))?;
            expire_credentials(&owner, expiry)?;
            close_restricted_credentials(&owner)?;
            fs::write(home.join("closed-0"), "")?;
        } else {
            wait_for(&home.join("closed-0"))?;
            assert!(!other.exists());
            assert!(config.exists());
            assert!(!fs::read_to_string(directory.join("credentials"))?.is_empty());
            apply_refresh(&owner, Ok(validated_response()?))?;
            assert_eq!(
                restricted_child_environment(&owner)?[OsStr::new("GIT_AUTHOR_NAME")],
                "bot-1"
            );
            close_restricted_credentials(&owner)?;
        }
        return Ok(());
    }
    let home = tempfile::tempdir()?;
    let mut children = Vec::new();
    for index in ["0", "1"] {
        children.push(Command::new(std::env::current_exe()?)
            .args(["--exact", "ai::agent_sdk::driver::git_credentials::restricted::local_tests::concurrent_local_processes_keep_stores_independent", "--nocapture"])
            .env("WARP_TEST_LOCAL_STORE_PROCESS", index)
            .env("HOME", home.path())
            .env("GIT_CONFIG_GLOBAL", home.path().join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?);
    }
    for child in children {
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[test]
fn negotiated_local_policy_reaches_native_and_third_party_launch_options() -> Result<()> {
    with_home(
        "local_tests::negotiated_local_policy_reaches_native_and_third_party_launch_options",
        &[],
        |home| {
            let home = home.to_owned();
            App::test((), |mut app| async move {
                let _well_known =
                    warp_core::features::FeatureFlag::WellKnownMcpIds.override_enabled(true);
                app.add_singleton_model(|_| ServerApiProvider::new_for_test());
                let runner = app.add_singleton_model(|_| AgentDriverRunner);
                let foreground = runner.update(&mut app, |_, ctx| ctx.spawner());
                for harness in [
                    Harness::Oz,
                    Harness::Claude,
                    Harness::Codex,
                    Harness::Gemini,
                    Harness::Pi,
                    Harness::OpenCode,
                ] {
                    let mut client = MockAIClient::new();
                    client
                        .expect_create_local_agent_task()
                        .times(1)
                        .returning(|_, _, _| {
                            Ok(
                                warp_graphql::mutations::create_agent_task::LocalAgentTaskOutput {
                                    task_id: cynic::Id::new("00000000-0000-0000-0000-000000000001"),
                                    github_credential_mode: Some(
                                        GitHubCredentialMode::RestrictedInstallation,
                                    ),
                                    execution_id: Some(cynic::Id::new("7")),
                                    mcp_servers: Some(r#"{"github":{"warp_id":"github"}}"#.into()),
                                },
                            )
                        });
                    client
                        .expect_get_restricted_task_git_credentials()
                        .times(1)
                        .returning(|_, token, _, _, _, execution| {
                            assert!(token.is_empty());
                            assert_eq!(execution.as_deref(), Some("7"));
                            let mut result = response();
                            result.credentials[0].expires_at =
                                Some((Utc::now() + TimeDelta::hours(1)).into());
                            Ok(result)
                        });
                    let client: Arc<dyn AIClient> = Arc::new(client);
                    let mut options = AgentDriverOptions {
                        selected_harness: harness,
                        working_dir: home.to_owned(),
                        ..Default::default()
                    };
                    let specs = AgentDriverRunner::initialize_new_task(
                        &foreground,
                        &client,
                        "prompt".into(),
                        AgentConfigSnapshot::default(),
                        HeadlessTeamScope::Personal,
                        &mut options,
                    )
                    .await
                    .unwrap();
                    assert_eq!(
                        options.github_credential_mode,
                        GitHubCredentialMode::RestrictedInstallation
                    );
                    assert_eq!(options.execution_id.as_deref(), Some("7"));
                    assert!(options.strict_mcp_startup);
                    assert!(
                        matches!(&specs[..], [warp_cli::mcp::MCPSpec::WellKnown(id)] if id == "github")
                    );
                    let (_, launch) = split_run_options(options).unwrap();
                    assert_eq!(
                        launch.resolved_env_vars[OsStr::new("HOME")],
                        home.as_os_str()
                    );
                    let config =
                        PathBuf::from(&launch.resolved_env_vars[OsStr::new("GIT_CONFIG_GLOBAL")]);
                    assert_ne!(config, home.join(".gitconfig"));
                    assert!(config.exists());
                    close_restricted_credentials("00000000-0000-0000-0000-000000000001").unwrap();
                }
            });
            Ok(())
        },
    )
}

#[test]
fn local_bootstrap_and_refresh_use_the_bound_execution_without_workload_token() -> Result<()> {
    with_home(
        "local_tests::local_bootstrap_and_refresh_use_the_bound_execution_without_workload_token",
        &[],
        |home| {
            let mut client = MockAIClient::new();
            client
                .expect_get_restricted_task_git_credentials()
                .times(2)
                .returning(|task, token, _, _, _, execution| {
                    assert_eq!(task, "local-task");
                    assert!(token.is_empty());
                    assert_eq!(execution.as_deref(), Some("7"));
                    let mut result = response();
                    result.credentials[0].expires_at =
                        Some((Utc::now() + TimeDelta::hours(1)).into());
                    Ok(result)
                });
            let client: Arc<dyn AIClient> = Arc::new(client);
            block_on(bootstrap_local_credentials(
                "local-task",
                "7".into(),
                home,
                &client,
                false,
            ))?;
            with_store("local-task", |store| {
                store.provider_refresh_at.clear();
                Ok(())
            })?;
            block_on(refresh_restricted_credentials("local-task", &client, false))?;
            close_restricted_credentials("local-task")
        },
    )
}
