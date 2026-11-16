//! The headless `warp-tui` front-end's session bootstrap.
//!
//! [`run`] boots the real headless Warp app via [`warp::run_tui`]. Once shared
//! initialization is done, the mount built here starts the TUI driver and
//! creates the first terminal session once browser authentication starts, while
//! allowing authentication to complete in the background.

use std::io::{self, IsTerminal as _, Read as _};
use std::path::PathBuf;

use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use anyhow::{Context, Result, anyhow};
use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use inquire::{InquireError, Password, PasswordDisplayMode};
use sha2::{Digest, Sha256};
use warp::settings::{TuiThemeSettings, TuiZeroStateSettings, TuiZeroStateSettingsChangedEvent};
#[cfg(feature = "voice_input")]
use warp::settings::{TuiVoiceSettings, TuiVoiceSettingsChangedEvent};
use warp::tui_export::{
    AIConversationAutoexecuteMode, Appearance, AuthStateProvider, FactoryGitTokenResponse,
    ServerApiProvider, ServerConversationToken, authenticate_then_execute,
};
use warp::{TuiLoginEvent, TuiLoginModel, TuiLoginPhase};
use warp_core::channel::ChannelState;
use warp_core::settings::Setting as _;
use warp_core::telemetry::TelemetryEvent as _;
use warp_errors::report_error;
use warpui::SingletonEntity as _;
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{TuiDriverStartupError, TuiFocusPolicy, spawn_tui_driver};
use warpui_core::{AddWindowOptions, AppContext, ModelHandle, ViewHandle};
use warpui_extras::secure_storage::AppContextExt as _;

use crate::clipboard::copy_to_clipboard;
use crate::orchestration_model::TuiOrchestrationModel;
use crate::resume::TuiExitSummaryHandle;
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessions, TuiSessionsEvent};
use crate::telemetry::TuiStartupTelemetryEvent;
use crate::terminal_background::probe_and_select_theme;
use crate::terminal_session_view::{
    TuiConversationRestoreOrigin, TuiConversationRestoreTarget, tui_resume_shell_command,
};
#[cfg(feature = "voice_input")]
use crate::voice_input::requires_modifier_key_reporting;

/// Version string printed by `--version`. Release builds get `GIT_RELEASE_TAG`;
/// local cargo builds fall back to a numeric placeholder.
const CLI_VERSION: &str = match option_env!("GIT_RELEASE_TAG") {
    Some(version) => version,
    None => "v0.0.0.0.0.0",
};

#[derive(Debug, Parser)]
#[command(name = "warp", version = CLI_VERSION)]
struct TuiArgs {
    #[command(subcommand)]
    command: Option<TuiCommand>,

    /// Resume an Oz/Warp conversation by server token.
    #[arg(long)]
    resume: Option<String>,

    /// Enable auto-approve by default for new conversations.
    #[arg(long)]
    auto_approve: bool,

    /// API key for non-interactive authentication.
    #[arg(long, env = "WARP_API_KEY")]
    api_key: Option<String>,

    /// Securely store a model-provider API key for Warp Agent CLI.
    #[arg(
        long,
        value_name = LLMProvider::API_KEY_PROVIDER_VALUE_NAME,
        value_parser = LLMProvider::from_api_key_slug,
        conflicts_with_all = ["resume", "clear_provider_api_key"]
    )]
    set_provider_api_key: Option<LLMProvider>,

    /// Remove a securely stored model-provider API key from Warp Agent CLI.
    #[arg(
        long,
        value_name = LLMProvider::API_KEY_PROVIDER_VALUE_NAME,
        value_parser = LLMProvider::from_api_key_slug,
        conflicts_with_all = ["resume", "set_provider_api_key"]
    )]
    clear_provider_api_key: Option<LLMProvider>,
}

enum ProviderApiKeyCommand {
    Set {
        provider: LLMProvider,
        api_key: String,
    },
    Clear {
        provider: LLMProvider,
    },
}

/// One-shot commands available on the standalone TUI binaries. The full app
/// binary implements the same `dump-settings-schema` contract (see
/// `warp_cli::Command::DumpSettingsSchema`); this local copy lets
/// `script/run-tui` seed a real settings schema from whichever TUI binary is
/// about to run, without building the full GUI app binary just for that.
#[derive(Debug, Subcommand)]
enum TuiCommand {
    /// Print the JSON schema for the current Warp channel's settings and exit.
    DumpSettingsSchema {
        /// Write the schema to this path instead of standard output.
        output_path: Option<PathBuf>,
    },
    /// Generate a new Git access token for warp-hosted factory definitions
    /// for the current user.
    GetGitAccessJwt { factory_id: String },
}

