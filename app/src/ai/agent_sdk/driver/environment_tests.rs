use std::fs;
use std::path::Path;

use cloud_object_models::CodeForge;
use command::blocking::Command;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::future::{pending, ready};
use futures::poll;
use warp_cli::agent::{
    RepositoryForge, RepositoryHeadRef, RepositoryIdentity, RepositoryPreparationOverride,
};
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_core::command::ExitCode;

use super::{
    CloneFailureCredentialIdentity, CloneFailureIdentityDiagnostics, PrepareEnvironmentError,
    RepositoryCloneRequest, SETUP_COMMAND_OUTPUT_TRUNCATION_MARKER, SetupCommandPhase,
    await_setup_phase, build_checkout_helper_command, build_git_credential_query_command,
    build_remove_repository_origins_command, build_resolved_head_command,
    clone_failure_identity_diagnostics, environment_snapshot, is_valid_git_object_id,
    merge_repos_deduped, parse_resolved_head_sha, parse_resolved_head_shas, read_checkout_failures,
    repository_clone_requests, setup_command_failure, single_repo_name, unique_clone_hosts,
    validate_repository_preparation_overrides,
};
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::cloud_environments::{AmbientAgentEnvironment, SourceRepo};
use crate::terminal::shell::ShellType;

fn command_output(stdout: &str, stderr: &str, status: CommandExitStatus) -> CommandOutput {
    CommandOutput {
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
        status,
        exit_code: None,
    }
}

#[test]
fn setup_timeout_covers_command_start_and_exit_without_resetting_deadline() {
    for command_started in [false, true] {
        block_on(async {
            let (start_tx, start_rx) = oneshot::channel::<()>();
            let (exit_tx, exit_rx) = oneshot::channel::<ExitCode>();
            let (deadline_tx, deadline_rx) = oneshot::channel::<()>();
            let operation = async {
                start_rx.await.unwrap();
                Ok(exit_rx.await.unwrap())
            };
            let mut wait = Box::pin(await_setup_phase(
                2,
                "./setup.sh",
                SetupCommandPhase::Execute,
                operation,
                deadline_rx,
            ));
            assert!(poll!(wait.as_mut()).is_pending());
            let start_tx = if command_started {
                start_tx.send(()).unwrap();
                assert!(poll!(wait.as_mut()).is_pending());
                None
            } else {
                Some(start_tx)
            };
            deadline_tx.send(()).unwrap();
            let error = wait.await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "Setup command #2 timed out after 1800s while waiting for command to complete: `./setup.sh`"
            );
            if let Some(start_tx) = start_tx {
                assert!(start_tx.is_canceled());
            }
            assert!(exit_tx.is_canceled());
        });
    }
}

#[test]
fn setup_deadline_preserves_success_and_nonzero_exit_codes() {
    for code in [0, 7] {
        let result = block_on(await_setup_phase(
            1,
            "./setup.sh",
            SetupCommandPhase::Execute,
            ready(Ok(ExitCode::from(code))),
            pending::<()>(),
        ))
        .unwrap();
        assert_eq!(result, ExitCode::from(code));
    }
}

#[test]
fn setup_deadline_preserves_shell_exit_error() {
    let result = block_on(await_setup_phase::<()>(
        1,
        "./setup.sh",
        SetupCommandPhase::Execute,
        ready(Err(PrepareEnvironmentError::TerminalDriver {
            source: AgentDriverError::SetupCommandExitedShell {
                command: "./setup.sh".to_string(),
            },
        })),
        pending::<()>(),
    ));
    assert!(matches!(
        result,
        Err(PrepareEnvironmentError::TerminalDriver {
            source: AgentDriverError::SetupCommandExitedShell { .. }
        })
    ));
}

#[test]
fn setup_reset_timeout_is_distinct_and_names_the_configured_command() {
    let error = block_on(await_setup_phase::<ExitCode>(
        3,
        "./setup.sh",
        SetupCommandPhase::ResetWorkingDirectory,
        pending(),
        ready(()),
    ))
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Setup command #3 timed out after 30s while resetting the working directory after the command: `./setup.sh`"
    );
}

#[test]
fn setup_timeout_redacts_command_before_constructing_error() {
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let command = format!("./setup.sh {secret}");
    let error = block_on(await_setup_phase::<()>(
        1,
        &command,
        SetupCommandPhase::Execute,
        pending(),
        ready(()),
    ))
    .unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
    assert!(error.to_string().contains(&"*".repeat(secret.len())));
}

#[test]
fn setup_timeout_stops_following_work_and_drops_the_pending_command() {
    block_on(async {
        let (command_tx, command_rx) = oneshot::channel::<()>();
        let (deadline_tx, deadline_rx) = oneshot::channel::<()>();
        let mut reached_next_command = false;
        let mut setup = Box::pin(async {
            await_setup_phase(
                1,
                "./setup.sh",
                SetupCommandPhase::Execute,
                async {
                    command_rx.await.unwrap();
                    Ok(())
                },
                deadline_rx,
            )
            .await?;
            reached_next_command = true;
            Ok::<(), PrepareEnvironmentError>(())
        });
        assert!(poll!(setup.as_mut()).is_pending());
        deadline_tx.send(()).unwrap();
        assert!(setup.await.is_err());
        assert!(!reached_next_command);
        assert!(command_tx.is_canceled());
    });
}

