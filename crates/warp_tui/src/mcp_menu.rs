use warp::tui_export::{
    TuiMcpAction, TuiMcpConfigState, TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerStatus,
    TuiMcpTransport,
};
use warp_search_core::inline_menu::InlineMenuSelection;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity as _};

use crate::inline_menu::{
    keep_selected_visible, result_row_capacity, TuiInlineMenuHeader, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, MAX_INLINE_MENU_ROWS,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::ui::abbreviate_home_prefix;

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Clone, Debug)]
struct TuiMcpMenuRow {
    title: String,
    description: Option<String>,
    action: Option<TuiMcpAction>,
}

#[derive(Default)]
enum TuiMcpMenuState {
    #[default]
    Closed,
    Open {
        rows: Vec<TuiMcpMenuRow>,
        selection: InlineMenuSelection,
        scroll_offset: usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiMcpMenuEvent {
    Updated,
}

pub(crate) struct TuiMcpMenuModel {
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    state: TuiMcpMenuState,
}

impl TuiMcpMenuModel {
    pub(crate) fn new(
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &TuiMcpManager::handle(ctx),
            |model, _, _: &TuiMcpManagerEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            },
        );
        Self {
            suggestions_mode,
            state: TuiMcpMenuState::Closed,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiMcpMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::Mcp
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::Mcp, ctx)
        });
        if !did_open {
            return;
        }
        self.state = TuiMcpMenuState::Open {
            rows: Vec::new(),
            selection: InlineMenuSelection::default(),
            scroll_offset: 0,
        };
        self.refresh_rows(ctx);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            self.state = TuiMcpMenuState::Closed;
            self.suggestions_mode.update(ctx, |mode, ctx| {
                mode.close_if_active(TuiInputSuggestionsMode::Mcp, ctx);
            });
            ctx.emit(TuiMcpMenuEvent::Updated);
        }
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiMcpMenuState::Open {
            rows,
            selection,
            scroll_offset,
        } = &mut self.state
        else {
            return;
        };
        if let Some(index) =
            selection.select_previous(rows.len(), |index| rows[index].action.is_some())
        {
            keep_selected_visible(rows.len(), index, MAX_VISIBLE_ROWS, scroll_offset);
        }
        ctx.emit(TuiMcpMenuEvent::Updated);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiMcpMenuState::Open {
            rows,
            selection,
            scroll_offset,
        } = &mut self.state
        else {
            return;
        };
        if let Some(index) = selection.select_next(rows.len(), |index| rows[index].action.is_some())
        {
            keep_selected_visible(rows.len(), index, MAX_VISIBLE_ROWS, scroll_offset);
        }
        ctx.emit(TuiMcpMenuEvent::Updated);
    }

    pub(crate) fn accept_selected(
        &mut self,
        _ctx: &mut ModelContext<Self>,
    ) -> Option<TuiMcpAction> {
        let TuiMcpMenuState::Open {
            rows, selection, ..
        } = &self.state
        else {
            return None;
        };
        selection
            .selected_index()
            .and_then(|index| rows.get(index))
            .and_then(|row| row.action)
    }

    pub(crate) fn snapshot(&self, app: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(app) {
            return None;
        }
        let TuiMcpMenuState::Open {
            rows,
            selection,
            scroll_offset,
        } = &self.state
        else {
            return None;
        };
        let mcp = TuiMcpManager::as_ref(app);
        let snapshot = mcp.snapshot();
        let status = rows.is_empty().then(|| {
            let label = match &snapshot.config_state {
                TuiMcpConfigState::Missing => format!(
                    "No MCP config found at {}",
                    abbreviate_home_prefix(&snapshot.config_path.display().to_string())
                ),
                TuiMcpConfigState::Ready => "No MCP servers configured".to_string(),
                TuiMcpConfigState::Invalid { message } => format!("Config error: {message}"),
            };
            TuiInlineMenuStatus::Empty(label)
        });
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some(format!(
                    "MCP · {}",
                    abbreviate_home_prefix(&snapshot.config_path.display().to_string())
                )),
                tabs: Vec::new(),
            }),
            rows: rows
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.title.clone(),
                    description: row.description.clone(),
                    is_selectable: row.action.is_some(),
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: selection.selected_index(),
            scroll_offset: *scroll_offset,
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let snapshot = TuiMcpManager::as_ref(ctx).snapshot();
        let mut rows = Vec::new();
        if let TuiMcpConfigState::Invalid { message } = &snapshot.config_state {
            rows.push(TuiMcpMenuRow {
                title: "Config error".to_string(),
                description: Some(message.clone()),
                action: None,
            });
        }
        for server in &snapshot.servers {
            let transport = match server.transport {
                TuiMcpTransport::Stdio => "stdio",
                TuiMcpTransport::HttpOrSse => "HTTP/SSE",
            };
            let (status, action) = match &server.status {
                TuiMcpServerStatus::Offline => {
                    ("offline".to_string(), Some(TuiMcpAction::Start(server.id)))
                }
                TuiMcpServerStatus::Starting => ("starting…".to_string(), None),
                TuiMcpServerStatus::Authenticating => (
                    "authentication required".to_string(),
                    server
                        .authorization_url
                        .as_ref()
                        .map(|_| TuiMcpAction::ReopenAuthorization(server.id)),
                ),
                TuiMcpServerStatus::Running => (
                    format!("running · {} tools", server.tool_count),
                    Some(TuiMcpAction::Stop(server.id)),
                ),
                TuiMcpServerStatus::Stopping => ("stopping…".to_string(), None),
                TuiMcpServerStatus::Failed { message } => (
                    format!("failed · {message}"),
                    Some(TuiMcpAction::Retry(server.id)),
                ),
            };
            rows.push(TuiMcpMenuRow {
                title: server.name.clone(),
                description: Some(format!("{transport} · {status}")),
                action,
            });
            if server.has_credentials {
                rows.push(TuiMcpMenuRow {
                    title: format!("Log out {}", server.name),
                    description: Some("Remove saved OAuth credentials".to_string()),
                    action: Some(TuiMcpAction::LogOut(server.id)),
                });
            }
        }

        let mut selection = InlineMenuSelection::default();
        if let Some(index) = rows.iter().position(|row| row.action.is_some()) {
            selection.select(index, rows.len(), |candidate| {
                rows[candidate].action.is_some()
            });
        }
        self.state = TuiMcpMenuState::Open {
            rows,
            selection,
            scroll_offset: 0,
        };
        ctx.emit(TuiMcpMenuEvent::Updated);
    }
}

impl Entity for TuiMcpMenuModel {
    type Event = TuiMcpMenuEvent;
}
