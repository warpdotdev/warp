//! TUI keybinding registration and cross-surface validation.
//!
//! Mirrors the GUI convention: each TUI view module exposes a top-level
//! `init(app)` that registers its keybindings, aggregated here and called once
//! at TUI startup (from [`crate::session`]'s mount). Fixed bindings are
//! reserved keys (ctrl-c); editable bindings are named `tui:*` so they are
//! user-remappable by name via `keybindings.yaml` (loading overrides in the
//! TUI process is a follow-up — the names registered here are the stable
//! contract).
//!
//! # Cross-surface isolation
//! GUI bindings cannot fire in the TUI even though the TUI process registers
//! them all: predicate-scoped bindings never match TUI keymap contexts, and
//! even a predicate-less binding dispatches an action type that no TUI view
//! handles, so the keystroke falls through to the element pass unharmed. The
//! debug-time validators below enforce the remaining convention: any
//! *keystroke* binding that matches a TUI view's context must be TUI-owned.
//! This catches GUI bindings registered without a context predicate — which
//! would otherwise match everywhere and, for multi-keystroke chords, swallow
//! prefix keys via a pending match.

use warpui_core::keymap::macros::*;
use warpui_core::keymap::{
    BindingLens, Context, ContextPredicate, EditableBinding, IsBindingValid, Trigger,
};
use warpui_core::{Action, AppContext, TuiView};

use crate::attachment_bar::TuiAttachmentBar;
use crate::cloud_run_view::TuiCloudRunView;
use crate::editor_interaction::{TuiEditorBindingTarget, TuiEditorCommand, editor_binding_specs};
use crate::editor_view::{TuiEditorView, TuiEditorViewAction};
use crate::input::TuiInputView;
use crate::input::view::TuiInputAction;
use crate::option_selector::TuiOptionSelector;
use crate::orchestration_block::TuiOrchestrationBlock;
use crate::root_view::RootTuiView;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::transcript_view::TuiTranscriptView;

/// Group tag set on every TUI-registered binding. The validators treat it (or
/// a `tui:` name prefix) as proof of TUI ownership.
pub(crate) const TUI_BINDING_GROUP: &str = "tui";
pub(crate) const ATTACHMENTS_AVAILABLE_FLAG: &str = "TuiAttachmentsAvailable";
pub(crate) const PLAN_TOGGLE_AVAILABLE_FLAG: &str = "TuiPlanToggleAvailable";
pub(crate) const KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG: &str = "TuiKeyboardEnhancementAvailable";
pub(crate) const PLAN_TOGGLE_BINDING_NAME: &str = "tui:session:toggle_plan";
pub(crate) const CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME: &str =
    "tui:session:toggle_plan_when_available";
pub(crate) fn plan_toggle_hint(ctx: &AppContext) -> Option<String> {
    let mut context = Context::default();
    context.set.insert(TuiTerminalSessionView::ui_name());
    ctx.editable_bindings()
        .find(|binding| binding.name == PLAN_TOGGLE_BINDING_NAME && binding.in_context(&context))
        .and_then(|binding| match binding.trigger {
            Trigger::Keystrokes(keystrokes) if !keystrokes.is_empty() => Some(
                keystrokes
                    .iter()
                    .map(|keystroke| keystroke.displayed_expanded())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Trigger::Keystrokes(_) | Trigger::Standard(_) | Trigger::Custom(_) | Trigger::Empty => {
                None
            }
        })
}

/// Registers all TUI view keybindings and the cross-surface binding
/// validators. Called once at TUI startup, before the driver starts.
pub(crate) fn init(app: &mut AppContext) {
    crate::root_view::init(app);
    crate::cloud_run_view::init(app);
    crate::terminal_session_view::init(app);
    crate::attachment_bar::init(app);
    crate::input::init(app);
    register_editor_bindings(
        app,
        TuiEditorBindingTarget::Input,
        id!("TuiInputView"),
        TuiInputAction::EditorCommand,
    );
    register_editor_bindings(
        app,
        TuiEditorBindingTarget::Editor,
        id!("TuiEditorView"),
        TuiEditorViewAction::Command,
    );
    crate::orchestration_block::init(app);
    crate::tui_ask_question_view::init(app);
    crate::tui_permission_prompt::init(app);

    register_binding_validators(app);
}

/// Registers one editor binding target from interaction-owned metadata.
fn register_editor_bindings<A>(
    app: &mut AppContext,
    target: TuiEditorBindingTarget,
    context: ContextPredicate,
    action_for: impl Fn(TuiEditorCommand) -> A,
) where
    A: Action,
{
    let action_for = &action_for;
    let bindings = editor_binding_specs(target).flat_map(|spec| {
        let context = context.clone();
        spec.keys.iter().filter_map(move |key| {
            let context = context_for_editor_binding(target, spec.command, key, &context)?;
            Some(
                EditableBinding::new(spec.name, spec.description, action_for(spec.command))
                    .with_context_predicate(context)
                    .with_group(TUI_BINDING_GROUP)
                    .with_key_binding(key),
            )
        })
    });
    app.register_editable_bindings(bindings);
}

fn context_for_editor_binding(
    target: TuiEditorBindingTarget,
    command: TuiEditorCommand,
    key: &str,
    default_context: &ContextPredicate,
) -> Option<ContextPredicate> {
    match (target, command, key) {
        // The input editor reserves ctrl-d for session-level EOF and exit handling.
        (TuiEditorBindingTarget::Input, TuiEditorCommand::DeleteForward, "ctrl-d") => None,
        (TuiEditorBindingTarget::Input, TuiEditorCommand::MoveUp, "ctrl-p") => Some(
            default_context.clone()
                & (!id!(PLAN_TOGGLE_AVAILABLE_FLAG) | id!(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG)),
        ),
        _ => Some(default_context.clone()),
    }
}

/// Debug-time guard (no-op in release): every keystroke binding that matches a
/// TUI view's default keymap context must be TUI-owned.
fn register_binding_validators(app: &mut AppContext) {
    app.register_tui_binding_validator::<RootTuiView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiCloudRunView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiTerminalSessionView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiAttachmentBar>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiInputView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiEditorView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiTranscriptView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiOrchestrationBlock>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiOptionSelector>(is_tui_owned_binding);
}

pub(crate) fn is_tui_owned_binding(binding: BindingLens) -> IsBindingValid {
    // Non-keystroke triggers (palette-only `Empty`, `Standard`, `Custom`)
    // can never fire from TUI keyboard input, so they are exempt.
    if !matches!(binding.trigger, Trigger::Keystrokes(_)) {
        return IsBindingValid::Yes;
    }
    if is_tui_owned(binding.name, binding.group) {
        IsBindingValid::Yes
    } else {
        IsBindingValid::No
    }
}

/// Whether a binding's identity marks it as TUI-owned: a `tui:`-prefixed name
/// (editable bindings) or the [`TUI_BINDING_GROUP`] group (fixed bindings).
fn is_tui_owned(name: &str, group: Option<&str>) -> bool {
    name.starts_with("tui:") || group == Some(TUI_BINDING_GROUP)
}

#[cfg(test)]
#[path = "keybindings_tests.rs"]
mod tests;