#[test]
fn clone_error_includes_short_output() {
    let error = PrepareEnvironmentError::CloneRepo {
        repo_name: "warpdotdev/warp".to_string(),
        output: Some("fatal: repository not found".to_string()),
        identity_diagnostics: CloneFailureIdentityDiagnostics {
            author: None,
            credentials: Vec::new(),
        },
    };

    assert_eq!(
        error.to_string(),
        "Failed to clone warpdotdev/warp: fatal: repository not found\nGit identity diagnostics:\n  Author: unset"
    );
}

#[test]
fn setup_command_error_includes_short_output() {
    let error = setup_command_failure(
        "./setup.sh".to_string(),
        Some(" \n permission denied \n ".to_string()),
    );
    assert_eq!(
        error.to_string(),
        "Failed to run setup command: ./setup.sh\nCommand output:\npermission denied"
    );
}

#[test]
fn setup_command_error_includes_redacted_truncated_output() {
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let output = format!("START {secret}\n{} END", "x".repeat(5_000));
    let error = setup_command_failure("./setup.sh".to_string(), Some(output));
    let prefix = format!("START {}\n", "*".repeat(secret.len()));
    let marker = SETUP_COMMAND_OUTPUT_TRUNCATION_MARKER;
    let retained = 4_096 - marker.len();
    let expected_output = format!(
        "{}{}{}{} END",
        prefix,
        "x".repeat(retained / 2 - prefix.len()),
        marker,
        "x".repeat(retained - retained / 2 - " END".len()),
    );
    assert_eq!(
        error.to_string(),
        format!("Failed to run setup command: ./setup.sh\nCommand output:\n{expected_output}")
    );
}

#[test]
fn setup_command_error_without_readable_output_keeps_original_message() {
    for output in [None, Some("  \n  ".to_string())] {
        let error = setup_command_failure("./setup.sh".to_string(), output);
        assert_eq!(error.to_string(), "Failed to run setup command: ./setup.sh");
    }
}

fn commit_head_override(
    code_forge: RepositoryForge,
    owner: &str,
    repo: &str,
    sha: &str,
) -> RepositoryPreparationOverride {
    RepositoryPreparationOverride {
        code_forge,
        repo_owner: owner.to_string(),
        repo_name: repo.to_string(),
        head: RepositoryHeadRef::CommitSha(sha.to_string()),
        clone_from: None,
        preserve_origin: false,
    }
}

#[test]
fn substituted_branch_request_preserves_origin_without_affecting_other_repos() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let mut override_for_warp =
        substitution_override("source", "warp", sha, "copy", "warp-for-benchmarks");
    override_for_warp.head = RepositoryHeadRef::Branch(format!("benchmark-base/{sha}"));
    let requests = repository_clone_requests(
        &[
            repo(CodeForge::GitHub, "source", "warp"),
            repo(CodeForge::GitHub, "source", "other"),
        ],
        &[override_for_warp],
        true,
    )
    .unwrap();
    assert_eq!(requests[0].remote.repo, "warp-for-benchmarks");
    assert_eq!(requests[0].checkout_name, "warp");
    assert!(requests[0].fetch_branch_only);
    assert!(!requests[0].remove_origin);
    assert!(!requests[1].fetch_branch_only);
    assert!(requests[1].remove_origin);
}

#[test]
fn git_object_id_validation_accepts_lowercase_sha1_and_sha256() {
    assert!(is_valid_git_object_id(
        "0123456789abcdef0123456789abcdef01234567"
    ));
    assert!(is_valid_git_object_id(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    ));
    assert!(!is_valid_git_object_id(
        "0123456789ABCDEF0123456789ABCDEF01234567"
    ));
    assert!(!is_valid_git_object_id("0123456789abcdef"));
    assert!(!is_valid_git_object_id(
        "g123456789abcdef0123456789abcdef01234567"
    ));
}

