//! Stateful `/api-keys` inline menu backed by the shared TUI input editor.

use ai::LLMProvider;
use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent};
use ai::grok_subscription::oauth::OauthAttempt;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::{UserWorkspaces, UserWorkspacesEvent, notify_tui_api_keys_changed};
use warp_core::features::FeatureFlag;
use warp_core::settings::{Setting as _, ToggleableSetting as _};
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::grok_oauth::{TuiGrokOAuthController, TuiGrokOAuthControllerEvent};
use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuInputOwnership, TuiInlineMenuListState,
    TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuScrollAnchor, TuiInlineMenuSnapshot,
    TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{
    TuiInputSuggestionsMode, TuiInputSuggestionsModeEvent, TuiInputSuggestionsModeModel,
};
use crate::tui_builder::TuiUiBuilder;

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);
const FALLBACK_DESCRIPTION: &str = "in the event of an error, requests may be routed to use Warp \
credits. Warp will prioritize using your API keys over Warp credits.";
const CUSTOM_ENDPOINT_ANNOTATION: &str = "custom endpoint";
const CUSTOM_ENDPOINTS_HEADING: &str = "Custom endpoints";
const PARTIAL_SUCCESS_NOTIFY_FAILURE_MESSAGE: &str =
    "The API key changed, but other running Warp processes could not be notified.";

fn provider_rows() -> [TuiApiKeysRow; 4] {
    [
        TuiApiKeysRow {
            kind: TuiApiKeysRowKind::Provider(LLMProvider::Anthropic),
            title: "Anthropic API key".to_owned(),
        },
        TuiApiKeysRow {
            kind: TuiApiKeysRowKind::Provider(LLMProvider::Google),
            title: "Google API key".to_owned(),
        },
        TuiApiKeysRow {
            kind: TuiApiKeysRowKind::Provider(LLMProvider::OpenAI),
            title: "OpenAI API key".to_owned(),
        },
        TuiApiKeysRow {
            kind: TuiApiKeysRowKind::Provider(LLMProvider::Xai),
            title: "X premium or SuperGrok subscription".to_owned(),
        },
    ]
}

fn fallback_row() -> TuiApiKeysRow {
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::WarpCreditFallbackSetting,
        title: "Warp credit fallback".to_owned(),
    }
}

/// Non-selectable placeholder states shown instead of individual custom
/// endpoint rows (see `PRODUCT.md` Behaviors 17, 19, 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CustomEndpointStatusKind {
    /// The `BYO_ENDPOINT` entitlement is unavailable for this user.
    Unavailable,
    /// The active workspace disallows member-provided custom endpoints.
    DisabledByOrganization,
    /// Custom endpoints are allowed, but none are defined (valid or invalid).
    NoneConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TuiApiKeysRowKind {
    Provider(LLMProvider),
    CustomEndpoint(String),
    CustomEndpointStatus(CustomEndpointStatusKind),
    InvalidCustomEndpoint(String),
    WarpCreditFallbackSetting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiApiKeysRow {
    kind: TuiApiKeysRowKind,
    title: String,
}

/// Builds `/api-keys` rows in the order fixed by `TECH.md` #7: built-in
/// providers, then valid custom endpoints (or one status row), then invalid
/// endpoint rows, then the Warp credit fallback row.
fn build_rows(ctx: &AppContext) -> Vec<TuiApiKeysRow> {
    let mut rows: Vec<TuiApiKeysRow> = provider_rows().into_iter().collect();

    let workspaces = UserWorkspaces::as_ref(ctx);
    let entitled = workspaces.is_custom_inference_enabled(ctx);
    let policy_allowed = workspaces.are_member_byo_endpoints_allowed();
    let definitions = AISettings::as_ref(ctx).custom_endpoints.value();

    if !entitled {
        rows.push(status_row(CustomEndpointStatusKind::Unavailable));
    } else if !policy_allowed {
        rows.push(status_row(CustomEndpointStatusKind::DisabledByOrganization));
    } else if definitions.valid_len() == 0 && !definitions.has_diagnostics() {
        rows.push(status_row(CustomEndpointStatusKind::NoneConfigured));
    } else {
        let mut names: Vec<&str> = definitions.valid().map(|(name, _)| name).collect();
        names.sort_unstable();
        rows.extend(names.into_iter().map(custom_endpoint_row));
    }

    // Invalid rows remain visible after the policy/entitlement row regardless
    // of the gates above, so the user can still repair `settings.toml`.
    let mut invalid_names: Vec<&str> = definitions.invalid().map(|(name, _)| name).collect();
    invalid_names.sort_unstable();
    rows.extend(invalid_names.into_iter().map(invalid_custom_endpoint_row));

    rows.push(fallback_row());
    rows
}

fn status_row(kind: CustomEndpointStatusKind) -> TuiApiKeysRow {
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::CustomEndpointStatus(kind),
        title: CUSTOM_ENDPOINTS_HEADING.to_owned(),
    }
}

