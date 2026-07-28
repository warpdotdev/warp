use std::fmt;

use warp::editor::CodeEditorModel;
use warp::tui_export::{
    TuiMcpFileScope, TuiMcpInstallRequest, TuiMcpServerId, TuiMcpServerSource,
    TuiMcpTemplateVariable, TuiMcpVariableValue,
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
    Confirm,
}
impl fmt::Debug for TuiMcpInstallFlowAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProvideValue { key, .. } => formatter
                .debug_struct("ProvideValue")
                .field("key", key)
                .field("value", &"[REDACTED]")
                .finish(),
            Self::Confirm => formatter.write_str("Confirm"),
        }
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
    Confirmation,
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
        if !self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::McpInstall, ctx)
        }) {
            return false;
        }
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.values.clear();
        self.step = if request.variables.is_empty() {
            TuiMcpInstallStep::Confirmation
        } else {
            variable_step(&request.variables[0])
        };
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
            return matches!(self.step, TuiMcpInstallStep::Confirmation) && index == 0;
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
            TuiMcpInstallStep::Confirmation => Some(TuiMcpInstallFlowAction::Confirm),
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
    ) -> Result<(), String> {
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
        self.step = request
            .variables
            .get(next)
            .map(|variable| variable_step_at(next, variable))
            .unwrap_or(TuiMcpInstallStep::Confirmation);
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.emit(TuiMcpInstallFlowEvent::Updated);
        Ok(())
    }

    pub(crate) fn confirmation(
        &self,
    ) -> Option<(TuiMcpServerId, String, Vec<TuiMcpVariableValue>)> {
        matches!(self.step, TuiMcpInstallStep::Confirmation).then(|| {
            let request = self.request.as_ref().expect("open flow has a request");
            (request.id, request.name.clone(), self.values.clone())
        })
    }

    pub(crate) fn primary_action_hint(&self) -> Option<&'static str> {
        match &self.step {
            TuiMcpInstallStep::Closed => None,
            TuiMcpInstallStep::Variable { .. } => Some("to continue"),
            TuiMcpInstallStep::Confirmation => Some("to enable & start"),
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
        let title = format!(
            "Enable {} · {}",
            request.name,
            source_label(&request.source)
        );
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
            TuiMcpInstallStep::Confirmation => Some(TuiInlineMenuSnapshot {
                header,
                rows: vec![TuiInlineMenuRow {
                    title: "Enable and start".to_owned(),
                    prefix: None,
                    description: Some(confirmation_description(request)),
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                }],
                selected_index: Some(0),
                scroll_offset: 0,
                scroll_anchor: crate::inline_menu::TuiInlineMenuScrollAnchor::Selection,
                max_visible_rows: MAX_VISIBLE_ROWS,
                status: None,
            }),
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

fn confirmation_description(request: &TuiMcpInstallRequest) -> String {
    let mut parts = Vec::new();
    if let Some(description) = request.description.as_deref().and_then(concise_line) {
        parts.push(description);
    }
    if let Some(instructions) = request.instructions.as_deref().and_then(concise_line)
        && !parts.contains(&instructions)
    {
        parts.push(instructions);
    }
    if parts.is_empty() {
        "Create a TUI-local installation and start it".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn concise_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .map(|line| line.trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
}

fn source_label(source: &TuiMcpServerSource) -> String {
    match source {
        TuiMcpServerSource::Installation => "TUI local".to_owned(),
        TuiMcpServerSource::SyncedTemplate => "synced".to_owned(),
        TuiMcpServerSource::Gallery => "gallery".to_owned(),
        TuiMcpServerSource::FileBased { sources } => {
            let labels = sources
                .iter()
                .map(|source| match source.scope {
                    TuiMcpFileScope::Global => format!("{} global", source.provider),
                    TuiMcpFileScope::Project => {
                        let root = source
                            .root_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("project");
                        format!("{} · {root}", source.provider)
                    }
                })
                .collect::<Vec<_>>();
            if labels.is_empty() {
                "file config".to_owned()
            } else {
                labels.join(", ")
            }
        }
    }
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