#[test]
fn parse_resolved_head_sha_accepts_trimmed_valid_object_ids() {
    assert_eq!(
        parse_resolved_head_sha("0123456789abcdef0123456789abcdef01234567").as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(
        parse_resolved_head_sha(
            "  0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  "
        )
        .as_deref(),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        parse_resolved_head_sha("0123456789ABCDEF0123456789ABCDEF01234567"),
        None
    );
    assert_eq!(parse_resolved_head_sha("not-a-sha"), None);
    assert_eq!(parse_resolved_head_sha(""), None);
}

#[test]
fn parse_resolved_head_shas_keeps_one_line_per_repo_including_failures() {
    let first = "0123456789abcdef0123456789abcdef01234567";
    let second = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let stdout = format!("{first}\n\n{second}\n");

    assert_eq!(
        parse_resolved_head_shas(stdout.as_bytes(), 3),
        vec![Some(first.to_string()), None, Some(second.to_string())]
    );
    assert_eq!(
        parse_resolved_head_shas(first.as_bytes(), 2),
        vec![Some(first.to_string()), None]
    );
    assert_eq!(parse_resolved_head_shas(b"\xff", 1), vec![None]);
}

#[test]
fn environment_snapshot_keeps_resolved_heads_in_request_order() {
    let requests = vec![
        clone_request(
            repo(CodeForge::GitHub, "warpdotdev", "warp"),
            Some(RepositoryHeadRef::Branch("main".to_string())),
        ),
        clone_request(
            repo(CodeForge::GitLab, "platform/backend", "api"),
            Some(RepositoryHeadRef::CommitSha(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            )),
        ),
    ];
    let first_sha = "0123456789abcdef0123456789abcdef01234567";
    let second_sha = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    let snapshot = environment_snapshot(
        &requests,
        Path::new("/workspace"),
        &[Some(first_sha.to_string()), Some(second_sha.to_string())],
    );

    assert_eq!(snapshot.repositories.len(), 2);
    assert_eq!(snapshot.repositories[0].repo_owner, "warpdotdev");
    assert_eq!(snapshot.repositories[0].checkout_path, "warp");
    assert_eq!(
        snapshot.repositories[0].requested_checkout_ref.as_deref(),
        Some("main")
    );
    assert_eq!(snapshot.repositories[0].resolved_head_sha, first_sha);
    assert_eq!(snapshot.repositories[1].code_forge, CodeForge::GitLab);
    assert_eq!(snapshot.repositories[1].resolved_head_sha, second_sha);
}

#[test]
fn environment_snapshot_omits_unresolved_heads() {
    let requests = vec![
        clone_request(repo(CodeForge::GitHub, "one", "first"), None),
        clone_request(repo(CodeForge::GitHub, "two", "second"), None),
    ];

    let snapshot = environment_snapshot(
        &requests,
        Path::new("/workspace"),
        &[
            None,
            Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        ],
    );

    assert_eq!(snapshot.repositories.len(), 1);
    assert_eq!(snapshot.repositories[0].repo_name, "second");
}

fn substitution_override(
    owner: &str,
    repo: &str,
    sha: &str,
    target_owner: &str,
    target_repo: &str,
) -> RepositoryPreparationOverride {
    RepositoryPreparationOverride {
        code_forge: RepositoryForge::GitHub,
        repo_owner: owner.to_string(),
        repo_name: repo.to_string(),
        head: RepositoryHeadRef::CommitSha(sha.to_string()),
        clone_from: Some(RepositoryIdentity {
            code_forge: RepositoryForge::GitHub,
            repo_owner: target_owner.to_string(),
            repo_name: target_repo.to_string(),
        }),
        preserve_origin: true,
    }
}

fn clone_request(repo: SourceRepo, checkout: Option<RepositoryHeadRef>) -> RepositoryCloneRequest {
    let checkout_name = repo.repo.clone();
    RepositoryCloneRequest {
        remote: repo,
        checkout_name,
        checkout,
        remove_origin: false,
        fetch_branch_only: false,
    }
}

fn environment_with_repos(repos: Vec<SourceRepo>) -> AmbientAgentEnvironment {
    let mut environment =
        AmbientAgentEnvironment::new(String::new(), None, vec![], String::new(), vec![]);
    environment.source_repos = Some(repos);
    environment
}

#[test]
fn single_repo_name_returns_repo_when_exactly_one_repo() {
    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp-internal".to_string(),
    )];
    let selected_repo = single_repo_name(&repos);
    assert_eq!(selected_repo, Some("warp-internal".to_string()));
}

fn repo(forge: CodeForge, owner: &str, name: &str) -> SourceRepo {
    SourceRepo::new(forge, owner.to_string(), name.to_string())
}

#[test]
fn merge_repos_dedupes_case_insensitively_and_preserves_environment_order() {
    let environment = vec![repo(CodeForge::GitHub, "WarpDotDev", "Warp")];
    let additional = vec![
        repo(CodeForge::GitHub, "warpdotdev", "warp"),
        repo(CodeForge::GitHub, "warpdotdev", "warp-server"),
    ];

    assert_eq!(
        merge_repos_deduped(environment, additional).unwrap(),
        vec![
            repo(CodeForge::GitHub, "WarpDotDev", "Warp"),
            repo(CodeForge::GitHub, "warpdotdev", "warp-server"),
        ]
    );
}

#[test]
fn merge_repos_keeps_distinct_repositories() {
    let merged = merge_repos_deduped(
        vec![repo(CodeForge::GitHub, "a", "widget")],
        vec![
            repo(CodeForge::GitHub, "b", "widget-api"),
            repo(CodeForge::GitLab, "a", "widget-web"),
        ],
    )
    .unwrap();

    assert_eq!(merged.len(), 3);
}
#[test]
fn merge_repos_rejects_clone_directory_collisions() {
    let error = merge_repos_deduped(
        vec![repo(CodeForge::GitHub, "a", "widget")],
        vec![repo(CodeForge::GitLab, "b", "widget")],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        PrepareEnvironmentError::CloneDirectoryCollision {
            repo_name,
            first_owner,
            second_owner,
        } if repo_name == "widget" && first_owner == "a" && second_owner == "b"
    ));
}

