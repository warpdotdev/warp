//! The headless `warp-tui` front-end's session bootstrap.
//!
//! [`run`] boots the real headless Warp app via [`warp::run_tui`]. Once shared
//! initialization is done, the mount built here starts the TUI driver and
//! defers creating the first terminal session until login.

use anyhow::{Context, Result};
use clap::error::ErrorKind;
use clap::Parser;
use warp::tui_export::{Appearance, ServerConversationToken};
use warp::{TuiLoginEvent, TuiLoginModel, TuiLoginPhase};
use warp_core::telemetry::TelemetryEvent as _;
use warp_errors::report_error;
use warpui::SingletonEntity as _;
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::spawn_tui_driver;
use warpui_core::{AddWindowOptions, AppContext, ModelHandle, ViewHandle};

use crate::orchestration_model::TuiOrchestrationModel;
use crate::resume::TuiExitSummaryHandle;
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessions, TuiSessionsEvent};
use crate::telemetry::TuiStartupTelemetryEvent;
use crate::terminal_background::probe_and_select_theme;
use crate::terminal_session_view::{TuiConversationRestoreOrigin, TuiConversationRestoreTarget};

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

/// Creates the login-gated root and starts the headless draw and input driver.
fn init(
    resume_token: Option<ServerConversationToken>,
    exit_summary: TuiExitSummaryHandle,
    ctx: &mut AppContext,
) {
    warp_core::send_telemetry_from_app_ctx!(TuiStartupTelemetryEvent, ctx);
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

    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |_| RootTuiView::new(),
    );
    match spawn_tui_driver(ctx, window_id, root.clone()) {
        Ok(driver) => {
            let sessions =
                ctx.add_singleton_model(|_| TuiSessions::new(driver, exit_summary, resume_token));
            root.update(ctx, |_, ctx| {
                ctx.subscribe_to_model(&sessions, |_, _, event, ctx| match event {
                    TuiSessionsEvent::SessionRemoved(_) => ctx.notify(),
                    TuiSessionsEvent::FocusChanged(_) => ctx.notify(),
                });
            });
            let orchestration = TuiOrchestrationModel::register(ctx);
            TuiSessions::wire_orchestration(&sessions, &orchestration, ctx);
            if matches!(TuiLoginModel::as_ref(ctx).phase(), TuiLoginPhase::LoggedIn) {
                // Already authenticated at mount: create the first session now.
                create_terminal_session_after_login(&sessions, &root, ctx);
            } else {
                // Otherwise wait for login to complete and create it then.
                let sessions_for_login = sessions.clone();
                let root_for_login = root.clone();
                let login_model = TuiLoginModel::handle(ctx);
                ctx.subscribe_to_model(&login_model, move |_, event, ctx| match event {
                    TuiLoginEvent::LoggedIn => create_terminal_session_after_login(
                        &sessions_for_login,
                        &root_for_login,
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

/// Creates the focused bootstrap session and restores the requested conversation.
fn create_terminal_session_after_login(
    sessions: &ModelHandle<TuiSessions>,
    root: &ViewHandle<RootTuiView>,
    ctx: &mut AppContext,
) {
    if sessions.read(ctx, |sessions, _| !sessions.is_empty()) {
        return;
    }

    let resume_token = sessions.update(ctx, |sessions, _| sessions.take_resume_token());
    let window_id = root.window_id(ctx);
    let (_, surface) = TuiSessions::create_local_terminal_session(
        sessions,
        window_id,
        true,
        std::env::current_dir().ok(),
        ctx,
    );
    if let Some(token) = resume_token {
        surface.update(ctx, |view, ctx| {
            view.restore_conversation(
                TuiConversationRestoreTarget::Server(token),
                TuiConversationRestoreOrigin::Startup,
                ctx,
            );
        });
    }
    root.update(ctx, |root, ctx| root.show_terminal(ctx));
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
