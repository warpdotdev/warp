//! Metadata/configuration mutation handlers for local-control actions.
use std::str::FromStr as _;

use ::local_control::protocol::{
    ActionParams, PaneTarget, TabTarget, TargetSelector, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use settings::Setting as _;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::{ModelContext, SingletonEntity as _, ViewHandle, WindowId};

use crate::local_control::LocalControlBridge;
use crate::pane_group::{PaneGroup, PaneId};
use crate::settings::{AccessibilitySettings, FontSettings, InputSettings, ThemeSettings};
use crate::tab::SelectedTabColor;
use crate::themes::theme::{SelectedSystemThemes, ThemeKind};
use crate::user_config::WarpConfig;
use crate::window_settings::ZoomLevel;
use crate::workspace::Workspace;
use crate::WindowSettings;

const ALLOWLISTED_SETTING_KEYS: &[&str] = &[
    "accessibility.accessibility_verbosity",
    "appearance.text.font_name",
    "appearance.text.font_size",
    "appearance.themes.dark_theme",
    "appearance.themes.light_theme",
    "appearance.themes.system_theme",
    "appearance.themes.theme",
    "appearance.window.zoom_level",
    "terminal.input.error_underlining_enabled",
    "terminal.input.syntax_highlighting",
];

const PRIVATE_OR_SENSITIVE_SETTING_KEYS: &[&str] = &[
    "local_control.allow_outside_warp_control",
    "local_control.allow_outside_warp_metadata_reads",
    "local_control.allow_outside_warp_underlying_data_reads",
    "local_control.allow_outside_warp_app_state_mutations",
    "local_control.allow_outside_warp_metadata_configuration_mutations",
    "local_control.allow_outside_warp_underlying_data_mutations",
    "terminal.input.autosuggestion_accepted_count",
    "terminal.input.completions_menu_height",
    "terminal.input.completions_menu_width",
    "terminal.input.inline_menu_custom_content_heights",
    "terminal.input.workflows_box_expanded",
];

const DEBUG_SETTING_KEYS: &[&str] = &[
    "debug.are_in_band_generators_for_all_sessions_enabled",
    "debug.force_disable_in_band_generators",
    "debug.is_shell_debug_mode_enabled",
    "debug.recording_mode",
    "debug.show_memory_stats",
];

const DERIVED_OR_COMPOUND_SETTING_KEYS: &[&str] = &[
    "appearance.themes.active_theme",
    "appearance.themes.selected_system_themes",
    "terminal.input.input_box_type",
];

#[derive(Clone)]
struct TabEntry {
    window_id: WindowId,
    index: usize,
    pane_group: ViewHandle<PaneGroup>,
}

#[derive(Clone)]
struct PaneEntry {
    tab_id: String,
    pane_group: ViewHandle<PaneGroup>,
    pane_id: PaneId,
}

pub(crate) fn tab_rename(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let title = rename_title(action)?;
    let entry = select_single_tab_entry(target, ActionKind::TabRename, ctx)?;
    let tab_id = entry.pane_group.id().to_string();
    entry.pane_group.update(ctx, |pane_group, ctx| {
        pane_group.set_title(&title, ctx);
    });
    Ok(tab_mutation_result(
        instance_id,
        ActionKind::TabRename,
        tab_id,
        entry.window_id,
    ))
}

pub(crate) fn tab_reset_name(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_tab_entry(target, ActionKind::TabResetName, ctx)?;
    let tab_id = entry.pane_group.id().to_string();
    entry.pane_group.update(ctx, |pane_group, ctx| {
        pane_group.clear_title(ctx);
    });
    Ok(tab_mutation_result(
        instance_id,
        ActionKind::TabResetName,
        tab_id,
        entry.window_id,
    ))
}

pub(crate) fn tab_color_set(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let color = color_value(action)?;
    let entry = select_single_tab_entry(target, ActionKind::TabColorSet, ctx)?;
    set_tab_color(entry.clone(), SelectedTabColor::Color(color), ctx)?;
    Ok(tab_mutation_result(
        instance_id,
        ActionKind::TabColorSet,
        entry.pane_group.id().to_string(),
        entry.window_id,
    ))
}

pub(crate) fn tab_color_clear(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_tab_entry(target, ActionKind::TabColorClear, ctx)?;
    set_tab_color(entry.clone(), SelectedTabColor::Cleared, ctx)?;
    Ok(tab_mutation_result(
        instance_id,
        ActionKind::TabColorClear,
        entry.pane_group.id().to_string(),
        entry.window_id,
    ))
}

pub(crate) fn pane_rename(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let title = rename_title(action)?;
    let entry = select_single_pane_entry(target, ActionKind::PaneRename, ctx)?;
    set_pane_name(&entry, Some(title), ctx)?;
    Ok(pane_mutation_result(
        instance_id,
        ActionKind::PaneRename,
        entry.pane_id,
        entry.tab_id,
    ))
}

pub(crate) fn pane_reset_name(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_pane_entry(target, ActionKind::PaneResetName, ctx)?;
    set_pane_name(&entry, None, ctx)?;
    Ok(pane_mutation_result(
        instance_id,
        ActionKind::PaneResetName,
        entry.pane_id,
        entry.tab_id,
    ))
}

pub(crate) fn theme_set(
    action_kind: ActionKind,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    match action_kind {
        ActionKind::ThemeSet => set_theme(theme_name(action)?, ctx)?,
        ActionKind::ThemeSystemSet => set_system_theme(boolean_value(action)?, ctx)?,
        ActionKind::ThemeLightSet => set_system_theme_variant(theme_name(action)?, true, ctx)?,
        ActionKind::ThemeDarkSet => set_system_theme_variant(theme_name(action)?, false, ctx)?,
        _ => {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                format!("{} is not a theme mutation", action_kind.as_str()),
            ));
        }
    }
    Ok(acknowledgement(action_kind))
}

