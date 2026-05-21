use pathfinder_color::ColorU;
use settings::Setting as _;
use warp_editor::editor::NavigationKey;
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    elements::{
        Align, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DispatchEventResult, Element, Empty, EventHandler, Fill, Flex, Hoverable, Icon,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
        ParentElement, ParentOffsetBounds, Radius, Rect, SavePosition, ScrollStateHandle,
        Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable, Stack, Text, UniformList,
        UniformListState,
    },
    fonts::{FamilyId, Weight},
    geometry::vector::vec2f,
    keymap::FixedBinding,
    platform::{Cursor, SystemTheme},
    ui_components::components::{UiComponent, UiComponentStyles},
    windowing::{StateEvent, WindowManager},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, Tracked, TypedActionView,
    UpdateModel, View, ViewContext, ViewHandle,
};

use crate::resource_center::{mark_feature_used_and_write_to_user_defaults, Tip, TipAction};
use crate::themes::theme::{RespectSystemTheme, ThemeKind, WarpTheme};
use crate::util::traffic_lights::traffic_light_data;
use crate::workspace::PANEL_HEADER_HEIGHT;
use crate::{
    appearance::Appearance,
    editor::{
        Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
    },
    referral_theme_status::ReferralThemeStatus,
    report_if_error,
    settings::{respect_system_theme, ThemeSettings},
    themes::theme::SelectedSystemThemes,
    user_config::{load_theme_configs, themes_dir, WarpConfig, WarpConfigUpdateEvent},
    util::traffic_lights::{TrafficLightData, TrafficLightSide},
    window_settings::WindowSettings,
};
use crate::{appearance::AppearanceManager, send_telemetry_from_ctx};
use crate::{editor::EditorView, resource_center::TipsCompleted};
use crate::{
    server::telemetry::TelemetryEvent, ui_components::window_focus_dimming::WindowFocusDimming,
};
use crate::{
    themes::theme::WarpThemeConfig,
    ui_components::buttons::{close_button, icon_button},
    ui_components::icons,
};

use super::theme;

// All units in px
const THEME_CHOOSER_TITLE: &str = "Themes";
const CLOSE_BUTTON_MARGIN_RIGHT: f32 = 6.;
const TITLE_FONT_SIZE: f32 = 16.;
const TITLE_MARGIN: f32 = 12.;
const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const THEME_NAME_FONT_SIZE: f32 = 14.;
const THEME_NAME_MARGIN_LEFT: f32 = 16.;
const DELETE_BUTTON_LINE_WIDTH: f32 = 10.;
const DELETE_BUTTON_LINE_HEIGHT: f32 = 1.33;
const DELETE_BUTTON_SIZE: f32 = 16.;
const DELETE_BUTTON_MARGIN_RIGHT: f32 = 16.;
const THEME_CHOOSER_ITEM_PADDING: f32 = 16.;

#[derive(Default)]
struct MouseStateHandles {
    create_theme_button_hover_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
}

pub enum ThemeChooserEvent {
    Click,
    Close(ThemeChooserMode),
    OpenThemeCreatorModal,
    OpenThemeDeletionModal(ThemeKind),
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum ThemeChooserMode {
    /// Select a single fixed theme, independent of whether the system is using
    /// a light or dark theme.
    SystemAgnostic,
    /// Select a theme to use when the system is using a light theme.
    SystemLight,
    /// Select a theme to use when the system is using a dark theme.
    SystemDark,
}

impl ThemeChooserMode {
    /// Returns the mode the theme chooser should use if the aim is to change
    /// the active theme, as opposed to changing a specific theme option.
    pub fn for_active_theme(app: &AppContext) -> Self {
        match respect_system_theme(ThemeSettings::as_ref(app)) {
            RespectSystemTheme::On(_) => match app.system_theme() {
                SystemTheme::Dark => ThemeChooserMode::SystemDark,
                SystemTheme::Light => ThemeChooserMode::SystemLight,
            },
            RespectSystemTheme::Off => ThemeChooserMode::SystemAgnostic,
        }
    }

