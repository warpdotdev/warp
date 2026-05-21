use itertools::Itertools;
use warp_core::{settings::Setting, ui::appearance::Appearance};

use warpui::{
    elements::{
        Border, Container, CornerRadius, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    keymap::Keystroke,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
        radio_buttons::{self, RadioButtonItem},
    },
    Element, Entity, ModelContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext,
};

use warpui::ui_components::radio_buttons::RadioButtonStateHandle;

use crate::{
    report_if_error, send_telemetry_from_ctx,
    settings::{
        import::{
            config::{Config, ParsedTerminalSetting, SettingType},
            model::{ImportedConfigModel, TerminalTypeAndProfile},
        },
        AppEditorSettings, CursorBlink, FontSettings, GlobalHotkeyMode, SelectionSettings,
        ThemeSettings,
    },
    terminal::{
        alt_screen_reporting::AltScreenReporting, keys_settings::KeysSettings,
        session_settings::SessionSettings,
    },
    themes::theme::{CustomTheme, SelectedSystemThemes, ThemeKind},
    ui_components::blended_colors,
    user_config::{self, WarpConfig},
    window_settings::WindowSettings,
    GlobalResourceHandlesProvider, TelemetryEvent,
};

use super::config::{QuakeModeWindow, ThemeType};

// UI does not scale, so we set a fixed size for all text.
const FONT_SIZE: f32 = 14.;
const DROPDOWN_BORDER_RADIUS: f32 = 8.;
const DROPDOWN_HORIZONTAL_PADDING: f32 = 16.;
const DROPDOWN_VERTICAL_PADDING: f32 = 12.;
const DROPDOWN_HORIZONTAL_MARGIN: f32 = 16.;
const BLOCK_TOP_MARGIN: f32 = 32.;
const DROPDOWN_BOTTOM_MARGIN: f32 = 16.;

const IMPORT_BUTTON_WIDTH: f32 = 80.;
const IMPORT_BUTTON_HEIGHT: f32 = 40.;

const RESET_BUTTON_WIDTH: f32 = 180.;
const RESET_BUTTON_HEIGHT: f32 = 40.;

const BUTTON_SPACING: f32 = 16.;

const CHECKBOX_VERTICAL_PADDING: f32 = 16.;
const CHECKBOX_HORIZONTAL_PADDING: f32 = 32.;
const CHECKBOX_SIZE: f32 = 16.;
const CHECKBOX_SPACING: f32 = 10.;
const NUM_COLUMNS: usize = 3;

/// Settings that only take effect on a new session
const NEW_SESSION_SETTINGS: [SettingType; 3] = [
    SettingType::WindowSize,
    SettingType::DefaultShell,
    SettingType::WorkingDirectory,
];

#[derive(Debug)]
pub enum SettingsImportAction {
    ImportButtonClicked,
    ResetButtonClicked,
    SetSelectedConfig(usize),
    ToggleSetting(usize, SettingType),
}

enum State {
    Loading,
    Failed,
    Active,
    Completed { imported_idx: Option<usize> },
}

impl State {
    fn is_complete(&self) -> bool {
        match self {
            State::Active | State::Loading => false,
            State::Failed | State::Completed { .. } => true,
        }
    }
}

pub struct SettingsImportView {
    configs: Vec<ConfigMenuItem>,
    import_button_handle: MouseStateHandle,
    skip_button_handle: MouseStateHandle,
    radio_button_state: RadioButtonStateHandle,
    radio_button_mouse_states: Vec<MouseStateHandle>,
    state: State,
}

pub struct ConfigMenuItem {
    config_name: String,
    description: Option<String>,
    menu_item_mouse_state: MouseStateHandle,
    expanded: bool,
    terminal_type_and_profile: TerminalTypeAndProfile,
    settings: Vec<ToggleableSetting>,
}

pub struct ToggleableSetting {
    pub setting_type: SettingType,
    pub checkbox_handle: MouseStateHandle,
}

impl ToggleableSetting {
    pub fn new(setting_type: SettingType) -> Self {
        ToggleableSetting {
            setting_type,
            checkbox_handle: Default::default(),
        }
    }
}

