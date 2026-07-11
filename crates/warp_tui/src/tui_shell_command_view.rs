//! TUI view for a `RequestCommandOutput` tool call.
//!
//! The GUI and TUI share the action model and the terminal block that records
//! the command's ground-truth execution state. This view adds only TUI chrome:
//! the existing status-aware tool-call header becomes a collapsed disclosure,
//! and expanding it embeds the same terminal-cell renderer used by top-level
//! shell blocks.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionResultType, AIAgentActionType, BlockId,
    BlocklistAIActionModel, RequestCommandOutputResult, TerminalModel,
};
use warpui_core::elements::tui::{tui_collapsible, Modifier, TuiElement};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewContext};

use crate::agent_block_sections::{
    render_fallback_tool_call_section, tool_call_glyph_style, tool_call_label_style,
};
use crate::terminal_block::TerminalBlockElement;
use crate::tool_call_labels::{
    tool_call_display_state, tool_call_glyph, tool_call_label, CommandBlockState,
    ResolvedCommandBlock,
};
use crate::tui_builder::TuiUiBuilder;

#[derive(Clone)]
struct ShellCommandViewState {
    collapsed: Rc<Cell<bool>>,
}

impl ShellCommandViewState {
    fn new_collapsed() -> Self {
        Self {
            collapsed: Rc::new(Cell::new(true)),
        }
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed.get()
    }

    fn toggle(&self) {
        self.collapsed.set(!self.collapsed.get());
    }
}

struct ResolvedShellCommandBlock {
    block_id: BlockId,
    details: ResolvedCommandBlock,
}
/// Events emitted to the agent block that owns this shell-command view.
pub(super) enum TuiShellCommandViewEvent {
    /// The disclosure body was expanded or collapsed, changing the owning
    /// agent block's measured height.
    LayoutInvalidated,
}

/// Typed actions originating from the shell-command element tree.
#[derive(Clone, Debug)]
pub(super) enum TuiShellCommandViewAction {
    ToggleExpanded,
}

/// One stateful `RequestCommandOutput` child view in an agent exchange.
pub(super) struct TuiShellCommandView {
    action: AIAgentAction,
    output_streaming: bool,
    action_model: ModelHandle<BlocklistAIActionModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    state: ShellCommandViewState,
    header_mouse_state: MouseStateHandle,
}

impl TuiShellCommandView {
    pub(super) fn new(
        action: AIAgentAction,
        output_streaming: bool,
        action_model: ModelHandle<BlocklistAIActionModel>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
    ) -> Self {
        debug_assert!(matches!(
            &action.action,
            AIAgentActionType::RequestCommandOutput { .. }
        ));
        Self {
            action,
            output_streaming,
            action_model,
            terminal_model,
            state: ShellCommandViewState::new_collapsed(),
            header_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Refreshes streamed action arguments without replacing interaction state.
    pub(super) fn update_action(&mut self, action: AIAgentAction, output_streaming: bool) {
        self.action = action;
        self.output_streaming = output_streaming;
    }

    /// Resolves the shared terminal block exactly as the GUI requested-command
    /// view does: first by agent action metadata, then by a long-running
    /// snapshot's block ID for restored/view-only cases.
    fn resolved_block(&self, status: Option<&AIActionStatus>) -> Option<ResolvedShellCommandBlock> {
        let snapshot_block_id = match status
            .and_then(AIActionStatus::finished_result)
            .map(|result| &result.result)
        {
            Some(AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::LongRunningCommandSnapshot { block_id, .. },
            )) => Some(block_id),
            _ => None,
        };
        let model = self.terminal_model.lock();
        let block_list = model.block_list();
        let block = block_list
            .block_for_ai_action_id(&self.action.id)
            .or_else(|| snapshot_block_id.and_then(|id| block_list.block_with_id(id)))?;
        let command = block
            .command_with_secrets_obfuscated(false)
            .trim()
            .to_owned();
        let state = if block.finished() {
            CommandBlockState::Finished {
                exit_code: block.exit_code(),
            }
        } else {
            CommandBlockState::Running
        };
        Some(ResolvedShellCommandBlock {
            block_id: block.id().clone(),
            details: ResolvedCommandBlock {
                command: (!command.is_empty()).then_some(command),
                state,
            },
        })
    }
}

impl Entity for TuiShellCommandView {
    type Event = TuiShellCommandViewEvent;
}

impl TuiView for TuiShellCommandView {
    fn ui_name() -> &'static str {
        "TuiShellCommandView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action.id);
        let Some(block) = self.resolved_block(status.as_ref()) else {
            return render_fallback_tool_call_section(
                &self.action,
                status.as_ref(),
                self.output_streaming,
                None,
                app,
            );
        };

        let builder = TuiUiBuilder::from_app(app);
        let display_state =
            tool_call_display_state(status.as_ref(), false, Some(block.details.state));
        let glyph_style = tool_call_glyph_style(display_state, &builder);
        let mut label_style = tool_call_label_style(display_state, &builder);
        if self.header_mouse_state.lock().unwrap().is_hovered() {
            label_style = label_style.add_modifier(Modifier::BOLD);
        }
        let collapsed = self.state.is_collapsed();
        let label = tool_call_label(&self.action, status.as_ref(), false, Some(&block.details));
        let header_spans = vec![
            (format!("{} ", tool_call_glyph(display_state)), glyph_style),
            (format!("{label} "), label_style),
        ];

        tui_collapsible(
            collapsed,
            header_spans,
            label_style,
            self.header_mouse_state.clone(),
            || TerminalBlockElement::content(self.terminal_model.clone(), block.block_id).finish(),
            move |event_ctx, _app| {
                event_ctx.dispatch_typed_action(TuiShellCommandViewAction::ToggleExpanded);
            },
        )
    }
}

impl TypedActionView for TuiShellCommandView {
    type Action = TuiShellCommandViewAction;

    fn handle_action(&mut self, action: &TuiShellCommandViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiShellCommandViewAction::ToggleExpanded => {
                self.state.toggle();
                ctx.emit(TuiShellCommandViewEvent::LayoutInvalidated);
                ctx.notify();
            }
        }
    }
}
#[cfg(test)]
#[path = "tui_shell_command_view_tests.rs"]
mod tests;