#[test]
fn merge_repos_supports_additional_only_and_empty_inputs() {
    let additional = vec![repo(CodeForge::GitHub, "warpdotdev", "warp")];
    assert_eq!(
        merge_repos_deduped(Vec::new(), additional.clone()).unwrap(),
        additional
    );
    assert!(
        merge_repos_deduped(Vec::new(), Vec::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn single_repo_name_returns_none_for_zero_or_many_repos() {
    let no_repos = Vec::<SourceRepo>::new();
    assert_eq!(single_repo_name(&no_repos), None);

    let two_repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-internal".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        ),
    ];
    assert_eq!(single_repo_name(&two_repos), None);
}

#[test]
fn sparse_substitution_uses_target_remote_and_preserves_source_checkout_name() {
    let source_sha = "0123456789abcdef0123456789abcdef01234567";
    let repos = vec![
        repo(CodeForge::GitHub, "WarpDotDev", "Warp")
            .with_checkout_ref(Some("source-default".to_string())),
        repo(CodeForge::GitHub, "warpdotdev", "common-skills")
            .with_checkout_ref(Some("main".to_string())),
    ];
    let overrides = vec![substitution_override(
        "warpdotdev",
        "warp",
        source_sha,
        "warpdotdev",
        "warp-for-benchmarks",
    )];

    let prepared = repository_clone_requests(&repos, &overrides, true).unwrap();

    assert_eq!(prepared[0].remote.owner, "warpdotdev");
    assert_eq!(prepared[0].remote.repo, "warp-for-benchmarks");
    assert_eq!(prepared[0].checkout_name, "Warp");
    assert_eq!(
        prepared[0].checkout,
        Some(RepositoryHeadRef::CommitSha(source_sha.to_string()))
    );
    assert!(!prepared[0].remove_origin);
    assert_eq!(prepared[1].remote.repo, "common-skills");
    assert_eq!(prepared[1].checkout_name, "common-skills");
    assert_eq!(
        prepared[1].checkout,
        Some(RepositoryHeadRef::Branch("main".to_string()))
    );
    assert!(prepared[1].remove_origin);
}

#[test]
fn substituted_request_uses_target_identity_and_source_checkout_path() {
    let source_sha = "0123456789abcdef0123456789abcdef01234567";
    let repos = vec![repo(CodeForge::GitHub, "warpdotdev", "warp")];
    let overrides = vec![substitution_override(
        "warpdotdev",
        "warp",
        source_sha,
        "warpdotdev",
        "warp-for-benchmarks",
    )];
    let requests = repository_clone_requests(&repos, &overrides, true).unwrap();

    assert_eq!(requests[0].checkout_name, "warp");
    let snapshot = environment_snapshot(
        &requests,
        Path::new("/workspace"),
        &[Some(source_sha.to_string())],
    );
    assert_eq!(snapshot.repositories[0].repo_name, "warp-for-benchmarks");
    assert_eq!(snapshot.repositories[0].checkout_path, "warp");
}

#[test]
fn sparse_substitution_origin_removal_excludes_preserved_target() {
    let repos = vec![
        repo(CodeForge::GitHub, "warpdotdev", "warp"),
        repo(CodeForge::GitHub, "warpdotdev", "common-skills"),
    ];
    let overrides = vec![substitution_override(
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
        "warpdotdev",
        "warp-for-benchmarks",
    )];
    let requests = repository_clone_requests(&repos, &overrides, true).unwrap();
    let workspace = Path::new("/workspace");
    let command = build_remove_repository_origins_command(&requests, workspace, ShellType::Bash);
    let preserved_target = workspace.join("warp").to_string_lossy().into_owned();
    let removed_source = workspace
        .join("common-skills")
        .to_string_lossy()
        .into_owned();
    assert!(!command.contains(&preserved_target));
    assert!(command.contains(&removed_source));
}

#[test]
fn preparation_overrides_replace_checkout_ref_only_for_matching_repos() {
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        )
        .with_checkout_ref(Some("abc123".to_string())),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        )
        .with_checkout_ref(Some("old-pin".to_string())),
    ];
    let overrides = vec![commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    let prepared = repository_clone_requests(&repos, &overrides, false).unwrap();

    assert_eq!(
        prepared[0].checkout,
        Some(RepositoryHeadRef::CommitSha(
            "0123456789abcdef0123456789abcdef01234567".to_string(),
        ))
    );
    assert_eq!(
        prepared[1].checkout,
        Some(RepositoryHeadRef::Branch("old-pin".to_string()))
    );
}

#[test]
fn clone_requests_use_each_repository_host() {
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitLab,
            "platform/backend".to_string(),
            "api".to_string(),
        ),
    ];

    let prepared = repository_clone_requests(&repos, &[], false).unwrap();

    assert_eq!(
        prepared[0].remote.https_clone_url(),
        "https://github.com/warpdotdev/warp.git"
    );
    assert_eq!(
        prepared[1].remote.https_clone_url(),
        "https://gitlab.com/platform/backend/api.git"
    );
}

#[test]
fn clone_requests_accept_azure_devops_repositories() {
    let repos = vec![SourceRepo::new(
        CodeForge::AzureDevOps,
        "warpdotdev/test-project".to_string(),
        "test-project".to_string(),
    )];

    let prepared = repository_clone_requests(&repos, &[], false).unwrap();

    assert_eq!(
        prepared[0].remote.https_clone_url(),
        "https://dev.azure.com/warpdotdev/test-project/_git/test-project"
    );
}