impl SettingsImportView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model_handle = ImportedConfigModel::handle(ctx);
        let is_complete = model_handle.as_ref(ctx).finished_searching_for_settings();

        ctx.subscribe_to_model(&model_handle, |me, model_handle, _, ctx| {
            me.handle_completed_parse_event(ctx, model_handle);
        });

        Self {
            configs: if is_complete {
                Self::create_menu_items(model_handle.as_ref(ctx).configs())
            } else {
                vec![]
            },
            import_button_handle: Default::default(),
            skip_button_handle: Default::default(),
            radio_button_mouse_states: if is_complete {
                model_handle
                    .as_ref(ctx)
                    .configs()
                    .map(|_| MouseStateHandle::default())
                    .collect()
            } else {
                vec![]
            },
            radio_button_state: Default::default(),
            // We don't make the block if we finish parsing before the setup guide is initialized
            // but there are no configs, so we are guaranteed that if we are here, we are
            // either still loading or active.
            state: if is_complete {
                State::Active
            } else {
                State::Loading
            },
        }
    }

    fn create_menu_items<'a>(
        configs: impl Iterator<Item = (TerminalTypeAndProfile, &'a Config)>,
    ) -> Vec<ConfigMenuItem> {
        configs
            .map(|(terminal, config)| ConfigMenuItem {
                config_name: config.terminal_name.clone(),
                description: config.description.clone(),
                menu_item_mouse_state: Default::default(),
                expanded: false,
                settings: create_toggleable_settings(config),
                terminal_type_and_profile: terminal,
            })
            .collect()
    }

    fn set_configs<'a>(
        &mut self,
        configs: impl Iterator<Item = (TerminalTypeAndProfile, &'a Config)>,
    ) {
        self.configs = Self::create_menu_items(configs);
    }

    fn handle_completed_parse_event(
        &mut self,
        ctx: &mut ViewContext<Self>,
        model_handle: ModelHandle<ImportedConfigModel>,
    ) {
        if model_handle.as_ref(ctx).finished_searching_for_settings()
            && !matches!(self.state, State::Failed)
        {
            self.radio_button_mouse_states = model_handle
                .as_ref(ctx)
                .configs()
                .map(|_| MouseStateHandle::default())
                .collect();
            self.set_configs(model_handle.as_ref(ctx).configs());
            if self.configs.is_empty() {
                ctx.emit(SettingsImportEvent::NoConfigsFound);
                self.state = State::Failed;
            } else {
                self.state = State::Active;
            }

            ctx.notify();
        }
    }

    fn render_secondary_text(
        &self,
        appearance: &Appearance,
        name: impl Into<std::borrow::Cow<'static, str>>,
    ) -> Box<dyn warpui::Element> {
        let theme = appearance.theme();
        let font_color = theme.disabled_text_color(theme.background());
        let font_family = appearance.monospace_font_family();
        Shrinkable::new(
            1.0,
            Container::new(
                Text::new_inline(name, font_family, FONT_SIZE)
                    .with_color(font_color.into_solid())
                    .finish(),
            )
            .with_horizontal_margin(4.)
            .finish(),
        )
        .finish()
    }

    fn render_import_button(
        &self,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let model = ImportedConfigModel::as_ref(app);
        let button = if self
            .radio_button_state
            .get_selected_idx()
            .map(|idx| {
                self.configs[idx].settings.iter().any(|setting| {
                    model.should_import(
                        &self.configs[idx].terminal_type_and_profile,
                        &setting.setting_type,
                    )
                })
            })
            .unwrap_or(false)
        {
            appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.import_button_handle.clone())
        } else {
            appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.import_button_handle.clone())
                .disabled()
        };
        Container::new(
            button
                .with_style(UiComponentStyles {
                    font_weight: Some(Weight::Bold),
                    width: Some(IMPORT_BUTTON_WIDTH),
                    height: Some(IMPORT_BUTTON_HEIGHT),
                    font_size: Some(FONT_SIZE),
                    ..Default::default()
                })
                .with_centered_text_label(crate::i18n::tr_static(app, "Import").to_owned())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SettingsImportAction::ImportButtonClicked);
                })
                .finish(),
        )
        .with_margin_right(BUTTON_SPACING)
        .finish()
    }

    fn render_reset_button(
        &self,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.skip_button_handle.clone())
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().surface_3().into()),
                font_weight: Some(Weight::Medium),
                width: Some(RESET_BUTTON_WIDTH),
                height: Some(RESET_BUTTON_HEIGHT),
                font_size: Some(FONT_SIZE),
                ..Default::default()
            })
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().outline().into()),
                ..Default::default()
            })
            .with_centered_text_label(
                crate::i18n::tr_static(app, "Reset to Warp defaults").to_owned(),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SettingsImportAction::ResetButtonClicked);
            })
            .finish()
    }

    fn render_config_option(
        &self,
        appearance: &Appearance,
        setting: &ToggleableSetting,
        idx: usize,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let theme = appearance.theme();
        let font_family = appearance.monospace_font_family();
        let font_color = blended_colors::text_sub(theme, theme.background());
        let description = Container::new(
            Text::new_inline(setting.setting_type.get_name(), font_family, FONT_SIZE)
                .with_color(font_color)
                .finish(),
        )
        .with_margin_left(CHECKBOX_SPACING);
        let setting_type = setting.setting_type.to_owned();
        let should_import = ImportedConfigModel::as_ref(app)
            .should_import(&self.configs[idx].terminal_type_and_profile, &setting_type);
        Container::new(
            Flex::row()
                .with_child(
                    appearance
                        .ui_builder()
                        .checkbox(setting.checkbox_handle.clone(), Some(CHECKBOX_SIZE))
                        .check(should_import)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(SettingsImportAction::ToggleSetting(
                                idx,
                                setting_type.clone(),
                            ))
                        })
                        .finish(),
                )
                .with_child(Shrinkable::new(1.0, description.finish()).finish())
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
                .finish(),
        )
        .finish()
    }

    fn render_config(
        &self,
        appearance: &Appearance,
        config_menu_item: &ConfigMenuItem,
        is_selected: bool,
        hovered: bool,
        idx: usize,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let theme = appearance.theme();
        let font_family = appearance.monospace_font_family();
        let font_color = theme.main_text_color(theme.background());

        let terminal_text = Container::new(
            Text::new_inline(config_menu_item.config_name.clone(), font_family, FONT_SIZE)
                .with_color(font_color.into_solid())
                .finish(),
        )
        .with_horizontal_margin(8.)
        .finish();

        let mut config_name_text_elements = vec![terminal_text];

        if let Some(description) = config_menu_item.description.clone() {
            let description = Shrinkable::new(
                1.0,
                Container::new(
                    Text::new_inline(description, font_family, FONT_SIZE)
                        .with_color(theme.sub_text_color(theme.background()).into_solid())
                        .finish(),
                )
                .with_horizontal_margin(8.)
                .finish(),
            )
            .finish();

            config_name_text_elements.push(description);
        }

        let config_name_flex = Flex::row()
            .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
            .with_children(config_name_text_elements)
            .finish();

        let mut preference_text_elements = vec![];

        let config_model_handle = ImportedConfigModel::as_ref(app);

        let mut counted_settings = config_menu_item.settings.iter().collect();
        if matches!(self.state, State::Completed { imported_idx: _ }) {
            counted_settings = config_menu_item
                .settings
                .iter()
                .filter(|setting| {
                    config_model_handle.should_import(
                        &config_menu_item.terminal_type_and_profile,
                        &setting.setting_type,
                    )
                })
                .collect::<Vec<_>>();
        }

        let num_prefs = counted_settings.len();

        if !config_menu_item.expanded || !matches!(self.state, State::Active) {
            // Calculate the number of other preferences besides theme and add correct punctuation and grammar.
            let mut theme_subtraction: usize = 0;

            if counted_settings
                .into_iter()
                .any(|setting| setting.setting_type == SettingType::Theme)
            {
                if num_prefs == 1 {
                    preference_text_elements.push(self.render_secondary_text(appearance, "Theme"));
                } else {
                    preference_text_elements.push(self.render_secondary_text(appearance, "Theme,"));
                }
                theme_subtraction = 1;
            }
            match num_prefs - theme_subtraction {
                1 => preference_text_elements
                    .push(self.render_secondary_text(appearance, "1 other setting")),
                0 => (),
                _ => preference_text_elements.push(self.render_secondary_text(
                    appearance,
                    format!("{} other settings", num_prefs - theme_subtraction),
                )),
            }
        }

        let block_completed = self.state.is_complete();

        Container::new(
            Hoverable::new(config_menu_item.menu_item_mouse_state.clone(), |_| {
                // In general, do not modify the element on hover if we have already clicked
                // "Import" or "Skip."
                let border_color = if is_selected {
                    appearance.theme().accent().into_solid()
                } else if hovered && !block_completed {
                    theme.surface_2().into_solid()
                } else {
                    theme.surface_1().into_solid()
                };

                let background_color = if hovered && !is_selected && !block_completed {
                    theme.surface_2()
                } else {
                    theme.background()
                }
                .with_opacity(appearance.theme().settings_import_config_hover_opacity());

                let preference_flex = Flex::row()
                    .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
                    .with_children(preference_text_elements)
                    .finish();
                Container::new(
                    Shrinkable::new(
                        1.0,
                        Flex::column()
                            .with_child(
                                Flex::row()
                                    .with_child(Shrinkable::new(3.0, config_name_flex).finish())
                                    .with_child(Shrinkable::new(1.0, preference_flex).finish())
                                    .with_cross_axis_alignment(
                                        warpui::elements::CrossAxisAlignment::Center,
                                    )
                                    .with_main_axis_size(MainAxisSize::Max)
                                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                    .finish(),
                            )
                            .with_child(if config_menu_item.expanded && !block_completed {
                                self.render_config_options(
                                    appearance,
                                    &config_menu_item.settings,
                                    idx,
                                    app,
                                )
                            } else {
                                Flex::row().finish()
                            })
                            .finish(),
                    )
                    .finish(),
                )
                .with_horizontal_padding(DROPDOWN_HORIZONTAL_PADDING)
                .with_vertical_padding(DROPDOWN_VERTICAL_PADDING)
                .with_border(Border::all(1.0).with_border_color(border_color))
                .with_background(background_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    DROPDOWN_BORDER_RADIUS,
                )))
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SettingsImportAction::SetSelectedConfig(idx));
                ctx.notify();
            })
            .finish(),
        )
        .finish()
    }

    fn render_config_options(
        &self,
        appearance: &Appearance,
        settings: &[ToggleableSetting],
        idx: usize,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let mut iter = settings.iter();
        let mut column_holder = Flex::row().with_main_axis_size(MainAxisSize::Max);
        // Split items evenly among the columns, constructing one column at a time.
        // For example: 1,2,3,4,5 becomes
        // 1 3 5
        // 2 4
        let min_col_length = settings.len() / NUM_COLUMNS;
        let num_extra_items = settings.len() % NUM_COLUMNS;
        for col in 0..NUM_COLUMNS {
            let column_length = if col < num_extra_items {
                min_col_length + 1
            } else {
                min_col_length
            };
            let mut column =
                Flex::column().with_main_axis_alignment(MainAxisAlignment::SpaceBetween);
            for _ in 0..column_length {
                let Some(item) = iter.next() else {
                    break;
                };
                column.add_child(self.render_config_option(appearance, item, idx, app))
            }
            column_holder.add_child(
                Shrinkable::new(
                    1.0,
                    Container::new(column.finish())
                        .with_padding_top(CHECKBOX_VERTICAL_PADDING)
                        // Since we already pad the bottom, only add what we need in addition.
                        .with_padding_bottom(CHECKBOX_VERTICAL_PADDING - DROPDOWN_VERTICAL_PADDING)
                        .with_horizontal_padding(CHECKBOX_HORIZONTAL_PADDING)
                        .finish(),
                )
                .finish(),
            )
        }
        Container::new(column_holder.finish())
            .with_border(Border::top(1.0).with_border_fill(appearance.theme().surface_2()))
            .with_margin_top(DROPDOWN_VERTICAL_PADDING)
            .finish()
    }

    fn set_preferences(
        &self,
        ctx: &mut ViewContext<SettingsImportView>,
        terminal_type_and_profile: &TerminalTypeAndProfile,
    ) {
        ImportedConfigModel::handle(ctx).update(ctx, |model, ctx| {
            let Some(config) = model.config(terminal_type_and_profile) else {
                log::warn!(
                    "Attempted to write preferences from an invalid terminal type and profile."
                );
                return;
            };

            KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                if let Some(extra_meta_keys) = config.option_as_meta.importable_value() {
                    report_if_error!(keys_settings
                        .extra_meta_keys
                        .set_value(extra_meta_keys, ctx))
                }
            });
            if let Some(Some(mouse_and_scroll_reporting)) =
                config.mouse_and_scroll_reporting.importable_value()
            {
                AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
                    report_if_error!(reporting
                        .mouse_reporting_enabled
                        .set_value(mouse_and_scroll_reporting.mouse_reporting, ctx));
                    report_if_error!(reporting
                        .scroll_reporting_enabled
                        .set_value(mouse_and_scroll_reporting.scroll_reporting, ctx));
                });
            }
            if let Some(font) = config.font.importable_value() {
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    if let Some(font_size) = font.size {
                        report_if_error!(font_settings
                            .monospace_font_size
                            .set_value(font_size, ctx))
                    }
                    if let Some(font_family) = font.family {
                        report_if_error!(font_settings
                            .monospace_font_name
                            .set_value(font_family, ctx))
                    }
                });
            }
            if let Some(Some(default_shell)) = config.default_shell.importable_value() {
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .startup_shell_override
                        .set_value(default_shell, ctx));
                });
            }

            if let Some(Some(working_directory)) = config.working_directory.importable_value() {
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .working_directory_config
                        .set_value(working_directory, ctx))
                });
            }
            if let Some(Ok(hotkey_mode)) = config.hotkey_mode.importable_value() {
                match hotkey_mode {
                    super::config::GlobalHotkey::Activation(keystroke) => {
                        KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                            self.remove_old_keybindings(ctx, keys_settings);
                            self.save_activation_keybinding(ctx, keys_settings, keystroke)
                        });
                    }
                    super::config::GlobalHotkey::QuakeMode(window) => {
                        KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                            self.remove_old_keybindings(ctx, keys_settings);
                            self.save_quake_mode_settings(ctx, keys_settings, window);
                        });
                    }
                }
            }

            WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
                if let Some((cols, rows)) = config.window_size.importable_value() {
                    if cols.is_some() || rows.is_some() {
                        report_if_error!(window_settings
                            .open_windows_at_custom_size
                            .set_value(true, ctx));
                    }
                    if let Some(cols) = cols {
                        report_if_error!(window_settings
                            .new_windows_num_columns
                            .set_value(cols, ctx));
                    }
                    if let Some(rows) = rows {
                        report_if_error!(window_settings.new_windows_num_rows.set_value(rows, ctx));
                    }
                }
                if let Some(opacity_settings) = config.opacity.importable_value() {
                    if let Some(opacity) = opacity_settings.opacity {
                        report_if_error!(window_settings
                            .background_opacity
                            .set_value(opacity, ctx));
                    }
                    if let Some(blur_radius) = opacity_settings.blur_radius {
                        ctx.windows()
                            .set_all_windows_background_blur_radius(blur_radius);
                        report_if_error!(window_settings
                            .background_blur_radius
                            .set_value(blur_radius, ctx));
                    }
                }
            });

            if let Some(Some(copy_on_select)) = config.copy_on_select.importable_value() {
                SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
                    report_if_error!(selection_settings
                        .copy_on_select
                        .set_value(copy_on_select, ctx));
                });
            }

            if let Some(Some(cursor_blinking)) = config.cursor_blinking.importable_value() {
                AppEditorSettings::handle(ctx).update(ctx, |me, ctx| {
                    report_if_error!(me.cursor_blink.set_value(
                        if cursor_blinking {
                            CursorBlink::Enabled
                        } else {
                            CursorBlink::Disabled
                        },
                        ctx,
                    ));
                });
            }
        });

        self.send_completed_import_telemetry_event(terminal_type_and_profile, ctx);
        ctx.notify();
    }

    fn reset_preferences(&self, ctx: &mut ViewContext<SettingsImportView>) {
        ThemeSettings::handle(ctx).update(ctx, |theme_settings, ctx| {
            report_if_error!(theme_settings
                .selected_system_themes
                .set_value_to_default(ctx));
            report_if_error!(theme_settings.theme_kind.set_value_to_default(ctx));
            report_if_error!(theme_settings.use_system_theme.set_value_to_default(ctx));
        });
        AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
            report_if_error!(reporting.mouse_reporting_enabled.set_value_to_default(ctx));
            report_if_error!(reporting.scroll_reporting_enabled.set_value_to_default(ctx));
        });
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings.monospace_font_size.set_value_to_default(ctx));
            report_if_error!(font_settings.monospace_font_name.set_value_to_default(ctx));
        });
        SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.startup_shell_override.set_value_to_default(ctx));
            report_if_error!(settings.working_directory_config.set_value_to_default(ctx));
        });

        KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
            self.remove_old_keybindings(ctx, keys_settings);
            keys_settings.set_global_hotkey_mode_and_write_to_user_defaults(
                &GlobalHotkeyMode::Disabled,
                ctx,
            );
            report_if_error!(keys_settings.extra_meta_keys.set_value_to_default(ctx))
        });

        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .open_windows_at_custom_size
                .set_value_to_default(ctx));
            report_if_error!(window_settings.background_opacity.set_value_to_default(ctx));
            report_if_error!(window_settings
                .background_blur_radius
                .set_value_to_default(ctx));
        });

        SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
            report_if_error!(selection_settings.copy_on_select.set_value_to_default(ctx));
        });

        AppEditorSettings::handle(ctx).update(ctx, |me, ctx| {
            report_if_error!(me.cursor_blink.set_value_to_default(ctx,));
        });

        ctx.notify();
    }

    fn set_theme(
        ctx: &mut warpui::ViewContext<Self>,
        theme_type: ThemeType,
        terminal_name: &String,
    ) {
        let dir = user_config::themes_dir();
        match theme_type {
            ThemeType::LightAndDark { light, dark } => {
                let light_path = dir.join(format!("{terminal_name}_light_theme.yaml"));

                let light_kind = ThemeKind::Custom(CustomTheme::new(
                    light.name().unwrap_or_default(),
                    light_path,
                ));

                let dark_path = dir.join(format!("{terminal_name}_dark_theme.yaml"));

                let dark_kind =
                    ThemeKind::Custom(CustomTheme::new(dark.name().unwrap_or_default(), dark_path));
                ThemeSettings::handle(ctx).update(ctx, |theme_settings, ctx| {
                    report_if_error!(theme_settings.selected_system_themes.set_value(
                        SelectedSystemThemes {
                            light: light_kind.clone(),
                            dark: dark_kind.clone(),
                        },
                        ctx,
                    ));
                    report_if_error!(theme_settings.use_system_theme.set_value(true, ctx));
                });
                WarpConfig::handle(ctx).update(ctx, |config, ctx| {
                    config.add_new_theme_to_config(dark_kind, dark, ctx)
                });
                WarpConfig::handle(ctx).update(ctx, |config, ctx| {
                    config.add_new_theme_to_config(light_kind, light, ctx)
                });
            }
            ThemeType::Single(theme) => {
                let theme_path = dir.join(format!("{terminal_name}_theme.yaml"));

                let theme_kind = ThemeKind::Custom(CustomTheme::new(
                    theme.name().unwrap_or_default(),
                    theme_path,
                ));
                ThemeSettings::handle(ctx).update(ctx, |theme_settings, ctx| {
                    report_if_error!(theme_settings
                        .theme_kind
                        .set_value(theme_kind.clone(), ctx,));
                    report_if_error!(theme_settings.use_system_theme.set_value(false, ctx));
                });
                WarpConfig::handle(ctx).update(ctx, |config, ctx| {
                    config.add_new_theme_to_config(theme_kind, theme, ctx)
                });
            }
        }

        ctx.notify();
    }

    fn remove_old_keybindings(
        &self,
        ctx: &mut ModelContext<KeysSettings>,
        keys_settings: &mut KeysSettings,
    ) {
        if let Some(old_quake_mode_keystroke) = &keys_settings.quake_mode_settings.keybinding {
            ctx.unregister_global_shortcut(old_quake_mode_keystroke);
        }
        if let Some(old_activation_keystroke) = keys_settings.activation_hotkey_keybinding.value() {
            ctx.unregister_global_shortcut(old_activation_keystroke);
        }
    }

    fn save_quake_mode_settings(
        &self,
        ctx: &mut ModelContext<KeysSettings>,
        keys_settings: &mut KeysSettings,
        window: QuakeModeWindow,
    ) {
        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
        ctx.register_global_shortcut(
            window.keystroke.clone(),
            "root_view:toggle_quake_mode_window",
            global_resource_handles,
        );
        keys_settings
            .set_quake_mode_keybinding_and_write_to_user_defaults(Some(window.keystroke), ctx);
        keys_settings
            .set_global_hotkey_mode_and_write_to_user_defaults(&GlobalHotkeyMode::QuakeMode, ctx);
        keys_settings
            .set_quake_mode_pin_position_and_write_to_user_defaults(window.pin_position, ctx);
        keys_settings.set_quake_mode_pin_screen_and_write_to_user_defaults(window.screen, ctx);
        keys_settings.set_hide_quake_mode_window_when_unfocused_and_write_to_user_defaults(
            window.autohide,
            ctx,
        )
    }

    fn save_activation_keybinding(
        &self,
        ctx: &mut ModelContext<KeysSettings>,
        keys_settings: &mut KeysSettings,
        keystroke: Keystroke,
    ) {
        ctx.register_global_shortcut(
            keystroke.clone(),
            "root_view:show_or_hide_non_quake_mode_windows",
            (),
        );
        keys_settings
            .set_activation_hotkey_keybinding_and_write_to_user_defaults(Some(keystroke), ctx);
        keys_settings.set_global_hotkey_mode_and_write_to_user_defaults(
            &GlobalHotkeyMode::ActivationHotkey,
            ctx,
        );
    }

    pub(crate) fn interrupt_block(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        self.state = State::Completed { imported_idx: None };
        ctx.notify();
    }

    fn send_completed_import_telemetry_event(
        &self,
        terminal_type_and_profile: &TerminalTypeAndProfile,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = ImportedConfigModel::handle(ctx);
        let imported_settings = model.read(ctx, |model, _ctx| {
            let Some(config) = model.config(terminal_type_and_profile) else {
                log::error!("Could not find config for terminal {terminal_type_and_profile:?}");
                return Default::default();
            };

            config
                .valid_setting_types()
                .into_iter()
                .map(|setting_type| {
                    let was_imported_by_user =
                        model.should_import(terminal_type_and_profile, &setting_type);
                    ParsedTerminalSetting {
                        setting_type,
                        was_imported_by_user,
                    }
                })
                .collect_vec()
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::CompletedSettingsImport {
                terminal_type: terminal_type_and_profile.into(),
                imported_settings,
            },
            ctx
        );
    }
}