pub(crate) fn appearance_mutation(
    action_kind: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    match action_kind {
        ActionKind::AppearanceFontSizeIncrease => {
            adjust_font_size(FontSizeAdjustment::Increase, ctx)?
        }
        ActionKind::AppearanceFontSizeDecrease => {
            adjust_font_size(FontSizeAdjustment::Decrease, ctx)?
        }
        ActionKind::AppearanceFontSizeReset => adjust_font_size(FontSizeAdjustment::Reset, ctx)?,
        ActionKind::AppearanceZoomIncrease => adjust_zoom(true, ctx)?,
        ActionKind::AppearanceZoomDecrease => adjust_zoom(false, ctx)?,
        ActionKind::AppearanceZoomReset => reset_zoom(ctx)?,
        _ => {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                format!("{} is not an appearance mutation", action_kind.as_str()),
            ));
        }
    }
    Ok(acknowledgement(action_kind))
}

pub(crate) fn setting_set(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let (key, value) = key_value(action)?;
    set_allowlisted_setting(&key, value, ctx)?;
    Ok(json!({
        "action": ActionKind::SettingSet.as_str(),
        "setting": setting_summary_for_key(&key, ctx)?,
    }))
}

pub(crate) fn setting_toggle(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let key = key_value_key(action)?;
    let current = setting_summary_for_key(&key, ctx)?;
    let Some(value) = current["value"].as_bool() else {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{key} is not a boolean setting and cannot be toggled"),
        ));
    };
    set_allowlisted_setting(&key, json!(!value), ctx)?;
    Ok(json!({
        "action": ActionKind::SettingToggle.as_str(),
        "setting": setting_summary_for_key(&key, ctx)?,
    }))
}

fn rename_title(action: &::local_control::Action) -> Result<String, ControlError> {
    match action_params(action)? {
        ActionParams::Rename { title } if !title.trim().is_empty() => Ok(title.trim().to_owned()),
        ActionParams::Rename { .. } => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} title cannot be empty", action.kind.as_str()),
        )),
        _ => Err(unexpected_params(action.kind, "rename")),
    }
}

fn color_value(action: &::local_control::Action) -> Result<AnsiColorIdentifier, ControlError> {
    match action_params(action)? {
        ActionParams::ColorValue { color } => AnsiColorIdentifier::from_str(&color).map_err(|_| {
            ControlError::new(
                ErrorCode::InvalidParams,
                format!("{color} is not a supported tab color"),
            )
        }),
        _ => Err(unexpected_params(action.kind, "color_value")),
    }
}