    pub fn into_theme_kind(self, ctx: &AppContext) -> ThemeKind {
        let theme_settings = ThemeSettings::as_ref(ctx);
        let theme_kind = theme_settings.theme_kind.value();
        match (self, &respect_system_theme(theme_settings)) {
            (ThemeChooserMode::SystemAgnostic, _) => theme_kind.clone(),
            (ThemeChooserMode::SystemLight, RespectSystemTheme::On(system_themes)) => {
                system_themes.light.clone()
            }
            (ThemeChooserMode::SystemDark, RespectSystemTheme::On(system_themes)) => {
                system_themes.dark.clone()
            }
            (_, _) => ThemeKind::default(),
        }
    }

    fn render_hint_text(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let hint_text = match self {
            ThemeChooserMode::SystemAgnostic => appearance
                .ui_builder()
                .paragraph(crate::i18n::tr_static(app, "Change your current theme.").to_string()),
            ThemeChooserMode::SystemLight => appearance.ui_builder().paragraph(
                crate::i18n::tr_static(app, "Pick a theme for when your system is in light mode.")
                    .to_string(),
            ),
            ThemeChooserMode::SystemDark => appearance.ui_builder().paragraph(
                crate::i18n::tr_static(app, "Pick a theme for when your system is in dark mode.")
                    .to_string(),
            ),
        };
        hint_text
            .build()
            .with_margin_left(TITLE_MARGIN)
            .with_margin_right(TITLE_MARGIN)
            .finish()
    }
}

pub struct ThemeChooser {
    button_mouse_states: MouseStateHandles,
    header_dimming_mouse_state: MouseStateHandle,
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,
    selected_theme: Tracked<Option<ThemeKind>>,
    themes: Tracked<Vec<ThemeChooserItem>>,
    filtered_themes: Tracked<Option<Vec<ThemeChooserItem>>>,
    mode: ThemeChooserMode,
    search_editor: ViewHandle<EditorView>,
    referral_theme_status: ModelHandle<ReferralThemeStatus>,
    tips_completed: ModelHandle<TipsCompleted>,
    window_id: warpui::WindowId,
}

#[derive(Debug)]
pub enum ThemeChooserAction {
    Close,
    Enter,
    Click(ThemeKind),
    Up,
    Down,
    OpenThemeCreator,
    OpenThemeDeletionModal(ThemeKind),
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::new("up", ThemeChooserAction::Up, id!("ThemeChooser")),
        FixedBinding::new("down", ThemeChooserAction::Down, id!("ThemeChooser")),
        FixedBinding::new("escape", ThemeChooserAction::Close, id!("ThemeChooser")),
        FixedBinding::new("enter", ThemeChooserAction::Enter, id!("ThemeChooser")),
    ]);
}

fn theme_chooser_items(
    referral_theme_status: &ReferralThemeStatus,
    theme_config: &WarpThemeConfig,
) -> Vec<ThemeChooserItem> {
    let sent_referral_theme_active = referral_theme_status.sent_referral_theme_active();
    let received_referral_theme_active = referral_theme_status.received_referral_theme_active();

    let mut theme_items: Vec<ThemeChooserItem> = theme_config
        .theme_items()
        .filter(|(key, _)| match key {
            // Only show the referral reward themes if they are active
            ThemeKind::SentReferralReward => sent_referral_theme_active,
            ThemeKind::ReceivedReferralReward => received_referral_theme_active,
            // All other themes should show up always
            _ => true,
        })
        .map(|(key, theme)| ThemeChooserItem::new(key.clone(), theme.clone()))
        .collect();
    theme_items.sort_by(|a, b| a.kind.cmp(&b.kind));
    theme_items
}

impl ThemeChooser {
    pub fn new(
        referral_theme_status: ModelHandle<ReferralThemeStatus>,
        ctx: &mut ViewContext<Self>,
        tips_completed: ModelHandle<TipsCompleted>,
    ) -> Self {
        let search_editor = {
            ctx.add_typed_action_view(|ctx| {
                let appearance = Appearance::as_ref(ctx);
                let options = SingleLineEditorOptions {
                    text: TextOptions::ui_font_size(appearance),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                };
                EditorView::single_line(options, ctx)
            })
        };

        ctx.subscribe_to_view(&search_editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&referral_theme_status, |me, _, _, ctx| {
            me.update_themes(ctx);
        });

        let warp_config_handle = WarpConfig::handle(ctx);
        ctx.subscribe_to_model(&warp_config_handle, |me, _, event, ctx| {
            if let WarpConfigUpdateEvent::Themes = event {
                me.update_themes(ctx);
                ctx.notify();
            }
        });

        // Subscribe to window state changes for focus dimming updates
        let state_handle: ModelHandle<WindowManager> = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&state_handle, |_me, _, event, ctx| {
            match &event {
                StateEvent::ValueChanged { current, previous } => {
                    // Re-render if this window's focus state has changed
                    if WindowManager::did_window_change_focus(ctx.window_id(), current, previous) {
                        ctx.notify();
                    }
                }
            }
        });

