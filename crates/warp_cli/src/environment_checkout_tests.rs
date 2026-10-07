use clap::Parser as _;

use super::*;
use crate::{Args, CliCommand, Command};

#[test]
fn hidden_command_parses_paths_and_traces_without_payload() {
    let args = Args::try_parse_from([
        "oz",
        "environment-checkout",
        "--requests-file",
        "a path/request.json",
        "--report-file",
        "a path/report.json",
        "--remove-origins-only",
    ])
    .unwrap();
    let Some(Command::CommandLine(command)) = args.command() else {
        panic!("CLI command")
    };
    assert_eq!(command.as_str_for_tracing(), "environment_checkout");
    let CliCommand::EnvironmentCheckout(args) = command.as_ref() else {
        panic!("checkout command")
    };
    assert_eq!(args.requests_file, PathBuf::from("a path/request.json"));
    assert_eq!(args.report_file, PathBuf::from("a path/report.json"));
    assert!(args.remove_origins_only);
    assert!(
        !Args::clap_command()
            .render_long_help()
            .to_string()
            .contains("environment-checkout")
    );
}
