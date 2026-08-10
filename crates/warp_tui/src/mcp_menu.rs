use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    TuiMcpAction, TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId, TuiMcpServerStatus,
    TuiMcpSnapshot, TuiMcpTransport,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity as _};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Clone, Debug)]
struct TuiMcpMenuRow {
    server_id: Option<TuiMcpServerId>,
    title: String,
    description: Option<String>,
    primary_action: Option<TuiMcpAction>,
    logout_action: Option<TuiMcpAction>,
}

#[derive(Default)]
enum TuiMcpMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiMcpMenuRow>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiMcpMenuEvent {
    Updated,
}

pub(crate) struct TuiMcpMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    state: TuiMcpMenuState,
}

impl TuiMcpMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(
            &TuiMcpManager::handle(ctx),
            |model, _, _: &TuiMcpManagerEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            },
        );
        Self {
            input_editor,
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
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.state = TuiMcpMenuState::Open {
            list: TuiInlineMenuListState::default(),
        };
        self.refresh_rows(ctx);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            self.state = TuiMcpMenuState::Closed;
            self.suggestions_mode.update(ctx, |mode, ctx| {
                mode.close_if_active(TuiInputSuggestionsMode::Mcp, ctx);
            });
            self.input_editor
                .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
            ctx.emit(TuiMcpMenuEvent::Updated);
        }
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiMcpMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, row_is_selectable);
        ctx.emit(TuiMcpMenuEvent::Updated);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiMcpMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, row_is_selectable);
        ctx.emit(TuiMcpMenuEvent::Updated);
    }

    /// Selects the row at absolute snapshot index `index` (for mouse click).
    /// Returns `true` when the row was actually selected, `false` when the
    /// index is out of bounds, the menu is not open, or the row is not selectable.
    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiMcpMenuState::Open { list } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(index, MAX_VISIBLE_ROWS, row_is_selectable);
        ctx.emit(TuiMcpMenuEvent::Updated);
        selected
    }

    /// Scrolls the viewport by `delta` rows without changing the selection.
    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiMcpMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiMcpMenuEvent::Updated);
    }
    pub(crate) fn selected_primary_action(&self, ctx: &AppContext) -> Option<TuiMcpAction> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiMcpMenuState::Open { list } = &self.state else {
            return None;
        };
        list.selected_row().and_then(|row| row.primary_action)
    }
    pub(crate) fn accept_selected(&self, ctx: &AppContext) -> Option<TuiMcpAction> {
        self.selected_primary_action(ctx)
    }

    pub(crate) fn logout_selected(&self, ctx: &AppContext) -> Option<TuiMcpAction> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiMcpMenuState::Open { list } = &self.state else {
            return None;
        };
        list.selected_row().and_then(|row| row.logout_action)
    }
    pub(crate) fn can_log_out_selected(&self, ctx: &AppContext) -> bool {
        self.logout_selected(ctx).is_some()
    }

    pub(crate) fn input_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        (self.is_open(ctx) && input_text(&self.input_editor, ctx).is_empty())
            .then_some("Search MCP servers…")
    }

    pub(crate) fn snapshot(&self, app: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(app) {
            return None;
        }
        let TuiMcpMenuState::Open { list } = &self.state else {
            return None;
        };
        let query = input_text(&self.input_editor, app);
        let status = list.rows().is_empty().then(|| {
            let label = if !query.trim().is_empty() {
                "No matching MCP servers".to_owned()
            } else {
                "No MCP servers available".to_owned()
            };
            TuiInlineMenuStatus::Empty(label)
        });
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("MCP servers".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.title.clone(),
                    prefix: None,
                    description: row.description.clone(),
                    state_suffix: None,
                    promotional_suffix: None,
                    is_selectable: row_is_selectable(row),
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            scroll_anchor: list.scroll_anchor(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let snapshot = TuiMcpManager::as_ref(ctx).snapshot();
        let previous_server_id = match &self.state {
            TuiMcpMenuState::Open { list } => list.selected_row().and_then(|row| row.server_id),
            TuiMcpMenuState::Closed => return,
        };
        let rows = menu_rows(snapshot, &input_text(&self.input_editor, ctx));
        let preferred_index = previous_server_id
            .and_then(|server_id| {
                rows.iter()
                    .position(|row| row.server_id == Some(server_id) && row_is_selectable(row))
            })
            .or_else(|| rows.iter().position(row_is_selectable));
        let TuiMcpMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.replace_rows(
            rows,
            false,
            preferred_index,
            MAX_VISIBLE_ROWS,
            row_is_selectable,
        );
        ctx.emit(TuiMcpMenuEvent::Updated);
    }
}
fn menu_rows(snapshot: &TuiMcpSnapshot, query: &str) -> Vec<TuiMcpMenuRow> {
    let mut rows = Vec::new();
    for diagnostic in &snapshot.diagnostics {
        rows.push(TuiMcpMenuRow {
            server_id: None,
            title: format!("{} config error", diagnostic.provider),
            description: Some(format!(
                "{} · {}",
                diagnostic.config_path.display(),
                diagnostic.message
            )),
            primary_action: None,
            logout_action: None,
        });
    }
    let query = query.trim().to_lowercase();
    rows.extend(
        snapshot
            .servers
            .iter()
            .filter(|server| {
                query.is_empty()
                    || server.name.to_lowercase().contains(&query)
                    || server
                        .description
                        .as_deref()
                        .is_some_and(|description| description.to_lowercase().contains(&query))
                    || server.source.label().to_lowercase().contains(&query)
            })
            .map(|server| {
                let transport = server.transport.map(|transport| match transport {
                    TuiMcpTransport::Stdio => "stdio",
                    TuiMcpTransport::HttpOrSse => "HTTP/SSE",
                });
                let (status, primary_action) = match &server.status {
                    TuiMcpServerStatus::Available => (
                        "available".to_owned(),
                        Some(TuiMcpAction::Enable(server.id)),
                    ),
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
                let mut description = vec![server.source.label()];
                if let Some(transport) = transport {
                    description.push(transport.to_owned());
                }
                description.push(status);
                TuiMcpMenuRow {
                    server_id: Some(server.id),
                    title: server.name.clone(),
                    description: Some(description.join(" · ")),
                    primary_action,
                    logout_action: server
                        .can_log_out
                        .then_some(TuiMcpAction::LogOut(server.id)),
                }
            }),
    );
    rows
}

fn row_is_selectable(row: &TuiMcpMenuRow) -> bool {
    row.server_id.is_some()
}

fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

impl Entity for TuiMcpMenuModel {
    type Event = TuiMcpMenuEvent;
}

#[cfg(test)]
#[path = "mcp_menu_tests.rs"]
mod tests;
