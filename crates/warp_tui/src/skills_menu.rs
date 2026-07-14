//! Searchable TUI skill picker state.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    query_selectable_skills, AcceptSkill, ActiveSession, ActiveSessionEvent, SkillReference,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle};

use crate::inline_menu::{
    result_row_capacity, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, MAX_INLINE_MENU_ROWS,
};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone)]
struct TuiSkillMenuRow {
    name: String,
    reference: SkillReference,
    description: String,
}

#[derive(Debug, Clone, Default)]
enum TuiSkillMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiSkillMenuRow>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiSkillMenuEvent;

pub(crate) struct TuiSkillMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
    state: TuiSkillMenuState,
}

impl TuiSkillMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        active_session: ModelHandle<ActiveSession>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open() && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(&active_session, |model, _, event, ctx| {
            if model.is_open()
                && matches!(
                    event,
                    ActiveSessionEvent::UpdatedPwd | ActiveSessionEvent::Bootstrapped
                )
            {
                model.refresh_rows(ctx);
            }
        });
        Self {
            input_editor,
            active_session,
            terminal_view_id,
            state: TuiSkillMenuState::Closed,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.state, TuiSkillMenuState::Open { .. })
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open() {
            return;
        }
        self.state = TuiSkillMenuState::Open {
            list: TuiInlineMenuListState::default(),
        };
        self.refresh_rows(ctx);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open() {
            return;
        }
        self.state = TuiSkillMenuState::Closed;
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.emit(TuiSkillMenuEvent);
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSkillMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiSkillMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSkillMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiSkillMenuEvent);
    }

    pub(crate) fn accept_selected(&mut self, ctx: &mut ModelContext<Self>) -> Option<AcceptSkill> {
        let TuiSkillMenuState::Open { list } = &self.state else {
            return None;
        };
        let row = list.selected_row()?.clone();
        self.state = TuiSkillMenuState::Closed;
        ctx.emit(TuiSkillMenuEvent);
        Some(AcceptSkill {
            skill_name: row.name,
            skill_reference: row.reference,
        })
    }

    pub(crate) fn snapshot(&self) -> Option<TuiInlineMenuSnapshot> {
        let TuiSkillMenuState::Open { list } = &self.state else {
            return None;
        };
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Skills".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: format!("/{}", row.name),
                    description: (!row.description.is_empty()).then(|| row.description.clone()),
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::InlineMenuItem,
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status: list
                .rows()
                .is_empty()
                .then(|| TuiInlineMenuStatus::Empty("No skills found".to_owned())),
        })
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSkillMenuState::Open { list } = &self.state else {
            return;
        };
        let selected_reference = list.selected_row().map(|row| row.reference.clone());
        let query = input_text(&self.input_editor, ctx);
        let working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory_location(ctx);
        let rows = query_selectable_skills(
            working_directory.as_ref(),
            self.terminal_view_id,
            true,
            &query,
            ctx,
        )
        .into_iter()
        .map(|skill| TuiSkillMenuRow {
            name: skill.name,
            reference: skill.reference,
            description: skill.description,
        })
        .collect::<Vec<_>>();
        let preferred_index = selected_reference
            .and_then(|reference| rows.iter().position(|row| row.reference == reference))
            .or_else(|| rows.len().checked_sub(1));
        let TuiSkillMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiSkillMenuEvent);
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

impl Entity for TuiSkillMenuModel {
    type Event = TuiSkillMenuEvent;
}