/// Reads a provider API key from a masked TTY prompt or, when stdin is piped,
/// from stdin. Empty input and interactive cancellation return `Ok(None)`.
fn read_provider_api_key() -> Result<Option<String>> {
    if io::stdin().is_terminal() {
        return match Password::new("Provider API key:")
            .with_display_mode(PasswordDisplayMode::Masked)
            .without_confirmation()
            .prompt()
        {
            Ok(value) if value.trim().is_empty() => Ok(None),
            Ok(value) => Ok(Some(value)),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
            Err(error) => Err(error.into()),
        };
    }

    let mut value = String::new();
    io::stdin().read_to_string(&mut value)?;
    let value = value.trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

/// Validates and wraps a server conversation token from the command line.
fn parse_resume_token(token: String) -> Result<ServerConversationToken> {
    uuid::Uuid::parse_str(&token)
        .with_context(|| format!("invalid server conversation token: {token}"))?;
    Ok(ServerConversationToken::new(token))
}

fn build_jwt_cache_key(ctx: &AppContext, factory_id: &str) -> Result<String> {
    let server = ChannelState::server_root_url().to_string();
    let user_uid = AuthStateProvider::as_ref(ctx)
        .get()
        .user_id()
        .ok_or_else(|| anyhow!("Authenticated user has no user ID"))?
        .as_string();
    let permission = "read";

    let input = serde_json::to_vec(&(server, user_uid, factory_id, permission))?;
    let key = hex::encode(Sha256::digest(input));
    Ok(format!("factory-git-token-v1-{key}"))
}

fn read_cached_git_jwt(ctx: &AppContext, factory_id: &str) -> Option<String> {
    if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
        return None;
    }
    let cache_key = build_jwt_cache_key(ctx, factory_id).ok()?;

    let cached = ctx
        .secure_storage()
        .read_value(&cache_key)
        .map(|cached_str| serde_json::from_str::<FactoryGitTokenResponse>(&cached_str).ok())
        .ok()
        .flatten()?;

    // Request a new token if the cached one is expired or expiring soon.
    if cached.expires_in(chrono::TimeDelta::seconds(30)) {
        return None;
    }

    Some(cached.token)
}

fn cache_git_jwt_response(
    ctx: &AppContext,
    response: &FactoryGitTokenResponse,
    factory_id: &str,
) -> Result<()> {
    let cache_key = build_jwt_cache_key(ctx, factory_id)?;
    let serialized = serde_json::to_string(response)?;
    ctx.secure_storage().write_value(&cache_key, &serialized)?;
    Ok(())
}

