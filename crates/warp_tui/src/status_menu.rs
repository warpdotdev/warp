//! Read-only TUI `/status` inline-menu state.
//!
//! Mirrors [`crate::mcp_menu::TuiMcpMenuModel`] but every row is a non-selectable
//! info line (no actions): the menu lists session/account status — Warp version,
//! session name, session id, working directory, organization, and email — with
//! graceful fallbacks when a field is unavailable (logged-out, dev build, no
//! session name yet). Account/org fields come from the shared
//! [`TuiUserInfoManager`] snapshot so the `/status` menu and the zero-state
//! login line share one source of truth.

use warp::tui_export::{
    ActiveSession, ActiveSessionEvent, ConversationSelectionHandle, TuiUserInfoManager,
};
use warp_core::channel::ChannelState;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity as _};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuRow, TuiInlineMenuRowStyle,
    TuiInlineMenuSnapshot, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::ui::abbreviate_home_prefix;

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

const UNAVAILABLE: &str = "—";
const UNTITLED_SESSION: &str = "Untitled";
const DEV_BUILD: &str = "dev build";
const NOT_SIGNED_IN: &str = "Not signed in";

#[derive(Clone, Debug)]
struct TuiStatusMenuRow {
    label: &'static str,
    value: String,
}

#[derive(Default)]
enum TuiStatusMenuState {
    #[default]
    Closed,
    Open {
        rows: Vec<TuiStatusMenuRow>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiStatusMenuEvent {
    Updated,
}

pub(crate) struct TuiStatusMenuModel {
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    active_session: ModelHandle<ActiveSession>,
    conversation_selection: ConversationSelectionHandle,
    state: TuiStatusMenuState,
}

impl TuiStatusMenuModel {
    pub(crate) fn new(
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        active_session: ModelHandle<ActiveSession>,
        conversation_selection: ConversationSelectionHandle,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&TuiUserInfoManager::handle(ctx), |model, _, _, ctx| {
            if model.is_open(ctx) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(&active_session, |model, _, event, ctx| {
            if model.is_open(ctx)
                && matches!(
                    event,
                    ActiveSessionEvent::UpdatedPwd | ActiveSessionEvent::Bootstrapped
                )
            {
                model.refresh_rows(ctx);
            }
        });
        Self {
            suggestions_mode,
            active_session,
            conversation_selection,
            state: TuiStatusMenuState::Closed,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiStatusMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::Status
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::Status, ctx)
        });
        if !did_open {
            return;
        }
        self.state = TuiStatusMenuState::Open { rows: Vec::new() };
        self.refresh_rows(ctx);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            self.state = TuiStatusMenuState::Closed;
            self.suggestions_mode.update(ctx, |mode, ctx| {
                mode.close_if_active(TuiInputSuggestionsMode::Status, ctx);
            });
            ctx.emit(TuiStatusMenuEvent::Updated);
        }
    }

    // Selection navigation is a no-op for a read-only info menu, but the inline
    // menu routing still forwards these calls; keep them harmless.
    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            ctx.emit(TuiStatusMenuEvent::Updated);
        }
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            ctx.emit(TuiStatusMenuEvent::Updated);
        }
    }

    pub(crate) fn snapshot(&self, app: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(app) {
            return None;
        }
        let TuiStatusMenuState::Open { rows } = &self.state else {
            return None;
        };
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Status".to_owned()),
                tabs: Vec::new(),
            }),
            rows: rows
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.label.to_owned(),
                    description: Some(row.value.clone()),
                    state_suffix: None,
                    is_selectable: false,
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: None,
            scroll_offset: 0,
            max_visible_rows: MAX_VISIBLE_ROWS,
            status: None,
        })
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let user_info = TuiUserInfoManager::as_ref(ctx).snapshot(ctx);
        let session = self.active_session.as_ref(ctx).session(ctx);
        let cwd = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .map(|cwd| abbreviate_home_prefix(cwd))
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| abbreviate_home_prefix(&cwd.display().to_string()))
            });
        let session_name = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)
            .and_then(|conversation| conversation.title())
            .unwrap_or_else(|| UNTITLED_SESSION.to_owned());
        let session_id = session
            .map(|session| session.id().as_u64().to_string())
            .unwrap_or_else(|| UNAVAILABLE.to_owned());
        let version = ChannelState::app_version()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| DEV_BUILD.to_owned());
        let org = user_info.org.unwrap_or_else(|| UNAVAILABLE.to_owned());
        let email = user_info.email.unwrap_or_else(|| NOT_SIGNED_IN.to_owned());

        let rows = vec![
            TuiStatusMenuRow {
                label: "Version",
                value: version,
            },
            TuiStatusMenuRow {
                label: "Session",
                value: session_name,
            },
            TuiStatusMenuRow {
                label: "Session ID",
                value: session_id,
            },
            TuiStatusMenuRow {
                label: "Working directory",
                value: cwd.unwrap_or_else(|| UNAVAILABLE.to_owned()),
            },
            TuiStatusMenuRow {
                label: "Org",
                value: org,
            },
            TuiStatusMenuRow {
                label: "Email",
                value: email,
            },
        ];
        self.state = TuiStatusMenuState::Open { rows };
        ctx.emit(TuiStatusMenuEvent::Updated);
    }
}

impl Entity for TuiStatusMenuModel {
    type Event = TuiStatusMenuEvent;
}