fn custom_endpoint_row(name: &str) -> TuiApiKeysRow {
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::CustomEndpoint(name.to_owned()),
        title: name.to_owned(),
    }
}

fn invalid_custom_endpoint_row(name: &str) -> TuiApiKeysRow {
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::InvalidCustomEndpoint(name.to_owned()),
        title: format!("Invalid custom endpoint: {name}"),
    }
}

/// Whether `row` matches a lowercased search query. Custom endpoint rows also
/// match on the "custom endpoint" annotation (`PRODUCT.md` Behavior 13).
fn row_matches_query(row: &TuiApiKeysRow, query: &str) -> bool {
    if row.title.to_ascii_lowercase().contains(query) {
        return true;
    }
    matches!(row.kind, TuiApiKeysRowKind::CustomEndpoint(_))
        && CUSTOM_ENDPOINT_ANNOTATION.contains(query)
}

#[derive(Default)]
enum TuiApiKeysMenuState {
    #[default]
    Closed,
    Browsing {
        list: TuiInlineMenuListState<TuiApiKeysRow>,
        error: Option<String>,
    },
    EditingProvider {
        provider: LLMProvider,
        error: Option<String>,
    },
    EditingCustomEndpoint {
        name: String,
        error: Option<String>,
    },
    ConnectingGrok {
        controller: ModelHandle<TuiGrokOAuthController>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiApiKeysFooter {
    ProviderList { can_clear: bool },
    WarpCreditFallback,
    EditingProvider(LLMProvider),
    EditingCustomEndpoint(String),
    ConnectingGrok,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiApiKeysMenuEvent;

pub(crate) struct TuiApiKeysMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    state: TuiApiKeysMenuState,
}

impl TuiApiKeysMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                return;
            }
            match &mut model.state {
                TuiApiKeysMenuState::Browsing { error, .. } => {
                    error.take();
                    model.refresh_rows(ctx);
                }
                TuiApiKeysMenuState::EditingProvider { error, .. }
                | TuiApiKeysMenuState::EditingCustomEndpoint { error, .. } => {
                    if error.take().is_some() {
                        ctx.emit(TuiApiKeysMenuEvent);
                    }
                }
                TuiApiKeysMenuState::ConnectingGrok { controller } => {
                    controller.update(ctx, |controller, ctx| {
                        controller.clear_manual_error(ctx);
                    });
                }
                TuiApiKeysMenuState::Closed => {}
            }
        });
        ctx.subscribe_to_model(
            &ApiKeyManager::handle(ctx),
            |model, _, _: &ApiKeyManagerEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            },
        );
        ctx.subscribe_to_model(&AISettings::handle(ctx), |model, _, event, ctx| {
            if model.is_open(ctx)
                && matches!(
                    event,
                    AISettingsChangedEvent::CanUseWarpCreditsForFallback { .. }
                        | AISettingsChangedEvent::CustomEndpointDefinitions { .. }
                )
            {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, UserWorkspacesEvent::TeamsChanged) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(
            &suggestions_mode,
            |model, _, event: &TuiInputSuggestionsModeEvent, ctx| {
                if event.mode != TuiInputSuggestionsMode::ApiKeys {
                    model.deactivate(ctx);
                }
            },
        );
        Self {
            input_editor,
            suggestions_mode,
            state: TuiApiKeysMenuState::Closed,
        }
    }

    fn start_grok_oauth(&mut self, ctx: &mut ModelContext<Self>) {
        let workspaces = UserWorkspaces::as_ref(ctx);
        let policy_error = if !FeatureFlag::SuperGrok.is_enabled() {
            Some("Grok subscriptions aren't available in this build.")
        } else if !workspaces.is_byo_api_key_enabled(ctx) {
            Some("Grok subscriptions require BYOK access for this workspace.")
        } else if !workspaces.are_member_byo_keys_allowed() {
            Some("Your organization doesn't allow member-provided credentials.")
        } else {
            None
        };
        if let Some(error) = policy_error {
            self.set_browsing_error(error.to_owned(), ctx);
            return;
        }
        let attempt = match OauthAttempt::start() {
            Ok(attempt) => attempt,
            Err(error) => {
                self.set_browsing_error(error.to_string(), ctx);
                return;
            }
        };
        self.clear_input(ctx);
        let controller = ctx.add_model(move |ctx| TuiGrokOAuthController::new(attempt, ctx));
        ctx.subscribe_to_model(&controller, |menu, _, event, ctx| match event {
            TuiGrokOAuthControllerEvent::Connected => menu.transition_to_browsing(ctx),
            TuiGrokOAuthControllerEvent::Updated => ctx.emit(TuiApiKeysMenuEvent),
        });
        self.state = TuiApiKeysMenuState::ConnectingGrok { controller };
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        !matches!(self.state, TuiApiKeysMenuState::Closed)
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::ApiKeys
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::ApiKeys, ctx)
        });
        if !did_open {
            return;
        }
        self.transition_to_browsing(ctx);
    }

    /// Opens the menu and jumps straight into the Grok connect path, equivalent to selecting the
    /// "X premium or SuperGrok subscription" row. Reuses `edit_provider` so the already-connected
    /// and policy-gated cases surface the same messaging as the provider list.
    pub(crate) fn open_and_connect_grok(&mut self, ctx: &mut ModelContext<Self>) {
        self.open(ctx);
        if self.is_open(ctx) {
            self.edit_provider(LLMProvider::Xai, ctx);
        }
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        match self.state {
            TuiApiKeysMenuState::Closed => {}
            TuiApiKeysMenuState::Browsing { .. } => self.close(ctx),
            TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::EditingCustomEndpoint { .. }
            | TuiApiKeysMenuState::ConnectingGrok { .. } => self.transition_to_browsing(ctx),
        }
    }

    /// Returns the shared editor owner for the active API keys state.
    pub(crate) fn input_ownership(&self, ctx: &AppContext) -> TuiInlineMenuInputOwnership {
        if !self.is_open(ctx) {
            return TuiInlineMenuInputOwnership::Composer;
        }
        match self.state {
            TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::EditingCustomEndpoint { .. } => {
                TuiInlineMenuInputOwnership::InlineMenuMasked
            }
            TuiApiKeysMenuState::Browsing { .. } | TuiApiKeysMenuState::ConnectingGrok { .. } => {
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            }
            TuiApiKeysMenuState::Closed => TuiInlineMenuInputOwnership::Composer,
        }
    }

    pub(crate) fn uses_credential_border(&self, ctx: &AppContext) -> bool {
        self.is_open(ctx)
            && matches!(
                self.state,
                TuiApiKeysMenuState::EditingProvider { .. }
                    | TuiApiKeysMenuState::EditingCustomEndpoint { .. }
                    | TuiApiKeysMenuState::ConnectingGrok { .. }
            )
    }

    pub(crate) fn footer(&self, ctx: &AppContext) -> Option<TuiApiKeysFooter> {
        if !self.is_open(ctx) {
            return None;
        }
        match &self.state {
            TuiApiKeysMenuState::Closed => None,
            TuiApiKeysMenuState::Browsing { list, .. } => {
                Some(match list.selected_row().map(|row| &row.kind) {
                    Some(TuiApiKeysRowKind::WarpCreditFallbackSetting) => {
                        TuiApiKeysFooter::WarpCreditFallback
                    }
                    Some(TuiApiKeysRowKind::Provider(provider)) => TuiApiKeysFooter::ProviderList {
                        can_clear: provider_connected(*provider, ctx),
                    },
                    Some(TuiApiKeysRowKind::CustomEndpoint(name)) => {
                        TuiApiKeysFooter::ProviderList {
                            can_clear: ApiKeyManager::as_ref(ctx)
                                .custom_endpoint_key_is_connected(name),
                        }
                    }
                    Some(
                        TuiApiKeysRowKind::InvalidCustomEndpoint(_)
                        | TuiApiKeysRowKind::CustomEndpointStatus(_),
                    )
                    | None => TuiApiKeysFooter::ProviderList { can_clear: false },
                })
            }
            TuiApiKeysMenuState::EditingProvider { provider, .. } => {
                Some(TuiApiKeysFooter::EditingProvider(*provider))
            }
            TuiApiKeysMenuState::EditingCustomEndpoint { name, .. } => {
                Some(TuiApiKeysFooter::EditingCustomEndpoint(name.clone()))
            }
            TuiApiKeysMenuState::ConnectingGrok { .. } => Some(TuiApiKeysFooter::ConnectingGrok),
        }
    }

    pub(crate) fn can_clear_selected(&self, ctx: &AppContext) -> bool {
        match &self.state {
            TuiApiKeysMenuState::Browsing { list, .. } => {
                match list.selected_row().map(|row| &row.kind) {
                    Some(TuiApiKeysRowKind::Provider(provider)) => {
                        provider_connected(*provider, ctx)
                    }
                    Some(TuiApiKeysRowKind::CustomEndpoint(name)) => {
                        ApiKeyManager::as_ref(ctx).custom_endpoint_key_is_connected(name)
                    }
                    Some(
                        TuiApiKeysRowKind::WarpCreditFallbackSetting
                        | TuiApiKeysRowKind::InvalidCustomEndpoint(_)
                        | TuiApiKeysRowKind::CustomEndpointStatus(_),
                    )
                    | None => false,
                }
            }
            TuiApiKeysMenuState::Closed
            | TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::EditingCustomEndpoint { .. }
            | TuiApiKeysMenuState::ConnectingGrok { .. } => false,
        }
    }

    pub(crate) fn clear_selected(&mut self, ctx: &mut ModelContext<Self>) {
        enum ClearTarget {
            Provider(LLMProvider),
            CustomEndpoint(String),
        }
        let target = match &self.state {
            TuiApiKeysMenuState::Browsing { list, .. } => {
                match list.selected_row().map(|row| row.kind.clone()) {
                    Some(TuiApiKeysRowKind::Provider(provider)) => ClearTarget::Provider(provider),
                    Some(TuiApiKeysRowKind::CustomEndpoint(name)) => {
                        ClearTarget::CustomEndpoint(name)
                    }
                    Some(
                        TuiApiKeysRowKind::WarpCreditFallbackSetting
                        | TuiApiKeysRowKind::InvalidCustomEndpoint(_)
                        | TuiApiKeysRowKind::CustomEndpointStatus(_),
                    )
                    | None => return,
                }
            }
            TuiApiKeysMenuState::Closed
            | TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::EditingCustomEndpoint { .. }
            | TuiApiKeysMenuState::ConnectingGrok { .. } => return,
        };
        if let ClearTarget::CustomEndpoint(name) = target {
            match ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.persist_custom_endpoint_key(&name, None, ctx)
            }) {
                Ok(()) => {
                    self.refresh_rows(ctx);
                    if notify_tui_api_keys_changed().is_err() {
                        self.set_browsing_error(
                            PARTIAL_SUCCESS_NOTIFY_FAILURE_MESSAGE.to_owned(),
                            ctx,
                        );
                    }
                }
                Err(_) => self.set_browsing_error(
                    "Could not clear the selected API key. Try again.".to_owned(),
                    ctx,
                ),
            }
            return;
        }
        let ClearTarget::Provider(provider) = target else {
            unreachable!("custom endpoint target handled above")
        };
        let result = if provider == LLMProvider::Xai {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.set_grok_tokens(None, ctx);
            });
            Ok(())
        } else {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.persist_provider_key(provider, None, ctx)
            })
        };
        match result {
            Ok(()) => self.refresh_rows(ctx),
            Err(_) => self.set_browsing_error(
                "Could not clear the selected API key. Try again.".to_owned(),
                ctx,
            ),
        }
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, row_is_selectable);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, row_is_selectable);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(index, MAX_VISIBLE_ROWS, row_is_selectable);
        ctx.emit(TuiApiKeysMenuEvent);
        selected
    }

    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return;
        };
        list.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn accept_selected(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.state {
            TuiApiKeysMenuState::Closed => {}
            TuiApiKeysMenuState::Browsing { list, .. } => {
                let Some(kind) = list.selected_row().map(|row| row.kind.clone()) else {
                    return;
                };
                match kind {
                    TuiApiKeysRowKind::Provider(provider) => self.edit_provider(provider, ctx),
                    TuiApiKeysRowKind::CustomEndpoint(name) => self.edit_custom_endpoint(name, ctx),
                    TuiApiKeysRowKind::WarpCreditFallbackSetting => self.toggle_fallback(ctx),
                    TuiApiKeysRowKind::InvalidCustomEndpoint(_)
                    | TuiApiKeysRowKind::CustomEndpointStatus(_) => {}
                }
            }
            TuiApiKeysMenuState::EditingProvider { provider, .. } => {
                let provider = *provider;
                self.save_provider(provider, ctx);
            }
            TuiApiKeysMenuState::EditingCustomEndpoint { name, .. } => {
                let name = name.clone();
                self.save_custom_endpoint_key(name, ctx);
            }
            TuiApiKeysMenuState::ConnectingGrok { controller } => {
                let controller = controller.clone();
                let code = input_text(&self.input_editor, ctx);
                if !code.trim().is_empty() {
                    self.clear_input(ctx);
                }
                controller.update(ctx, |controller, ctx| {
                    controller.submit_manual_code(code, ctx);
                });
            }
        }
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        match &self.state {
            TuiApiKeysMenuState::Closed => None,
            TuiApiKeysMenuState::Browsing { list, error } => Some(TuiInlineMenuSnapshot {
                header: Some(TuiInlineMenuHeader {
                    title: Some(error.clone().unwrap_or_else(|| "API keys".to_owned())),
                    tabs: Vec::new(),
                }),
                rows: list
                    .rows()
                    .iter()
                    .map(|row| self.snapshot_row(row, ctx, false))
                    .collect(),
                selected_index: list.selected_index(),
                scroll_offset: list.scroll_offset(),
                scroll_anchor: list.scroll_anchor(),
                max_visible_rows: MAX_VISIBLE_ROWS,
                status: None,
            }),
            TuiApiKeysMenuState::EditingProvider { provider, error } => {
                Some(TuiInlineMenuSnapshot {
                    header: Some(TuiInlineMenuHeader {
                        title: Some(format!("{} API key", provider.display_name())),
                        tabs: Vec::new(),
                    }),
                    rows: Vec::new(),
                    selected_index: None,
                    scroll_offset: 0,
                    scroll_anchor: TuiInlineMenuScrollAnchor::Selection,
                    max_visible_rows: MAX_VISIBLE_ROWS,
                    status: error.clone().map(TuiInlineMenuStatus::Empty),
                })
            }
            TuiApiKeysMenuState::EditingCustomEndpoint { name, error } => {
                Some(TuiInlineMenuSnapshot {
                    header: Some(TuiInlineMenuHeader {
                        title: Some(format!("{name} API key")),
                        tabs: Vec::new(),
                    }),
                    rows: Vec::new(),
                    selected_index: None,
                    scroll_offset: 0,
                    scroll_anchor: TuiInlineMenuScrollAnchor::Selection,
                    max_visible_rows: MAX_VISIBLE_ROWS,
                    status: error.clone().map(TuiInlineMenuStatus::Empty),
                })
            }
            TuiApiKeysMenuState::ConnectingGrok { controller } => {
                let error = controller.as_ref(ctx).error().map(ToOwned::to_owned);
                let rows = provider_rows()
                    .into_iter()
                    .chain(std::iter::once(fallback_row()))
                    .map(|row| self.snapshot_row(&row, ctx, true))
                    .collect();
                Some(TuiInlineMenuSnapshot {
                    header: Some(TuiInlineMenuHeader {
                        title: Some(error.unwrap_or_else(|| "API keys".to_owned())),
                        tabs: Vec::new(),
                    }),
                    rows,
                    selected_index: Some(3),
                    scroll_offset: 0,
                    scroll_anchor: TuiInlineMenuScrollAnchor::Selection,
                    max_visible_rows: MAX_VISIBLE_ROWS,
                    status: None,
                })
            }
        }
    }

    fn snapshot_row(
        &self,
        row: &TuiApiKeysRow,
        ctx: &AppContext,
        connecting_grok: bool,
    ) -> TuiInlineMenuRow {
        match &row.kind {
            TuiApiKeysRowKind::Provider(provider) => {
                let connected = provider_connected(*provider, ctx);
                let suffix = if connecting_grok && *provider == LLMProvider::Xai {
                    "(Connecting...)"
                } else if connected {
                    "(Connected)"
                } else {
                    "(Not connected)"
                };
                TuiInlineMenuRow {
                    title: row.title.clone(),
                    prefix: None,
                    description: Some(String::new()),
                    state_suffix: Some(suffix.to_owned()),
                    promotional_suffix: None,
                    is_selectable: !connecting_grok,
                    style: TuiInlineMenuRowStyle::InlineMenuItem,
                }
            }
            TuiApiKeysRowKind::WarpCreditFallbackSetting => TuiInlineMenuRow {
                title: row.title.clone(),
                prefix: None,
                description: Some(FALLBACK_DESCRIPTION.to_owned()),
                state_suffix: Some(format!(
                    "({})",
                    if *AISettings::as_ref(ctx).can_use_warp_credits_for_fallback {
                        "on"
                    } else {
                        "off"
                    }
                )),
                promotional_suffix: None,
                is_selectable: !connecting_grok,
                style: TuiInlineMenuRowStyle::StateWithDetail,
            },
            TuiApiKeysRowKind::CustomEndpoint(name) => {
                let connected = ApiKeyManager::as_ref(ctx).custom_endpoint_key_is_connected(name);
                TuiInlineMenuRow {
                    title: row.title.clone(),
                    prefix: None,
                    description: Some(CUSTOM_ENDPOINT_ANNOTATION.to_owned()),
                    // A custom model row cannot appear until its endpoint has a
                    // key, so `(key connected)` would be redundant here too —
                    // only the existing `(Connected)` provider-style suffix
                    // applies, and only once a key exists.
                    state_suffix: connected.then(|| "(Connected)".to_owned()),
                    promotional_suffix: None,
                    is_selectable: !connecting_grok,
                    style: TuiInlineMenuRowStyle::CustomEndpoint,
                }
            }
            TuiApiKeysRowKind::InvalidCustomEndpoint(_) => TuiInlineMenuRow {
                title: row.title.clone(),
                prefix: None,
                // A non-`None`, even-if-empty description is required so the
                // `InlineMenuItem` style's second column (which carries
                // `state_suffix`) actually renders; see the provider row above.
                description: Some(String::new()),
                state_suffix: Some("(Skipped)".to_owned()),
                promotional_suffix: None,
                is_selectable: false,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            },
            TuiApiKeysRowKind::CustomEndpointStatus(status) => {
                let (description, state_suffix) = match status {
                    CustomEndpointStatusKind::Unavailable => (
                        "Custom endpoints are not available for this workspace.".to_owned(),
                        "(Unavailable)",
                    ),
                    CustomEndpointStatusKind::DisabledByOrganization => (
                        "Your organization does not allow member custom endpoints.".to_owned(),
                        "(Disabled by organization)",
                    ),
                    CustomEndpointStatusKind::NoneConfigured => (
                        "Add one in settings.toml or use /modify-settings.".to_owned(),
                        "(None configured)",
                    ),
                };
                TuiInlineMenuRow {
                    title: row.title.clone(),
                    prefix: None,
                    description: Some(description),
                    state_suffix: Some(state_suffix.to_owned()),
                    promotional_suffix: None,
                    is_selectable: false,
                    style: TuiInlineMenuRowStyle::InlineMenuItem,
                }
            }
        }
    }

    fn edit_provider(&mut self, provider: LLMProvider, ctx: &mut ModelContext<Self>) {
        if provider == LLMProvider::Xai {
            if ApiKeyManager::as_ref(ctx).has_grok_subscription() {
                self.set_browsing_error(
                    "Grok is already connected. Press Ctrl-X to disconnect.".to_owned(),
                    ctx,
                );
            } else {
                self.start_grok_oauth(ctx);
            }
            return;
        }
        let key = provider
            .api_key(ApiKeyManager::as_ref(ctx).keys())
            .unwrap_or_default()
            .to_owned();
        self.set_input(&key, ctx);
        self.state = TuiApiKeysMenuState::EditingProvider {
            provider,
            error: None,
        };
        ctx.emit(TuiApiKeysMenuEvent);
    }

    fn save_provider(&mut self, provider: LLMProvider, ctx: &mut ModelContext<Self>) {
        let value = input_text(&self.input_editor, ctx);
        let value = (!value.is_empty()).then_some(value);
        match ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.persist_provider_key(provider, value, ctx)
        }) {
            Ok(()) => self.transition_to_browsing(ctx),
            Err(_) => {
                if let TuiApiKeysMenuState::EditingProvider { error, .. } = &mut self.state {
                    *error = Some("Could not save this API key. Try again.".to_owned());
                }
                ctx.emit(TuiApiKeysMenuEvent);
            }
        }
    }

    /// Opens the masked editor for a custom endpoint's key, prefilled with any
    /// existing value for replacement (`PRODUCT.md` Behavior 14).
    fn edit_custom_endpoint(&mut self, name: String, ctx: &mut ModelContext<Self>) {
        let key = ApiKeyManager::as_ref(ctx)
            .keys()
            .custom_endpoints
            .iter()
            .find(|endpoint| endpoint.name == name)
            .map(|endpoint| endpoint.api_key.clone())
            .unwrap_or_default();
        self.set_input(&key, ctx);
        self.state = TuiApiKeysMenuState::EditingCustomEndpoint { name, error: None };
        ctx.emit(TuiApiKeysMenuEvent);
    }

    /// Saves (or, on an empty input, clears) a custom endpoint's key. A
    /// successful save notifies other running TUI processes; if that
    /// notification fails, the save itself still stands and the current TUI
    /// shows the partial-success message (`PRODUCT.md` Behavior 14).
    fn save_custom_endpoint_key(&mut self, name: String, ctx: &mut ModelContext<Self>) {
        let value = input_text(&self.input_editor, ctx);
        let value = (!value.is_empty()).then_some(value);
        match ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.persist_custom_endpoint_key(&name, value, ctx)
        }) {
            Ok(()) => {
                self.transition_to_browsing(ctx);
                if notify_tui_api_keys_changed().is_err() {
                    self.set_browsing_error(PARTIAL_SUCCESS_NOTIFY_FAILURE_MESSAGE.to_owned(), ctx);
                }
            }
            Err(_) => {
                if let TuiApiKeysMenuState::EditingCustomEndpoint { error, .. } = &mut self.state {
                    *error = Some("Could not save this API key. Try again.".to_owned());
                }
                ctx.emit(TuiApiKeysMenuEvent);
            }
        }
    }

    fn toggle_fallback(&mut self, ctx: &mut ModelContext<Self>) {
        let result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .can_use_warp_credits_for_fallback
                .toggle_and_save_value(ctx)
        });
        match result {
            Ok(_) => self.refresh_rows(ctx),
            Err(_) => self.set_browsing_error(
                "Could not save the Warp credit fallback setting.".to_owned(),
                ctx,
            ),
        }
    }

    fn transition_to_browsing(&mut self, ctx: &mut ModelContext<Self>) {
        if let TuiApiKeysMenuState::ConnectingGrok { controller } = &self.state
            && controller.as_ref(ctx).is_active()
        {
            controller.update(ctx, |controller, ctx| controller.cancel(ctx));
        }
        self.clear_input(ctx);
        self.state = TuiApiKeysMenuState::Browsing {
            list: TuiInlineMenuListState::default(),
            error: None,
        };
        self.refresh_rows(ctx);
    }

    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        self.deactivate(ctx);
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::ApiKeys, ctx);
        });
    }

    /// Clears API-key-specific state when shared menu arbitration moves elsewhere.
    fn deactivate(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.state, TuiApiKeysMenuState::Closed) {
            return;
        }
        let grok_controller = match &self.state {
            TuiApiKeysMenuState::ConnectingGrok { controller } => Some(controller.clone()),
            TuiApiKeysMenuState::Closed
            | TuiApiKeysMenuState::Browsing { .. }
            | TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::EditingCustomEndpoint { .. } => None,
        };
        self.state = TuiApiKeysMenuState::Closed;
        if let Some(controller) = grok_controller
            && controller.as_ref(ctx).is_active()
        {
            controller.update(ctx, |controller, ctx| controller.cancel(ctx));
        }
        self.clear_input(ctx);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            ctx.emit(TuiApiKeysMenuEvent);
            return;
        };
        let query = input_text(&self.input_editor, ctx).to_ascii_lowercase();
        // The Warp credit fallback row is always the last row `build_rows`
        // produces and stays pinned regardless of the search query.
        let mut all_rows = build_rows(ctx);
        let fallback = all_rows.pop();
        let mut rows: Vec<TuiApiKeysRow> = all_rows
            .into_iter()
            .filter(|row| row_matches_query(row, &query))
            .collect();
        rows.extend(fallback);
        let previous_kind = list.selected_row().map(|row| row.kind.clone());
        let preferred_index = previous_kind
            .and_then(|kind| rows.iter().position(|row| row.kind == kind))
            .or(Some(0));
        list.replace_rows(
            rows,
            false,
            preferred_index,
            MAX_VISIBLE_ROWS,
            row_is_selectable,
        );
        ctx.emit(TuiApiKeysMenuEvent);
    }

    fn set_browsing_error(&mut self, message: String, ctx: &mut ModelContext<Self>) {
        if let TuiApiKeysMenuState::Browsing { error, .. } = &mut self.state {
            *error = Some(message);
            ctx.emit(TuiApiKeysMenuEvent);
        }
    }

    fn clear_input(&self, ctx: &mut ModelContext<Self>) {
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
    }

    fn set_input(&self, text: &str, ctx: &mut ModelContext<Self>) {
        self.input_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
            editor.user_insert(text, ctx);
        });
    }
}

