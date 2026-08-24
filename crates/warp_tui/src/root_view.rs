//! [`RootTuiView`]: the login-gated root view of the `warp-tui` front-end.
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use warp::tui_export::{ServerId, UserWorkspaces};
use warp::{TuiLoginModel, TuiLoginPhase};
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::SingletonEntity as _;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{TuiChildView, TuiElement};
use warpui_core::keymap::FixedBinding;
use warpui_core::keymap::macros::*;
use warpui_core::platform::TerminationMode;
use warpui_core::{
    AppContext, Entity, EntityId, FocusContext, TuiView, TypedActionView, ViewContext, WindowId,
    keymap,
};

use crate::clipboard::copy_to_clipboard;
use crate::keybindings::TUI_BINDING_GROUP;
use crate::session_registry::{TuiSessionView, TuiSessions};
use crate::transient_hint::TransientHint;
use crate::ui::{
    LoginBrowserOpenFailedParams, LoginFailedParams, LoginWaitingParams, login_browser_open_failed,
    login_failed, login_waiting, signed_out_welcome, terminal_starting,
};
use crate::zero_state_animation::ZeroStateAnimationConfig;
const LAST_TEAM_STORAGE_KEY: &str = "TuiLastTeamUid";

/// Typed actions handled by [`RootTuiView`].
#[derive(Debug, Clone)]
pub enum RootTuiAction {
    /// Exits the app while no terminal session is focused.
    ExitApp,
    /// Starts or retries browser device authorization from a signed-out screen.
    StartDeviceLogin,
    /// Starts device authorization and copies its exact URL once generated.
    StartDeviceLoginAndCopyUrl,
    /// Opens the current device-authorization URL.
    OpenLoginUrl(String),
    /// Copies the manual browser fallback shown while authorization is pending.
    CopyLoginUrl(String),
}

/// Whether the root is presenting authentication or the live session container.
enum RootTuiState {
    Auth,
    Terminal,
}

/// The app-level TUI shell, projecting only the focused full session view.
pub struct RootTuiView {
    state: RootTuiState,
    auth_animation_clock: AnimationClock,
    auth_animation_config: Arc<ZeroStateAnimationConfig>,
    welcome_login_mouse: MouseStateHandle,
    welcome_copy_mouse: MouseStateHandle,
    waiting_login_mouse: MouseStateHandle,
    waiting_login_copy_mouse: MouseStateHandle,
    waiting_login_retry_mouse: MouseStateHandle,
    failed_login_retry_mouse: MouseStateHandle,
    copy_login_url_when_available: bool,
    login_copy_hint: TransientHint,
}

/// Registers the root view's keybindings.
pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        RootTuiAction::ExitApp,
        id!(RootTuiView::ui_name()),
    )
    .with_group(TUI_BINDING_GROUP)]);
}

