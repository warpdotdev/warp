use settings::{
    macros::define_settings_group, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};
use warpui::{keymap::Keystroke, AppContext, DisplayIdx, ModelContext};

use crate::{
    report_if_error,
    root_view::{update_quake_window_bounds, QuakeModePinPosition},
    settings::{
        CtrlTabBehavior, ExtraMetaKeys as ExtraMetaKeysEnum, GlobalHotkeyMode, SizePercentages,
        DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES,
    },
};

define_settings_group!(KeysSettings, settings: [
    quake_mode_settings: QuakeModeSettings {
        type: crate::settings::QuakeModeSettings,
        default: crate::settings::QuakeModeSettings::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "global_hotkey.dedicated_window.settings",
        max_table_depth: 2,
        description: "Configuration options for Quake Mode window behavior.",
    },
    quake_mode_enabled: QuakeModeEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "global_hotkey.dedicated_window.enabled",
        description: "Whether the dedicated hotkey window is enabled. Mutually exclusive with `global_hotkey.toggle_all_windows.enabled`; only one should be true at a time.",
    },
    activation_hotkey_enabled: ActivationHotkeyEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "global_hotkey.toggle_all_windows.enabled",
        description: "Whether the hotkey that toggles visibility of all windows is enabled. Mutually exclusive with `global_hotkey.dedicated_window.enabled`; only one should be true at a time.",
    },
    activation_hotkey_keybinding: ActivationHotkeyKeybinding {
        type: Option<Keystroke>,
        default: None,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "global_hotkey.toggle_all_windows.keybinding",
        description: "The keybinding used for the global activation hotkey. Format: modifiers (cmd, ctrl, alt, shift, meta) and a key joined by '-', e.g. \"cmd-shift-a\" or \"alt-enter\". Bindings are case-sensitive: when shift is present, the key must be its shifted form (e.g., \"ctrl-shift-E\", not \"ctrl-shift-e\").",
    }
    extra_meta_keys: ExtraMetaKeys {
        type: ExtraMetaKeysEnum,
        default: ExtraMetaKeysEnum::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.input.extra_meta_keys",
        description: "Controls which additional keys are treated as meta keys.",
    }
    ctrl_tab_behavior: CtrlTabBehaviorSetting {
        type: CtrlTabBehavior,
        default: CtrlTabBehavior::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "keys.ctrl_tab_behavior_setting",
        description: "Controls the behavior of Ctrl+Tab.",
    }
]);

impl KeysSettings {
    pub fn set_global_hotkey_mode_and_write_to_user_defaults(
        &mut self,
        global_hotkey_mode: &GlobalHotkeyMode,
        ctx: &mut ModelContext<Self>,
    ) {
        // currently the activation hotkey and quake mode cannot be enabled simultaneously
        // if we enable quake mode, we must disable activation hotkey and vice versa
        let (enable_quake_mode, enable_activation_hotkey) = match global_hotkey_mode {
            GlobalHotkeyMode::QuakeMode => (true, false),
            GlobalHotkeyMode::ActivationHotkey => (false, true),
            GlobalHotkeyMode::Disabled => (false, false),
        };

        report_if_error!(self.quake_mode_enabled.set_value(enable_quake_mode, ctx));
        report_if_error!(self
            .activation_hotkey_enabled
            .set_value(enable_activation_hotkey, ctx));
    }

    // Note that registering an empty keybinding when enabling quake mode will be a no-op.
    // No global hotkey will be active.
    pub fn set_quake_mode_keybinding_and_write_to_user_defaults(
        &mut self,
        keystroke: Option<Keystroke>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut quake_mode_settings = self.quake_mode_settings.value().clone();
        quake_mode_settings.keybinding = keystroke;

        report_if_error!(self.quake_mode_settings.set_value(quake_mode_settings, ctx));
    }

    pub fn set_activation_hotkey_keybinding_and_write_to_user_defaults(
        &mut self,
        keystroke: Option<Keystroke>,
        ctx: &mut ModelContext<Self>,
    ) {
        report_if_error!(self.activation_hotkey_keybinding.set_value(keystroke, ctx));
    }

    pub fn set_quake_mode_pin_screen_and_write_to_user_defaults(
        &mut self,
        pin_screen: Option<DisplayIdx>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut quake_mode_settings = self.quake_mode_settings.value().clone();
        quake_mode_settings.pin_screen = pin_screen;

        update_quake_window_bounds(&quake_mode_settings, ctx);
        report_if_error!(self.quake_mode_settings.set_value(quake_mode_settings, ctx));
    }

    pub fn set_quake_mode_pin_position_and_write_to_user_defaults(
        &mut self,
        pin_position: QuakeModePinPosition,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut quake_mode_settings = self.quake_mode_settings.value().clone();
        quake_mode_settings.active_pin_position = pin_position;

        update_quake_window_bounds(&quake_mode_settings, ctx);
        report_if_error!(self.quake_mode_settings.set_value(quake_mode_settings, ctx));
    }

    pub fn set_quake_mode_width_or_height_and_write_to_user_defaults(
        &mut self,
        width_percentage: Option<u8>,
        height_percentage: Option<u8>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut quake_mode_settings = self.quake_mode_settings.value().clone();
        let current_pin_position = quake_mode_settings.active_pin_position;
        let default_percentages = DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES
            .get(&current_pin_position)
            .expect("should exist");

        quake_mode_settings
            .pin_position_to_size_percentages
            .entry(current_pin_position)
            .and_modify(|size_percentages| {
                if let Some(width_percentage) = width_percentage {
                    size_percentages.width = width_percentage;
                }
                if let Some(height_percentage) = height_percentage {
                    size_percentages.height = height_percentage;
                }
            })
            .or_insert_with(|| SizePercentages {
                width: width_percentage.unwrap_or(default_percentages.width),
                height: height_percentage.unwrap_or(default_percentages.height),
            });

        update_quake_window_bounds(&quake_mode_settings, ctx);
        report_if_error!(self.quake_mode_settings.set_value(quake_mode_settings, ctx));
    }

    pub fn toggle_hide_quake_mode_window_when_unfocused_and_write_to_user_defaults(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut quake_mode_settings = self.quake_mode_settings.value().clone();
        quake_mode_settings.hide_window_when_unfocused =
            !quake_mode_settings.hide_window_when_unfocused;

        report_if_error!(self.quake_mode_settings.set_value(quake_mode_settings, ctx));
    }

    pub fn set_hide_quake_mode_window_when_unfocused_and_write_to_user_defaults(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut quake_mode_settings = self.quake_mode_settings.value().clone();
        quake_mode_settings.hide_window_when_unfocused = value;

        report_if_error!(self.quake_mode_settings.set_value(quake_mode_settings, ctx));
    }

    pub fn global_hotkey_mode(&self, app: &AppContext) -> GlobalHotkeyMode {
        let mut selected = GlobalHotkeyMode::Disabled;

        if app.is_wayland() {
            return selected;
        }

        if *self.quake_mode_enabled && *self.activation_hotkey_enabled {
            log::error!("Both quake mode AND activation hotkey enabled. Either one or the other should be active.");
            // Quake mode takes precedence
            selected = GlobalHotkeyMode::QuakeMode;
        } else if *self.quake_mode_enabled {
            selected = GlobalHotkeyMode::QuakeMode;
        } else if *self.activation_hotkey_enabled {
            selected = GlobalHotkeyMode::ActivationHotkey
        }

        selected
    }
}