#[test]
fn clone_requests_reject_a_mixed_repository_missing_forge() {
    let repos = AmbientAgentEnvironment {
        name: "mixed-omitted".into(),
        description: None,
        code_forge: Some(CodeForge::GitHub),
        code_forges: Some(vec![CodeForge::GitHub, CodeForge::GitLab]),
        github_repos: vec![],
        source_repos: Some(vec![
            SourceRepo::new(
                CodeForge::GitHub,
                "warpdotdev".to_string(),
                "warp".to_string(),
            ),
            SourceRepo {
                code_forge: None,
                owner: "platform/backend".to_string(),
                repo: "api".to_string(),
                checkout_ref: None,
            },
        ]),
        base_image: None,
        setup_commands: vec![],
        providers: Default::default(),
        secrets: None,
        default_runner_uid: None,
    }
    .effective_repos();

    let error = repository_clone_requests(&repos, &[], false).unwrap_err();

    assert!(matches!(
        error,
        PrepareEnvironmentError::UnsupportedRepositoryForge { repo_name }
            if repo_name == "platform/backend/api"
    ));
}

#[test]
fn clone_requests_reject_an_omitted_forge_when_github_is_paired_with_an_unknown_forge() {
    let repos = AmbientAgentEnvironment {
        name: "github-plus-future".into(),
        description: None,
        code_forge: Some(CodeForge::GitHub),
        code_forges: Some(vec![CodeForge::GitHub, CodeForge::Unknown]),
        github_repos: vec![],
        source_repos: Some(vec![
            SourceRepo::new(
                CodeForge::GitHub,
                "warpdotdev".to_string(),
                "warp".to_string(),
            ),
            SourceRepo {
                code_forge: None,
                owner: "acme".to_string(),
                repo: "widgets".to_string(),
                checkout_ref: None,
            },
        ]),
        base_image: None,
        setup_commands: vec![],
        providers: Default::default(),
        secrets: None,
        default_runner_uid: None,
    }
    .effective_repos();

    assert_eq!(repos[1].code_forge, None);
    let error = repository_clone_requests(&repos, &[], false).unwrap_err();

    assert!(matches!(
        error,
        PrepareEnvironmentError::UnsupportedRepositoryForge { repo_name }
            if repo_name == "acme/widgets"
    ));
}

#[test]
fn clone_requests_reject_a_repository_with_an_unrecognized_forge() {
    // An environment forge value newer than this client build (see
    // CodeForge::Unknown) can still be assigned to a real repository by a
    // newer server. Building clone requests for it must fail clearly rather
    // than panic or silently attempt a clone with no host.
    let repos = vec![SourceRepo::new(
        CodeForge::Unknown,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];

    let error = repository_clone_requests(&repos, &[], false).unwrap_err();

    assert!(matches!(
        error,
        PrepareEnvironmentError::UnsupportedRepositoryForge { repo_name }
            if repo_name == "warpdotdev/warp"
    ));
}

#[test]
fn clone_requests_reject_an_unrecognized_forge_repository_even_with_unrelated_overrides() {
    // A head override targeting a different, supported-forge repository must
    // not mask the unsupported repository elsewhere in the same environment:
    // every repository is checked, not just the ones an override names.
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        ),
        SourceRepo::new(
            CodeForge::Unknown,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        ),
    ];
    let overrides = vec![commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    let error = repository_clone_requests(&repos, &overrides, false).unwrap_err();

    assert!(matches!(
        error,
        PrepareEnvironmentError::UnsupportedRepositoryForge { repo_name }
            if repo_name == "warpdotdev/warp-server"
    ));
}

#[test]
fn head_override_validation_treats_an_unrecognized_forge_repository_as_never_matching() {
    // No head override can target a repository whose forge this client can't
    // represent; validation must reject it as "not declared" (an override
    // that names a repository the environment doesn't have) rather than
    // panicking while checking whether it matches.
    let environment = environment_with_repos(vec![SourceRepo::new(
        CodeForge::Unknown,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )]);
    let override_for_it = commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );

    let error = validate_repository_preparation_overrides(
        &environment.effective_repos(),
        &[override_for_it],
    )
    .expect_err("an unrecognized-forge repository can never match an override");
    assert!(error.to_string().contains("support"));
}

#[test]
fn repository_head_override_validation_rejects_duplicates_and_mismatches() {
    let environment = environment_with_repos(vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )]);
    let github = commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );

    let duplicate_error = validate_repository_preparation_overrides(
        &environment.effective_repos(),
        &[github.clone(), github.clone()],
    )
    .expect_err("duplicate repository identity must fail");
    assert!(duplicate_error.to_string().contains("duplicate"));

    let forge_mismatch = commit_head_override(
        RepositoryForge::GitLab,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let mismatch_error = validate_repository_preparation_overrides(
        &environment.effective_repos(),
        &[forge_mismatch],
    )
    .expect_err("forge mismatch must fail");
    assert!(mismatch_error.to_string().contains("not declared"));
}

#[test]
fn repository_head_override_validation_accepts_partial_multi_repo_sets() {
    let environment = environment_with_repos(vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitLab,
            "platform/backend".to_string(),
            "api".to_string(),
        ),
    ]);
    let partial_overrides = vec![commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    validate_repository_preparation_overrides(&environment.effective_repos(), &partial_overrides)
        .expect("repositories without overrides should use their default branches");
}

#[test]
fn repository_origin_removal_targets_all_environment_repositories() {
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        ),
    ];

    let workspace = Path::new("/workspace");
    let requests = repository_clone_requests(&repos, &[], true).unwrap();
    let command = build_remove_repository_origins_command(&requests, workspace, ShellType::Bash);

    let warp_dir = workspace.join("warp").to_string_lossy().into_owned();
    let warp_server_dir = workspace.join("warp-server").to_string_lossy().into_owned();
    assert!(command.contains(&warp_dir));
    assert!(command.contains(&warp_server_dir));
    assert!(command.contains("remote get-url origin"));
    assert!(command.contains("config --remove-section remote.origin"));
}

