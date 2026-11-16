use std::fmt;

use warp::editor::CodeEditorModel;
use warp::tui_export::{
    TuiMcpInstallRequest, TuiMcpServerId, TuiMcpTemplateVariable, TuiMcpVariableValue,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Clone, Eq, PartialEq)]
pub enum TuiMcpInstallFlowAction {
    ProvideValue { key: String, value: String },
}
impl fmt::Debug for TuiMcpInstallFlowAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::ProvideValue { key, .. } = self;
        formatter
            .debug_struct("ProvideValue")
            .field("key", key)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug)]
struct TuiMcpInstallChoice {
    value: String,
}

#[derive(Default)]
enum TuiMcpInstallStep {
    #[default]
    Closed,
    Variable {
        index: usize,
        choices: TuiInlineMenuListState<TuiMcpInstallChoice>,
    },
}

pub(crate) struct TuiMcpInstallCompletion {
    pub id: TuiMcpServerId,
    pub name: String,
    pub values: Vec<TuiMcpVariableValue>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiMcpInstallFlowEvent {
    Updated,
    Dismissed,
}

pub(crate) struct TuiMcpInstallFlowModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    request: Option<TuiMcpInstallRequest>,
    values: Vec<TuiMcpVariableValue>,
    step: TuiMcpInstallStep,
}

impl TuiMcpInstallFlowModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    ) -> Self {
        Self {
            input_editor,
            suggestions_mode,
            request: None,
            values: Vec::new(),
            step: TuiMcpInstallStep::Closed,
        }
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        !matches!(self.step, TuiMcpInstallStep::Closed)
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::McpInstall
    }

    pub(crate) fn start(
        &mut self,
        request: TuiMcpInstallRequest,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(first_variable) = request.variables.first() else {
            return false;
        };
        if !self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::McpInstall, ctx)
        }) {
            return false;
        }
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.values.clear();
        self.step = variable_step(first_variable);
        self.request = Some(request);
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
        true
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        self.request = None;
        self.values.clear();
        self.step = TuiMcpInstallStep::Closed;
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::McpInstall, ctx);
        });
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.emit(TuiMcpInstallFlowEvent::Dismissed);
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiMcpInstallStep::Variable { choices, .. } = &mut self.step else {
            return;
        };
        choices.select_previous(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiMcpInstallStep::Variable { choices, .. } = &mut self.step else {
            return;
        };
        choices.select_next(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
    }

    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiMcpInstallStep::Variable { choices, .. } = &mut self.step else {
            return false;
        };
        let selected = choices.select_absolute(index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
        selected
    }

    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiMcpInstallStep::Variable { choices, .. } = &mut self.step else {
            return;
        };
        choices.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
    }

    pub(crate) fn accept(&self, ctx: &AppContext) -> Option<TuiMcpInstallFlowAction> {
        let request = self.request.as_ref()?;
        match &self.step {
            TuiMcpInstallStep::Closed => None,
            TuiMcpInstallStep::Variable { index, choices } => {
                let variable = request.variables.get(*index)?;
                let value = match &variable.allowed_values {
                    Some(_) => choices.selected_row()?.value.clone(),
                    None => input_text(&self.input_editor, ctx),
                };
                (!value.is_empty()).then(|| TuiMcpInstallFlowAction::ProvideValue {
                    key: variable.key.clone(),
                    value,
                })
            }
        }
    }

    pub(crate) fn apply_value(
        &mut self,
        key: String,
        value: String,
        ctx: &mut ModelContext<Self>,
    ) -> Result<Option<TuiMcpInstallCompletion>, String> {
        let request = self
            .request
            .as_ref()
            .ok_or_else(|| "The MCP installation flow is no longer active".to_owned())?;
        let TuiMcpInstallStep::Variable { index, .. } = &self.step else {
            return Err("The MCP installation flow is not collecting a variable".to_owned());
        };
        let variable = request
            .variables
            .get(*index)
            .ok_or_else(|| "The MCP variable is no longer available".to_owned())?;
        if variable.key != key || value.is_empty() {
            return Err("Enter a value for the required MCP variable".to_owned());
        }
        if variable
            .allowed_values
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(&value))
        {
            return Err("Select one of the listed values".to_owned());
        }

        self.values.push(TuiMcpVariableValue { key, value });
        let next = *index + 1;
        let completion = if let Some(variable) = request.variables.get(next) {
            self.step = variable_step_at(next, variable);
            None
        } else {
            Some(TuiMcpInstallCompletion {
                id: request.id,
                name: request.name.clone(),
                values: self.values.clone(),
            })
        };
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
        Ok(completion)
    }

    pub(crate) fn primary_action_hint(&self) -> Option<&'static str> {
        let request = self.request.as_ref()?;
        match &self.step {
            TuiMcpInstallStep::Closed => None,
            TuiMcpInstallStep::Variable { index, .. } if *index + 1 == request.variables.len() => {
                Some("to install and enable")
            }
            TuiMcpInstallStep::Variable { .. } => Some("to continue"),
        }
    }

    pub(crate) fn input_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        if !self.is_open(ctx) {
            return None;
        }
        let request = self.request.as_ref()?;
        let TuiMcpInstallStep::Variable { index, .. } = &self.step else {
            return None;
        };
        request
            .variables
            .get(*index)
            .is_some_and(|variable| variable.allowed_values.is_none())
            .then_some("Enter value…")
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let request = self.request.as_ref()?;
        let title = format!("Install and enable {}", request.name);
        let header = Some(TuiInlineMenuHeader {
            title: Some(title),
            tabs: Vec::new(),
        });
        match &self.step {
            TuiMcpInstallStep::Closed => None,
            TuiMcpInstallStep::Variable { index, choices } => {
                let variable = request.variables.get(*index)?;
                let status = variable.allowed_values.is_none().then(|| {
                    TuiInlineMenuStatus::Empty(format!(
                        "Enter a value for {} ({}/{})",
                        variable.key,
                        index + 1,
                        request.variables.len()
                    ))
                });
                Some(TuiInlineMenuSnapshot {
                    header,
                    rows: choices
                        .rows()
                        .iter()
                        .map(|choice| TuiInlineMenuRow {
                            title: choice.value.clone(),
                            prefix: None,
                            description: Some(format!(
                                "{} · {}/{}",
                                variable.key,
                                index + 1,
                                request.variables.len()
                            )),
                            state_suffix: None,
                            promotional_suffix: None,
                            is_selectable: true,
                            style: TuiInlineMenuRowStyle::Default,
                        })
                        .collect(),
                    selected_index: choices.selected_index(),
                    scroll_offset: choices.scroll_offset(),
                    scroll_anchor: choices.scroll_anchor(),
                    max_visible_rows: MAX_VISIBLE_ROWS,
                    status,
                })
            }
        }
    }
}

fn variable_step(variable: &TuiMcpTemplateVariable) -> TuiMcpInstallStep {
    variable_step_at(0, variable)
}

fn variable_step_at(index: usize, variable: &TuiMcpTemplateVariable) -> TuiMcpInstallStep {
    let rows = variable
        .allowed_values
        .as_ref()
        .into_iter()
        .flatten()
        .cloned()
        .map(|value| TuiMcpInstallChoice { value })
        .collect();
    let mut choices = TuiInlineMenuListState::default();
    choices.replace_rows(rows, false, Some(0), MAX_VISIBLE_ROWS, |_| true);
    TuiMcpInstallStep::Variable { index, choices }
}

fn input_text(editor: &ModelHandle<CodeEditorModel>, ctx: &AppContext) -> String {
    let model = editor.as_ref(ctx);
    let buffer = model.content().as_ref(ctx);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

impl Entity for TuiMcpInstallFlowModel {
    type Event = TuiMcpInstallFlowEvent;
}

#[cfg(test)]
#[path = "mcp_install_flow_tests.rs"]
mod tests;