impl View for SettingsImportView {
    fn ui_name() -> &'static str {
        "SettingsImportView"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let font_family = appearance.monospace_font_family();
        let font_size = appearance.monospace_font_size();
        let font_color = appearance
            .theme()
            .main_text_color(appearance.theme().background());

        let config_item_builders = self
            .configs
            .iter()
            .enumerate()
            .map(|(idx, config)| {
                RadioButtonItem::custom_item(Box::new(move |_, is_selected, hovered| {
                    Shrinkable::new(
                        1.0,
                        self.render_config(appearance, config, is_selected, hovered, idx, app),
                    )
                    .finish()
                }))
                .with_disabled(self.state.is_complete())
            })
            .collect::<Vec<RadioButtonItem>>();

        let imported_config_idx = match self.state {
            State::Completed { imported_idx } => imported_idx,
            _ => None,
        };

        let hide_buttons = matches!(
            self.state,
            State::Completed { imported_idx: None } | State::Failed,
        );
        let buttons = match self.state {
            State::Loading | State::Active => self.render_import_button(appearance, app),
            State::Completed { imported_idx: None } | State::Failed => {
                Container::new(Flex::row().finish()).finish()
            }
            State::Completed { imported_idx: _ } => self.render_reset_button(appearance, app),
        };

        let config_radio_buttons = appearance
            .ui_builder()
            .radio_buttons(
                self.radio_button_mouse_states.clone(),
                config_item_builders,
                self.radio_button_state.clone(),
                imported_config_idx,
                FONT_SIZE,
                radio_buttons::RadioButtonLayout::Column,
            )
            .with_style(UiComponentStyles {
                padding: Some(Coords::default()),
                margin: Some(Coords {
                    top: 0.,
                    bottom: DROPDOWN_BOTTOM_MARGIN,
                    right: DROPDOWN_HORIZONTAL_MARGIN,
                    left: DROPDOWN_HORIZONTAL_MARGIN,
                }),
                ..Default::default()
            })
            .with_button_vertical_offset(DROPDOWN_VERTICAL_PADDING);

