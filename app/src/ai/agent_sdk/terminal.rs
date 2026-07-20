//! Execution handler for the `warp terminal share` CLI command.
//!
//! Starts a headless terminal session by reusing the existing
//! [`TerminalDriver`] (the same driver that hosts ambient-agent sessions),
//! shares it, prints the resulting join link to stdout, and blocks until the
//! underlying session exits.
//!
//! The clap argument types live in the `warp_cli` crate; all code that touches
//! the `pub(crate)` [`TerminalDriver`] lives here in the `app` crate so the
//! `warp_cli` ↔ `app` visibility boundary stays clean.

use std::collections::HashMap;
use std::io::Write;

use anyhow::Context as _;
use warp_cli::GlobalOptions;
use warp_cli::share::ShareRequest;
use warp_cli::terminal::{TerminalCommand, TerminalShareArgs};
use warpui::platform::TerminationMode;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::driver::AgentDriverError;
use super::driver::terminal::{TerminalDriver, TerminalDriverEvent, TerminalDriverOptions};
use super::report_fatal_error;
use crate::terminal::view::Event;

/// Dispatch a `warp terminal ...` command.
pub(crate) fn run(
    ctx: &mut AppContext,
    _global_options: GlobalOptions,
    command: TerminalCommand,
) -> anyhow::Result<()> {
    match command {
        TerminalCommand::Share(args) => run_share(ctx, args),
    }
}

/// Start a headless shared terminal session and drive it to completion.
fn run_share(ctx: &mut AppContext, args: TerminalShareArgs) -> anyhow::Result<()> {
    let working_dir = match args.working_dir {
        Some(dir) => dir,
        None => std::env::current_dir().context("Failed to determine the current directory")?,
    };
    let share_requests = args.share.share.unwrap_or_default();

    let terminal_driver = TerminalDriver::create(
        TerminalDriverOptions {
            working_dir,
            env_vars: HashMap::new(),
            should_share: true,
            task_id: None,
            conversation_restoration: None,
        },
        ctx,
    )?;

    // Register the runner as a singleton so the UI framework keeps it (and the
    // terminal driver it owns) alive while the session runs. The process stays
    // alive until the runner terminates it on session exit or fatal error.
    ctx.add_singleton_model(|ctx| TerminalShareRunner::new(terminal_driver, share_requests, ctx));

    Ok(())
}

/// Owns the [`TerminalDriver`] for a `warp terminal share` invocation and drives
/// it to completion, printing the join link and terminating the process when the
/// shared session exits.
struct TerminalShareRunner {
    terminal_driver: ModelHandle<TerminalDriver>,
    /// Set once the session has bootstrapped, so a subsequent shell exit is
    /// treated as a clean session end rather than a bootstrap failure.
    bootstrapped: bool,
}

impl Entity for TerminalShareRunner {
    type Event = ();
}

/// Singleton only so the UI framework does not drop the runner (and its terminal
/// driver) while the shared session is live.
impl SingletonEntity for TerminalShareRunner {}

impl TerminalShareRunner {
    fn new(
        terminal_driver: ModelHandle<TerminalDriver>,
        share_requests: Vec<ShareRequest>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Print the join link to stdout once the shared session is established.
        ctx.subscribe_to_model(&terminal_driver, |_, _, event, _| match event {
            TerminalDriverEvent::EstablishedSharedSession { join_url, .. } => {
                print_join_link(join_url);
            }
            TerminalDriverEvent::SlowBootstrap => {
                eprintln!("Warning: Terminal session is slow to bootstrap.");
            }
        });

        // Terminate the process cleanly when the shared shell PTY exits. Only
        // acts post-bootstrap; a pre-bootstrap exit is surfaced as a bootstrap
        // failure by the drive loop below.
        let terminal_view = terminal_driver.as_ref(ctx).terminal_view().clone();
        ctx.subscribe_to_view(&terminal_view, |me, _, event, ctx| {
            if me.bootstrapped && matches!(event, Event::Exited) {
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
        });

        // Apply the requested share recipients (applied immediately once the
        // session is established).
        terminal_driver.update(ctx, |driver, ctx| {
            driver.add_share_requests(share_requests, ctx);
        });

        // Drive bootstrap → sharing, surfacing any failure as a non-zero exit.
        let foreground = ctx.spawner();
        ctx.spawn(
            async move {
                // Wait for the terminal session to bootstrap.
                let Ok(bootstrap) = foreground
                    .spawn(|me, ctx| {
                        me.terminal_driver
                            .update(ctx, |driver, _| driver.wait_for_session_bootstrapped())
                    })
                    .await
                else {
                    // The runner (and driver) was dropped — the app is shutting
                    // down, so there is nothing left to do.
                    return Ok(());
                };
                if let Err(error) = bootstrap.await {
                    return Err(anyhow::Error::new(AgentDriverError::BootstrapFailed {
                        error,
                    }));
                }

                // Record bootstrap completion so a subsequent shell exit is
                // treated as a clean session end.
                if foreground
                    .spawn(|me, _| me.bootstrapped = true)
                    .await
                    .is_err()
                {
                    return Ok(());
                }

                // Wait for session sharing to be established.
                let Ok(shared) = foreground
                    .spawn(|me, ctx| {
                        me.terminal_driver
                            .update(ctx, |driver, _| driver.wait_for_session_shared())
                    })
                    .await
                else {
                    return Ok(());
                };
                shared.await.map_err(anyhow::Error::new)
            },
            |_, result, ctx| {
                if let Err(error) = result {
                    // Prints to stderr and exits non-zero; no join link is
                    // printed on the failure paths.
                    report_fatal_error(error, ctx);
                }
                // On success the process keeps running until the shared shell
                // PTY exits, which the `Event::Exited` subscription handles.
            },
        );

        Self {
            terminal_driver,
            bootstrapped: false,
        }
    }
}

/// Write the join link to stdout on its own line and flush, so scripts see it
/// immediately even when stdout is not a TTY. This is the command's only
/// non-diagnostic stdout output; diagnostics go to stderr / the log file.
fn print_join_link(join_url: &str) {
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{join_url}");
    let _ = stdout.flush();
}