impl RootTuiView {
    /// Creates the login-gated root view.
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        let window_id = ctx.window_id();
        let team_uid = Self::restore_last_team_uid(ctx)
            .or_else(|| UserWorkspaces::as_ref(ctx).inherited_or_default_team_uid(None));
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, team_uid, ctx);
        });
        Self {
            state: RootTuiState::Auth,
            auth_animation_clock: AnimationClock::starting_at(Duration::ZERO),
            auth_animation_config: Arc::new(ZeroStateAnimationConfig::default()),
            welcome_login_mouse: MouseStateHandle::default(),
            welcome_copy_mouse: MouseStateHandle::default(),
            waiting_login_mouse: MouseStateHandle::default(),
            waiting_login_copy_mouse: MouseStateHandle::default(),
            waiting_login_retry_mouse: MouseStateHandle::default(),
            failed_login_retry_mouse: MouseStateHandle::default(),
            copy_login_url_when_available: false,
            login_copy_hint: TransientHint::default(),
        }
    }

    pub(crate) fn switch_window_to_team(
        window_id: WindowId,
        team_uid: ServerId,
        ctx: &mut AppContext,
    ) {
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.switch_window_to_team(window_id, team_uid, ctx);
        });
        Self::store_last_team_uid(team_uid, ctx);
    }

    fn restore_last_team_uid(ctx: &AppContext) -> Option<ServerId> {
        ctx.private_user_preferences()
            .read_value(LAST_TEAM_STORAGE_KEY)
            .ok()
            .flatten()
            .and_then(|stored| serde_json::from_str::<ServerId>(&stored).ok())
    }

    fn store_last_team_uid(team_uid: ServerId, ctx: &AppContext) {
        let Ok(serialized) = serde_json::to_string(&team_uid) else {
            return;
        };
        let _ = ctx
            .private_user_preferences()
            .write_value(LAST_TEAM_STORAGE_KEY, serialized);
    }

    /// Transitions from the authentication gate to the live session container.
    pub(crate) fn show_terminal(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset_login_copy_state();
        self.state = RootTuiState::Terminal;
        ctx.notify();
    }

    /// Returns to the authentication gate after the current user logs out.
    pub(crate) fn show_auth(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset_login_copy_state();
        self.state = RootTuiState::Auth;
        ctx.focus_self();
        ctx.notify();
    }

    fn focused_session_view(&self, ctx: &AppContext) -> Option<TuiSessionView> {
        if !ctx.has_singleton_model::<TuiSessions>() {
            return None;
        }

        TuiSessions::as_ref(ctx)
            .focused_session()
            .map(|session| session.view().clone())
    }

    fn reset_login_copy_state(&mut self) {
        self.copy_login_url_when_available = false;
        self.login_copy_hint.clear();
    }

    pub(crate) fn handle_login_phase_changed(
        &mut self,
        ctx: &mut ViewContext<Self>,
        copy: impl FnOnce(&str) -> Result<()>,
    ) {
        if !self.copy_login_url_when_available {
            ctx.notify();
            return;
        }

        let browser_url = match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::AwaitingLogin {
                browser_url: Some(browser_url),
            }
            | TuiLoginPhase::BrowserOpenFailed { browser_url } => Some(browser_url.clone()),
            TuiLoginPhase::AwaitingLogin { browser_url: None } => None,
            TuiLoginPhase::SignedOutWelcome
            | TuiLoginPhase::Failed { .. }
            | TuiLoginPhase::LoggedIn => {
                self.copy_login_url_when_available = false;
                None
            }
        };
        if let Some(browser_url) = browser_url {
            self.copy_login_url_when_available = false;
            self.copy_login_url_with(&browser_url, ctx, copy);
        }
        ctx.notify();
    }

    fn copy_login_url_with(
        &mut self,
        url: &str,
        ctx: &mut ViewContext<Self>,
        copy: impl FnOnce(&str) -> Result<()>,
    ) {
        let is_current_url = matches!(
            TuiLoginModel::as_ref(ctx).phase(),
            TuiLoginPhase::AwaitingLogin {
                browser_url: Some(current_url),
            } if current_url == url
        ) || matches!(
            TuiLoginModel::as_ref(ctx).phase(),
            TuiLoginPhase::BrowserOpenFailed {
                browser_url: current_url,
            } if current_url == url
        );
        if !is_current_url {
            return;
        }

        match copy(url) {
            Ok(()) => {
                self.login_copy_hint.show_success(
                    "Login URL copied to clipboard".to_owned(),
                    ctx,
                    |view| &mut view.login_copy_hint,
                );
                TuiLoginModel::record_login_url_copied(true, ctx);
            }
            Err(error) => {
                log::warn!("Failed to copy TUI login URL: {error}");
                self.login_copy_hint.show_error(
                    "Unable to copy login URL".to_owned(),
                    ctx,
                    |view| &mut view.login_copy_hint,
                );
                TuiLoginModel::record_login_url_copied(false, ctx);
            }
        }
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn child_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        match self.state {
            RootTuiState::Auth => Vec::new(),
            RootTuiState::Terminal => self
                .focused_session_view(ctx)
                .map(|view| vec![view.id()])
                .unwrap_or_default(),
        }
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused()
            && matches!(self.state, RootTuiState::Terminal)
            && let Some(view) = self.focused_session_view(ctx)
        {
            view.activate(ctx);
        }
    }
    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match self.state {
            RootTuiState::Auth => match TuiLoginModel::as_ref(ctx).phase() {
                TuiLoginPhase::SignedOutWelcome => signed_out_welcome(
                    self.auth_animation_clock,
                    self.auth_animation_config.clone(),
                    self.welcome_login_mouse.clone(),
                    self.welcome_copy_mouse.clone(),
                    ctx,
                    |event_ctx, _| {
                        event_ctx.dispatch_typed_action(RootTuiAction::StartDeviceLogin);
                    },
                    |event_ctx, _| {
                        event_ctx.dispatch_typed_action(RootTuiAction::StartDeviceLoginAndCopyUrl);
                    },
                ),
                TuiLoginPhase::LoggedIn => terminal_starting(),
                TuiLoginPhase::AwaitingLogin { browser_url } => login_waiting(
                    self.auth_animation_clock,
                    self.auth_animation_config.clone(),
                    LoginWaitingParams {
                        browser_url: browser_url.as_deref(),
                        login_mouse: self.waiting_login_mouse.clone(),
                        copy_mouse: self.waiting_login_copy_mouse.clone(),
                        copy_feedback: self.login_copy_hint.current(),
                    },
                    ctx,
                    {
                        let browser_url = browser_url.clone();
                        move |event_ctx, _| {
                            if let Some(browser_url) = browser_url.clone() {
                                event_ctx.dispatch_typed_action(RootTuiAction::OpenLoginUrl(
                                    browser_url,
                                ));
                            }
                        }
                    },
                    {
                        let browser_url = browser_url.clone();
                        move |event_ctx, _| {
                            if let Some(browser_url) = browser_url.clone() {
                                event_ctx.dispatch_typed_action(RootTuiAction::CopyLoginUrl(
                                    browser_url,
                                ));
                            }
                        }
                    },
                ),
                TuiLoginPhase::BrowserOpenFailed { browser_url } => login_browser_open_failed(
                    self.auth_animation_clock,
                    self.auth_animation_config.clone(),
                    LoginBrowserOpenFailedParams {
                        browser_url,
                        login_mouse: self.waiting_login_mouse.clone(),
                        copy_mouse: self.waiting_login_copy_mouse.clone(),
                        retry_mouse: self.waiting_login_retry_mouse.clone(),
                        copy_feedback: self.login_copy_hint.current(),
                    },
                    ctx,
                    {
                        let browser_url = browser_url.clone();
                        move |event_ctx, _| {
                            event_ctx.dispatch_typed_action(RootTuiAction::OpenLoginUrl(
                                browser_url.clone(),
                            ));
                        }
                    },
                    {
                        let browser_url = browser_url.clone();
                        move |event_ctx, _| {
                            event_ctx.dispatch_typed_action(RootTuiAction::CopyLoginUrl(
                                browser_url.clone(),
                            ));
                        }
                    },
                ),
                TuiLoginPhase::Failed { message } => login_failed(
                    self.auth_animation_clock,
                    self.auth_animation_config.clone(),
                    LoginFailedParams {
                        message,
                        retry_mouse: self.failed_login_retry_mouse.clone(),
                    },
                    ctx,
                    |event_ctx, _| {
                        event_ctx.dispatch_typed_action(RootTuiAction::StartDeviceLogin);
                    },
                ),
            },
            RootTuiState::Terminal => self
                .focused_session_view(ctx)
                .map(|view| match view {
                    TuiSessionView::Terminal(view) => TuiChildView::new(&view).finish(),
                    TuiSessionView::Cloud(view) => TuiChildView::new(&view).finish(),
                })
                .unwrap_or_else(terminal_starting),
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = RootTuiAction;

    fn handle_action(&mut self, action: &RootTuiAction, ctx: &mut ViewContext<Self>) {
        match action {
            RootTuiAction::ExitApp => {
                if matches!(self.state, RootTuiState::Auth) {
                    TuiLoginModel::record_authentication_abandoned(ctx);
                }
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            RootTuiAction::StartDeviceLogin => {
                if matches!(
                    TuiLoginModel::as_ref(ctx).phase(),
                    TuiLoginPhase::SignedOutWelcome | TuiLoginPhase::Failed { .. }
                ) {
                    self.reset_login_copy_state();
                    TuiLoginModel::start_device_login(ctx);
                }
            }
            RootTuiAction::StartDeviceLoginAndCopyUrl => {
                if matches!(
                    TuiLoginModel::as_ref(ctx).phase(),
                    TuiLoginPhase::SignedOutWelcome | TuiLoginPhase::Failed { .. }
                ) {
                    self.reset_login_copy_state();
                    self.copy_login_url_when_available = true;
                    TuiLoginModel::start_device_login_and_copy_url(ctx);
                }
            }
            RootTuiAction::OpenLoginUrl(url) => {
                TuiLoginModel::open_login_url(url, ctx);
            }
            RootTuiAction::CopyLoginUrl(url) => {
                self.copy_login_url_with(url, ctx, copy_to_clipboard);
            }
        }
    }
}

#[cfg(test)]
#[path = "root_view_tests.rs"]
mod tests;