        let themes = theme_chooser_items(
            referral_theme_status.as_ref(ctx),
            WarpConfig::as_ref(ctx).theme_config(),
        );

        Self {
            themes: Tracked::new(themes),
            button_mouse_states: Default::default(),
            header_dimming_mouse_state: Default::default(),
            list_state: Default::default(),
            scroll_state: Default::default(),
            selected_theme: Tracked::new(None),
            filtered_themes: Tracked::new(None),
            mode: ThemeChooserMode::for_active_theme(ctx),
            search_editor,
            referral_theme_status,
            tips_completed,
            window_id: ctx.window_id(),
        }
    }

    pub fn handle_theme_change(&mut self, ctx: &mut ViewContext<Self>) {
        // Ensure that we are still showing the right mode and have the correct theme selected.
        // The only time this can get out of sync is if there's a cloud preferences change affecting settings.
        // Note that we intentionally read from the settings model, not appearance here, as
        // the appearance will give us the derived theme, but we are trying to stay in sync
        // with the actual theme settings.
        let theme_settings = ThemeSettings::as_ref(ctx);
        let respect_system_theme = respect_system_theme(theme_settings);
        let system_theme = ctx.system_theme();
        match (respect_system_theme, self.mode, system_theme) {
            (
                RespectSystemTheme::On(selected_system_themes),
                ThemeChooserMode::SystemLight,
                SystemTheme::Light,
            )
            | (
                RespectSystemTheme::On(selected_system_themes),
                ThemeChooserMode::SystemDark,
                SystemTheme::Dark,
            ) => {
                // If we are choosing the theme for the current mode, ensure that we update the chooser state to match the
                // model state.
                let theme = match system_theme {
                    SystemTheme::Light => selected_system_themes.light.clone(),
                    SystemTheme::Dark => selected_system_themes.dark.clone(),
                };
                self.select_theme(theme, ctx);
            }
            (RespectSystemTheme::Off, ThemeChooserMode::SystemAgnostic, _) => {
                // If we are choosing the global theme, ensure that we update the chooser state to match the
                // model state
                let theme = ThemeSettings::as_ref(ctx).theme_kind.value().clone();
                self.select_theme(theme, ctx);
            }
            _ => {
                // Otherwise, we don't need to update anything, as we are in a state where we are
                // choosing a theme for an inactive mode.
            }
        }
    }

    pub fn reload_and_set_custom_theme(&mut self, theme: ThemeKind, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move { load_theme_configs(&themes_dir()) },
            move |theme_chooser, loaded_themes, ctx| {
                ctx.update_model(&WarpConfig::handle(ctx), move |warp_config, ctx| {
                    warp_config.update_theme_config(loaded_themes, ctx);
                });
                theme_chooser.update_themes(ctx);
                theme_chooser.select_and_save_theme(&theme, ctx);
            },
        );
    }

    pub fn reload_and_set_latest_theme(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move { load_theme_configs(&themes_dir()) },
            move |theme_chooser, loaded_themes, ctx| {
                ctx.update_model(&WarpConfig::handle(ctx), move |warp_config, ctx| {
                    warp_config.update_theme_config(loaded_themes, ctx);
                });
                theme_chooser.update_themes(ctx);
                theme_chooser.select_latest_theme(ctx);
            },
        );
    }
    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let search_term = self.search_editor.as_ref(ctx).buffer_text(ctx);
                *self.filtered_themes = if search_term.is_empty() {
                    None
                } else {
                    Some(
                        self.themes
                            .iter()
                            .filter(|item| item.kind.matches(&search_term))
                            .cloned()
                            .collect::<Vec<_>>(),
                    )
                };
                // Finding the position of the selected theme to adjust the scroll position of the
                // list of visible themes.
                let index = self.theme_position(self.selected_theme.clone().unwrap_or_default());
                self.list_state.scroll_to(index.unwrap_or_default());
            }
            EditorEvent::Navigate(NavigationKey::Up) => self.up(ctx),
            EditorEvent::Navigate(NavigationKey::Down) => self.down(ctx),
            EditorEvent::Enter => self.enter(ctx),
            EditorEvent::Escape => self.close(ctx),
            _ => {}
        }
    }

    pub fn record_open_theme(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        send_telemetry_from_ctx!(TelemetryEvent::OpenThemeChooser, ctx);
        true
    }

    pub fn open_theme_creator_modal(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::OpenThemeCreatorModal, ctx);
        ctx.emit(ThemeChooserEvent::OpenThemeCreatorModal);
    }

    pub fn open_theme_deletion_modal(
        &mut self,
        theme_kind: ThemeKind,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(ThemeChooserEvent::OpenThemeDeletionModal(theme_kind));
    }

    pub fn set_mode(&mut self, mode: ThemeChooserMode) {
        self.mode = mode;
    }

    // this is actually used in our integration test assertions,
    // but rust thinks it's unused when running unit tests in this crate
    #[allow(dead_code)]
    pub fn themes(&self) -> impl Iterator<Item = &ThemeKind> {
        self.themes
            .iter()
            .map(|theme_chooser_item| &theme_chooser_item.kind)
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        *self.selected_theme = None;
        *self.filtered_themes = None;
        AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
            appearance_manager.clear_transient_theme(ctx);
        });
        self.search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        ctx.emit(ThemeChooserEvent::Close(self.mode));
    }

    fn enter(&mut self, ctx: &mut ViewContext<Self>) {
        // "Enter" should close the theme picker is there is a theme that's visibly selected
        // If the user has entered a search term, but no theme is selected, we should not close
        if self.is_selected_theme_visible() {
            self.close(ctx);
        } else {
            log::info!("Handled enter key in theme chooser, but no theme is visibly selected.")
        }
    }

    pub fn select_and_save_theme(
        &mut self,
        selected_kind: &ThemeKind,
        ctx: &mut ViewContext<Self>,
    ) {
        self.select_theme(selected_kind.clone(), ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::ThemeSelection {
                theme: selected_kind.to_string(),
                entrypoint: "theme_chooser".to_string()
            },
            ctx
        );
        let theme_settings = ThemeSettings::handle(ctx);

        let selected_themes = respect_system_theme(theme_settings.as_ref(ctx))
            .selected_system_themes()
            .cloned()
            .unwrap_or_default();
        match self.mode {
            ThemeChooserMode::SystemAgnostic => {
                theme_settings.update(ctx, |theme_settings, ctx| {
                    report_if_error!(theme_settings
                        .theme_kind
                        .set_value(selected_kind.clone(), ctx,));
                });
            }
            ThemeChooserMode::SystemLight => {
                theme_settings.update(ctx, |theme_settings, ctx| {
                    report_if_error!(theme_settings.selected_system_themes.set_value(
                        SelectedSystemThemes {
                            light: selected_kind.clone(),
                            dark: selected_themes.dark,
                        },
                        ctx,
                    ));
                });
            }
            ThemeChooserMode::SystemDark => {
                theme_settings.update(ctx, |theme_settings, ctx| {
                    report_if_error!(theme_settings.selected_system_themes.set_value(
                        SelectedSystemThemes {
                            light: selected_themes.light,
                            dark: selected_kind.clone(),
                        },
                        ctx,
                    ));
                });
            }
        };
    }

    fn theme_position(&self, kind: ThemeKind) -> Option<usize> {
        self.filtered_themes
            .as_ref()
            .unwrap_or_else(|| self.themes.as_ref())
            .iter()
            .position(|item| item.kind == kind)
    }

    pub fn select_theme(&mut self, kind: ThemeKind, ctx: &mut ViewContext<Self>) {
        let index = self.theme_position(kind.clone()).unwrap_or_default();

        self.list_state.scroll_to(index);

        *self.selected_theme = Some(kind.clone());

        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::ThemePicker),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });

        AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
            appearance_manager.set_transient_theme(kind, ctx);
        });
    }

    pub fn select_latest_theme(&mut self, ctx: &mut ViewContext<Self>) {
        let index = self.themes.len() - 1;

        self.list_state.scroll_to(index);

        *self.selected_theme = Some(self.themes[index].kind.clone());

        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::ThemePicker),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });

        AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
            appearance_manager.set_transient_theme(self.themes[index].kind.clone(), ctx);
        });
    }

    fn update_themes(&mut self, ctx: &mut ViewContext<Self>) {
        *self.themes = theme_chooser_items(
            self.referral_theme_status.as_ref(ctx),
            WarpConfig::as_ref(ctx).theme_config(),
        );
    }

    fn up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.visible_theme_count() == 0 {
            return;
        }

        let index = match &*self.selected_theme {
            None => 0,
            Some(selected_kind) => self
                .theme_position(selected_kind.clone())
                .unwrap_or_default()
                .saturating_sub(1),
        };
        self.list_state.scroll_to(index);
        self.select_and_save_theme(&self.selected_theme(index), ctx);
    }

    fn down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.visible_theme_count() == 0 {
            return;
        }

        let index = match &*self.selected_theme {
            None => 0,
            Some(selected_kind) => {
                match self.theme_position(selected_kind.clone()) {
                    None => 0, // selected element is not visible
                    Some(index) => (index + 1).min(self.visible_theme_count() - 1),
                }
            }
        };
        self.list_state.scroll_to(index);
        self.select_and_save_theme(&self.selected_theme(index), ctx);
    }

    fn visible_theme_count(&self) -> usize {
        match &*self.filtered_themes {
            None => self.themes.len(),
            Some(themes) => themes.len(),
        }
    }

    fn selected_theme(&self, index: usize) -> ThemeKind {
        match &*self.filtered_themes {
            None => self.themes[index].kind.clone(),
            Some(themes) => themes[index].kind.clone(),
        }
    }

    fn is_selected_theme_visible(&self) -> bool {
        self.selected_theme
            .as_ref()
            .map(|selected_kind| self.theme_position(selected_kind.clone()).is_some())
            .unwrap_or(false)
    }

    fn click(&mut self, kind: ThemeKind, ctx: &mut ViewContext<Self>) {
        self.select_and_save_theme(&kind, ctx);
        ctx.emit(ThemeChooserEvent::Click);
    }

    fn render_header(
        &self,
        traffic_light_data: Option<&TrafficLightData>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut margin_left = 16.;

        let zoom_factor = WindowSettings::as_ref(app).zoom_level.as_zoom_factor();
        // Since this panel is always on the left, only account for left-side traffic lights.
        if let Some(width) = traffic_light_data
            .filter(|data| data.side == TrafficLightSide::Left)
            .map(|data| data.width(zoom_factor))
        {
            margin_left += width;
        }

        let close_button = close_button(
            appearance,
            self.button_mouse_states.close_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(ThemeChooserAction::Close))
        .finish();

        let header_element = ConstrainedBox::new(
            Flex::row()
                .with_child(
                    Container::new(Align::new(close_button).finish())
                        .with_margin_left(margin_left)
                        .with_margin_right(CLOSE_BUTTON_MARGIN_RIGHT)
                        .finish(),
                )
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_height(PANEL_HEADER_HEIGHT)
        .finish();

        // Apply dimming if window is not focused
        WindowFocusDimming::apply_panel_header_dimming(
            header_element,
            self.header_dimming_mouse_state.clone(),
            PANEL_HEADER_HEIGHT,
            appearance.theme().surface_1().into(),
            self.window_id,
            app,
        )
    }

    fn render_title_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut title_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        appearance
                            .ui_builder()
                            .span(THEME_CHOOSER_TITLE.to_string())
                            .with_style(UiComponentStyles {
                                font_family_id: Some(appearance.ui_font_family()),
                                font_size: Some(TITLE_FONT_SIZE),
                                font_weight: Some(Weight::Semibold),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );

        // Custom themes are only supported on desktop platforms currently.
        if cfg!(not(target_family = "wasm")) {
            let create_theme_button = SavePosition::new(
                icon_button(
                    appearance,
                    icons::Icon::Plus,
                    false,
                    self.button_mouse_states
                        .create_theme_button_hover_state
                        .clone(),
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThemeChooserAction::OpenThemeCreator)
                })
                .finish(),
                "create_theme_button",
            )
            .finish();

            title_row = title_row.with_child(create_theme_button);
        }

        Container::new(title_row.finish())
            .with_margin_bottom(6.)
            .with_margin_left(TITLE_MARGIN)
            .with_margin_right(TITLE_MARGIN)
            .finish()
    }

    fn render_search_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::new(
                                "bundled/svg/find.svg",
                                appearance.theme().active_ui_detail(),
                            )
                            .finish(),
                        )
                        .with_height(10.)
                        .with_width(10.)
                        .finish(),
                    )
                    .with_margin_right(3.)
                    .with_padding_top(12.)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        appearance
                            .ui_builder()
                            .text_input(self.search_editor.clone())
                            .with_style(UiComponentStyles {
                                border_radius: Some(CornerRadius::with_all(Radius::Pixels(0.))),
                                background: Some(Fill::None),
                                border_width: Some(0.),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_margin_left(TITLE_MARGIN)
        .finish()
    }

    fn render_list(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let themes = self
            .filtered_themes
            .as_ref()
            .unwrap_or_else(|| self.themes.as_ref())
            .to_vec();

        let selected_kind = self.selected_theme.clone();

        let list_len = themes.len();
        let element = if list_len == 0 {
            // renders a text & an empty rectangle that expands over the panel
            // without it, the theme picker panel would be shorter than the terminal window
            Flex::column()
                .with_child(
                    appearance
                        .ui_builder()
                        .span(crate::i18n::tr_static(app, "No matching themes!").to_string())
                        .build()
                        .finish(),
                )
                .with_child(Empty::new().finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish()
        } else {
            let list = UniformList::new(self.list_state.clone(), list_len, move |range, ctx| {
                let appearance = Appearance::as_ref(ctx);
                let font_family = appearance.ui_font_family();
                let monospace_font_family = appearance.monospace_font_family();
                let text_color = appearance.theme().active_ui_text_color().into();
                let selected_background_color = appearance.theme().surface_2();

                themes
                    .clone()
                    .into_iter()
                    .enumerate()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .map(|(_, item)| {
                        let selected = match &selected_kind {
                            Some(selected_kind) => selected_kind == &item.kind,
                            None => false,
                        };
                        let element = item.render(
                            selected,
                            font_family,
                            monospace_font_family,
                            text_color,
                            selected_background_color.into(),
                        );
                        EventHandler::new(element)
                            .on_left_mouse_down(move |ctx, _, _| {
                                ctx.dispatch_typed_action(ThemeChooserAction::Click(
                                    item.kind.clone(),
                                ));
                                DispatchEventResult::StopPropagation
                            })
                            .finish()
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            });
            let warp_theme = appearance.theme();

            Scrollable::vertical(
                self.scroll_state.clone(),
                list.finish_scrollable(),
                SCROLLBAR_WIDTH,
                warp_theme
                    .disabled_text_color(warp_theme.surface_2())
                    .into(),
                warp_theme.main_text_color(warp_theme.surface_2()).into(),
                Fill::None,
            )
            .finish()
        };

        Shrinkable::new(
            1.,
            ConstrainedBox::new(element).with_height(f32::MAX).finish(),
        )
        .finish()
    }
}

impl Entity for ThemeChooser {
    type Event = ThemeChooserEvent;
}

impl TypedActionView for ThemeChooser {
    type Action = ThemeChooserAction;

    fn handle_action(&mut self, action: &ThemeChooserAction, ctx: &mut ViewContext<Self>) {
        use ThemeChooserAction::*;

        match action {
            Up => self.up(ctx),
            Down => self.down(ctx),
            Click(kind) => self.click(kind.clone(), ctx),
            Close => self.close(ctx),
            Enter => self.enter(ctx),
            OpenThemeCreator => self.open_theme_creator_modal(ctx),
            OpenThemeDeletionModal(kind) => self.open_theme_deletion_modal(kind.clone(), ctx),
        }
    }
}

impl View for ThemeChooser {
    fn ui_name() -> &'static str {
        "ThemeChooser"
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
                "Theme chooser. Unfortunately, theme chooser window isn't compatible with screen readers yet.",
                "Press escape to close.",
                WarpA11yRole::WindowRole,
        ))
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let traffic_light_data = traffic_light_data(app, self.window_id);

        Container::new(
            Flex::column()
                .with_child(self.render_header(traffic_light_data.as_ref(), appearance, app))
                .with_child(self.render_title_row(appearance))
                .with_child(self.mode.render_hint_text(appearance, app))
                .with_child(self.render_search_bar(appearance))
                .with_child(self.render_list(appearance, app))
                .finish(),
        )
        .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_editor);
        }
    }
}