        const WELCOME_TEXT: &str = "Select a settings profile to import:";
        const LOADING_TEXT: &str = "Looking for settings to import...";

        let mut display_new_session_text = false;

        if let State::Completed {
            imported_idx: Some(idx),
        } = self.state
        {
            let config_menu_item: &ConfigMenuItem = &self.configs[idx];
            let model_handle = ImportedConfigModel::as_ref(app);
            display_new_session_text = NEW_SESSION_SETTINGS.iter().any(|setting| {
                model_handle.should_import(&config_menu_item.terminal_type_and_profile, setting)
                    && config_menu_item
                        .settings
                        .iter()
                        .any(|toggleable_setting| toggleable_setting.setting_type == *setting)
            });
        }

        let mut new_session_setting_text = Flex::row().finish();

        if display_new_session_text {
            new_session_setting_text = Container::new(
                Text::new(
                    "Some settings will take effect when you open a new session.",
                    font_family,
                    font_size,
                )
                .with_color(font_color.into_solid())
                .finish(),
            )
            .with_margin_bottom(14.)
            .with_horizontal_margin(DROPDOWN_HORIZONTAL_MARGIN)
            .finish()
        }

        // Add margin if the buttons are shown. Otherwise, the component above the buttons provides sufficient margin.
        let button_margin = if hide_buttons {
            0.
        } else {
            DROPDOWN_BOTTOM_MARGIN
        };

