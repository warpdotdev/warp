use clap::Parser;

use super::{CreateProvider, CreateSecretArgs, ListSecretsArgs, SecretCommand};

#[derive(Debug, Parser)]
struct TestSecret {
    #[command(subcommand)]
    command: SecretCommand,
}

fn parse_list(argv: &[&str]) -> ListSecretsArgs {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    let command = TestSecret::try_parse_from(full)
        .expect("parse succeeds")
        .command;
    let SecretCommand::List(args) = command else {
        panic!("expected list command");
    };
    args
}

fn parse_create(argv: &[&str]) -> CreateSecretArgs {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    let command = TestSecret::try_parse_from(full)
        .expect("parse succeeds")
        .command;
    let SecretCommand::Create(args) = command else {
        panic!("expected create command");
    };
    args
}

#[test]
fn list_accepts_explicit_team_uid() {
    let args = parse_list(&["list", "--team=team-uid"]);
    assert_eq!(args.scope.requested_team_uid(), Some("team-uid"));
}

#[test]
fn list_accepts_bare_team_selection() {
    let args = parse_list(&["list", "--team"]);
    assert!(args.scope.is_team());
    assert_eq!(args.scope.requested_team_uid(), None);
}

#[test]
fn list_accepts_no_team_flag() {
    let args = parse_list(&["list"]);
    assert!(!args.scope.is_team());
    assert!(!args.scope.personal);
}

#[test]
fn list_accepts_personal_scope() {
    let args = parse_list(&["list", "--personal"]);

    assert!(args.scope.personal);
    assert!(!args.scope.is_team());
}

#[test]
fn create_docker_registry_parses_minimal() {
    let args = parse_create(&["create", "docker-registry", "my-registry"]);
    let Some(CreateProvider::DockerRegistry(docker_registry)) = &args.provider else {
        panic!("expected docker-registry provider subcommand");
    };

    assert_eq!(docker_registry.common.name, "my-registry");
    assert!(docker_registry.host.is_none());
    assert!(docker_registry.username.is_none());
    assert!(docker_registry.password.is_none());
}

#[test]
fn create_docker_registry_accepts_host_username_password() {
    let args = parse_create(&[
        "create",
        "docker-registry",
        "my-registry",
        "--host",
        "ghcr.io",
        "--username",
        "octocat",
        "--password",
        "token-value",
    ]);
    let Some(CreateProvider::DockerRegistry(docker_registry)) = &args.provider else {
        panic!("expected docker-registry provider subcommand");
    };

    assert_eq!(docker_registry.host.as_deref(), Some("ghcr.io"));
    assert_eq!(docker_registry.username.as_deref(), Some("octocat"));
    assert_eq!(docker_registry.password.as_deref(), Some("token-value"));
}

#[test]
fn create_docker_registry_requires_name() {
    let result = TestSecret::try_parse_from(["test", "create", "docker-registry"]);
    assert!(result.is_err());
}

#[test]
fn create_docker_registry_accepts_password_file() {
    let args = parse_create(&[
        "create",
        "docker-registry",
        "my-registry",
        "--password-file",
        "password.txt",
    ]);
    let Some(CreateProvider::DockerRegistry(docker_registry)) = &args.provider else {
        panic!("expected docker-registry provider subcommand");
    };

    assert!(docker_registry.password.is_none());
    assert_eq!(
        docker_registry
            .password_file
            .as_ref()
            .and_then(|p| p.to_str()),
        Some("password.txt")
    );
}

#[test]
fn create_docker_registry_rejects_password_and_password_file() {
    let result = TestSecret::try_parse_from([
        "test",
        "create",
        "docker-registry",
        "my-registry",
        "--password",
        "token-value",
        "--password-file",
        "password.txt",
    ]);
    assert!(result.is_err());
}
