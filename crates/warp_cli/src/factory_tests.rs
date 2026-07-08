use clap::Parser;

use super::*;

fn file_listing(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir succeeds") {
            let path = entry.expect("entry is readable").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).expect("path is under root");
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files.sort();
    files
}

fn export_file_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(path, content)| (path.to_string(), content.to_string()))
        .collect()
}

#[derive(Debug, Parser)]
struct TestFactory {
    #[command(subcommand)]
    command: FactoryCommand,
}

fn parse_command(argv: &[&str]) -> FactoryCommand {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestFactory::try_parse_from(full)
        .expect("parse succeeds")
        .command
}

fn parse_command_err(argv: &[&str]) -> clap::Error {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestFactory::try_parse_from(full).expect_err("parse fails")
}

#[test]
fn link_parses_repo_branch_and_path() {
    let command = parse_command(&[
        "link",
        "fac-123",
        "--repo",
        "warpdotdev/factory-config",
        "--branch",
        "main",
        "--path",
        "factories/prod",
    ]);
    let FactoryCommand::Link(args) = command else {
        panic!("Expected link command");
    };

    assert_eq!(args.factory_uid, "fac-123");
    assert_eq!(
        args.repo,
        Some(RepoArg {
            owner: "warpdotdev".to_string(),
            repo: "factory-config".to_string(),
        })
    );
    assert_eq!(args.branch.as_deref(), Some("main"));
    assert_eq!(args.path.as_deref(), Some("factories/prod"));
    assert!(!args.unlink);
}

#[test]
fn link_defaults_branch_and_path_to_none() {
    let command = parse_command(&["link", "fac-123", "--repo", "warpdotdev/factory-config"]);
    let FactoryCommand::Link(args) = command else {
        panic!("Expected link command");
    };

    assert!(args.branch.is_none());
    assert!(args.path.is_none());
}

