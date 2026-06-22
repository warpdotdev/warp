//! Utilities for notebook keybindings.

use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::util::bindings::{
    custom_tag_to_keystroke, keybinding_name_to_display_string, CustomAction,
};

pub const RUN_COMMANDS_KEYBINDING_NAME: &str = "editor_view:run_commands";

/// Cache of keybindings used in notebooks.
pub struct NotebookKeybindings {
    // Cache of editable keybinding names, to render in tooltips. This cache is necessary because
    // looking up a keybinding requires a [`AppContext`], so it can't be done when
    // rendering.
    //
    // Inspired by https://github.com/warpdotdev/warp-internal/pull/5676 (see the `Workspace` view)
    run_commands_keybinding: Option<String>,
}

impl NotebookKeybindings {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &KeybindingChangedNotifier::handle(ctx),
            Self::handle_keybinding_change,
        );
        Self {
            run_commands_keybinding: keybinding_name_to_display_string(
                RUN_COMMANDS_KEYBINDING_NAME,
                ctx,
            ),
        }
    }

    /// Display label for the keybinding to run commands in a notebook.
    pub fn run_commands_keybinding(&self) -> Option<String> {
        self.run_commands_keybinding.clone()
    }

    fn handle_keybinding_change(
        &mut self,
        _: ModelHandle<KeybindingChangedNotifier>,
        event: &KeybindingChangedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let KeybindingChangedEvent::BindingChanged {
            binding_name,
            new_trigger,
        } = event;
        if binding_name == RUN_COMMANDS_KEYBINDING_NAME {
            self.run_commands_keybinding = new_trigger.as_ref().map(|key| key.displayed());
            ctx.notify();
        }
    }
}

impl Entity for NotebookKeybindings {
    type Event = ();
}

impl SingletonEntity for NotebookKeybindings {}

/// The keybinding label to display for a [`CustomAction`].
pub fn custom_action_to_display(action: CustomAction) -> Option<String> {
    custom_tag_to_keystroke(action.into()).map(|keystroke| keystroke.displayed())
}