        if matches!(self.state, State::Loading) {
            return Container::new(
                Text::new(LOADING_TEXT, font_family, font_size)
                    .with_color(font_color.into_solid())
                    .finish(),
            )
            .with_margin_top(14.)
            .with_horizontal_margin(DROPDOWN_HORIZONTAL_MARGIN)
            .with_margin_bottom(DROPDOWN_BOTTOM_MARGIN)
            .finish();
        }

        Container::new(
            Flex::column()
                .with_child(
                    Container::new(
                        Text::new(WELCOME_TEXT, font_family, font_size)
                            .with_color(font_color.into_solid())
                            .with_style(Properties::default().weight(Weight::Bold))
                            .finish(),
                    )
                    .with_horizontal_margin(DROPDOWN_HORIZONTAL_MARGIN)
                    .with_margin_top(BLOCK_TOP_MARGIN)
                    .with_margin_bottom(DROPDOWN_BOTTOM_MARGIN)
                    .finish(),
                )
                .with_child(config_radio_buttons.build().finish())
                .with_child(new_session_setting_text)
                .with_child(
                    Container::new(buttons)
                        .with_horizontal_margin(DROPDOWN_HORIZONTAL_MARGIN)
                        .with_margin_bottom(button_margin)
                        .finish(),
                )
                .finish(),
        )
        .with_border(Border::top(1.0).with_border_fill(appearance.theme().surface_2()))
        .finish()
    }
}