/// Boots the headless Warp app and mounts the transcript-capable TUI session.
pub fn run() -> Result<()> {
    // Protect this managed version before any worker dispatch or resource
    // access. The guard stays alive until this process exits.
    let _version_lease = crate::autoupdate::VersionLease::acquire_for_current_process()?;
    // If this process was re-exec'd as a Warp worker (e.g. the terminal
    // server), dispatch that instead of starting another TUI — otherwise the
    // worker re-exec would recursively launch TUIs.
    if let Some(result) = warp::run_tui_worker_if_requested() {
        return result;
    }
    let args = match TuiArgs::try_parse() {
        Ok(args) => args,
        // Match the zero-state version line: bare tag/version, no binary name prefix.
        Err(error) if error.kind() == ErrorKind::DisplayVersion => {
            println!("{CLI_VERSION}");
            return Ok(());
        }
        Err(error) if error.kind() == ErrorKind::DisplayHelp => {
            error.print()?;
            return Ok(());
        }
        Err(error) => return Err(anyhow::Error::new(error)),
    };
    if let Some(command) = args.command {
        return match command {
            TuiCommand::DumpSettingsSchema { output_path } => {
                warp::features::init_feature_flags();
                warp::settings::dump_settings_schema(output_path.as_deref())
            }
            TuiCommand::GetGitAccessJwt { factory_id } => {
                let cached_factory_id = factory_id.clone();
                warp::run_tui_cli_command(Box::new(move |ctx| {
                    if let Some(token) = read_cached_git_jwt(ctx, &cached_factory_id) {
                        println!("{token}");
                        ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        return;
                    };
                    let login_hint =
                        "You are not logged in - please open Warp Agent CLI to log in.";
                    if let Err(error) =
                        authenticate_then_execute(None, login_hint, ctx, move |ctx| {
                            let factory = ServerApiProvider::as_ref(ctx).get_factory_client();
                            let app = ctx.weak_app();
                            ctx.foreground_executor()
                                .spawn(async move {
                                    let result =
                                        factory.get_factory_git_token(factory_id.clone()).await;
                                    let Some(mut app) = app.upgrade() else {
                                        return;
                                    };
                                    app.update(|ctx| match result {
                                        Ok(response) => {
                                            let _ =
                                                cache_git_jwt_response(ctx, &response, &factory_id);
                                            println!("{}", response.token);
                                            ctx.terminate_app(
                                                TerminationMode::ForceTerminate,
                                                None,
                                            );
                                        }
                                        Err(error) => {
                                            ctx.terminate_app(
                                                TerminationMode::ForceTerminate,
                                                Some(Err(error)),
                                            );
                                        }
                                    });
                                })
                                .detach();
                            Ok(())
                        })
                    {
                        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
                    }
                }))
            }
        };
    }
    let provider_api_key_command = if let Some(provider) = args.set_provider_api_key {
        if !provider.supports_pasted_api_key() {
            return Err(anyhow!(
                "Grok credentials must be connected with /api-keys in an active TUI"
            ));
        }
        let Some(api_key) = read_provider_api_key()? else {
            return Err(anyhow!("No provider API key was supplied"));
        };
        Some(ProviderApiKeyCommand::Set { provider, api_key })
    } else {
        match args.clear_provider_api_key {
            Some(LLMProvider::Xai) => {
                return Err(anyhow!(
                    "Grok credentials must be cleared with /api-keys in an active TUI"
                ));
            }
            Some(provider) => Some(ProviderApiKeyCommand::Clear { provider }),
            None => None,
        }
    };
    if let Some(command) = provider_api_key_command {
        return warp::run_tui_cli_command(Box::new(move |ctx| {
            let (provider, api_key, success_verb) = match command {
                ProviderApiKeyCommand::Set { provider, api_key } => {
                    (provider, Some(api_key), "saved")
                }
                ProviderApiKeyCommand::Clear { provider } => (provider, None, "cleared"),
            };
            let result = ApiKeyManager::handle(ctx)
                .update(ctx, |manager, ctx| {
                    manager.persist_provider_key(provider, api_key, ctx)
                })
                .and_then(|()| warp::tui_export::notify_tui_api_keys_changed());
            match result {
                Ok(()) => {
                    println!("{} API key {success_verb}", provider.display_name());
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(error) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
                }
            }
        }));
    }
    let resume_token = args.resume.map(parse_resume_token).transpose()?;
    let default_autoexecute_mode = if args.auto_approve {
        AIConversationAutoexecuteMode::RunToCompletion
    } else {
        AIConversationAutoexecuteMode::RespectUserSettings
    };
    let exit_summary = TuiExitSummaryHandle::default();
    let exit_summary_for_app = exit_summary.clone();
    let result = warp::run_tui(
        args.api_key,
        Box::new(move |ctx| {
            init(
                resume_token,
                default_autoexecute_mode,
                exit_summary_for_app,
                ctx,
            )
        }),
    );
    if result.is_ok()
        && let Some(token) = exit_summary.token()
    {
        let token = token.as_str();
        println!("To continue this conversation, run:");
        let command = tui_resume_shell_command(ChannelState::channel(), token);
        println!("{command}");
    }
    result
}