#[test]
fn clone_failure_identity_hosts_are_deduplicated_in_request_order() {
    let requests = [
        clone_request(repo(CodeForge::GitHub, "warpdotdev", "warp"), None),
        clone_request(repo(CodeForge::GitHub, "warpdotdev", "warp-server"), None),
        clone_request(repo(CodeForge::GitLab, "platform", "api"), None),
    ];

    assert_eq!(
        unique_clone_hosts(&requests),
        vec!["github.com".to_string(), "gitlab.com".to_string()]
    );
}

#[test]
fn credential_query_is_noninteractive_and_contains_only_the_requested_host() {
    let command = build_git_credential_query_command("github.com", ShellType::Bash);

    assert!(command.contains("GIT_TERMINAL_PROMPT=0"));
    assert!(command.contains("GCM_INTERACTIVE=never"));
    assert!(command.contains("git credential fill"));
    assert!(command.contains("protocol=https"));
    assert!(command.contains("host=github.com"));
    assert!(!command.contains("https://"));
    assert!(!command.contains("password"));
}

#[test]
fn checkout_helper_quotes_paths_without_embedding_repository_payloads() {
    for shell in [
        ShellType::Bash,
        ShellType::Zsh,
        ShellType::Fish,
        ShellType::PowerShell,
    ] {
        let command = build_checkout_helper_command(
            Path::new("/Applications/Warp's App/Contents/MacOS/warp"),
            Path::new("/private/a path/requests.json"),
            Path::new("/private/a path/failures.json"),
            shell,
        );
        assert!(command.contains("environment-checkout --requests-file"));
        assert!(command.contains("' --failure-report '"));
        assert!(!command.contains("github.com") && !command.contains("sh -c"));
        assert_eq!(command.starts_with("& "), shell == ShellType::PowerShell);
    }
}

#[test]
fn failure_reports_are_validated_and_removed_on_every_read() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("failures.json");
    std::fs::write(
        &path,
        r#"{"failures":[{"request_index":1,"kind":"Checkout","output":"unknown ref"}]}"#,
    )
    .unwrap();
    let failures = read_checkout_failures(&path, 2).unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].request_index, 1);
    assert!(!path.exists());
    for bytes in [
        "{",
        r#"{"failures":[{"request_index":2,"kind":"Clone","output":""}]}"#,
        r#"{"failures":[{"request_index":0,"kind":"Clone","output":""},{"request_index":0,"kind":"Clone","output":""}]}"#,
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(read_checkout_failures(&path, 2).is_none());
        assert!(!path.exists());
    }
}

#[test]
fn powershell_post_checkout_commands_do_not_depend_on_posix_shells() {
    let requests = repository_clone_requests(
        &[
            repo(CodeForge::GitHub, "fixtures", "one"),
            repo(CodeForge::GitHub, "fixtures", "two"),
        ],
        &[],
        true,
    )
    .unwrap();
    let working_dir = Path::new("C:\\working directory");
    for command in [
        build_resolved_head_command(&requests, working_dir, ShellType::PowerShell),
        build_git_credential_query_command("github.com", ShellType::PowerShell),
        build_remove_repository_origins_command(&requests, working_dir, ShellType::PowerShell),
    ] {
        assert!(!command.contains("sh -c") && !command.contains("/dev/null"));
        assert!(command.contains("git"));
    }
}

#[cfg(feature = "local_tty")]
#[test]
fn snapshot_and_origin_removal_preserve_shell_output_and_failure_status() {
    use crate::terminal::model::session::command_executor::{
        ExecuteCommandOptions, LocalCommandExecutor,
    };

    let directory = tempfile::tempdir().unwrap();
    let working_dir = directory.path().join("quoted's working directory");
    fs::create_dir(&working_dir).unwrap();
    let requests = repository_clone_requests(
        &[
            repo(CodeForge::GitHub, "fixtures", "one"),
            repo(CodeForge::GitHub, "fixtures", "two"),
        ],
        &[],
        true,
    )
    .unwrap();
    let target = working_dir.join("one");
    fs::create_dir(&target).unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
        vec![
            "remote",
            "add",
            "origin",
            "https://github.com/fixtures/one.git",
        ],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&target)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    #[cfg(windows)]
    let (shell, executable) = (ShellType::PowerShell, "powershell.exe");
    #[cfg(not(windows))]
    let (shell, executable) = (ShellType::Bash, "/bin/bash");
    let executor = LocalCommandExecutor::new(Some(executable.into()), shell);
    let execute = |command: String| {
        block_on(executor.execute_local_command(
            &command,
            None,
            None,
            ExecuteCommandOptions::default(),
        ))
        .unwrap()
    };
    let output = execute(build_resolved_head_command(&requests, &working_dir, shell));
    let heads = parse_resolved_head_shas(&output.stdout, 2);
    assert!(heads[0].is_some());
    assert!(heads[1].is_none());

    fs::write(target.join(".git/config.lock"), "").unwrap();
    assert!(
        !execute(build_remove_repository_origins_command(
            &requests,
            &working_dir,
            shell
        ))
        .success()
    );
    fs::remove_file(target.join(".git/config.lock")).unwrap();
    assert!(
        execute(build_remove_repository_origins_command(
            &requests,
            &working_dir,
            shell
        ))
        .success()
    );
    assert!(
        !Command::new("git")
            .current_dir(&target)
            .args(["remote", "get-url", "origin"])
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn clone_failure_identity_diagnostics_keep_only_sanitized_expected_fields() {
    let author = command_output("Ada Lovelace\n", "", CommandExitStatus::Success);
    let github = command_output(
        "protocol=https\nhost=github.com\nusername=octocat\npassword=github-secret-token\n",
        "",
        CommandExitStatus::Success,
    );
    let gitlab = command_output(
        "username=gitlab-user\npassword=gitlab-secret-token\n",
        "",
        CommandExitStatus::Success,
    );

    let diagnostics = clone_failure_identity_diagnostics(
        Some(&author),
        [("github.com", Some(&github)), ("gitlab.com", Some(&gitlab))],
    );

    assert_eq!(diagnostics.author.as_deref(), Some("Ada Lovelace"));
    assert_eq!(
        diagnostics.credentials,
        vec![
            CloneFailureCredentialIdentity {
                host: "github.com".to_string(),
                username: Some("octocat".to_string()),
            },
            CloneFailureCredentialIdentity {
                host: "gitlab.com".to_string(),
                username: Some("gitlab-user".to_string()),
            },
        ]
    );
    let rendered = diagnostics.to_string();
    assert_eq!(
        rendered,
        "\nGit identity diagnostics:\n  Author: Ada Lovelace\n  Credential username for github.com: octocat\n  Credential username for gitlab.com: gitlab-user"
    );
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("protocol="));
    assert!(!rendered.contains("host="));
}