fn theme_name(action: &::local_control::Action) -> Result<String, ControlError> {
    match action_params(action)? {
        ActionParams::ThemeName { theme_name } if !theme_name.trim().is_empty() => {
            Ok(theme_name.trim().to_owned())
        }
        ActionParams::ThemeName { .. } => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} theme name cannot be empty", action.kind.as_str()),
        )),
        _ => Err(unexpected_params(action.kind, "theme_name")),
    }
}

fn boolean_value(action: &::local_control::Action) -> Result<bool, ControlError> {
    match action_params(action)? {
        ActionParams::BooleanValue { value } => Ok(value),
        _ => Err(unexpected_params(action.kind, "boolean_value")),
    }
}

fn key_value(
    action: &::local_control::Action,
) -> Result<(String, serde_json::Value), ControlError> {
    match action_params(action)? {
        ActionParams::KeyValue { key, value } => Ok((key, value)),
        _ => Err(unexpected_params(action.kind, "key_value")),
    }
}

fn key_value_key(action: &::local_control::Action) -> Result<String, ControlError> {
    match action_params(action)? {
        ActionParams::Key { key } => Ok(key),
        _ => Err(unexpected_params(action.kind, "key")),
    }
}

fn action_params(action: &::local_control::Action) -> Result<ActionParams, ControlError> {
    serde_json::from_value(action.params.clone()).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            format!("{} parameters are invalid", action.kind.as_str()),
            err.to_string(),
        )
    })
}

fn unexpected_params(action: ActionKind, expected: &str) -> ControlError {
    ControlError::new(
        ErrorCode::InvalidParams,
        format!("{} requires {expected} parameters", action.as_str()),
    )
}

fn select_single_tab_entry(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<TabEntry, ControlError> {
    if target.pane.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept pane selectors", action.as_str()),
        ));
    }
    let entries = select_tab_entries(target, action, ctx)?;
    match entries.as_slice() {
        [entry] => Ok(entry.clone()),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a target tab", action.as_str()),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("{} resolved multiple tabs", action.as_str()),
        )),
    }
}

fn select_single_pane_entry(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<PaneEntry, ControlError> {
    let tab = select_single_tab_entry_for_pane(target, action, ctx)?;
    let entries = pane_entries_for_tab(tab, ctx)?;
    let selected = match target.pane.as_ref() {
        None | Some(PaneTarget::Active) => {
            let focused = entries
                .first()
                .map(|entry| {
                    entry
                        .pane_group
                        .read(ctx, |pane_group, ctx| pane_group.focused_pane_id(ctx))
                })
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::MissingTarget,
                        format!("{} requires an active pane", action.as_str()),
                    )
                })?;
            entries
                .into_iter()
                .filter(|entry| entry.pane_id == focused)
                .collect::<Vec<_>>()
        }
        Some(PaneTarget::Id { id }) => entries
            .into_iter()
            .filter(|entry| entry.pane_id.to_string() == id.0)
            .collect::<Vec<_>>(),
        Some(PaneTarget::Index { index }) => entries
            .into_iter()
            .enumerate()
            .filter_map(|(entry_index, entry)| (entry_index as u32 == *index).then_some(entry))
            .collect::<Vec<_>>(),
    };
    match selected.as_slice() {
        [entry] => Ok(entry.clone()),
        [] => Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{} cannot resolve the requested pane", action.as_str()),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("{} resolved multiple panes", action.as_str()),
        )),
    }
}

fn select_single_tab_entry_for_pane(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<TabEntry, ControlError> {
    let entries = select_tab_entries(target, action, ctx)?;
    match entries.as_slice() {
        [entry] => Ok(entry.clone()),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a target tab", action.as_str()),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("{} resolved multiple tabs", action.as_str()),
        )),
    }
}