/// Creates the login-gated root and starts the headless draw and input driver.
fn init(
    resume_token: Option<ServerConversationToken>,
    default_autoexecute_mode: AIConversationAutoexecuteMode,
    exit_summary: TuiExitSummaryHandle,
    ctx: &mut AppContext,
) {
    warp_core::send_telemetry_from_app_ctx!(TuiStartupTelemetryEvent::from_environment(), ctx);
    // Register the TUI views' keybindings (and, in debug builds, the
    // cross-surface binding validators) before any input can be dispatched.
    crate::keybindings::init(ctx);

    // Kick off the background auto-updater (its polling loop only runs for
    // release builds installed via the managed versioned layout; see the
    // `autoupdate` module docs).
    crate::autoupdate::TuiAutoupdater::register(ctx);
    crate::zero_state_animation::ZeroStateAnimationConfig::register(ctx);

    // Honor an explicit TUI theme or match the host terminal automatically.
    // Keep this scoped to the TUI process by overriding the already-initialized
    // Appearance theme at mount time, without changing normal GUI theme
    // selection or font settings.
    let selected_theme = TuiThemeSettings::as_ref(ctx).selected_theme();
    let theme = probe_and_select_theme(selected_theme);
    Appearance::handle(ctx).update(ctx, |appearance, ctx| {
        appearance.set_theme(theme, ctx);
    });

    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        RootTuiView::new,
    );
    #[cfg(feature = "voice_input")]
    let modifier_key_lifecycle_enabled = requires_modifier_key_reporting(ctx);
    #[cfg(not(feature = "voice_input"))]
    let modifier_key_lifecycle_enabled = false;
    let freeze_repaints_when_unfocused = *TuiZeroStateSettings::as_ref(ctx)
        .freeze_animation_when_unfocused
        .value();
    match spawn_tui_driver(
        ctx,
        window_id,
        root.clone(),
        TuiFocusPolicy::PresentedTree,
        modifier_key_lifecycle_enabled,
        freeze_repaints_when_unfocused,
    ) {
        Ok(driver) => {
            let sessions = ctx.add_singleton_model(|_| {
                TuiSessions::new(driver, exit_summary, resume_token, default_autoexecute_mode)
            });
            let sessions_for_zero_state_settings = sessions.clone();
            ctx.subscribe_to_model(
                &TuiZeroStateSettings::handle(ctx),
                move |settings, event, ctx| {
                    let TuiZeroStateSettingsChangedEvent::TuiZeroStateFreezeAnimationWhenUnfocusedSetting {
                        ..
                    } = event
                    else {
                        return;
                    };
                    let freeze =
                        *settings.as_ref(ctx).freeze_animation_when_unfocused.value();
                    sessions_for_zero_state_settings.update(ctx, |sessions, _| {
                        sessions.set_freeze_repaints_when_unfocused(freeze);
                    });
                    ctx.invalidate_all_views();
                },
            );
            #[cfg(feature = "voice_input")]
            let sessions_for_voice_settings = sessions.clone();
            #[cfg(feature = "voice_input")]
            ctx.subscribe_to_model(&TuiVoiceSettings::handle(ctx), move |_, event, ctx| {
                let TuiVoiceSettingsChangedEvent::TuiVoiceInputHoldKeySetting { .. } = event;
                let enabled = requires_modifier_key_reporting(ctx);
                let result = sessions_for_voice_settings.update(ctx, |sessions, ctx| {
                    sessions.set_modifier_key_lifecycle_enabled(enabled, ctx)
                });
                if let Err(error) = result {
                    report_error!(
                        anyhow::Error::new(error)
                            .context("failed to update TUI modifier key reporting")
                    );
                }
            });
            root.update(ctx, |_, ctx| {
                ctx.subscribe_to_model(&sessions, |_, _, event, ctx| match event {
                    TuiSessionsEvent::SessionRemoved(_) => ctx.notify(),
                    TuiSessionsEvent::FocusChanged(_) => ctx.notify(),
                });
            });
            let orchestration = TuiOrchestrationModel::register(ctx);
            TuiSessions::wire_orchestration(&sessions, &orchestration, ctx);
            let sessions_for_login = sessions.clone();
            let root_for_login = root.clone();
            let login_model = TuiLoginModel::handle(ctx);
            ctx.subscribe_to_model(&login_model, move |_, event, ctx| match event {
                TuiLoginEvent::PhaseChanged => {
                    root_for_login.update(ctx, |root, ctx| {
                        root.handle_login_phase_changed(ctx, copy_to_clipboard);
                    });
                }
                TuiLoginEvent::LoggedIn => {
                    ensure_terminal_session(&sessions_for_login, &root_for_login, ctx)
                }
                TuiLoginEvent::LoggedOut => {
                    root_for_login.update(ctx, |root, ctx| root.show_auth(ctx));
                    sessions_for_login.update(ctx, |sessions, ctx| sessions.clear(ctx));
                }
            });
            if matches!(TuiLoginModel::as_ref(ctx).phase(), TuiLoginPhase::LoggedIn) {
                // Already authenticated at mount: create the first session now.
                ensure_terminal_session(&sessions, &root, ctx);
            }
        }
        Err(error) => handle_tui_driver_startup_error(error, ctx),
    }
}

fn handle_tui_driver_startup_error(error: TuiDriverStartupError, ctx: &mut AppContext) {
    match error {
        TuiDriverStartupError::TerminalDisconnected(error) => {
            log::error!("failed to start the TUI driver: {error}");
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
        TuiDriverStartupError::Unexpected(error) => {
            let error = anyhow::Error::new(error);
            report_error!(&error);
            ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
        }
    }
}

/// Creates the focused bootstrap session and restores the requested conversation.
fn ensure_terminal_session(
    sessions: &ModelHandle<TuiSessions>,
    root: &ViewHandle<RootTuiView>,
    ctx: &mut AppContext,
) {
    if sessions.read(ctx, |sessions, _| !sessions.is_empty()) {
        return;
    }

    let resume_token = sessions.update(ctx, |sessions, _| sessions.take_resume_token());
    let window_id = root.window_id(ctx);
    let handles_first_run_onboarding = resume_token.is_none();
    let (_, surface) = TuiSessions::create_local_terminal_session(
        sessions,
        window_id,
        true,
        handles_first_run_onboarding,
        std::env::current_dir().ok(),
        ctx,
    );
    surface.update(ctx, |view, ctx| {
        view.enable_cli_agent_osc_event_publishing(ctx);
    });
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
    TuiLoginModel::record_terminal_shown(ctx);
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