#[test]
fn clone_failure_identity_diagnostics_fall_back_on_timeout_malformed_or_failed_queries() {
    let malformed_author = command_output(
        "Ada\npassword=author-secret\n",
        "",
        CommandExitStatus::Success,
    );
    let malformed_username = command_output(
        "username=https://token@github.com\npassword=credential-secret\n",
        "",
        CommandExitStatus::Success,
    );
    let helper_failure = command_output(
        "username=ignored",
        "helper failed with password=stderr-secret",
        CommandExitStatus::Failure,
    );
    let duplicate_username = command_output(
        "username=first\nusername=second\npassword=duplicate-secret\n",
        "",
        CommandExitStatus::Success,
    );

    let diagnostics = clone_failure_identity_diagnostics(
        Some(&malformed_author),
        [
            ("github.com", Some(&malformed_username)),
            ("gitlab.com", Some(&helper_failure)),
            ("bitbucket.org", Some(&duplicate_username)),
            ("dev.azure.com", None),
        ],
    );

    assert_eq!(diagnostics.author, None);
    assert_eq!(
        diagnostics.credentials,
        vec![
            CloneFailureCredentialIdentity {
                host: "github.com".to_string(),
                username: None,
            },
            CloneFailureCredentialIdentity {
                host: "gitlab.com".to_string(),
                username: None,
            },
            CloneFailureCredentialIdentity {
                host: "bitbucket.org".to_string(),
                username: None,
            },
            CloneFailureCredentialIdentity {
                host: "dev.azure.com".to_string(),
                username: None,
            },
        ]
    );
    let rendered = diagnostics.to_string();
    assert_eq!(
        rendered,
        "\nGit identity diagnostics:\n  Author: unset\n  Credential username for github.com: unavailable\n  Credential username for gitlab.com: unavailable\n  Credential username for bitbucket.org: unavailable\n  Credential username for dev.azure.com: unavailable"
    );
    for secret in [
        "author-secret",
        "credential-secret",
        "stderr-secret",
        "duplicate-secret",
        "https://token@github.com",
    ] {
        assert!(!rendered.contains(secret));
    }
}

#[test]
fn clone_failure_errors_preserve_the_original_failure_before_diagnostics() {
    let diagnostics = CloneFailureIdentityDiagnostics {
        author: None,
        credentials: Vec::new(),
    };
    let clone_error = PrepareEnvironmentError::CloneRepo {
        repo_name: "warpdotdev/warp".to_string(),
        output: None,
        identity_diagnostics: diagnostics.clone(),
    };
    let checkout_error = PrepareEnvironmentError::CheckoutFailed {
        repo_name: "warpdotdev/warp".to_string(),
        checkout_ref: "deadbeef".to_string(),
        identity_diagnostics: diagnostics,
    };

    assert_eq!(
        clone_error.to_string(),
        "Failed to clone warpdotdev/warp\nGit identity diagnostics:\n  Author: unset"
    );
    assert_eq!(
        checkout_error.to_string(),
        "Failed to check out deadbeef in warpdotdev/warp\nGit identity diagnostics:\n  Author: unset"
    );
}

fn git(args: &[&str], cwd: &Path) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "saga-test")
        .env("GIT_AUTHOR_EMAIL", "saga-test@example.com")
        .env("GIT_COMMITTER_NAME", "saga-test")
        .env("GIT_COMMITTER_EMAIL", "saga-test@example.com")
        .output()
        .expect("git should be runnable");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Regression test for APP-5509: a `--filter=blob:none` clone must carry
