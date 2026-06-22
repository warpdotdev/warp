//! Utilities for terminal keybindings.

use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::terminal::input::{
    SET_INPUT_MODE_AGENT_ACTION_NAME, SET_INPUT_MODE_TERMINAL_ACTION_NAME,
    SET_INPUT_MODE_UNLOCKED_AGENT_ACTION_NAME, SET_INPUT_MODE_UNLOCKED_TERMINAL_ACTION_NAME,
};
use crate::util::bindings::{
    custom_tag_to_keystroke, keybinding_name_to_display_string, CustomAction,
};

/// Cache of keybindings used in terminal.
pub struct TerminalKeybindings {
    // Cache of editable keybinding names, to render in tooltips. This cache is necessary because
    // looking up a keybinding requires a [`AppContext`], so it can't be done when
    // rendering.
    //
    // Inspired by https://github.com/warpdotdev/warp-internal/pull/8274
    set_input_mode_agent_keybinding: Option<String>,
    set_input_mode_terminal_keybinding: Option<String>,
    set_input_mode_unlocked_agent_keybinding: Option<String>,
    set_input_mode_unlocked_terminal_keybinding: Option<String>,
}

impl TerminalKeybindings {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &KeybindingChangedNotifier::handle(ctx),
            Self::handle_keybinding_change,
        );
        Self {
            set_input_mode_agent_keybinding: keybinding_name_to_display_string(
                SET_INPUT_MODE_AGENT_ACTION_NAME,
                ctx,
            ),
            set_input_mode_terminal_keybinding: keybinding_name_to_display_string(
                SET_INPUT_MODE_TERMINAL_ACTION_NAME,
                ctx,
            ),
            set_input_mode_unlocked_agent_keybinding: keybinding_name_to_display_string(
                SET_INPUT_MODE_UNLOCKED_AGENT_ACTION_NAME,
                ctx,
            ),
            set_input_mode_unlocked_terminal_keybinding: keybinding_name_to_display_string(
                SET_INPUT_MODE_UNLOCKED_TERMINAL_ACTION_NAME,
                ctx,
            ),
        }
    }

    /// Display label for the keybinding to set input mode to agent mode
    pub fn set_input_mode_agent_keybinding(&self) -> Option<String> {
        self.set_input_mode_agent_keybinding.clone()
    }

    /// Display label for the keybinding to set input mode to terminal mode
    pub fn set_input_mode_terminal_keybinding(&self) -> Option<String> {
        self.set_input_mode_terminal_keybinding.clone()
    }

    /// Display label for the keybinding to set input mode to unlocked agent mode
    pub fn set_input_mode_unlocked_agent_keybinding(&self) -> Option<String> {
        self.set_input_mode_unlocked_agent_keybinding.clone()
    }

    /// Display label for the keybinding to set input mode to unlocked terminal mode
    pub fn set_input_mode_unlocked_terminal_keybinding(&self) -> Option<String> {
        self.set_input_mode_unlocked_terminal_keybinding.clone()
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
        if binding_name == SET_INPUT_MODE_AGENT_ACTION_NAME {
            self.set_input_mode_agent_keybinding = new_trigger.as_ref().map(|key| key.displayed());
            ctx.notify();
        } else if binding_name == SET_INPUT_MODE_TERMINAL_ACTION_NAME {
            self.set_input_mode_terminal_keybinding =
                new_trigger.as_ref().map(|key| key.displayed());
            ctx.notify();
        } else if binding_name == SET_INPUT_MODE_UNLOCKED_AGENT_ACTION_NAME {
            self.set_input_mode_unlocked_agent_keybinding =
                new_trigger.as_ref().map(|key| key.displayed());
            ctx.notify();
        } else if binding_name == SET_INPUT_MODE_UNLOCKED_TERMINAL_ACTION_NAME {
            self.set_input_mode_unlocked_terminal_keybinding =
                new_trigger.as_ref().map(|key| key.displayed());
            ctx.notify();
        }
    }
}

impl Entity for TerminalKeybindings {
    type Event = ();
}

impl SingletonEntity for TerminalKeybindings {}

/// The keybinding label to display for a [`CustomAction`].
pub fn custom_action_to_display(action: CustomAction) -> Option<String> {
    custom_tag_to_keystroke(action.into()).map(|keystroke| keystroke.displayed())
}
