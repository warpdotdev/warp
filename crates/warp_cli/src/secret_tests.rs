use clap::Parser;

use super::{ListSecretsArgs, SecretCommand};

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

#[test]
fn list_accepts_explicit_team_uid() {
    let args = parse_list(&["list", "--team=team-uid"]);

    assert_eq!(args.team_selection.requested_team_uid(), Some("team-uid"));
}

#[test]
fn list_accepts_bare_team_selection() {
    let args = parse_list(&["list", "--team"]);

    assert!(args.team_selection.is_team());
    assert_eq!(args.team_selection.requested_team_uid(), None);
}

#[test]
fn list_accepts_no_team_flag() {
    let args = parse_list(&["list"]);

    assert!(!args.team_selection.is_team());
    assert_eq!(args.team_selection.requested_team_uid(), None);
}
