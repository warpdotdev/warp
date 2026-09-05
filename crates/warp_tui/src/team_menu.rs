//! Searchable TUI team switcher backing `/team`.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{ServerId, UserWorkspaces, UserWorkspacesEvent};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, WindowId};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone)]
struct TuiTeamMenuRow {
    uid: ServerId,
    title: String,
    is_active: bool,
}

#[derive(Debug, Clone, Default)]
enum TuiTeamMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiTeamMenuRow>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiTeamMenuEvent;

pub(crate) struct TuiTeamMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    window_id: WindowId,
    state: TuiTeamMenuState,
}

impl TuiTeamMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |model, _, event, ctx| {
            if model.is_open(ctx)
                && matches!(
                    event,
                    UserWorkspacesEvent::TeamsChanged
                        | UserWorkspacesEvent::WindowTeamChanged { .. }
                )
            {
                model.refresh_rows(ctx);
            }
        });
        Self {
            input_editor,
            suggestions_mode,
            window_id,
            state: TuiTeamMenuState::Closed,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiTeamMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::TeamSelector
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::TeamSelector, ctx)
        });
        if !did_open {
            return;
        }
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.state = TuiTeamMenuState::Open {
            list: TuiInlineMenuListState::default(),
        };
        self.refresh_rows(ctx);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        self.state = TuiTeamMenuState::Closed;
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::TeamSelector, ctx);
        });
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.emit(TuiTeamMenuEvent);
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiTeamMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |row| row.is_selectable());
        ctx.emit(TuiTeamMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiTeamMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |row| row.is_selectable());
        ctx.emit(TuiTeamMenuEvent);
    }

    /// Selects the row at absolute snapshot index `index` (for mouse click).
    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiTeamMenuState::Open { list } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(index, MAX_VISIBLE_ROWS, |row| row.is_selectable());
        ctx.emit(TuiTeamMenuEvent);
        selected
    }

    /// Scrolls the viewport by `delta` rows without changing the selection.
    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiTeamMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiTeamMenuEvent);
    }

    pub(crate) fn accept_selected(&self, ctx: &AppContext) -> Option<ServerId> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiTeamMenuState::Open { list } = &self.state else {
            return None;
        };
        list.selected_row().map(|row| row.uid)
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiTeamMenuState::Open { list } = &self.state else {
            return None;
        };
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Teams".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list.rows().iter().map(snapshot_row).collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            scroll_anchor: list.scroll_anchor(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status: list
                .rows()
                .is_empty()
                .then(|| TuiInlineMenuStatus::Empty("No teams found".to_owned())),
        })
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let query = input_text(&self.input_editor, ctx).trim().to_lowercase();
        let user_workspaces = UserWorkspaces::as_ref(ctx);
        let active_uid = user_workspaces.team_uid_for_window(self.window_id);
        let rows = user_workspaces
            .current_workspace()
            .map(|workspace| workspace.teams.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|team| query.is_empty() || team.name.to_lowercase().contains(&query))
            .map(|team| TuiTeamMenuRow {
                uid: team.uid,
                title: team.name.clone(),
                is_active: Some(team.uid) == active_uid,
            })
            .collect::<Vec<_>>();
        let preferred_index = preferred_row_index(&rows);
        let TuiTeamMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |row| {
            row.is_selectable()
        });
        ctx.emit(TuiTeamMenuEvent);
    }
}

impl TuiTeamMenuRow {
    /// Every team the user belongs to can be switched to, including the active one, so that
    /// accepting the highlighted row is always a no-op rather than an error.
    fn is_selectable(&self) -> bool {
        true
    }
}

/// Prefers the active team, falling back to the first selectable row.
///
/// The fallback is what makes search work: a query that filters the active team out would
/// otherwise leave nothing selected, so typing a name and pressing enter would do nothing.
fn preferred_row_index(rows: &[TuiTeamMenuRow]) -> Option<usize> {
    rows.iter()
        .position(|row| row.is_active)
        .or_else(|| rows.iter().position(|row| row.is_selectable()))
}

fn snapshot_row(row: &TuiTeamMenuRow) -> TuiInlineMenuRow {
    TuiInlineMenuRow {
        title: row.title.clone(),
        prefix: None,
        description: None,
        state_suffix: row.is_active.then(|| "(active)".to_owned()),
        promotional_suffix: None,
        is_selectable: row.is_selectable(),
        style: TuiInlineMenuRowStyle::Default,
    }
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

impl Entity for TuiTeamMenuModel {
    type Event = TuiTeamMenuEvent;
}

#[cfg(test)]
#[path = "team_menu_tests.rs"]
mod tests;