#[derive(Clone)]
struct ThemeChooserItem {
    pub kind: ThemeKind,
    warp_theme: WarpTheme,
    mouse_state: MouseStateHandle,
}

impl ThemeChooserItem {
    pub fn new(kind: ThemeKind, warp_theme: WarpTheme) -> Self {
        Self {
            kind,
            warp_theme,
            mouse_state: MouseStateHandle::default(),
        }
    }

    fn render_thumbnail(&self, font_family: FamilyId) -> Box<dyn Element> {
        theme::render_preview(&self.warp_theme, font_family, None)
    }

    pub fn render(
        &self,
        is_selected: bool,
        font_family: FamilyId,
        monospace_font_family: FamilyId,
        text_color: ColorU,
        selected_background_color: ColorU,
    ) -> Box<dyn Element> {
        Hoverable::new(self.mouse_state.clone(), |state| {
            let thumbnail = self.render_thumbnail(monospace_font_family);

            let name_text = Shrinkable::new(
                1.,
                Container::new(
                    ConstrainedBox::new(
                        Text::new_inline(self.kind.to_string(), font_family, THEME_NAME_FONT_SIZE)
                            .with_color(text_color)
                            .finish(),
                    )
                    .with_max_width(190.)
                    .finish(),
                )
                .with_margin_left(THEME_NAME_MARGIN_LEFT)
                .finish(),
            )
            .finish();

            let mut name_with_delete = Flex::row()
                .with_child(name_text)
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

            // Only show deletion button if custom theme and on hover
            if matches!(self.kind, ThemeKind::Custom(_)) && state.is_hovered() {
                let horizontal_line = ConstrainedBox::new(
                    Rect::new()
                        .with_background_color(ColorU::from_u32(0x000000ff))
                        .finish(),
                )
                .with_width(DELETE_BUTTON_LINE_WIDTH)
                .with_height(DELETE_BUTTON_LINE_HEIGHT)
                .finish();

                let delete_theme_button_circle = ConstrainedBox::new(
                    Rect::new()
                        .with_background(ColorU::from_u32(0xFF8272FF))
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .finish(),
                )
                .with_height(DELETE_BUTTON_SIZE)
                .with_width(DELETE_BUTTON_SIZE)
                .finish();

                let mut stack = Stack::new().with_child(delete_theme_button_circle);
                stack.add_positioned_child(
                    horizontal_line,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 0.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::Center,
                        ChildAnchor::Center,
                    ),
                );

                let theme_kind = self.kind.clone();
                name_with_delete.add_child(
                    EventHandler::new(
                        Container::new(stack.finish())
                            .with_margin_right(DELETE_BUTTON_MARGIN_RIGHT)
                            .finish(),
                    )
                    .on_left_mouse_down(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ThemeChooserAction::OpenThemeDeletionModal(
                            theme_kind.clone(),
                        ));
                        DispatchEventResult::StopPropagation
                    })
                    .finish(),
                );
            }

            let mut container = Container::new(
                Flex::column()
                    .with_child(thumbnail)
                    .with_child(
                        Container::new(name_with_delete.finish())
                            .with_margin_top(8.)
                            .finish(),
                    )
                    .finish(),
            )
            .with_padding_top(THEME_CHOOSER_ITEM_PADDING)
            .with_padding_bottom(THEME_CHOOSER_ITEM_PADDING);

            if is_selected {
                container = container.with_background_color(selected_background_color);
            }

            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}