/// enough tree data that a path-limited `git log` never reaches the network.
/// A regression to `--filter=tree:0` would instead try to lazily fetch a
/// tree per commit walked, so repointing `origin` at an unreachable URL
/// after the clone turns that regression into a fast failure here instead of
/// the hang seen in production.
#[test]
fn blobless_clone_walks_path_limited_history_without_network() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let origin = root.join("origin.git");
    fs::create_dir_all(&origin).unwrap();
    git(&["init", "-b", "main", "--bare", "."], &origin);
    git(&["config", "uploadpack.allowFilter", "true"], &origin);
    let origin_url = format!("file://{}", origin.display());

    let seed = root.join("seed");
    fs::create_dir_all(&seed).unwrap();
    git(&["init", "-b", "main", "."], &seed);
    git(&["remote", "add", "origin", &origin_url], &seed);
    for (contents, message) in [("one\n", "first"), ("one\ntwo\n", "second")] {
        fs::write(seed.join("notes.md"), contents).unwrap();
        git(&["add", "."], &seed);
        git(&["commit", "-m", message], &seed);
    }
    git(&["push", "origin", "main"], &seed);

    let repo_dir = root.join("clone");
    git(
        &[
            "clone",
            "--filter=blob:none",
            &origin_url,
            repo_dir.to_str().unwrap(),
        ],
        root,
    );

    // Simulate the origin becoming unreachable after the clone (e.g. the
    // sandbox network being torn down), so a lazy tree fetch fails fast
    // instead of hanging.
    git(
        &[
            "remote",
            "set-url",
            "origin",
            "https://127.0.0.1:1/unreachable.git",
        ],
        &repo_dir,
    );

    let output = Command::new("git")
        .args(["--no-pager", "log", "--oneline", "--", "notes.md"])
        .current_dir(&repo_dir)
        .output()
        .expect("git should be runnable");
    assert!(
        output.status.success(),
        "path-limited git log must stay local on a blobless clone: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let commit_count = String::from_utf8(output.stdout).unwrap().lines().count();
    assert_eq!(commit_count, 2, "expected both commits touching notes.md");
}

#[test]
fn factory_clone_is_prepended_when_clone_values_are_present() {
    let mut setup_commands = vec!["make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "https://t:token@definitions.example.com/team/factory.git",
        "acme_factory_repo",
        ShellType::Bash,
        &mut setup_commands,
    );
    assert_eq!(
        setup_commands,
        vec![
            "git clone \"$WARP_FACTORY_REPO_CLONE_URL\" \"$WARP_FACTORY_REPO_DIR\"".to_string(),
            "make setup".to_string(),
        ]
    );
}

#[test]
fn factory_clone_references_env_vars_with_the_session_shells_syntax() {
    let posix_command = "git clone \"$WARP_FACTORY_REPO_CLONE_URL\" \"$WARP_FACTORY_REPO_DIR\"";
    let powershell_command =
        "git clone \"$env:WARP_FACTORY_REPO_CLONE_URL\" \"$env:WARP_FACTORY_REPO_DIR\"";
    for (shell_type, expected) in [
        (ShellType::Bash, posix_command),
        (ShellType::Zsh, posix_command),
        (ShellType::Fish, posix_command),
        (ShellType::PowerShell, powershell_command),
    ] {
        let mut setup_commands = Vec::new();
        super::prepend_factory_definition_clone_for_values(
            "https://t:token@definitions.example.com/team/factory.git",
            "acme_factory_repo",
            shell_type,
            &mut setup_commands,
        );
        assert_eq!(setup_commands, vec![expected.to_string()], "{shell_type:?}");
    }
}

#[test]
fn factory_clone_is_skipped_without_clone_values() {
    let mut setup_commands = vec!["make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "",
        "",
        ShellType::Bash,
        &mut setup_commands,
    );
    super::prepend_factory_definition_clone_for_values(
        "url",
        "  ",
        ShellType::Bash,
        &mut setup_commands,
    );
    super::prepend_factory_definition_clone_for_values(
        "  ",
        "dir",
        ShellType::Bash,
        &mut setup_commands,
    );
    assert_eq!(setup_commands, vec!["make setup".to_string()]);
}

#[test]
fn factory_clone_defers_to_a_persisted_environment_copy() {
    // Environments provisioned before run-scoped cloning persist their own
    // copy of the clone command. Detection keys off the URL env var name,
    // not the command's exact shape, so this must still be recognized and
    // left alone rather than duplicated.
    let persisted =
        "git clone \"$WARP_FACTORY_REPO_CLONE_URL\" \"$WARP_FACTORY_REPO_DIR\"".to_string();
    let mut setup_commands = vec![persisted.clone(), "make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "https://t:token@definitions.example.com/team/factory.git",
        "acme_factory_repo",
        ShellType::Bash,
        &mut setup_commands,
    );
    assert_eq!(setup_commands, vec![persisted, "make setup".to_string()]);
}

#[test]
fn factory_clone_defers_to_a_persisted_bare_clone_copy() {
    // A persisted copy in the bare (no target dir) shape must also be
    // recognized, since detection is shape-independent.
    let persisted = "git clone \"$WARP_FACTORY_REPO_CLONE_URL\"".to_string();
    let mut setup_commands = vec![persisted.clone(), "make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "https://t:token@definitions.example.com/team/factory.git",
        "acme_factory_repo",
        ShellType::Bash,
        &mut setup_commands,
    );
    assert_eq!(setup_commands, vec![persisted, "make setup".to_string()]);
}
