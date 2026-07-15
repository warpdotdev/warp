//! The headless `warp-tui` front-end's session bootstrap.
//!
//! [`run`] boots the real headless Warp app via [`warp::run_tui`]. Once shared
//! initialization is done, the mount built here starts the TUI driver and
//! defers creating the transcript-capable terminal session until login.

use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::{Context, Result};
use clap::error::ErrorKind;
use clap::Parser;
use pathfinder_geometry::vector::Vector2F;
use warp::tui_export::{
    Appearance, BannerState, IsSharedSessionCreator, LocalTtyTerminalManager,
    ServerConversationToken, TerminalManagerTrait, TerminalSurfaceResult,
};
use warp::{TuiLoginEvent, TuiLoginModel, TuiLoginPhase};
use warp_errors::report_error;
use warpui::SingletonEntity;
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{AddWindowOptions, AppContext, Entity, ModelHandle, ViewHandle};

use crate::resume::TuiExitSummaryHandle;
use crate::root_view::RootTuiView;
use crate::terminal_background::probe_and_select_theme;
use crate::terminal_session_view::{
    TuiConversationRestoreOrigin, TuiConversationRestoreTarget, TuiTerminalSessionView,
};
use crate::transcript_view::TRANSCRIPT_BLOCK_SPACING;

#[derive(Parser)]
#[command(name = "warp-tui")]
struct TuiArgs {
    /// Resume an Oz/Warp conversation by server token.
    #[arg(long)]
    resume: Option<String>,

    /// API key for non-interactive authentication.
    #[arg(long, env = "WARP_API_KEY")]
    api_key: Option<String>,
}

/// Validates and wraps a server conversation token from the command line.
fn parse_resume_token(token: String) -> Result<ServerConversationToken> {
    uuid::Uuid::parse_str(&token)
        .with_context(|| format!("invalid server conversation token: {token}"))?;
    Ok(ServerConversationToken::new(token))
}

/// Holds the live TUI driver and, after login, the terminal manager.
struct TuiSession {
    #[expect(dead_code, reason = "keeps the TUI driver alive for the TUI session")]
    driver: TuiDriverHandle,
    manager: Option<ModelHandle<Box<dyn TerminalManagerTrait>>>,
    resume_token: Option<ServerConversationToken>,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Boots the headless Warp app and mounts the transcript-capable TUI session.
pub fn run() -> Result<()> {
    // If this process was re-exec'd as a Warp worker (e.g. the terminal
    // server), dispatch that instead of starting another TUI — otherwise the
    // worker re-exec would recursively launch TUIs.
    if let Some(result) = warp::run_tui_worker_if_requested() {
        return result;
    }
    let args = match TuiArgs::try_parse() {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            return Ok(());
        }
        Err(error) => return Err(anyhow::Error::new(error)),
    };
    let resume_token = args.resume.map(parse_resume_token).transpose()?;
    let exit_summary = TuiExitSummaryHandle::default();
    let exit_summary_for_app = exit_summary.clone();
    let result = warp::run_tui(
        args.api_key,
        Box::new(move |ctx| init(resume_token, exit_summary_for_app, ctx)),
    );
    if result.is_ok() {
        if let Some(token) = exit_summary.token() {
            let token = token.as_str();
            println!("To continue this conversation, run: warp-tui --resume {token}");
        }
    }
    result
}

/// Creates the login-gated TUI root and starts the headless draw + input driver.
fn init(
    resume_token: Option<ServerConversationToken>,
    exit_summary: TuiExitSummaryHandle,
    ctx: &mut AppContext,
) {
    // Register the TUI views' keybindings (and, in debug builds, the
    // cross-surface binding validators) before any input can be dispatched.
    crate::keybindings::init(ctx);

    // Kick off the background auto-updater (its polling loop only runs for
    // release builds installed via the managed versioned layout; see the
    // `autoupdate` module docs).
    crate::autoupdate::TuiAutoupdater::register(ctx);

    // Theme the transcript to match the host terminal. Keep this scoped to
    // the TUI process by overriding the already-initialized Appearance theme at
    // mount time, without changing normal GUI theme selection or font settings.
    let theme = probe_and_select_theme();
    Appearance::handle(ctx).update(ctx, |appearance, ctx| {
        appearance.set_theme(theme, ctx);
    });

    let banner = ctx.add_model(|_| BannerState::default());
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |_| RootTuiView::new(exit_summary),
    );
    match spawn_tui_driver(ctx, window_id, root.clone()) {
        Ok(driver) => {
            let session = ctx.add_singleton_model(|_| TuiSession {
                driver,
                manager: None,
                resume_token,
            });
            if matches!(TuiLoginModel::as_ref(ctx).phase(), TuiLoginPhase::LoggedIn) {
                // Already authenticated at mount: create the session now.
                create_terminal_session_after_login(&session, &root, &banner, ctx);
            } else {
                // Otherwise wait for login to complete and create it then.
                let session_for_login = session.clone();
                let root_for_login = root.clone();
                let banner_for_login = banner.clone();
                let login_model = TuiLoginModel::handle(ctx);
                ctx.subscribe_to_model(&login_model, move |_, event, ctx| match event {
                    TuiLoginEvent::LoggedIn => create_terminal_session_after_login(
                        &session_for_login,
                        &root_for_login,
                        &banner_for_login,
                        ctx,
                    ),
                });
            }
        }
        Err(error) => {
            let error = anyhow::Error::new(error);
            report_error!(&error);
            ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
        }
    }
}

/// Creates and retains the terminal manager after login.
fn create_terminal_session_after_login(
    session: &ModelHandle<TuiSession>,
    root: &ViewHandle<RootTuiView>,
    banner: &ModelHandle<BannerState>,
    ctx: &mut AppContext,
) {
    if session.read(ctx, |session, _| session.manager.is_some()) {
        return;
    }

    let root = root.clone();
    let resume_token = session.read(ctx, |session, _| session.resume_token.clone());
    let manager = LocalTtyTerminalManager::<TuiTerminalSessionView>::create_tui_model(
        std::env::current_dir().ok(),
        HashMap::<OsString, OsString>::from_iter(std::env::vars_os()),
        IsSharedSessionCreator::No,
        None,
        banner.clone(),
        Vector2F::new(120., 24.),
        None,
        None,
        TRANSCRIPT_BLOCK_SPACING,
        ctx,
        move |surface_init, ctx| {
            let surface = root.update(ctx, |root, ctx| {
                let surface = root.create_terminal_session(surface_init, ctx);
                // Re-render the root so it swaps the login placeholder for the session.
                ctx.notify();
                surface
            });
            TerminalSurfaceResult {
                surface,
                post_wire: move |_manager: &mut LocalTtyTerminalManager<TuiTerminalSessionView>,
                                 surface: &ViewHandle<TuiTerminalSessionView>,
                                 ctx: &mut AppContext| {
                    if let Some(token) = resume_token {
                        surface.update(ctx, |view, ctx| {
                            view.restore_conversation(
                                TuiConversationRestoreTarget::Server(token),
                                TuiConversationRestoreOrigin::Startup,
                                ctx,
                            );
                        });
                    }
                },
            }
        },
    );

    session.update(ctx, |session, ctx| {
        session.manager = Some(manager.manager);
        session.resume_token = None;
        ctx.notify();
    });
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
