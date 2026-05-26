use ::local_control::protocol::{
    AppearanceStateResult, BindingNameParams, KeybindingGetResult, KeybindingListResult,
    KeybindingSummary, SettingGetParams, SettingGetResult, SettingListResult, SettingSummary,
    ThemeListResult, ThemeStateResult, ThemeSummary,
};
use ::local_control::{ControlError, ErrorCode};
use serde::Serialize;
use serde_json::{json, Value};
use settings::Setting as _;
use warpui::keymap::DescriptionContext;
use warpui::{ModelContext, SingletonEntity};

use crate::local_control::LocalControlBridge;
use crate::settings::{
    derived_theme_kind, AccessibilitySettings, FontSettings, InputSettings, ThemeSettings,
};
use crate::themes::theme::ThemeKind;
use crate::user_config::WarpConfig;
use crate::util::bindings::trigger_to_keystroke;
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
    "local_control.allow_inside_warp_control",
    "local_control.allow_inside_warp_metadata_reads",
    "local_control.allow_inside_warp_underlying_data_reads",
    "local_control.allow_inside_warp_app_state_mutations",
    "local_control.allow_inside_warp_metadata_configuration_mutations",
    "local_control.allow_inside_warp_underlying_data_mutations",
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

pub(crate) fn theme_list(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    to_control_data(theme_list_result(ctx)?)
}


pub(crate) fn theme_get(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    to_control_data(theme_state_result(ctx)?)
}

pub(crate) fn appearance_get(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    to_control_data(appearance_state_result(ctx)?)
}

pub(crate) fn setting_list(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    to_control_data(setting_list_result(ctx)?)
}

pub(crate) fn setting_get(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<SettingGetParams>()?;
    to_control_data(setting_get_result(&params.key, ctx)?)
}

pub(crate) fn keybinding_list(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    to_control_data(KeybindingListResult {
        keybindings: keybinding_summaries(ctx),
    })
}

pub(crate) fn keybinding_get(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<BindingNameParams>()?;
    let keybinding = keybinding_summaries(ctx)
        .into_iter()
        .find(|summary| summary.name == params.binding_name)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("keybinding.get could not find {}", params.binding_name),
            )
        })?;
    to_control_data(KeybindingGetResult { keybinding })
}

pub(crate) fn theme_list_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ThemeListResult, ControlError> {
    let current_theme = active_theme_kind(ThemeSettings::as_ref(ctx), ctx);
    let mut themes = WarpConfig::as_ref(ctx)
        .theme_config()
        .theme_items()
        .map(|(kind, _)| ThemeSummary {
            name: public_theme_name(kind),
            is_current: *kind == current_theme,
        })
        .collect::<Vec<_>>();
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ThemeListResult { themes })
}


pub(crate) fn theme_state_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ThemeStateResult, ControlError> {
    let theme_settings = ThemeSettings::as_ref(ctx);
    let system_themes = theme_settings.selected_system_themes.value();
    Ok(ThemeStateResult {
        name: public_theme_name(theme_settings.theme_kind.value()),
        follow_system_theme: *theme_settings.use_system_theme.value(),
        light_theme: Some(public_theme_name(&system_themes.light)),
        dark_theme: Some(public_theme_name(&system_themes.dark)),
    })
}

pub(crate) fn appearance_state_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<AppearanceStateResult, ControlError> {
    let theme_settings = ThemeSettings::as_ref(ctx);
    let font_settings = FontSettings::as_ref(ctx);
    let window_settings = WindowSettings::as_ref(ctx);
    let system_themes = theme_settings.selected_system_themes.value();
    Ok(AppearanceStateResult {
        theme: Some(public_theme_name(theme_settings.theme_kind.value())),
        follow_system_theme: *theme_settings.use_system_theme.value(),
        light_theme: Some(public_theme_name(&system_themes.light)),
        dark_theme: Some(public_theme_name(&system_themes.dark)),
        font_size: rounded_u32(*font_settings.monospace_font_size.value()),
        ui_zoom_percent: Some(u32::from(*window_settings.zoom_level.value())),
    })
}

pub(crate) fn setting_list_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingListResult, ControlError> {
    let settings = ALLOWLISTED_SETTING_KEYS
        .iter()
        .map(|key| setting_summary_for_key(key, ctx))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SettingListResult { settings })
}

pub(crate) fn setting_get_result(
    key: &str,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingGetResult, ControlError> {
    Ok(SettingGetResult {
        setting: setting_summary_for_key(key, ctx)?,
    })
}

pub(crate) fn rejected_setting_key(key: &str) -> ControlError {
    if PRIVATE_OR_SENSITIVE_SETTING_KEYS.contains(&key) {
        return ControlError::new(
            ErrorCode::NotAllowlisted,
            format!("{key} is private or sensitive and is not available through local control"),
        );
    }
    ControlError::new(
        ErrorCode::NotAllowlisted,
        format!("{key} is not an allowlisted local-control setting"),
    )
}

fn setting_summary_for_key(
    key: &str,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingSummary, ControlError> {
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

fn setting_summary(key: &str, value: Value, value_type: &str) -> SettingSummary {
    SettingSummary {
        key: key.to_owned(),
        value,
        value_type: value_type.to_owned(),
    }
}

fn public_theme_name(theme: &ThemeKind) -> String {
    match theme {
        ThemeKind::Custom(custom) | ThemeKind::CustomBase16(custom) => custom.name(),
        ThemeKind::InMemory(_) => "In-memory theme".to_owned(),
        _ => theme.to_string(),
    }
}

fn active_theme_kind(
    theme_settings: &ThemeSettings,
    ctx: &ModelContext<LocalControlBridge>,
) -> ThemeKind {
    derived_theme_kind(theme_settings, ctx.system_theme())
}

fn rounded_u32(value: f32) -> Option<u32> {
    if value.is_finite() && value >= 0.0 && value <= u32::MAX as f32 {
        return Some(value.round() as u32);
    }
    None
}

fn keybinding_summaries(ctx: &mut ModelContext<LocalControlBridge>) -> Vec<KeybindingSummary> {
    let mut keybindings = ctx
        .editable_bindings()
        .map(|binding| {
            let keystroke = trigger_to_keystroke(binding.trigger);
            KeybindingSummary {
                name: binding.name.to_owned(),
                description: binding
                    .description
                    .materialized(ctx)
                    .in_context(DescriptionContext::Default)
                    .to_owned(),
                group: binding.group.map(str::to_owned),
                keystroke: keystroke.as_ref().map(|keystroke| keystroke.displayed()),
                normalized_keystroke: keystroke.map(|keystroke| keystroke.normalized()),
            }
        })
        .collect::<Vec<_>>();
    keybindings.sort_by(|left, right| {
        left.description
            .cmp(&right.description)
            .then_with(|| left.name.cmp(&right.name))
    });
    keybindings
        .dedup_by(|left, right| left.name == right.name && left.description == right.description);
    keybindings
}

fn to_control_data<T: Serialize>(value: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(value).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control response",
            err.to_string(),
        )
    })
}