fn select_tab_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<TabEntry>, ControlError> {
    let window_ids = select_window_ids(target, action, ctx)?;
    let entries = tab_entries_for_windows(window_ids, ctx);
    match target.tab.as_ref() {
        None | Some(TabTarget::Active) => Ok(entries
            .into_iter()
            .filter(|entry| {
                workspace_for_window(entry.window_id, action, ctx)
                    .map(|workspace| {
                        workspace.read(ctx, |workspace, _| {
                            entry.index == workspace.active_tab_index()
                        })
                    })
                    .unwrap_or(false)
            })
            .collect()),
        Some(TabTarget::Id { id }) => Ok(entries
            .into_iter()
            .filter(|entry| entry.pane_group.id().to_string() == id.0)
            .collect()),
        Some(TabTarget::Index { index }) => Ok(entries
            .into_iter()
            .filter(|entry| entry.index as u32 == *index)
            .collect()),
        Some(TabTarget::Title { title }) => Ok(entries
            .into_iter()
            .filter(|entry| {
                entry.pane_group.read(ctx, |pane_group, ctx| {
                    pane_group.display_title(ctx) == *title
                })
            })
            .collect()),
    }
}

fn select_window_ids(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<WindowId>, ControlError> {
    match target.window.as_ref() {
        None | Some(WindowTarget::Active) => Ok(vec![active_window_id(action, ctx)?]),
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .map(|window_id| vec![window_id])
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested window id", action.as_str()),
                )
            }),
        Some(WindowTarget::Index { .. } | WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active and opaque window id selectors",
                action.as_str()
            ),
        )),
    }
}

fn tab_entries_for_windows(
    window_ids: Vec<WindowId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<TabEntry> {
    window_ids
        .into_iter()
        .filter_map(|window_id| {
            let workspace = ctx
                .views_of_type::<Workspace>(window_id)
                .and_then(|workspaces| workspaces.into_iter().next())?;
            Some(workspace.read(ctx, |workspace, _| {
                workspace
                    .tab_views()
                    .enumerate()
                    .map(|(index, pane_group)| TabEntry {
                        window_id,
                        index,
                        pane_group: pane_group.clone(),
                    })
                    .collect::<Vec<_>>()
            }))
        })
        .flatten()
        .collect()
}

fn pane_entries_for_tab(
    tab: TabEntry,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<PaneEntry>, ControlError> {
    let tab_id = tab.pane_group.id().to_string();
    let pane_ids = tab
        .pane_group
        .read(ctx, |pane_group, _| pane_group.visible_pane_ids());
    if pane_ids.is_empty() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            "target tab has no visible panes",
        ));
    }
    Ok(pane_ids
        .into_iter()
        .map(|pane_id| PaneEntry {
            tab_id: tab_id.clone(),
            pane_group: tab.pane_group.clone(),
            pane_id,
        })
        .collect())
}

fn active_window_id(
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<WindowId, ControlError> {
    ctx.windows().active_window().ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires an active Warp window", action.as_str()),
        )
    })
}

fn workspace_for_window(
    window_id: WindowId,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!(
                    "{} requires a workspace in the target window",
                    action.as_str()
                ),
            )
        })
}

fn set_tab_color(
    entry: TabEntry,
    color: SelectedTabColor,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let workspace = workspace_for_window(entry.window_id, ActionKind::TabColorSet, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.set_tab_color(entry.index, color, ctx);
    });
    Ok(())
}

fn set_pane_name(
    entry: &PaneEntry,
    title: Option<String>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    entry.pane_group.update(ctx, |pane_group, ctx| {
        let Some(pane) = pane_group.pane_by_id(entry.pane_id) else {
            return Err(ControlError::new(
                ErrorCode::StaleTarget,
                "pane metadata mutation cannot resolve the requested pane",
            ));
        };
        pane.pane_configuration().update(ctx, |configuration, ctx| {
            if let Some(title) = title {
                configuration.set_custom_vertical_tabs_title(title, ctx);
            } else {
                configuration.clear_custom_vertical_tabs_title(ctx);
            }
        });
        ctx.emit(crate::pane_group::Event::AppStateChanged);
        Ok(())
    })
}