#[test]
fn link_requires_repo() {
    let err = parse_command_err(&["link", "fac-123"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn link_rejects_repo_without_slash() {
    let err = parse_command_err(&["link", "fac-123", "--repo", "warpdotdev"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    assert!(
        err.to_string().contains("owner/name"),
        "expected error to reference owner/name, got: {err}"
    );
}

#[test]
fn link_rejects_repo_with_empty_owner_or_name() {
    for repo in ["/factory-config", "warpdotdev/", "/"] {
        let err = parse_command_err(&["link", "fac-123", "--repo", repo]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }
}

#[test]
fn link_rejects_repo_with_extra_path_segments() {
    let err = parse_command_err(&["link", "fac-123", "--repo", "warpdotdev/factory/config"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn link_accepts_unlink_flag_without_repo() {
    let command = parse_command(&["link", "fac-123", "--unlink"]);
    let FactoryCommand::Link(args) = command else {
        panic!("Expected link command");
    };

    assert!(args.unlink);
    assert!(args.repo.is_none());
}

#[test]
fn link_rejects_unlink_with_repo() {
    let err = parse_command_err(&[
        "link",
        "fac-123",
        "--unlink",
        "--repo",
        "warpdotdev/factory-config",
    ]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn link_rejects_unlink_with_branch() {
    let err = parse_command_err(&["link", "fac-123", "--unlink", "--branch", "main"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn link_rejects_branch_without_repo() {
    let err = parse_command_err(&["link", "fac-123", "--branch", "main"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn unlink_parses() {
    let command = parse_command(&["unlink", "fac-123"]);
    let FactoryCommand::Unlink(args) = command else {
        panic!("Expected unlink command");
    };

    assert_eq!(args.factory_uid, "fac-123");
}

#[test]
fn status_parses() {
    let command = parse_command(&["status", "fac-123"]);
    let FactoryCommand::Status(args) = command else {
        panic!("Expected status command");
    };

    assert_eq!(args.factory_uid, "fac-123");
}

#[test]
fn status_requires_factory_uid() {
    let err = parse_command_err(&["status"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn plan_parses_without_sha() {
    let command = parse_command(&["plan", "fac-123"]);
    let FactoryCommand::Plan(args) = command else {
        panic!("Expected plan command");
    };

    assert_eq!(args.factory_uid, "fac-123");
    assert!(args.sha.is_none());
}

#[test]
fn plan_parses_with_sha() {
    let command = parse_command(&["plan", "fac-123", "--sha", "abc123"]);
    let FactoryCommand::Plan(args) = command else {
        panic!("Expected plan command");
    };

    assert_eq!(args.sha.as_deref(), Some("abc123"));
}

#[test]
fn apply_parses_sha_and_wait() {
    let command = parse_command(&["apply", "fac-123", "--sha", "abc123", "--wait"]);
    let FactoryCommand::Apply(args) = command else {
        panic!("Expected apply command");
    };

    assert_eq!(args.factory_uid, "fac-123");
    assert_eq!(args.sha.as_deref(), Some("abc123"));
    assert!(args.wait);
}

#[test]
fn apply_defaults_to_no_wait() {
    let command = parse_command(&["apply", "fac-123"]);
    let FactoryCommand::Apply(args) = command else {
        panic!("Expected apply command");
    };

    assert!(args.sha.is_none());
    assert!(!args.wait);
}

#[test]
fn tracing_str_distinguishes_link_unlink_shorthand() {
    assert_eq!(
        parse_command(&["link", "fac-123", "--repo", "a/b"]).as_str_for_tracing(),
        "factory link"
    );
    assert_eq!(
        parse_command(&["link", "fac-123", "--unlink"]).as_str_for_tracing(),
        "factory unlink"
    );
    assert_eq!(
        parse_command(&["unlink", "fac-123"]).as_str_for_tracing(),
        "factory unlink"
    );
}

#[test]
fn init_parses_dir_and_force() {
    let command = parse_command(&["init", "my-factory", "--force"]);
    let FactoryCommand::Init(args) = command else {
        panic!("Expected init command");
    };

    assert_eq!(args.dir, Some(PathBuf::from("my-factory")));
    assert!(args.force);
}

#[test]
fn init_defaults_to_current_dir_without_force() {
    let command = parse_command(&["init"]);
    let FactoryCommand::Init(args) = command else {
        panic!("Expected init command");
    };

    assert!(args.dir.is_none());
    assert!(!args.force);
}

#[test]
fn export_parses_out_and_force() {
    let command = parse_command(&["export", "fac-123", "--out", "exported", "--force"]);
    let FactoryCommand::Export(args) = command else {
        panic!("Expected export command");
    };

    assert_eq!(args.factory_uid, "fac-123");
    assert_eq!(args.out, Some(PathBuf::from("exported")));
    assert!(args.force);
}

#[test]
fn export_defaults_out_and_force() {
    let command = parse_command(&["export", "fac-123"]);
    let FactoryCommand::Export(args) = command else {
        panic!("Expected export command");
    };

    assert!(args.out.is_none());
    assert!(!args.force);
}

#[test]
fn export_requires_factory_uid() {
    let err = parse_command_err(&["export"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn tracing_str_covers_init_and_export() {
    assert_eq!(parse_command(&["init"]).as_str_for_tracing(), "factory init");
    assert_eq!(
        parse_command(&["export", "fac-123"]).as_str_for_tracing(),
        "factory export"
    );
}

#[test]
fn help_lists_every_factory_subcommand() {
    let err = parse_command_err(&["--help"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);

    let help = err.to_string();
    for subcommand in ["link", "unlink", "status", "plan", "apply", "init", "export"] {
        assert!(help.contains(subcommand), "help lists {subcommand}: {help}");
    }
    assert!(
        help.contains("Write a scaffold factory config directory"),
        "help describes init: {help}"
    );
    assert!(
        help.contains("Download a factory's rendered config files"),
        "help describes export: {help}"
    );
}

#[test]
fn init_creates_exact_scaffold_file_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("acme-factory");

    let written = init_factory_dir(&root, false).expect("init succeeds");

    let expected = vec![
        "README.md".to_string(),
        "agents/example-agent/agent.md".to_string(),
        "factory.yaml".to_string(),
        "secrets.yaml".to_string(),
    ];
    assert_eq!(written, expected);
    assert_eq!(file_listing(&root), expected);
    for subdir in SCAFFOLD_DIRS {
        let subdir_path = root.join(subdir);
        assert!(subdir_path.is_dir(), "{subdir} exists");
        assert_eq!(
            fs::read_dir(&subdir_path).expect("read_dir succeeds").count(),
            0,
            "{subdir} is empty"
        );
    }
}

#[test]
fn init_scaffold_content_matches_golden() {
    let files = scaffold_files("acme-factory");

    let golden_factory_yaml = "\
kind: Factory
schema_version: 1
name: \"acme-factory\"
# description: Describe this factory
# repositories:
#   - your-org/your-repo
# default_environment: staging
# default_model: claude-sonnet
# agent_defaults:
#   harness: claude_code
#   model: claude-sonnet
# agent_attribution_strategy: agent_identity
";
    assert_eq!(files["factory.yaml"], golden_factory_yaml);

    let golden_agent = "\
---
kind: Agent
schema_version: 1
description: An example agent. Replace this with your own.
# harness: claude_code
# model: claude-sonnet
# environment: staging
# secrets:
#   - GITHUB_TOKEN
# skills:
#   - name: my-skill
#     path: skills/my-skill
---
Describe the agent here. The markdown body becomes the agent's instructions.
";
    assert_eq!(files[SCAFFOLD_AGENT_PATH], golden_agent);

    let golden_secrets = "\
kind: SecretManifest
schema_version: 1
# Managed secret names this factory depends on. Never secret values.
secrets: []
# secrets:
#   - name: GITHUB_TOKEN
#     description: Token for GitHub API access
";
    assert_eq!(files["secrets.yaml"], golden_secrets);

    assert!(
        files["README.md"].contains("https://docs.warp.dev"),
        "README points at docs"
    );
}

#[test]
fn init_uses_directory_name_as_factory_name() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("acme-factory");

    init_factory_dir(&root, false).expect("init succeeds");

    let factory_yaml = fs::read_to_string(root.join("factory.yaml")).expect("factory.yaml exists");
    assert!(
        factory_yaml.contains("name: \"acme-factory\""),
        "got: {factory_yaml}"
    );
}

#[test]
fn init_refuses_non_empty_dir_without_force_and_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("existing");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("keep.txt"), "keep").expect("write keep.txt");

    let err = init_factory_dir(&root, false).expect_err("init fails");

    assert!(err.to_string().contains("--force"), "got: {err}");
    assert_eq!(file_listing(&root), vec!["keep.txt".to_string()]);
}

#[test]
fn init_force_overwrites_scaffold_files_and_keeps_others() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("existing");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("factory.yaml"), "stale").expect("write stale factory.yaml");
    fs::write(root.join("keep.txt"), "keep").expect("write keep.txt");

    init_factory_dir(&root, true).expect("forced init succeeds");

    let factory_yaml = fs::read_to_string(root.join("factory.yaml")).expect("factory.yaml exists");
    assert!(
        factory_yaml.starts_with("kind: Factory"),
        "got: {factory_yaml}"
    );
    assert_eq!(
        fs::read_to_string(root.join("keep.txt")).expect("keep.txt exists"),
        "keep"
    );
}

#[test]
fn export_write_preserves_subdirectories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("out");
    let files = export_file_map(&[
        ("factory.yaml", "kind: Factory\n"),
        ("agents/reviewer/agent.md", "agent body\n"),
        ("environments/staging.yaml", "kind: Environment\n"),
    ]);

    let written = write_export_files(&root, &files, false).expect("export write succeeds");

    let expected = vec![
        "agents/reviewer/agent.md".to_string(),
        "environments/staging.yaml".to_string(),
        "factory.yaml".to_string(),
    ];
    assert_eq!(written, expected);
    assert_eq!(file_listing(&root), expected);
    assert_eq!(
        fs::read_to_string(root.join("agents/reviewer/agent.md")).expect("agent.md exists"),
        "agent body\n"
    );
}

#[test]
fn export_write_rejects_path_traversal_and_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("out");

    for path in ["../escape.txt", "/absolute.txt", "agents/../../escape.txt", ""] {
        let files = export_file_map(&[("factory.yaml", "ok"), (path, "evil")]);

        let err = write_export_files(&root, &files, false).expect_err("export write fails");

        assert!(err.to_string().contains("unsafe path"), "path `{path}` got: {err}");
        assert!(!root.exists(), "path `{path}` must not create the out dir");
    }
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn export_write_refuses_non_empty_dir_without_force() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("out");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("keep.txt"), "keep").expect("write keep.txt");
    let files = export_file_map(&[("factory.yaml", "fresh")]);

    let err = write_export_files(&root, &files, false).expect_err("export write fails");

    assert!(err.to_string().contains("--force"), "got: {err}");
    assert_eq!(file_listing(&root), vec!["keep.txt".to_string()]);
}

#[test]
fn export_write_force_overwrites_existing_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("out");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("factory.yaml"), "stale").expect("write stale factory.yaml");
    fs::write(root.join("keep.txt"), "keep").expect("write keep.txt");
    let files = export_file_map(&[("factory.yaml", "fresh")]);

    write_export_files(&root, &files, true).expect("forced export write succeeds");

    assert_eq!(
        fs::read_to_string(root.join("factory.yaml")).expect("factory.yaml exists"),
        "fresh"
    );
    assert_eq!(
        fs::read_to_string(root.join("keep.txt")).expect("keep.txt exists"),
        "keep"
    );
}
