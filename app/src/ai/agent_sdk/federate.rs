use std::process;

use anyhow::{anyhow, Result};
use serde_json::json;
use warp_cli::agent::OutputFormat;
use warp_cli::federate::{FederateCommand, IssueGcpTokenArgs, IssueTokenArgs};
use warp_cli::GlobalOptions;
use warp_core::features::FeatureFlag;
use warp_core::report_error;
use warp_managed_secrets::ManagedSecretManager;
use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity as _};

use super::common::set_ambient_task_context_from_run_id;

/// Run identity federation commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: FederateCommand,
) -> Result<()> {
    if !FeatureFlag::OzIdentityFederation.is_enabled() {
        return Err(anyhow::anyhow!("This feature is not enabled"));
    }
    match command {
        FederateCommand::IssueToken(args) => issue_token(ctx, args, global_options.output_format),
        FederateCommand::IssueGcpToken(args) => issue_gcp_token(ctx, args),
    }
}

fn issue_token(
    ctx: &mut AppContext,
    args: IssueTokenArgs,
    output_format: OutputFormat,
) -> Result<()> {
    // Set the task ID so the ambient agent workload token header is sent.
    set_ambient_task_context_from_run_id(ctx, &args.run_id)?;

    let duration: std::time::Duration = args.duration.into();
    let audience = args.audience;
    let subject_template = match args.subject_template {
        Some(template) => vec1::Vec1::try_from_vec(template)
            .map_err(|_| anyhow::anyhow!("--subject-template requires at least one value"))?,
        None => vec1::vec1!["principal".to_owned()],
    };

    ManagedSecretManager::handle(ctx).update(ctx, move |manager, ctx| {
        let future =
            manager.issue_task_identity_token(warp_managed_secrets::client::IdentityTokenOptions {
                audience,
                requested_duration: duration,
                subject_template,
            });
        ctx.spawn(future, move |_, result, ctx| match result {
            Ok(token) => {
                let token_value = token.token;
                let expires_at = token.expires_at.to_rfc3339();
                let issuer = token.issuer;
                match output_format {
                    OutputFormat::Json | OutputFormat::Ndjson => {
                        let output = json!({
                            "token": token_value,
                            "expires_at": expires_at,
                            "issuer": issuer,
                        });
                        let output =
                            serde_json::to_string(&output).expect("token output should serialize");
                        println!("{output}");
                    }
                    OutputFormat::Text => {
                        println!("{token_value}");
                    }
                    OutputFormat::Pretty => {
                        println!("Token: {token_value}");
                        println!("Expires at: {expires_at}");
                        println!("Issuer: {issuer}");
                    }
                }
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(err) => {
                super::report_fatal_error(err, ctx);
            }
        });
    });

    Ok(())
}
fn issue_gcp_token(ctx: &mut AppContext, args: IssueGcpTokenArgs) -> Result<()> {
    // Set the task ID so the ambient agent workload token header is sent.
    set_ambient_task_context_from_run_id(ctx, &args.run_id)?;

    let duration: std::time::Duration = args.duration.into();
    let audience = args.audience;
    let token_type = args.token_type;
    let output_file = args.output_file;

    ManagedSecretManager::handle(ctx).update(ctx, move |manager, ctx| {
        let future =
            manager.issue_gcp_workload_identity_federation_token(audience, token_type, duration);
        ctx.spawn(future, move |_, result, ctx| match result {
            Ok(token) => {
                let output =
                    serde_json::to_string(&token).expect("gcp token output should serialize");

                // If we can't cache the token, report an error but don't fail the command.
                if let Some(output_path) = output_file {
                    if let Err(err) = std::fs::write(&output_path, &output) {
                        report_error!(anyhow!(err)
                            .context(format!("Error writing GCP token to {output_path}")));
                    }
                }

                println!("{output}");
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(err) => {
                // The GCP SDK requires the executable to print a JSON error to stderr
                // and exit with a non-zero status code. Because of this, we exit
                // directly instead of via `ctx.terminate_app` (which would print a
                // non-JSON error).
                let output =
                    serde_json::to_string(&err).expect("gcp error output should serialize");
                eprintln!("{output}");
                process::exit(1);
            }
        });
    });

    Ok(())
}
