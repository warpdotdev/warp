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
use warp_cli::share::ShareRequest;
use warp_cli::terminal::{TerminalCommand, TerminalShareArgs};
use warpui::platform::TerminationMode;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::driver::AgentDriverError;
use super::driver::terminal::{TerminalDriver, TerminalDriverEvent, TerminalDriverOptions};
use super::report_fatal_error;
use crate::terminal::shared_session::SharedSessionSource;
use crate::terminal::view::Event;

/// Dispatch a `warp terminal ...` command.
pub(crate) fn run(ctx: &mut AppContext, command: TerminalCommand) -> anyhow::Result<()> {
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
    // Recipients from `--share`. When the flag is omitted (`None`) or passed
    // bare with no value (`Some(vec![])`), this resolves to an empty list, which
    // shares the session owner-only (share-with-self): the invoking user retains
    // access and no additional team/public/user ACL is applied. Recipients are
    // added only when explicitly requested. This matches spec Behavior #4.
    let share_requests = args.share.share.unwrap_or_default();

    let terminal_driver = TerminalDriver::create(
        TerminalDriverOptions {
            working_dir,
            env_vars: HashMap::new(),
            should_share: true,
            // A `warp terminal share` session is user-initiated, not an ambient
            // agent session, so attribute its share source to the user.
            share_source: SharedSessionSource::user(None),
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
    /// Set synchronously once the shared session is established and the join
    /// link has been printed. Only after this is a shell-PTY exit treated as a
    /// clean session end; before it, an exit is surfaced as a bootstrap/share
    /// failure by the drive loop (or the share timeout). Recording it in the
    /// same synchronous event turn that prints the link — rather than on a
    /// later async hop — avoids a race where an `Event::Exited` arriving in the
    /// gap would be dropped, hanging the process.
    shared_session_established: bool,
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
        // Print the join link to stdout once the shared session is established,
        // and record establishment in the same synchronous turn so a subsequent
        // shell exit is reliably treated as a clean session end.
        ctx.subscribe_to_model(&terminal_driver, |me, _, event, _| match event {
            TerminalDriverEvent::EstablishedSharedSession { join_url, .. } => {
                me.shared_session_established = true;
                print_join_link(join_url);
            }
            TerminalDriverEvent::SlowBootstrap => {
                eprintln!("Warning: Terminal session is slow to bootstrap.");
            }
        });

        // Terminate the process cleanly when the shared shell PTY exits. Only
        // acts once the shared session is established and the join link has been
        // printed, so a shell that exits during bootstrap or before sharing
        // completes never yields a spurious exit-0 with no link — those paths
        // are surfaced as failures by the drive loop below (or the share
        // timeout).
        let terminal_view = terminal_driver.as_ref(ctx).terminal_view().clone();
        ctx.subscribe_to_view(&terminal_view, |me, _, event, ctx| {
            if me.shared_session_established && matches!(event, Event::Exited) {
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

                // Wait for session sharing to be established. When it succeeds,
                // the `EstablishedSharedSession` event prints the join link and
                // records establishment (see the subscription above); only then
                // does a shell exit terminate the process cleanly.
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
            shared_session_established: false,
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