fn set_theme(
    theme_name: String,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let theme = theme_kind_for_name(&theme_name, ctx)?;
    ThemeSettings::handle(ctx)
        .update(ctx, |theme_settings, ctx| {
            theme_settings.use_system_theme.set_value(false, ctx)?;
            theme_settings.theme_kind.set_value(theme, ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::ThemeSet, err))
}

fn set_system_theme(
    enabled: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    ThemeSettings::handle(ctx)
        .update(ctx, |theme_settings, ctx| {
            theme_settings.use_system_theme.set_value(enabled, ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::ThemeSystemSet, err))
}

fn set_system_theme_variant(
    theme_name: String,
    light: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let theme = theme_kind_for_name(&theme_name, ctx)?;
    let action = if light {
        ActionKind::ThemeLightSet
    } else {
        ActionKind::ThemeDarkSet
    };
    ThemeSettings::handle(ctx)
        .update(ctx, |theme_settings, ctx| {
            let current = theme_settings.selected_system_themes.value().clone();
            let next = if light {
                SelectedSystemThemes {
                    light: theme,
                    dark: current.dark,
                }
            } else {
                SelectedSystemThemes {
                    light: current.light,
                    dark: theme,
                }
            };
            theme_settings.selected_system_themes.set_value(next, ctx)
        })
        .map_err(|err| settings_write_error(action, err))
}

enum FontSizeAdjustment {
    Increase,
    Decrease,
    Reset,
}

fn adjust_font_size(
    adjustment: FontSizeAdjustment,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let current = *FontSettings::as_ref(ctx).monospace_font_size.value();
    let next = match adjustment {
        FontSizeAdjustment::Increase => (current + 1.0).clamp(5.0, 25.0),
        FontSizeAdjustment::Decrease => (current - 1.0).clamp(5.0, 25.0),
        FontSizeAdjustment::Reset => crate::settings::MonospaceFontSize::default_value(),
    };
    FontSettings::handle(ctx)
        .update(ctx, |font_settings, ctx| {
            font_settings.monospace_font_size.set_value(next, ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::AppearanceFontSizeReset, err))
}

fn adjust_zoom(
    increase: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let current = *WindowSettings::as_ref(ctx).zoom_level.value();
    let next = adjacent_zoom_level(current, increase);
    WindowSettings::handle(ctx)
        .update(ctx, |window_settings, ctx| {
            window_settings.zoom_level.set_value(next, ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::AppearanceZoomReset, err))
}

fn reset_zoom(ctx: &mut ModelContext<LocalControlBridge>) -> Result<(), ControlError> {
    WindowSettings::handle(ctx)
        .update(ctx, |window_settings, ctx| {
            window_settings
                .zoom_level
                .set_value(ZoomLevel::default_value(), ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::AppearanceZoomReset, err))
}

fn adjacent_zoom_level(current: u16, increase: bool) -> u16 {
    let default_index = ZoomLevel::VALUES
        .iter()
        .position(|zoom| *zoom == ZoomLevel::default_value())
        .unwrap_or(0);
    let current_index = ZoomLevel::VALUES
        .iter()
        .position(|zoom| *zoom == current)
        .unwrap_or(default_index);
    let next_index = if increase {
        (current_index + 1).min(ZoomLevel::VALUES.len() - 1)
    } else {
        current_index.saturating_sub(1)
    };
    ZoomLevel::VALUES[next_index]
}

fn set_allowlisted_setting(
    key: &str,
    value: serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if !ALLOWLISTED_SETTING_KEYS.contains(&key) {
        return Err(rejected_setting_key(key));
    }
    match key {
        "appearance.themes.theme" => set_theme(string_setting_value(key, &value)?, ctx),
        "appearance.themes.system_theme" => set_system_theme(bool_setting_value(key, &value)?, ctx),
        "appearance.themes.light_theme" => {
            set_system_theme_variant(string_setting_value(key, &value)?, true, ctx)
        }
        "appearance.themes.dark_theme" => {
            set_system_theme_variant(string_setting_value(key, &value)?, false, ctx)
        }
        "appearance.text.font_name" => {
            let font_name = string_setting_value(key, &value)?;
            if font_name.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "appearance.text.font_name cannot be empty",
                ));
            }
            FontSettings::handle(ctx)
                .update(ctx, |font_settings, ctx| {
                    font_settings.monospace_font_name.set_value(font_name, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.text.font_size" => {
            let font_size = valid_font_size(u32_setting_value(key, &value)?)?;
            FontSettings::handle(ctx)
                .update(ctx, |font_settings, ctx| {
                    font_settings.monospace_font_size.set_value(font_size, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.window.zoom_level" => {
            let zoom_level = valid_zoom_level(u32_setting_value(key, &value)?)?;
            WindowSettings::handle(ctx)
                .update(ctx, |window_settings, ctx| {
                    window_settings.zoom_level.set_value(zoom_level, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "terminal.input.syntax_highlighting" => {
            let enabled = bool_setting_value(key, &value)?;
            InputSettings::handle(ctx)
                .update(ctx, |input_settings, ctx| {
                    input_settings.syntax_highlighting.set_value(enabled, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "terminal.input.error_underlining_enabled" => {
            let enabled = bool_setting_value(key, &value)?;
            InputSettings::handle(ctx)
                .update(ctx, |input_settings, ctx| {
                    input_settings.error_underlining.set_value(enabled, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "accessibility.accessibility_verbosity" => {
            let verbosity = accessibility_verbosity_value(key, &value)?;
            AccessibilitySettings::handle(ctx)
                .update(ctx, |accessibility_settings, ctx| {
                    accessibility_settings
                        .a11y_verbosity
                        .set_value(verbosity, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        _ => Err(rejected_setting_key(key)),
    }
}

fn setting_summary_for_key(
    key: &str,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if !ALLOWLISTED_SETTING_KEYS.contains(&key) {
        return Err(rejected_setting_key(key));
    }
    let theme_settings = ThemeSettings::as_ref(ctx);
    let font_settings = FontSettings::as_ref(ctx);
    let input_settings = InputSettings::as_ref(ctx);
    let accessibility_settings = AccessibilitySettings::as_ref(ctx);
    let window_settings = WindowSettings::as_ref(ctx);
    match key {
        "appearance.themes.theme" => Ok(setting_summary(
            key,
            json!(public_theme_name(theme_settings.theme_kind.value())),
            "string",
        )),
        "appearance.themes.system_theme" => Ok(setting_summary(
            key,
            json!(*theme_settings.use_system_theme.value()),
            "bool",
        )),
        "appearance.themes.light_theme" => Ok(setting_summary(
            key,
            json!(public_theme_name(
                &theme_settings.selected_system_themes.value().light
            )),
            "string",
        )),
        "appearance.themes.dark_theme" => Ok(setting_summary(
            key,
            json!(public_theme_name(
                &theme_settings.selected_system_themes.value().dark
            )),
            "string",
        )),
        "appearance.text.font_name" => Ok(setting_summary(
            key,
            json!(font_settings.monospace_font_name.value()),
            "string",
        )),
        "appearance.text.font_size" => Ok(setting_summary(
            key,
            json!(*font_settings.monospace_font_size.value()),
            "number",
        )),
        "appearance.window.zoom_level" => Ok(setting_summary(
            key,
            json!(*window_settings.zoom_level.value()),
            "number",
        )),
        "terminal.input.syntax_highlighting" => Ok(setting_summary(
            key,
            json!(*input_settings.syntax_highlighting.value()),
            "bool",
        )),
        "terminal.input.error_underlining_enabled" => Ok(setting_summary(
            key,
            json!(*input_settings.error_underlining.value()),
            "bool",
        )),
        "accessibility.accessibility_verbosity" => Ok(setting_summary(
            key,
            json!(format!(
                "{:?}",
                accessibility_settings.a11y_verbosity.value()
            )),
            "string",
        )),
        _ => Err(rejected_setting_key(key)),
    }
}

fn rejected_setting_key(key: &str) -> ControlError {
    if PRIVATE_OR_SENSITIVE_SETTING_KEYS.contains(&key) {
        return ControlError::new(
            ErrorCode::NotAllowlisted,
            format!("{key} is private or sensitive and is not available through local control"),
        );
    }
    if DEBUG_SETTING_KEYS.contains(&key) {
        return ControlError::new(
            ErrorCode::NotAllowlisted,
            format!("{key} is debug-only and is not available through local control"),
        );
    }
    if DERIVED_OR_COMPOUND_SETTING_KEYS.contains(&key) {
        return ControlError::new(
            ErrorCode::NotAllowlisted,
            format!(
                "{key} is derived or compound state and is not available through local control"
            ),
        );
    }
    ControlError::new(
        ErrorCode::NotAllowlisted,
        format!("{key} is not an allowlisted local-control setting"),
    )
}

fn theme_kind_for_name(
    name: &str,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<ThemeKind, ControlError> {
    let matches = WarpConfig::as_ref(ctx)
        .theme_config()
        .theme_items()
        .filter_map(|(kind, _)| (public_theme_name(kind) == name).then_some(kind.clone()))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [theme] => Ok(theme.clone()),
        [] => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{name} is not an available theme"),
        )),
        _ => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{name} matches multiple themes"),
        )),
    }
}

fn public_theme_name(theme: &ThemeKind) -> String {
    match theme {
        ThemeKind::Custom(custom) | ThemeKind::CustomBase16(custom) => custom.name(),
        ThemeKind::InMemory(_) => "In-memory theme".to_owned(),
        _ => theme.to_string(),
    }
}

fn valid_font_size(value: u32) -> Result<f32, ControlError> {
    if (5..=25).contains(&value) {
        return Ok(value as f32);
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        "font size must be between 5 and 25",
    ))
}

fn valid_zoom_level(value: u32) -> Result<u16, ControlError> {
    let value = u16::try_from(value).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "zoom level is outside the supported range",
            err.to_string(),
        )
    })?;
    if ZoomLevel::VALUES.contains(&value) {
        return Ok(value);
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        "zoom level must be one of the supported zoom percentages",
    ))
}

fn bool_setting_value(key: &str, value: &serde_json::Value) -> Result<bool, ControlError> {
    value.as_bool().ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("{key} requires a boolean value"),
        )
    })
}

fn string_setting_value(key: &str, value: &serde_json::Value) -> Result<String, ControlError> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("{key} requires a string value"),
        )
    })
}

fn u32_setting_value(key: &str, value: &serde_json::Value) -> Result<u32, ControlError> {
    if let Some(value) = value.as_u64().and_then(|value| u32::try_from(value).ok()) {
        return Ok(value);
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!("{key} requires a non-negative integer value"),
    ))
}

fn accessibility_verbosity_value(
    key: &str,
    value: &serde_json::Value,
) -> Result<warpui::accessibility::AccessibilityVerbosity, ControlError> {
    match string_setting_value(key, value)?.as_str() {
        "Verbose" | "verbose" | "VERBOSE" => {
            Ok(warpui::accessibility::AccessibilityVerbosity::Verbose)
        }
        "Concise" | "concise" | "CONCISE" => {
            Ok(warpui::accessibility::AccessibilityVerbosity::Concise)
        }
        _ => Err(ControlError::new(
            ErrorCode::InvalidParams,
            "accessibility.accessibility_verbosity must be Verbose or Concise",
        )),
    }
}

fn setting_summary(key: &str, value: serde_json::Value, value_type: &str) -> serde_json::Value {
    json!({
        "key": key,
        "value": value,
        "value_type": value_type,
    })
}

fn settings_write_error(action: ActionKind, err: anyhow::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::Internal,
        format!("{} failed to update app settings", action.as_str()),
        err.to_string(),
    )
}

fn acknowledgement(action: ActionKind) -> serde_json::Value {
    json!({
        "action": action.as_str(),
        "ok": true,
    })
}

fn tab_mutation_result(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    tab_id: String,
    window_id: WindowId,
) -> serde_json::Value {
    json!({
        "action": action.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "window_id": window_id.to_string(),
        "tab_id": tab_id,
    })
}

fn pane_mutation_result(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    pane_id: PaneId,
    tab_id: String,
) -> serde_json::Value {
    json!({
        "action": action.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "tab_id": tab_id,
        "pane_id": pane_id.to_string(),
    })
}