/// Whether a row can be keyboard/mouse-selected. Invalid-definition and
/// custom-endpoint policy/status rows are informational only.
fn row_is_selectable(row: &TuiApiKeysRow) -> bool {
    !matches!(
        row.kind,
        TuiApiKeysRowKind::InvalidCustomEndpoint(_) | TuiApiKeysRowKind::CustomEndpointStatus(_)
    )
}

/// Returns whether the provider has a configured API key or connected subscription.
fn provider_connected(provider: LLMProvider, ctx: &AppContext) -> bool {
    if provider == LLMProvider::Xai {
        ApiKeyManager::as_ref(ctx).has_grok_subscription()
    } else {
        provider
            .api_key(ApiKeyManager::as_ref(ctx).keys())
            .is_some_and(|key| !key.is_empty())
    }
}

/// Returns the shared input editor's current text.
fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

/// Renders the footer actions for the current API keys menu state.
pub(crate) fn render_api_keys_footer(
    footer: TuiApiKeysFooter,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let key = builder.link_text_style();
    let muted = builder.muted_text_style();
    let accent = builder.credential_entry_accent_style();
    let spans = match footer {
        TuiApiKeysFooter::ProviderList { can_clear } => {
            let mut spans = vec![
                ("enter".to_owned(), key),
                (" to set api key | ".to_owned(), muted),
            ];
            if can_clear {
                spans.extend([
                    ("ctrl + x".to_owned(), key),
                    (" to clear api key | ".to_owned(), muted),
                ]);
            }
            spans.extend([
                ("esc".to_owned(), key),
                (" to close menu".to_owned(), muted),
            ]);
            spans
        }
        TuiApiKeysFooter::WarpCreditFallback => vec![
            ("enter".to_owned(), key),
            (" to toggle warp credit fallback | ".to_owned(), muted),
            ("esc".to_owned(), key),
            (" to close menu".to_owned(), muted),
        ],
        TuiApiKeysFooter::EditingProvider(provider) => vec![
            (
                format!("Connect {} API key", provider.display_name()),
                accent,
            ),
            (" | ".to_owned(), muted),
            ("enter".to_owned(), key),
            (" to save key | ".to_owned(), muted),
            ("esc".to_owned(), key),
            (" to cancel".to_owned(), muted),
        ],
        TuiApiKeysFooter::EditingCustomEndpoint(name) => vec![
            (format!("Connect {name} API key"), accent),
            (" | ".to_owned(), muted),
            ("enter".to_owned(), key),
            (" to save key | ".to_owned(), muted),
            ("esc".to_owned(), key),
            (" to cancel".to_owned(), muted),
        ],
        TuiApiKeysFooter::ConnectingGrok => vec![
            ("Connect X premium/SuperGrok".to_owned(), accent),
            (" | ".to_owned(), muted),
            ("enter".to_owned(), key),
            (" to confirm | ".to_owned(), muted),
            ("esc".to_owned(), key),
            (" to cancel".to_owned(), muted),
        ],
    };
    TuiText::from_spans(spans).truncate().finish()
}
impl Entity for TuiApiKeysMenuModel {
    type Event = TuiApiKeysMenuEvent;
}

#[cfg(test)]
#[path = "api_keys_menu_tests.rs"]
mod tests;
