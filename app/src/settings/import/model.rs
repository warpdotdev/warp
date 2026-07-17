use std::collections::HashMap;

use serde::Serialize;
use strum::IntoEnumIterator;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

#[cfg(target_os = "macos")]
use super::config::HotkeyError;
use super::config::{SettingType, ThemeType};
use crate::interval_timer::IntervalTimer;
use crate::settings::import::config::{Config, ConfigError};
#[cfg(target_os = "macos")]
use crate::{send_telemetry_from_ctx, TelemetryEvent};

#[derive(Clone, Copy, Debug, EnumDiscriminants, Eq, Hash, PartialEq)]
#[strum_discriminants(derive(EnumIter, Hash, Serialize))]
#[strum_discriminants(name(TerminalType))]
pub enum TerminalTypeAndProfile {
    Alacritty,
    #[cfg(target_os = "macos")]
    ITerm(usize),
}

pub struct CompletedParseEvent {
    pub terminal: TerminalType,
}

pub struct ImportedConfigModel {
    started: bool,
    parsed_terminals: HashMap<TerminalType, Result<Vec<Config>, ConfigError>>,
}

impl ImportedConfigModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        ImportedConfigModel {
            parsed_terminals: Default::default(),
            started: false,
        }
    }

    #[cfg(feature = "local_fs")]
    pub fn search_for_settings_to_import(&mut self, ctx: &mut ModelContext<Self>) {
        use std::sync::Arc;

        use itertools::Itertools;
        use strum::IntoEnumIterator;
        self.started = true;

        let loaded_system_fonts = warpui::fonts::Cache::handle(ctx)
            .update(ctx, |font_cache, ctx| font_cache.all_system_fonts(ctx));
        ctx.spawn(loaded_system_fonts, |_, fonts, ctx| {
            let fonts = fonts
                .into_iter()
                .map(|(_family_id, font_info)| font_info)
                .collect_vec();
            let fonts_ref = Arc::new(fonts);
            for terminal_type in TerminalType::iter() {
                if terminal_type == TerminalType::Alacritty
                    && !FeatureFlag::AlacrittySettingsImport.is_enabled()
                {
                    continue;
                }
                let fonts_ref_clone = Arc::clone(&fonts_ref);
                ctx.spawn(
                    async move {
                        Config::create_from_terminal_type(terminal_type, fonts_ref_clone).await
                    },
                    move |model, output, ctx| {
                        model.write_parse_results(terminal_type, output, ctx);
                    },
                );
            }
        });
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    #[cfg(target_os = "macos")]
    fn maybe_send_multiple_hotkeys_telemetry_event(
        &self,
        terminal_type: &TerminalType,
        configs: &Result<Vec<Config>, ConfigError>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let TerminalType::ITerm = terminal_type {
            if let Ok(configs) = configs {
                if configs.iter().any(|config| {
                    matches!(
                        config.hotkey_mode.setting,
                        Err(HotkeyError::MultipleHotkeys)
                    )
                }) {
                    send_telemetry_from_ctx!(TelemetryEvent::ITermMultipleHotkeys, ctx);
                }
            }
        }
    }

    pub fn write_parse_results(
        &mut self,
        terminal_type: TerminalType,
        (configs, _timer): (Result<Vec<Config>, ConfigError>, IntervalTimer),
        ctx: &mut ModelContext<Self>,
    ) {
        #[cfg(target_os = "macos")]
        self.maybe_send_multiple_hotkeys_telemetry_event(&terminal_type, &configs, ctx);
        self.parsed_terminals.insert(terminal_type, configs);
        ctx.emit(CompletedParseEvent {
            terminal: terminal_type,
        });
    }

    pub fn configs(&self) -> impl Iterator<Item = (TerminalTypeAndProfile, &Config)> {
        self.parsed_terminals
            .iter()
            .filter_map(|(terminal, parse_result)| {
                parse_result.as_ref().ok().map(|value| (terminal, value))
            })
            .flat_map(|(discriminant, vec)| match discriminant {
                TerminalType::Alacritty => vec
                    .iter()
                    .take(1)
                    .map(|item| (TerminalTypeAndProfile::Alacritty, item))
                    .collect::<Vec<(TerminalTypeAndProfile, &Config)>>()
                    .into_iter(),
                #[cfg(target_os = "macos")]
                TerminalType::ITerm => vec
                    .iter()
                    .enumerate()
                    .map(|(idx, item)| (TerminalTypeAndProfile::ITerm(idx), item))
                    .collect::<Vec<(TerminalTypeAndProfile, &Config)>>()
                    .into_iter(),
            })
    }

    pub(super) fn config(&self, profile: &TerminalTypeAndProfile) -> Option<&Config> {
        self.parsed_terminals
            .get(&TerminalType::from(profile))
            .map(|vec| match profile {
                TerminalTypeAndProfile::Alacritty => vec.as_ref().ok().and_then(|vec| vec.first()),
                #[cfg(target_os = "macos")]
                TerminalTypeAndProfile::ITerm(idx) => {
                    vec.as_ref().ok().and_then(|vec| vec.get(*idx))
                }
            })
            .unwrap_or_else(|| {
                log::warn!("Attempted to access an invalid profile.");
                None
            })
    }

    fn config_mut(&mut self, profile: &TerminalTypeAndProfile) -> Option<&mut Config> {
        self.parsed_terminals
            .get_mut(&TerminalType::from(profile))
            .map(|vec| match profile {
                TerminalTypeAndProfile::Alacritty => {
                    vec.as_mut().ok().and_then(|vec| vec.first_mut())
                }
                #[cfg(target_os = "macos")]
                TerminalTypeAndProfile::ITerm(idx) => {
                    vec.as_mut().ok().and_then(|vec| vec.get_mut(*idx))
                }
            })
            .unwrap_or_else(|| {
                log::warn!("Attempted to access an invalid profile.");
                None
            })
    }

    pub fn toggle_should_import(
        &mut self,
        profile: &TerminalTypeAndProfile,
        setting: &SettingType,
    ) {
        let Some(config) = self.config_mut(profile) else {
            log::warn!("Attempted to toggle import on an invalid profile!");
            return;
        };
        match setting {
            SettingType::Theme => config.theme.should_import = !config.theme.should_import,
            SettingType::OptionAsMeta => {
                config.option_as_meta.should_import = !config.option_as_meta.should_import
            }
            SettingType::MouseAndScrollReporting => {
                config.mouse_and_scroll_reporting.should_import =
                    !config.mouse_and_scroll_reporting.should_import
            }
            SettingType::Font => config.font.should_import = !config.font.should_import,
            SettingType::DefaultShell => {
                config.default_shell.should_import = !config.default_shell.should_import
            }
            SettingType::WorkingDirectory => {
                config.working_directory.should_import = !config.working_directory.should_import
            }
            SettingType::HotkeyMode => {
                config.hotkey_mode.should_import = !config.hotkey_mode.should_import
            }
            SettingType::Opacity => config.opacity.should_import = !config.opacity.should_import,
            SettingType::WindowSize => {
                config.window_size.should_import = !config.window_size.should_import
            }
            SettingType::CopyOnSelect => {
                config.copy_on_select.should_import = !config.copy_on_select.should_import
            }
            SettingType::CursorBlinking => {
                config.cursor_blinking.should_import = !config.cursor_blinking.should_import
            }
        }
    }

    pub fn should_import(&self, profile: &TerminalTypeAndProfile, setting: &SettingType) -> bool {
        let Some(config) = self.config(profile) else {
            log::warn!("Attempted to read should_import on an invalid profile!");
            return false;
        };
        match setting {
            SettingType::Theme => config.theme.should_import,
            SettingType::OptionAsMeta => config.option_as_meta.should_import,
            SettingType::MouseAndScrollReporting => config.mouse_and_scroll_reporting.should_import,
            SettingType::Font => config.font.should_import,
            SettingType::DefaultShell => config.default_shell.should_import,
            SettingType::WorkingDirectory => config.working_directory.should_import,
            SettingType::HotkeyMode => config.hotkey_mode.should_import,
            SettingType::Opacity => config.opacity.should_import,
            SettingType::WindowSize => config.window_size.should_import,
            SettingType::CopyOnSelect => config.copy_on_select.should_import,
            SettingType::CursorBlinking => config.cursor_blinking.should_import,
        }
    }

    pub fn write_theme(&self, profile: &TerminalTypeAndProfile) -> Option<ThemeType> {
        self.config(profile)
            .map(|config| config.write_theme())
            .unwrap_or_else(|| {
                log::warn!("Attempted to write the theme from an invalid profile.");
                None
            })
    }

    pub fn finished_searching_for_settings(&self) -> bool {
        TerminalType::iter()
            .filter(|terminal_type| {
                if !FeatureFlag::AlacrittySettingsImport.is_enabled() {
                    *terminal_type != TerminalType::Alacritty
                } else {
                    true
                }
            })
            .all(|terminal| self.parsed_terminals.contains_key(&terminal))
    }
}

impl Entity for ImportedConfigModel {
    type Event = CompletedParseEvent;
}

impl SingletonEntity for ImportedConfigModel {}