impl TypedActionView for SettingsImportView {
    type Action = SettingsImportAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SettingsImportAction::ImportButtonClicked => {
                let Some(selected_idx) = self.radio_button_state.get_selected_idx() else {
                    return;
                };
                let terminal_type_and_profile =
                    &self.configs[selected_idx].terminal_type_and_profile;

                // Handle should_import within the model.

                // set_preferences should not fail because it is writing directly to Warp's preferences.
                self.set_preferences(ctx, terminal_type_and_profile);

                // write_theme can fail because we write themes in a separate directory.
                if let Some(theme_type) =
                    ImportedConfigModel::as_ref(ctx).write_theme(terminal_type_and_profile)
                {
                    SettingsImportView::set_theme(
                        ctx,
                        theme_type,
                        &self.configs[selected_idx].config_name,
                    );
                    ctx.emit(SettingsImportEvent::Completed(true));
                } else {
                    ctx.emit(SettingsImportEvent::Completed(false));
                }
                self.state = State::Completed {
                    imported_idx: self.radio_button_state.get_selected_idx(),
                };

                ctx.notify();
            }
            SettingsImportAction::SetSelectedConfig(idx) => {
                let old_selected_idx = self
                    .configs
                    .iter()
                    .find_position(|config| config.expanded)
                    .map(|(position, _item)| position);
                // Collapse all other configs.
                self.configs
                    .iter_mut()
                    .enumerate()
                    .filter(|(config_idx, _)| idx != config_idx)
                    .for_each(|(_, config)| config.expanded = false);
                // Set the current config to expand.
                self.configs[*idx].expanded = true;
                // Only send the telemetry event if the new selected item is different.
                if old_selected_idx.is_none_or(|old_idx| old_idx != *idx) {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::SettingsImportConfigFocused(
                            self.configs[*idx].terminal_type_and_profile.into()
                        ),
                        ctx
                    );
                }
                // The radio button state already updates, since each element is a child of a RadioButtonItem.
                ctx.notify();
            }
            SettingsImportAction::ToggleSetting(idx, setting_type) => {
                let model = ImportedConfigModel::handle(ctx);
                model.update(ctx, |model, _| {
                    model.toggle_should_import(
                        &self.configs[*idx].terminal_type_and_profile,
                        setting_type,
                    )
                })
            }
            SettingsImportAction::ResetButtonClicked => {
                // Reset the state so that nothing is selected.
                self.radio_button_state = Default::default();
                self.reset_preferences(ctx);
                if matches!(
                    self.state,
                    State::Completed {
                        imported_idx: Some(_),
                    }
                ) {
                    self.state = State::Completed { imported_idx: None }
                }
                send_telemetry_from_ctx!(TelemetryEvent::SettingsImportResetButtonClicked, ctx);
            }
        }
    }
}

pub enum SettingsImportEvent {
    /// Completed, with whether or not a theme was imported.
    Completed(bool),
    NoConfigsFound,
}

impl Entity for SettingsImportView {
    type Event = SettingsImportEvent;
}

fn create_toggleable_settings(config: &Config) -> Vec<ToggleableSetting> {
    config
        .valid_setting_types()
        .into_iter()
        .map(ToggleableSetting::new)
        .collect()
}
