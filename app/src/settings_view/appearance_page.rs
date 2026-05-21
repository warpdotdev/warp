use super::directory_color_add_picker::{DirectoryColorAddPicker, DirectoryColorAddPickerEvent};
use super::settings_page::{
    AdditionalInfo, Category, LocalOnlyIconState, MatchData, PageType, SettingsWidget,
    CONTENT_FONT_SIZE,
};
use super::{flags, SettingsSection};
use super::{
    settings_page::{
        build_reset_button, render_body_item, render_body_item_label, render_dropdown_item,
        SettingsPageEvent, SettingsPageMeta, SettingsPageViewHandle, ToggleState, HEADER_PADDING,
    },
    SettingsAction,
};
use super::{SettingActionPairContexts, SettingActionPairDescriptions, ToggleSettingActionPair};
use crate::appearance::{Appearance, AppearanceEvent};
use crate::channel::{Channel, ChannelState};
use crate::context_chips::prompt::PromptEvent;
use crate::context_chips::renderer::ChipDragState;
use crate::context_chips::{
    prompt::Prompt, renderer::Renderer as ContextChipRenderer, ChipAvailability,
};
use crate::editor::{
    EditOrigin, Event as EditorEvent, InteractionState, SingleLineEditorOptions, TextOptions,
};
use crate::gpu_state::{GPUState, GPUStateEvent};
use crate::i18n::{self, I18nKey};
use crate::prompt::editor_modal::OpenSource as PromptEditorOpenSource;
use crate::server::telemetry::InputUXChangeOrigin;
use crate::settings::{
    active_theme_kind,
    app_icon::{AppIcon, AppIconSettings},
    respect_system_theme, AIFontName, AppEditorSettings, CursorBlink, CursorBlinkEnabled,
    DisplayLanguage, DisplayLanguageSetting, EnforceMinimumContrast, FocusPaneOnHover,
    FontSettings, FontSettingsChangedEvent, InputBoxType, InputModeSettings, InputModeState,
    LanguageSettings, MonospaceFontName, PaneSettings, ShouldDimInactivePanes, ThemeSettings,
    UseSystemTheme, DEFAULT_MONOSPACE_FONT_NAME,
};
use crate::settings::{CursorDisplayType, GPUSettings, InputSettings, InputSettingsChangedEvent};
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::blockgrid_element::BlockGridElement;
use crate::terminal::ligature_settings::{LigatureRenderingEnabled, LigatureSettings};
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::settings::{
    AltScreenPadding, AltScreenPaddingMode, Spacing, SpacingMode, TerminalSettings,
};
use crate::terminal::{BlockListSettings, ShowBlockDividers};
use crate::terminal::{ShowJumpToBottomOfBlockButton, SizeInfo};
use crate::themes::theme::{self, RespectSystemTheme, SelectedSystemThemes, ThemeKind, WarpTheme};
use crate::user_config::WarpConfig;
use crate::util::bindings;
use crate::window_settings::{
    BackgroundBlurRadius, BackgroundBlurTexture, BackgroundOpacity, LeftPanelVisibilityAcrossTabs,
    OpenWindowsAtCustomSize, WindowSettings, WindowSettingsChangedEvent, ZoomLevel,
};
use crate::workspace::header_toolbar_editor::HeaderToolbarInlineEditor;
use crate::workspace::tab_settings::{
    DirectoryTabColor, PreserveActiveTabColor, ShowCodeReviewButton, ShowIndicatorsButton,
    ShowVerticalTabPanelInRestoredWindows, TabCloseButtonPosition, TabSettings,
    TabSettingsChangedEvent, UseLatestUserPromptAsConversationTitleInTabNames, UseVerticalTabs,
    WorkspaceDecorationVisibility,
};
use crate::workspace::WorkspaceAction;
use crate::{editor::EditorView, themes::theme_chooser::ThemeChooserMode};
use crate::{
    features::FeatureFlag,
    view_components::{Dropdown, DropdownItem, FilterableDropdown},
};
use crate::{report_error, report_if_error, themes};
use crate::{send_telemetry_from_ctx, server::telemetry::TelemetryEvent};
use ::settings::{Setting, SettingSection, ToggleableSetting};
use enum_iterator::all;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use warp_core::ui::theme::color::internal_colors;
use warp_util::path::user_friendly_path;
use warpui::elements::{
    Clipped, Empty, FormattedTextElement, MainAxisAlignment, MainAxisSize, Text, Wrap,
};
use warpui::fonts::{FamilyId, FontInfo, Weight};
use warpui::keymap::{ContextPredicate, FixedBinding};
use warpui::platform::{Cursor, FilePickerConfiguration, GraphicsBackend};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::radio_buttons::{
    RadioButtonItem, RadioButtonLayout, RadioButtonStateHandle,
};
use warpui::ui_components::slider::SliderStateHandle;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::units::IntoPixels;

use warpui::id;
use warpui::{
    elements::{
        Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Dismiss, Element, Fill, Flex, Hoverable, MouseStateHandle, ParentElement, Radius,
        Shrinkable, DEFAULT_UI_LINE_HEIGHT_RATIO,
    },
    rendering::ThinStrokes,
};
use warpui::{platform::SystemTheme, Action};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, UpdateModel, View,
    ViewContext, ViewHandle, WindowId,
};

use crate::settings::UseThinStrokes;
use crate::ui_components::color_dot::{render_color_dot, TAB_COLOR_OPTIONS};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};

const FONT_SIZE_INPUT_BOX_WIDTH: f32 = 80.;
const NOTEBOOK_FONT_SIZE_INPUT_BOX_WIDTH: f32 = 50.;
const FONT_FAMILY_DROPDOWN_WIDTH: f32 = 225.;
const FONT_WEIGHT_DROPDOWN_WIDTH: f32 = 100.;
const LINE_HEIGHT_INPUT_BOX_WIDTH: f32 = 80.;
const OPACITY_SLIDER_WIDTH: f32 = 150.;
const MIN_FONT_SIZE: usize = 1;
const MAX_FONT_SIZE: usize = 120;
const MIN_LINE_SPACING: f32 = 0.1;
const MAX_LINE_SPACING: f32 = 5.;

const INPUT_MODE_DROPDOWN_WIDTH: f32 = 225.;

// Max and min sizes for new window creation in terms of rows and cols
const MIN_NEW_WINDOW_ROWS_OR_COLS: u16 = 5;
const MAX_NEW_WINDOW_ROWS_OR_COLS: u16 = 2000;

fn default_font_label(app: &AppContext, is_ai_font: bool) -> String {
    let suffix = i18n::tr(app, I18nKey::AppearanceTextDefaultSuffix);
    if is_ai_font {
        format!("{} ({suffix})", AIFontName::default_value())
    } else {
        format!("{} ({suffix})", MonospaceFontName::default_value())
    }
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    // Add all the toggle settings from the Appearance Page that you want to show up on the Command Palette here.
    let mut toggle_binding_pairs = vec![
        ToggleSettingActionPair::new(
            "compact mode",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleCompactMode,
            )),
            context,
            flags::COMPACT_MODE_CONTEXT_FLAG,
        ),
        ToggleSettingActionPair::new(
            "themes: sync with OS",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleRespectSystemTheme,
            )),
            context,
            flags::RESPECT_SYSTEM_THEME_CONTEXT_FLAG,
        ),
    ];

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "cursor blink",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleCursorBlink,
            )),
            context,
            flags::CURSOR_BLINK_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            AppEditorSettings::as_ref(app)
                .cursor_blink
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "jump to bottom of block button",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleJumpToBottomOfBlockButton,
            )),
            context,
            flags::JUMP_TO_BOTTOM_OF_BLOCK_BUTTON_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            BlockListSettings::as_ref(app)
                .show_jump_to_bottom_of_block_button
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "block dividers",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleShowBlockDividers,
            )),
            context,
            flags::BLOCK_DIVIDERS_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            BlockListSettings::as_ref(app)
                .show_block_dividers
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(ToggleSettingActionPair::new(
        "dim inactive panes",
        builder(SettingsAction::AppearancePageToggle(
            AppearancePageAction::ToggleDimInactivePanes,
        )),
        context,
        flags::DIM_INACTIVE_PANES_FLAG,
    ));

    app.register_fixed_bindings(vec![FixedBinding::empty(
        "Start Input at the Top".to_string(),
        builder(SettingsAction::AppearancePageToggle(
            AppearancePageAction::SetInputMode {
                new_mode: InputMode::Waterfall,
                from_binding: true,
            },
        )),
        context.to_owned(),
    )
    .with_group(bindings::BindingGroup::Settings.as_str())]);

    app.register_fixed_bindings(vec![FixedBinding::empty(
        "Pin Input to the Top".to_string(),
        builder(SettingsAction::AppearancePageToggle(
            AppearancePageAction::SetInputMode {
                new_mode: InputMode::PinnedToTop,
                from_binding: true,
            },
        )),
        context.to_owned(),
    )
    .with_group(bindings::BindingGroup::Settings.as_str())]);

    app.register_fixed_bindings(vec![FixedBinding::empty(
        "Pin Input to the Bottom".to_string(),
        builder(SettingsAction::AppearancePageToggle(
            AppearancePageAction::SetInputMode {
                new_mode: InputMode::PinnedToBottom,
                from_binding: true,
            },
        )),
        context.to_owned(),
    )]);

    // Add command palette entry for toggling between Warp and Classic input modes
    app.register_fixed_bindings(vec![FixedBinding::empty(
        "Toggle Input Mode (Warp/Classic)".to_string(),
        builder(SettingsAction::AppearancePageToggle(
            AppearancePageAction::ToggleInputMode,
        )),
        context.to_owned(),
    )
    .with_group(bindings::BindingGroup::Settings.as_str())]);

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "tab indicators",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleTabIndicators,
            )),
            context,
            flags::TAB_INDICATORS_FLAG,
        )
        .is_supported_on_current_platform(
            TabSettings::as_ref(app)
                .show_indicators
                .is_supported_on_current_platform(),
        ),
    );

    if !FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
        toggle_binding_pairs.push(
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Show code review button in tab bar",
                    "Hide code review button in tab bar",
                ),
                builder(SettingsAction::AppearancePageToggle(
                    AppearancePageAction::ToggleShowCodeReviewButton,
                )),
                SettingActionPairContexts::new(
                    context.to_owned() & !id!(flags::SHOW_CODE_REVIEW_BUTTON_FLAG),
                    context.to_owned() & id!(flags::SHOW_CODE_REVIEW_BUTTON_FLAG),
                ),
                None,
            )
            .is_supported_on_current_platform(
                TabSettings::as_ref(app)
                    .show_code_review_button
                    .is_supported_on_current_platform(),
            ),
        );
    }

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "focus follows mouse",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleFocusPaneOnHover,
            )),
            context,
            flags::FOCUS_PANES_ON_HOVER_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            PaneSettings::as_ref(app)
                .focus_panes_on_hover
                .is_supported_on_current_platform(),
        ),
    );

    if FeatureFlag::FullScreenZenMode.is_enabled() {
        // Add bindings for each visibility option.
        app.register_fixed_bindings([
            FixedBinding::empty(
                "Always show tab bar".to_string(),
                builder(SettingsAction::AppearancePageToggle(
                    AppearancePageAction::SetWorkspaceDecorationVisibility(
                        WorkspaceDecorationVisibility::AlwaysShow,
                    ),
                )),
                context.to_owned(),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
            FixedBinding::empty(
                "Hide tab bar if fullscreen".to_string(),
                builder(SettingsAction::AppearancePageToggle(
                    AppearancePageAction::SetWorkspaceDecorationVisibility(
                        WorkspaceDecorationVisibility::HideFullscreen,
                    ),
                )),
                context.to_owned(),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
            FixedBinding::empty(
                "Only show tab bar on hover".to_string(),
                builder(SettingsAction::AppearancePageToggle(
                    AppearancePageAction::SetWorkspaceDecorationVisibility(
                        WorkspaceDecorationVisibility::OnHover,
                    ),
                )),
                context.to_owned(),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
        ]);

        // Add a toggle alias for "Zen mode".
        toggle_binding_pairs.push(
            ToggleSettingActionPair::new(
                "zen mode",
                builder(SettingsAction::AppearancePageToggle(
                    AppearancePageAction::ToggleWorkspaceDecorationVisibility,
                )),
                context,
                flags::HIDE_WORKSPACE_DECORATIONS_CONTEXT_FLAG,
            )
            .is_supported_on_current_platform(
                TabSettings::as_ref(app)
                    .workspace_decoration_visibility
                    .is_supported_on_current_platform(),
            ),
        )
    }

    if FeatureFlag::VerticalTabs.is_enabled() {
        toggle_binding_pairs.push(ToggleSettingActionPair::new(
            "vertical tab layout",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleVerticalTabs,
            )),
            context,
            flags::USE_VERTICAL_TABS_FLAG,
        ));
        toggle_binding_pairs.push(ToggleSettingActionPair::new(
            "show vertical tabs panel in restored windows",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleShowVerticalTabPanelInRestoredWindows,
            )),
            context,
            flags::USE_VERTICAL_TABS_FLAG,
        ));
    }

    if FeatureFlag::Ligatures.is_enabled() {
        toggle_binding_pairs.push(ToggleSettingActionPair::new(
            "ligature rendering",
            builder(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleLigatureRendering,
            )),
            context,
            flags::LIGATURE_RENDERING_CONTEXT_FLAG,
        ));
    }

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(toggle_binding_pairs, app);
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum FontType {
    Any,
    #[default]
    Monospace,
}

impl FontType {
    fn toggle(self) -> Self {
        match self {
            Self::Monospace => Self::Any,
            Self::Any => Self::Monospace,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppearancePageAction {
    LineHeightEditorResetRatio,
    SetNewWindowsCustomColumns,
    SetNewWindowsCustomRows,
    SetFontSize,
    SetFontWeight(Weight),
    SetNotebookFontSize,
    SetLineHeight,
    SetOpacity(f32),
    SetBlur(f32),
    OpacitySliderDragged(f32),
    BlurSliderDragged(f32),
    SetFontFamily(String),
    SetAIFontFamily(String),
    SetThinStrokes(ThinStrokes),
    SetInputMode {
        new_mode: InputMode,
        from_binding: bool,
    },
    SetInputType(InputBoxType),
    SetDisplayLanguage(DisplayLanguage),
    SetAppIcon(AppIcon),
    SetCursorType(CursorDisplayType),
    SetWorkspaceDecorationVisibility(WorkspaceDecorationVisibility),
    ToggleWorkspaceDecorationVisibility,
    ToggleJumpToBottomOfBlockButton,
    ToggleShowBlockDividers,
    ToggleCompactMode,
    ToggleCursorBlink,
    ToggleRespectSystemTheme,
    ToggleOpenWindowsAtCustomSize,
    ToggleDimInactivePanes,
    ToggleAllAvailableFonts,
    ToggleMatchNotebookToMonospaceFontSize,
    ToggleMatchAIToTerminalFontFamily,
    ToggleTabIndicators,
    ToggleShowCodeReviewButton,
    TogglePreserveActiveTabColor,
    ToggleVerticalTabs,
    ToggleShowVerticalTabPanelInRestoredWindows,
    ToggleUseLatestUserPromptAsConversationTitleInTabNames,
    ToggleLigatureRendering,
    ToggleBlurTexture,
    ToggleLeftPanelVisibility,
    SetEnforceMinimumContrast(EnforceMinimumContrast),
    OpenUrl(String),
    ToggleFocusPaneOnHover,
    ToggleInputMode,
    UpdateAltScreenPaddingMode(AltScreenPaddingMode),
    SetTabCloseButtonPosition(TabCloseButtonPosition),
    SetZoomLevel(u16),
    ResetZoomLevel,
    SetDefaultDirectoryTabColor {
        path: PathBuf,
        color: DirectoryTabColor,
    },
    RemoveDefaultDirectoryTabColor {
        path: PathBuf,
    },
}

pub struct AppearanceSettingsPageView {
    page: PageType<Self>,
    window_id: WindowId,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    font_size_editor: ViewHandle<EditorView>,
    line_height_editor: ViewHandle<EditorView>,
    notebook_font_size_editor: ViewHandle<EditorView>,
    ai_font_family_dropdown: ViewHandle<FilterableDropdown<AppearancePageAction>>,
    new_window_columns_editor: ViewHandle<EditorView>,
    valid_new_window_columns: bool,
    new_window_rows_editor: ViewHandle<EditorView>,
    valid_new_window_rows: bool,
    opacity_state: SliderStateHandle,
    blur_state: SliderStateHandle,
    font_family_dropdown: ViewHandle<FilterableDropdown<AppearancePageAction>>,
    font_weight_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    #[allow(dead_code)]
    thin_strokes_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    enforce_min_contrast_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    input_mode_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    input_type_radio_state: RadioButtonStateHandle,
    language_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    app_icon_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    workspace_decorations_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    tab_close_button_position_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    zoom_level_dropdown: ViewHandle<Dropdown<AppearancePageAction>>,
    zoom_reset_button_mouse_state: MouseStateHandle,
    available_families: HashMap<String, (Option<FamilyId>, FontType)>,
    view_font_type: FontType,
    alt_screen_padding_editor: ViewHandle<EditorView>,
    color_picker_dot_states: Vec<Vec<MouseStateHandle>>,
    directory_tab_color_delete_buttons: Vec<ViewHandle<ActionButton>>,
    header_toolbar_inline_editor: ViewHandle<HeaderToolbarInlineEditor>,

    /// The context chip renderers based on the most recently
    /// selected Warp prompt configuration.
    context_chips: Vec<ContextChipRenderer>,

    /// The information we need to render the PS1 as a grid when we're
    /// honoring the user's PS1.
    ps1_grid_info: Option<(BlockGrid, SizeInfo)>,
}

impl Entity for AppearanceSettingsPageView {
    type Event = SettingsPageEvent;
}

impl TypedActionView for AppearanceSettingsPageView {
    type Action = AppearancePageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use AppearancePageAction::*;

        match action {
            LineHeightEditorResetRatio => self.reset_line_height_ratio(ctx),
            SetNewWindowsCustomColumns => self.update_new_windows_num_columns(true, ctx),
            SetNewWindowsCustomRows => self.update_new_windows_num_rows(true, ctx),
            SetFontSize => self.set_font_size(ctx),
            SetFontWeight(value) => self.set_font_weight(*value, ctx),
            ToggleMatchNotebookToMonospaceFontSize => {
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    report_if_error!(font_settings
                        .match_notebook_to_monospace_font_size
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleMatchAIToTerminalFontFamily => self.toggle_match_ai_font_to_terminal_font(ctx),
            SetNotebookFontSize => self.set_notebook_font_size(ctx),
            SetLineHeight => self.set_line_height_ratio(ctx),
            SetOpacity(value) => self.set_opacity(*value, true, ctx),
            SetBlur(value) => self.set_blur(*value, true, ctx),
            SetFontFamily(name) => self.set_font_family(name, ctx),
            SetAIFontFamily(name) => {
                self.set_ai_font_family(name, ctx);
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    report_if_error!(font_settings
                        .match_ai_font_to_terminal_font
                        .set_value(false, ctx));
                });
            }
            SetThinStrokes(value) => self.set_thin_strokes(value, ctx),
            SetEnforceMinimumContrast(value) => {
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    report_if_error!(font_settings
                        .enforce_minimum_contrast
                        .set_value(*value, ctx,));
                });
            }
            SetWorkspaceDecorationVisibility(value) => {
                self.set_workspace_decoration_visibility(*value, ctx)
            }
            ToggleWorkspaceDecorationVisibility => self.toggle_workspace_decoration_visiblity(ctx),
            ToggleJumpToBottomOfBlockButton => self.toggle_jump_to_bottom_of_block_button(ctx),
            ToggleShowBlockDividers => self.toggle_show_block_dividers(ctx),
            ToggleCompactMode => self.toggle_compact_mode(ctx),
            ToggleCursorBlink => self.toggle_cursor_blink(ctx),
            ToggleOpenWindowsAtCustomSize => self.toggle_open_windows_at_custom_size(ctx),
            ToggleRespectSystemTheme => self.toggle_respect_system_theme(ctx),
            ToggleAllAvailableFonts => self.toggle_all_available_fonts(ctx),
            ToggleDimInactivePanes => self.toggle_dim_inactive_panes(ctx),
            ToggleBlurTexture => self.toggle_blur_texture(ctx),
            ToggleLeftPanelVisibility => self.toggle_left_panel_visibility(ctx),
            SetInputMode {
                new_mode,
                from_binding,
            } => self.set_input_mode(*new_mode, *from_binding, ctx),
            SetInputType(input_type) => self.set_input_type(*input_type, ctx),
            SetDisplayLanguage(language) => self.set_display_language(*language, ctx),
            SetAppIcon(new_icon) => self.set_app_icon(*new_icon, ctx),
            SetCursorType(cursor_display_type) => self.set_cursor_type(*cursor_display_type, ctx),
            OpacitySliderDragged(val) => self.set_opacity(*val, false, ctx),
            BlurSliderDragged(val) => self.set_blur(*val, false, ctx),
            OpenUrl(url) => {
                ctx.open_url(url);
            }
            ToggleTabIndicators => self.toggle_tab_indicators(ctx),
            ToggleShowCodeReviewButton => self.toggle_show_code_review_button(ctx),
            TogglePreserveActiveTabColor => self.toggle_preserve_active_tab_color(ctx),
            ToggleVerticalTabs => self.toggle_vertical_tabs(ctx),
            ToggleShowVerticalTabPanelInRestoredWindows => {
                self.toggle_show_vertical_tab_panel_in_restored_windows(ctx)
            }
            ToggleUseLatestUserPromptAsConversationTitleInTabNames => {
                self.toggle_use_latest_user_prompt_as_conversation_title_in_tab_names(ctx)
            }
            ToggleLigatureRendering => self.toggle_ligature_rendering(ctx),
            ToggleFocusPaneOnHover => {
                PaneSettings::handle(ctx).update(ctx, |pane_settings, ctx| {
                    match pane_settings
                        .focus_panes_on_hover
                        .toggle_and_save_value(ctx)
                    {
                        Ok(new_val) => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::ToggleFocusPaneOnHover { enabled: new_val },
                                ctx
                            );
                        }
                        Err(e) => {
                            report_error!(e);
                        }
                    }
                });
                ctx.notify();
            }
            ToggleInputMode => {
                self.toggle_input_mode(ctx);
            }
            UpdateAltScreenPaddingMode(new_mode) => {
                TerminalSettings::handle(ctx).update(ctx, |terminal_settings, ctx| {
                    report_if_error!(terminal_settings
                        .alt_screen_padding
                        .set_value(*new_mode, ctx));
                });
                self.set_alt_screen_padding_editor_text(ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::UpdateAltScreenPaddingMode {
                        new_mode: *new_mode,
                    },
                    ctx
                );
            }
            SetTabCloseButtonPosition(position) => {
                self.update_tab_close_button_position(*position, ctx);
            }
            SetZoomLevel(zoom_level) => {
                WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
                    report_if_error!(window_settings.zoom_level.set_value(*zoom_level, ctx));
                });
                ctx.notify();
            }
            ResetZoomLevel => {
                WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
                    report_if_error!(window_settings.zoom_level.clear_value(ctx));
                });
                ctx.notify();
            }
            SetDefaultDirectoryTabColor { path, color } => {
                let path = path.clone();
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_value = settings
                        .directory_tab_colors
                        .value()
                        .with_color(&path, *color);
                    let _ = settings.directory_tab_colors.set_value(new_value, ctx);
                });
                ctx.notify();
            }
            RemoveDefaultDirectoryTabColor { path } => {
                let path = path.clone();
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_value = settings
                        .directory_tab_colors
                        .value()
                        .with_color(&path, DirectoryTabColor::Suppressed);
                    let _ = settings.directory_tab_colors.set_value(new_value, ctx);
                });
                ctx.notify();
            }
        }
    }
}

impl View for AppearanceSettingsPageView {
    fn ui_name() -> &'static str {
        "AppearanceSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl AppearanceSettingsPageView {
    fn editor<F>(
        mut event_handler: F,
        buffer_text: &str,
        ui_font_size: f32,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<EditorView>
    where
        F: 'static + FnMut(&mut AppearanceSettingsPageView, &EditorEvent, &mut ViewContext<Self>),
    {
        let editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(ui_font_size),
                    ..Default::default()
                },
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });

        editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(buffer_text, ctx);
        });

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            event_handler(me, event, ctx);
        });

        editor
    }

    pub fn new(ctx: &mut ViewContext<AppearanceSettingsPageView>) -> Self {
        let (
            ui_font_size,
            monospace_font_size,
            line_height_ratio,
            monospace_font_weight,
            notebook_font_size,
            match_notebook_to_monospace_font_size,
        ) = {
            let appearance = Appearance::as_ref(ctx);
            let font_settings = FontSettings::as_ref(ctx);
            (
                appearance.ui_font_size(),
                appearance.monospace_font_size(),
                appearance.line_height_ratio(),
                appearance.monospace_font_weight(),
                *font_settings.notebook_font_size,
                *font_settings.match_notebook_to_monospace_font_size,
            )
        };

        let font_size_editor = Self::editor(
            |me, event, ctx| me.handle_font_size_editor_event(event, ctx),
            &format!("{monospace_font_size}"),
            ui_font_size,
            ctx,
        );

        let notebook_font_size_editor = Self::editor(
            |me, event, ctx| me.handle_notebook_font_size_editor_event(event, ctx),
            &format!("{notebook_font_size}"),
            ui_font_size,
            ctx,
        );

        if match_notebook_to_monospace_font_size {
            notebook_font_size_editor.update(ctx, |editor_view, ctx| {
                editor_view.set_interaction_state(InteractionState::Disabled, ctx);
            })
        }

        ctx.subscribe_to_model(&GPUState::handle(ctx), |_, _, event, ctx| {
            if matches!(event, GPUStateEvent::LowPowerGPUAvailable) {
                ctx.notify();
            }
        });

        let appearance_handle = Appearance::handle(ctx);
        ctx.subscribe_to_model(&appearance_handle, Self::handle_appearance_update);

        ctx.subscribe_to_model(&PaneSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&TerminalSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(
            &FontSettings::handle(ctx),
            |me, font_settings, event, ctx| match event {
                FontSettingsChangedEvent::NotebookFontSize { .. }
                | FontSettingsChangedEvent::MatchNotebookToMonospaceFontSize { .. } => {
                    let font_settings = font_settings.as_ref(ctx);
                    let should_match_notebook_to_monospace_font_size =
                        *font_settings.match_notebook_to_monospace_font_size;
                    let notebook_font_size = *font_settings.notebook_font_size;

                    me.notebook_font_size_editor
                        .update(ctx, move |editor, ctx| {
                            let interaction_state = if should_match_notebook_to_monospace_font_size
                            {
                                InteractionState::Disabled
                            } else {
                                InteractionState::Editable
                            };
                            editor.set_buffer_text(&format!("{notebook_font_size}"), ctx);
                            editor.set_interaction_state(interaction_state, ctx);
                        });

                    ctx.notify();
                }
                FontSettingsChangedEvent::EnforceMinimumContrast { .. } => {
                    me.enforce_min_contrast_dropdown
                        .update(ctx, |dropdown, ctx| {
                            let enforce_minimum_contrast =
                                *FontSettings::as_ref(ctx).enforce_minimum_contrast;
                            let name = Self::enforce_minimum_contrast_dropdown_item_label(
                                ctx,
                                enforce_minimum_contrast,
                            );
                            dropdown.set_selected_by_name(name, ctx);
                        });
                    ctx.notify();
                }
                FontSettingsChangedEvent::UseThinStrokes { .. } => {
                    me.thin_strokes_dropdown.update(ctx, |dropdown, ctx| {
                        let thin_strokes = *FontSettings::as_ref(ctx).use_thin_strokes;
                        dropdown.set_selected_by_name(
                            Self::thin_strokes_dropdown_item_label(ctx, thin_strokes),
                            ctx,
                        );
                    });
                    ctx.notify();
                }
                _ => {}
            },
        );

        let block_list_settings_handle = BlockListSettings::handle(ctx);
        ctx.subscribe_to_model(&block_list_settings_handle, |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&Prompt::handle(ctx), Self::handle_prompt_update);

        let ligature_settings_handle = LigatureSettings::handle(ctx);
        ctx.subscribe_to_model(&ligature_settings_handle, |_, _, _, ctx| ctx.notify());

        ctx.subscribe_to_model(&InputModeSettings::handle(ctx), |me, _, _, ctx| {
            me.input_mode_dropdown.update(ctx, |dropdown, ctx| {
                let input_mode = *InputModeSettings::as_ref(ctx).input_mode;
                dropdown.set_selected_by_name(
                    Self::input_mode_dropdown_item_label(ctx, input_mode),
                    ctx,
                );
                ctx.notify();
            });
            ctx.notify()
        });

        ctx.subscribe_to_model(&InputSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, InputSettingsChangedEvent::InputBoxTypeSetting { .. }) {
                let input_type = *InputSettings::as_ref(ctx).input_box_type;
                me.input_type_radio_state
                    .set_selected_idx(input_type as usize);
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&LanguageSettings::handle(ctx), |_, _, _, ctx| {
            let _ = ctx.spawn(
                async {
                    warpui::r#async::Timer::after(std::time::Duration::from_millis(1)).await;
                },
                |me, _, ctx| {
                    me.update_localized_dropdowns(ctx);
                    me.update_language_dropdown(ctx);
                    me.update_font_dropdown(ctx);
                    ctx.notify();
                },
            );
        });
        ctx.subscribe_to_model(&AppIconSettings::handle(ctx), |me, _, _, ctx| {
            me.app_icon_dropdown.update(ctx, |dropdown, ctx| {
                let app_icon = *AppIconSettings::as_ref(ctx).app_icon;
                dropdown
                    .set_selected_by_name(Self::app_icon_dropdown_item_label(ctx, app_icon), ctx);
                ctx.notify();
            });
            ctx.notify()
        });
        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&BlockListSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&WindowSettings::handle(ctx), |me, _, evt, ctx| {
            match evt {
                WindowSettingsChangedEvent::NewWindowsNumColumns { .. } => {
                    // Update the value of the columns input to match the new setting
                    me.new_window_columns_editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text(
                            &format!(
                                "{}",
                                WindowSettings::handle(ctx)
                                    .as_ref(ctx)
                                    .new_windows_num_columns
                                    .value()
                            ),
                            ctx,
                        );
                    });
                }
                WindowSettingsChangedEvent::NewWindowsNumRows { .. } => {
                    // Update the value of the rows input to match the new setting
                    me.new_window_rows_editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text(
                            &format!(
                                "{}",
                                WindowSettings::handle(ctx)
                                    .as_ref(ctx)
                                    .new_windows_num_rows
                                    .value()
                            ),
                            ctx,
                        );
                    });
                }
                WindowSettingsChangedEvent::BackgroundOpacity { .. } => {
                    // Reset the slider state so that it uses the current opacity value on the next render.
                    me.opacity_state.reset_offset();
                }
                WindowSettingsChangedEvent::BackgroundBlurRadius { .. } => {
                    // Reset the slider state so that it uses the current opacity value on the next render.
                    me.blur_state.reset_offset();
                }
                WindowSettingsChangedEvent::ZoomLevel { .. } => {
                    let zoom_level = *WindowSettings::as_ref(ctx).zoom_level;

                    me.zoom_level_dropdown.update(ctx, |dropdown, ctx| {
                        dropdown.set_selected_by_action(
                            AppearancePageAction::SetZoomLevel(zoom_level),
                            ctx,
                        );
                    });

                    let zoom_factor = WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor();
                    ctx.set_zoom_factor(zoom_factor);
                }
                _ => {}
            };
            ctx.notify();
        });
        ctx.subscribe_to_model(&TabSettings::handle(ctx), |me, _, event, ctx| {
            me.handle_tab_settings_event(event, ctx)
        });

        // we need to update the switch if the setting gets changed elsewhere, like command palette
        ctx.subscribe_to_model(&AppEditorSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        let line_height_editor = Self::editor(
            |me, event, ctx| me.handle_line_editor_event(event, ctx),
            &format!("{line_height_ratio}"),
            ui_font_size,
            ctx,
        );

        let window_settings = WindowSettings::as_ref(ctx);
        let (num_columns, num_rows) = (
            *window_settings.new_windows_num_columns.value(),
            *window_settings.new_windows_num_rows.value(),
        );
        let new_window_columns_editor = Self::editor(
            |me, event, ctx| {
                me.update_new_windows_num_columns(
                    matches!(event, EditorEvent::Blurred | EditorEvent::Enter),
                    ctx,
                );
                if let EditorEvent::Escape = event {
                    ctx.emit(SettingsPageEvent::FocusModal);
                }
            },
            &format!("{num_columns}"),
            ui_font_size,
            ctx,
        );

        let new_window_rows_editor = Self::editor(
            |me, event, ctx| {
                me.update_new_windows_num_rows(
                    matches!(event, EditorEvent::Blurred | EditorEvent::Enter),
                    ctx,
                );
                if let EditorEvent::Escape = event {
                    ctx.emit(SettingsPageEvent::FocusModal);
                }
            },
            &format!("{num_rows}"),
            ui_font_size,
            ctx,
        );

        // Don't load all available system fonts in integration tests; we don't
        // have any integration tests which interact with the font dropdown, and
        // loading them in the background slows down test execution.
        if ChannelState::channel() != Channel::Integration {
            // There's no such thing as a "system font" on the web, so the
            // `all_system_fonts` API doesn't exist.
            #[cfg(not(target_family = "wasm"))]
            {
                let all_system_fonts = warpui::fonts::Cache::handle(ctx)
                    .update(ctx, |font_cache, ctx| font_cache.all_system_fonts(ctx));
                ctx.spawn(all_system_fonts, Self::set_system_fonts);
            }
        }
        let font_family_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(FONT_FAMILY_DROPDOWN_WIDTH);
            dropdown.set_menu_width(FONT_FAMILY_DROPDOWN_WIDTH, ctx);

            // Initialize dropdown with the default font in case system fonts failed to load.
            dropdown.add_items(vec![Self::default_font_item(ctx, false)], ctx);
            dropdown.set_selected_by_index(0, ctx);
            dropdown
        });

        let ai_font_family_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(FONT_FAMILY_DROPDOWN_WIDTH);
            dropdown.set_menu_width(FONT_FAMILY_DROPDOWN_WIDTH, ctx);

            // Initialize dropdown with the default font in case system fonts failed to load.
            dropdown.add_items(vec![Self::default_font_item(ctx, true)], ctx);
            dropdown.set_selected_by_index(0, ctx);
            dropdown
        });

        let font_weight_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(FONT_WEIGHT_DROPDOWN_WIDTH);
            dropdown.set_menu_width(FONT_WEIGHT_DROPDOWN_WIDTH, ctx);

            let selectable_weights = [Weight::Normal, Weight::Bold];
            let items = selectable_weights
                .iter()
                .map(|weight| {
                    DropdownItem::new(
                        weight.to_string(),
                        AppearancePageAction::SetFontWeight(*weight),
                    )
                })
                .collect();
            dropdown.add_items(items, ctx);
            dropdown.set_selected_by_name(monospace_font_weight.to_string(), ctx);
            dropdown
        });

        let thin_strokes_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);

            let values = vec![
                ThinStrokes::Never,
                ThinStrokes::OnLowDpiDisplays,
                ThinStrokes::OnHighDpiDisplays,
                ThinStrokes::Always,
            ];

            let current_value = ctx.rendering_config().glyphs.use_thin_strokes;
            let selected_index = values
                .iter()
                .position(|val| *val == current_value)
                .unwrap_or_else(|| {
                    log::error!("Could not find current ThinStrokes value in dropdown option list");
                    0
                });

            dropdown.add_items(Self::thin_strokes_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        });

        let input_mode_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(INPUT_MODE_DROPDOWN_WIDTH);
            dropdown.set_menu_width(INPUT_MODE_DROPDOWN_WIDTH, ctx);

            let values = vec![
                InputMode::PinnedToBottom,
                InputMode::Waterfall,
                InputMode::PinnedToTop,
            ];
            let current_value = *InputModeSettings::as_ref(ctx).input_mode.value();
            let selected_index: usize = values
                .iter()
                .position(|val| *val == current_value)
                .unwrap_or_else(|| {
                    log::error!("Could not find current InputMode value in dropdown option list");
                    0
                });

            dropdown.add_items(Self::input_mode_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        });

        let app_icon_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(INPUT_MODE_DROPDOWN_WIDTH);
            dropdown.set_menu_width(INPUT_MODE_DROPDOWN_WIDTH, ctx);

            let values: Vec<AppIcon> = all::<AppIcon>().collect();
            let current_value = *AppIconSettings::as_ref(ctx).app_icon;
            let selected_index = values
                .iter()
                .position(|val| *val == current_value)
                .unwrap_or_else(|| {
                    log::error!("Could not find current AppIcon value in dropdown option list");
                    0
                });

            dropdown.add_items(Self::app_icon_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        });

        let enforce_min_contrast_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);

            let values = vec![
                EnforceMinimumContrast::Always,
                EnforceMinimumContrast::OnlyNamedColors,
                EnforceMinimumContrast::Never,
            ];
            let current_value = *FontSettings::as_ref(ctx)
                .enforce_minimum_contrast;
            let selected_index = values.iter().position(|val| *val == current_value).unwrap_or_else(|| {
                log::error!("Could not find current EnforceMinimumContrast value in dropdown option list");
                0
            });

            dropdown.add_items(Self::enforce_minimum_contrast_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_index(selected_index, ctx);
            dropdown
        });

        let context_chips = Self::get_context_chip_renderers(ctx);

        let alt_screen_padding_editor = {
            let width_and_height_editor_options = SingleLineEditorOptions {
                text: TextOptions::ui_font_size(appearance_handle.as_ref(ctx)),
                ..Default::default()
            };
            ctx.add_typed_action_view(|ctx| {
                let mut editor =
                    EditorView::single_line(width_and_height_editor_options.clone(), ctx);
                if let AltScreenPaddingMode::Custom { uniform_padding } =
                    *TerminalSettings::as_ref(ctx).alt_screen_padding
                {
                    let val = format!("{:.1}", uniform_padding.as_f32());
                    // Do a system edit to avoid counting this update as part of telemetry.
                    editor.system_reset_buffer_text(val.trim_end_matches(".0"), ctx);
                }
                editor
            })
        };
        ctx.subscribe_to_view(&alt_screen_padding_editor, |me, _, event, ctx| {
            me.handle_alt_screen_padding_editor_event(event, ctx);
        });

        // Initialize the input type radio state
        let input_type = InputSettings::as_ref(ctx).input_type(ctx);
        let input_type_radio_state = RadioButtonStateHandle::default();
        input_type_radio_state.set_selected_idx(input_type as usize);
        let header_toolbar_inline_editor =
            ctx.add_typed_action_view(HeaderToolbarInlineEditor::new);

        AppearanceSettingsPageView {
            page: Self::build_page(ctx),
            window_id: ctx.window_id(),
            local_only_icon_tooltip_states: Default::default(),
            ai_font_family_dropdown,
            notebook_font_size_editor,
            font_size_editor,
            line_height_editor,
            new_window_columns_editor,
            valid_new_window_columns: true,
            new_window_rows_editor,
            valid_new_window_rows: true,
            opacity_state: Default::default(),
            blur_state: Default::default(),
            font_family_dropdown,
            font_weight_dropdown,
            thin_strokes_dropdown,
            input_mode_dropdown,
            input_type_radio_state,
            language_dropdown: Self::build_language_dropdown(ctx),
            app_icon_dropdown,
            enforce_min_contrast_dropdown,
            workspace_decorations_dropdown: Self::build_workspace_decoration_visibility_dropdown(
                ctx,
            ),
            tab_close_button_position_dropdown: Self::build_tab_close_button_position_dropdown(ctx),
            zoom_level_dropdown: Self::build_zoom_level_dropdown(ctx),
            zoom_reset_button_mouse_state: MouseStateHandle::default(),
            available_families: Default::default(),
            view_font_type: Default::default(),
            color_picker_dot_states: (0..directory_tab_colors(ctx).len())
                .map(|_| {
                    (0..TAB_COLOR_OPTIONS.len() + 1)
                        .map(|_| MouseStateHandle::default())
                        .collect()
                })
                .collect(),
            directory_tab_color_delete_buttons: build_directory_delete_buttons(ctx),
            header_toolbar_inline_editor,
            alt_screen_padding_editor,
            context_chips,
            ps1_grid_info: None,
        }
    }

    fn build_page(ctx: &mut ViewContext<Self>) -> PageType<Self> {
        let mut categories = vec![Category::new_i18n(
            I18nKey::AppearanceCategoryLanguage,
            vec![Box::new(LanguageWidget::default())],
        )];

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryThemes,
            vec![
                Box::new(CreateCustomThemeWidget::default()),
                Box::new(ThemeSelectWidget::default()),
            ],
        ));

        if AppIconSettings::as_ref(ctx).is_supported_on_current_platform() {
            categories.push(Category::new_i18n(
                I18nKey::AppearanceCategoryIcon,
                vec![Box::new(CustomAppIconWidget::default())],
            ));
        }

        let window_settings = WindowSettings::as_ref(ctx);
        let mut window_settings_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        if window_settings
            .open_windows_at_custom_size
            .is_supported_on_current_platform()
        {
            window_settings_widgets.push(Box::new(CustomWindowSizeWidget::default()));
        }
        if window_settings
            .background_opacity
            .is_supported_on_current_platform()
        {
            window_settings_widgets.push(Box::new(WindowOpacityWidget::default()));
        }
        if window_settings
            .background_blur_radius
            .is_supported_on_current_platform()
        {
            window_settings_widgets.push(Box::new(WindowBlurWidget::default()));
        }
        if window_settings
            .background_blur_texture
            .is_supported_on_current_platform()
        {
            window_settings_widgets.push(Box::new(WindowBlurTextureWidget::default()));
        }

        if FeatureFlag::UIZoom.is_enabled() {
            window_settings_widgets.push(Box::new(ZoomLevelWidget));
        }

        if window_settings
            .left_panel_visibility_across_tabs
            .is_supported_on_current_platform()
        {
            window_settings_widgets.push(Box::new(ToolsPanelStateScopeWidget::default()));
        }

        if !window_settings_widgets.is_empty() {
            categories.push(Category::new_i18n(
                I18nKey::AppearanceCategoryWindow,
                window_settings_widgets,
            ));
        }

        // Create the Input category with all widgets
        // The PromptWidget and InputModeWidget will handle their own visibility

        let category_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(InputTypeWidget::default()),
            Box::new(PromptWidget::default()),
            Box::new(InputModeWidget::default()),
        ];

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryInput,
            category_widgets,
        ));

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryPanes,
            vec![
                Box::new(DimInactivePanesWidget::default()),
                Box::new(FocusFollowsMouseWidget::default()),
            ],
        ));

        let mut block_settings_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(CompactModeWidget::default()),
            Box::new(JumpToBottomOfBlockWidget::default()),
        ];
        if FeatureFlag::MinimalistUI.is_enabled() {
            block_settings_widgets.push(Box::new(ShowBlockDividersWidget::default()));
        }
        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryBlocks,
            block_settings_widgets,
        ));

        let font_settings = FontSettings::as_ref(ctx);
        let mut text_settings_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(TerminalFontWidget::default()),
            Box::new(AIFontWidget::default()),
            Box::new(NotebookFontSizeWidget::default()),
        ];
        if font_settings
            .use_thin_strokes
            .is_supported_on_current_platform()
        {
            text_settings_widgets.push(Box::new(ThinStrokesWidget::default()));
        }
        if font_settings
            .enforce_minimum_contrast
            .is_supported_on_current_platform()
        {
            text_settings_widgets.push(Box::new(MinimumContrastWidget::default()));
        }
        let ligature_settings = LigatureSettings::as_ref(ctx);
        if FeatureFlag::Ligatures.is_enabled()
            && ligature_settings
                .ligature_rendering_enabled
                .is_supported_on_current_platform()
        {
            text_settings_widgets.push(Box::new(LigaturesWidget::default()));
        }

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryText,
            text_settings_widgets,
        ));

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryCursor,
            vec![
                Box::new(CursorTypeWidget::default()),
                Box::new(BlinkingCursorWidget::default()),
            ],
        ));

        let tab_settings = TabSettings::as_ref(ctx);
        let mut tab_settings_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(TabIndicatorWidget::default())];
        if !FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            tab_settings_widgets.push(Box::new(CodeReviewButtonWidget::default()));
        }
        if FeatureFlag::FullScreenZenMode.is_enabled()
            && tab_settings
                .workspace_decoration_visibility
                .is_supported_on_current_platform()
        {
            tab_settings_widgets.push(Box::new(ZenModeWidget::default()));
        }
        if FeatureFlag::TabCloseButtonOnLeft.is_enabled() {
            tab_settings_widgets.push(Box::new(TabCloseButtonPositionWidget::default()));
        }
        tab_settings_widgets.push(Box::new(PreserveActiveTabColorWidget::default()));

        if FeatureFlag::VerticalTabs.is_enabled() {
            tab_settings_widgets.push(Box::new(VerticalTabsWidget::default()));
            tab_settings_widgets.push(Box::new(
                ShowVerticalTabPanelInRestoredWindowsWidget::default(),
            ));
            tab_settings_widgets.push(Box::new(
                UseLatestUserPromptAsConversationTitleInTabNamesWidget::default(),
            ));
            if FeatureFlag::ConfigurableToolbar.is_enabled() {
                tab_settings_widgets.push(Box::new(EditToolbarWidget));
            }
        }

        if FeatureFlag::DirectoryTabColors.is_enabled() {
            let add_picker = ctx.add_typed_action_view(DirectoryColorAddPicker::new);
            ctx.subscribe_to_view(&add_picker, |me, _, event, ctx| {
                me.handle_directory_color_add_picker_event(event, ctx);
            });
            tab_settings_widgets.push(Box::new(DirectoryTabColorsWidget { add_picker }));
        }

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryTabs,
            tab_settings_widgets,
        ));

        categories.push(Category::new_i18n(
            I18nKey::AppearanceCategoryFullscreenApps,
            vec![Box::new(AltScreenPaddingWidget::default())],
        ));

        PageType::new_categorized(categories, None)
    }

    fn set_alt_screen_padding_editor_text(&mut self, ctx: &mut ViewContext<Self>) {
        if let AltScreenPaddingMode::Custom { uniform_padding } =
            *TerminalSettings::as_ref(ctx).alt_screen_padding
        {
            self.alt_screen_padding_editor.update(ctx, |editor, ctx| {
                let val = format!("{:.1}", uniform_padding.as_f32());
                editor.set_buffer_text(val.trim_end_matches(".0"), ctx);
            });
            ctx.notify();
        }
    }

    fn handle_appearance_update(
        &mut self,
        handle: ModelHandle<Appearance>,
        event: &AppearanceEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AppearanceEvent::MonospaceFontFamilyChanged { .. } => {
                self.update_font_dropdown(ctx);
            }
            AppearanceEvent::MonospaceFontSizeChanged { .. } => {
                let font_size = handle.as_ref(ctx).monospace_font_size();
                self.font_size_editor.update(ctx, move |editor, ctx| {
                    editor.set_buffer_text(&format!("{font_size}"), ctx);
                });
            }
            AppearanceEvent::MonospaceFontWeightChanged { .. } => {
                let font_weight = handle.as_ref(ctx).monospace_font_weight();
                self.font_weight_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_name(font_weight.to_string(), ctx);
                });
            }
            AppearanceEvent::LineHeightRatioChanged { .. } => {
                let line_height_ratio = handle.as_ref(ctx).line_height_ratio();
                self.line_height_editor.update(ctx, move |editor, ctx| {
                    editor.set_buffer_text(&format!("{line_height_ratio}"), ctx);
                });
            }
            _ => {}
        }

        ctx.notify();
    }

    fn get_context_chip_renderers(app: &AppContext) -> Vec<ContextChipRenderer> {
        let appearance = Appearance::as_ref(app);
        let prompt = Prompt::as_ref(app);
        prompt
            .chip_kinds()
            .into_iter()
            .filter_map(|kind| {
                ContextChipRenderer::default_from_kind(kind, ChipAvailability::Enabled, appearance)
            })
            .collect()
    }

    fn handle_prompt_update(
        &mut self,
        _prompt: ModelHandle<Prompt>,
        _event: &PromptEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        self.context_chips = Self::get_context_chip_renderers(ctx);
    }

    fn default_font_item<V>(
        ctx: &mut ViewContext<V>,
        is_ai_font: bool,
    ) -> DropdownItem<AppearancePageAction>
    where
        V: View,
    {
        let font_name = if is_ai_font {
            AIFontName::default_value()
        } else {
            MonospaceFontName::default_value()
        };
        let mut initial_dropdown_item = DropdownItem::new(
            default_font_label(ctx, is_ai_font),
            if is_ai_font {
                AppearancePageAction::SetAIFontFamily(font_name.clone())
            } else {
                AppearancePageAction::SetFontFamily(font_name.clone())
            },
        );

        // If we're on a non-Linux platform, render the dropdown item in the
        // actual font.  We currently don't do this on Linux because
        // pre-loading all of the fonts is too expensive.
        if cfg!(not(any(target_os = "linux", target_os = "freebsd"))) {
            if let Some(family_id) = ctx.font_cache().family_id_for_name(&font_name) {
                initial_dropdown_item = initial_dropdown_item.with_font_override(family_id);
            }
        }

        initial_dropdown_item
    }

    fn input_mode_dropdown_item_label(app: &AppContext, val: InputMode) -> &'static str {
        match val {
            InputMode::PinnedToBottom => i18n::tr(app, I18nKey::AppearanceInputModePinnedBottom),
            InputMode::PinnedToTop => i18n::tr(app, I18nKey::AppearanceInputModePinnedTop),
            InputMode::Waterfall => i18n::tr(app, I18nKey::AppearanceInputModeWaterfall),
        }
    }

    fn app_icon_dropdown_item_label(app: &AppContext, val: AppIcon) -> &'static str {
        match val {
            AppIcon::Aurora => "Aurora",
            AppIcon::Default => i18n::tr(app, I18nKey::AppearanceAppIconDefault),
            AppIcon::Classic1 => "Classic 1",
            AppIcon::Classic2 => "Classic 2",
            AppIcon::Classic3 => "Classic 3",
            AppIcon::Comets => "Comets",
            AppIcon::GlassSky => "Glass Sky",
            AppIcon::Glitch => "Glitch",
            AppIcon::Cow => "Cow",
            AppIcon::Glow => "Glow",
            AppIcon::Holographic => "Holographic",
            AppIcon::Mono => "Mono",
            AppIcon::Neon => "Neon",
            AppIcon::Original => "Original",
            AppIcon::Starburst => "Starburst",
            AppIcon::Sticker => "Sticker",
            AppIcon::WarpOne => "Warp 1",
        }
    }

    fn app_icon_dropdown_items(app: &AppContext) -> Vec<DropdownItem<AppearancePageAction>> {
        all::<AppIcon>()
            .map(|val| {
                DropdownItem::new(
                    Self::app_icon_dropdown_item_label(app, val),
                    AppearancePageAction::SetAppIcon(val),
                )
            })
            .collect()
    }

    fn language_dropdown_items(app: &AppContext) -> Vec<DropdownItem<AppearancePageAction>> {
        DisplayLanguage::ALL
            .into_iter()
            .map(|language| {
                DropdownItem::new(
                    i18n::display_language_label(app, language),
                    AppearancePageAction::SetDisplayLanguage(language),
                )
            })
            .collect()
    }

    fn input_mode_dropdown_items(app: &AppContext) -> Vec<DropdownItem<AppearancePageAction>> {
        [
            InputMode::PinnedToBottom,
            InputMode::Waterfall,
            InputMode::PinnedToTop,
        ]
        .into_iter()
        .map(|val| {
            DropdownItem::new(
                Self::input_mode_dropdown_item_label(app, val),
                AppearancePageAction::SetInputMode {
                    new_mode: val,
                    from_binding: false,
                },
            )
        })
        .collect()
    }

    fn thin_strokes_dropdown_items(app: &AppContext) -> Vec<DropdownItem<AppearancePageAction>> {
        [
            ThinStrokes::Never,
            ThinStrokes::OnLowDpiDisplays,
            ThinStrokes::OnHighDpiDisplays,
            ThinStrokes::Always,
        ]
        .into_iter()
        .map(|val| {
            DropdownItem::new(
                Self::thin_strokes_dropdown_item_label(app, val),
                AppearancePageAction::SetThinStrokes(val),
            )
        })
        .collect()
    }

    fn enforce_minimum_contrast_dropdown_items(
        app: &AppContext,
    ) -> Vec<DropdownItem<AppearancePageAction>> {
        [
            EnforceMinimumContrast::Always,
            EnforceMinimumContrast::OnlyNamedColors,
            EnforceMinimumContrast::Never,
        ]
        .into_iter()
        .map(|val| {
            DropdownItem::new(
                Self::enforce_minimum_contrast_dropdown_item_label(app, val),
                AppearancePageAction::SetEnforceMinimumContrast(val),
            )
        })
        .collect()
    }

    fn workspace_decoration_visibility_dropdown_items(
        app: &AppContext,
    ) -> Vec<DropdownItem<AppearancePageAction>> {
        [
            WorkspaceDecorationVisibility::AlwaysShow,
            WorkspaceDecorationVisibility::OnHover,
            WorkspaceDecorationVisibility::HideFullscreen,
        ]
        .into_iter()
        .map(|value| {
            DropdownItem::new(
                Self::workspace_decoration_visibility_dropdown_item_label(app, value),
                AppearancePageAction::SetWorkspaceDecorationVisibility(value),
            )
        })
        .collect()
    }

    fn tab_close_button_position_dropdown_items(
        app: &AppContext,
    ) -> Vec<DropdownItem<AppearancePageAction>> {
        [TabCloseButtonPosition::Right, TabCloseButtonPosition::Left]
            .into_iter()
            .map(|value| {
                DropdownItem::new(
                    Self::tab_close_button_position_dropdown_item_label(app, value),
                    AppearancePageAction::SetTabCloseButtonPosition(value),
                )
            })
            .collect()
    }

    fn thin_strokes_dropdown_item_label(app: &AppContext, val: ThinStrokes) -> &'static str {
        match val {
            ThinStrokes::Never => i18n::tr(app, I18nKey::AppearanceOptionNever),
            ThinStrokes::OnLowDpiDisplays => i18n::tr(app, I18nKey::AppearanceOptionLowDpiDisplays),
            ThinStrokes::OnHighDpiDisplays => {
                i18n::tr(app, I18nKey::AppearanceOptionHighDpiDisplays)
            }
            ThinStrokes::Always => i18n::tr(app, I18nKey::AppearanceOptionAlways),
        }
    }

    fn enforce_minimum_contrast_dropdown_item_label(
        app: &AppContext,
        val: EnforceMinimumContrast,
    ) -> &'static str {
        match val {
            EnforceMinimumContrast::Always => i18n::tr(app, I18nKey::AppearanceOptionAlways),
            EnforceMinimumContrast::OnlyNamedColors => {
                i18n::tr(app, I18nKey::AppearanceContrastOnlyNamedColors)
            }
            EnforceMinimumContrast::Never => i18n::tr(app, I18nKey::AppearanceOptionNever),
        }
    }

    fn workspace_decoration_visibility_dropdown_item_label(
        app: &AppContext,
        value: WorkspaceDecorationVisibility,
    ) -> &'static str {
        match value {
            WorkspaceDecorationVisibility::AlwaysShow => {
                i18n::tr(app, I18nKey::AppearanceWorkspaceDecorationsAlways)
            }
            WorkspaceDecorationVisibility::HideFullscreen => {
                i18n::tr(app, I18nKey::AppearanceWorkspaceDecorationsWindowed)
            }
            WorkspaceDecorationVisibility::OnHover => {
                i18n::tr(app, I18nKey::AppearanceWorkspaceDecorationsHover)
            }
        }
    }

    fn tab_close_button_position_dropdown_item_label(
        app: &AppContext,
        value: TabCloseButtonPosition,
    ) -> &'static str {
        match value {
            TabCloseButtonPosition::Right => i18n::tr(app, I18nKey::AppearanceOptionRight),
            TabCloseButtonPosition::Left => i18n::tr(app, I18nKey::AppearanceOptionLeft),
        }
    }

    fn cursor_display_type_label(app: &AppContext, value: CursorDisplayType) -> &'static str {
        match value {
            CursorDisplayType::Bar => i18n::tr(app, I18nKey::AppearanceCursorBar),
            CursorDisplayType::Block => i18n::tr(app, I18nKey::AppearanceCursorBlock),
            CursorDisplayType::Underline => i18n::tr(app, I18nKey::AppearanceCursorUnderline),
        }
    }

    fn handle_alt_screen_padding_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(EditOrigin::UserTyped | EditOrigin::UserInitiated) => {
                let buffer_text = self.alt_screen_padding_editor.as_ref(ctx).buffer_text(ctx);

                if let Ok(padding) = buffer_text.parse::<f32>() {
                    if padding >= 0. {
                        TerminalSettings::handle(ctx).update(ctx, |terminal_settings, ctx| {
                            let new_mode = AltScreenPaddingMode::Custom {
                                uniform_padding: padding.into_pixels(),
                            };
                            report_if_error!(terminal_settings
                                .alt_screen_padding
                                .set_value(new_mode, ctx));
                            send_telemetry_from_ctx!(
                                TelemetryEvent::UpdateAltScreenPaddingMode { new_mode },
                                ctx
                            );
                        });
                    }
                }

                ctx.notify();
            }
            EditorEvent::Escape | EditorEvent::Blurred | EditorEvent::Enter => {
                self.set_alt_screen_padding_editor_text(ctx);
                ctx.emit(SettingsPageEvent::FocusModal)
            }
            _ => {}
        }
    }

    pub fn handle_font_size_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => self.set_font_size(ctx),
            EditorEvent::Escape => ctx.emit(SettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    pub fn handle_notebook_font_size_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => self.set_notebook_font_size(ctx),
            EditorEvent::Escape => ctx.emit(SettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    pub fn handle_line_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => self.set_line_height_ratio(ctx),
            EditorEvent::Escape => ctx.emit(SettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    /// Updates the prompt that is shown when PS1 is selected, in the 'Prompt' section.
    pub(super) fn set_ps1_info(
        &mut self,
        ps1_grid_info: Option<(BlockGrid, SizeInfo)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ps1_grid_info = ps1_grid_info;
        ctx.notify();
    }

    pub(super) fn get_ps1_info(&self) -> Option<&(BlockGrid, SizeInfo)> {
        self.ps1_grid_info.as_ref()
    }

    fn update_new_windows_num_columns(&mut self, blurred: bool, ctx: &mut ViewContext<Self>) {
        let user_input = self.new_window_columns_editor.as_ref(ctx).buffer_text(ctx);
        if let Some(columns) = Self::parse_new_window_size(user_input) {
            self.valid_new_window_columns = true;
            if blurred {
                self.set_new_windows_num_columns(columns, ctx);
            }
        } else {
            self.valid_new_window_columns = false;
            if blurred {
                let window_settings: &WindowSettings = WindowSettings::as_ref(ctx);
                // Revert to the saved value
                self.set_new_windows_num_columns(
                    *window_settings.new_windows_num_columns.value(),
                    ctx,
                );
            }
        }
        if blurred {
            ctx.focus_self();
        }
        ctx.notify();
    }

    fn update_new_windows_num_rows(&mut self, blurred: bool, ctx: &mut ViewContext<Self>) {
        let user_input = self.new_window_rows_editor.as_ref(ctx).buffer_text(ctx);
        if let Some(rows) = Self::parse_new_window_size(user_input) {
            self.valid_new_window_rows = true;
            if blurred {
                self.set_new_windows_num_rows(rows, ctx);
            }
        } else {
            self.valid_new_window_rows = false;
            if blurred {
                let window_settings: &WindowSettings = WindowSettings::as_ref(ctx);
                // Revert to the saved value
                self.set_new_windows_num_columns(
                    *window_settings.new_windows_num_columns.value(),
                    ctx,
                );
            }
        }
        if blurred {
            ctx.focus_self();
        }
        ctx.notify();
    }

    fn parse_new_window_size(user_input: String) -> Option<u16> {
        user_input.parse::<u16>().ok().filter(|parsed| {
            (MIN_NEW_WINDOW_ROWS_OR_COLS..=MAX_NEW_WINDOW_ROWS_OR_COLS).contains(parsed)
        })
    }

    fn set_font_size(&mut self, ctx: &mut ViewContext<Self>) {
        let user_input = self.font_size_editor.as_ref(ctx).buffer_text(ctx);
        if let Ok(num) = user_input.parse::<usize>() {
            if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&num) {
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    report_if_error!(font_settings
                        .monospace_font_size
                        .set_value(num as f32, ctx,));
                });
            }
        }
    }

    pub fn set_font_weight(&mut self, value: Weight, ctx: &mut ViewContext<Self>) {
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings.monospace_font_weight.set_value(value, ctx))
        });
    }

    fn set_notebook_font_size(&mut self, ctx: &mut ViewContext<Self>) {
        let user_input = self.notebook_font_size_editor.as_ref(ctx).buffer_text(ctx);
        if let Ok(num) = user_input.parse::<usize>() {
            if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&num) {
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    report_if_error!(font_settings.notebook_font_size.set_value(num as f32, ctx,));
                });
            }
        }
    }

    fn set_opacity(
        &mut self,
        opacity_value: f32,
        should_set_defaults: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if should_set_defaults {
            send_telemetry_from_ctx!(
                TelemetryEvent::SetOpacity {
                    opacity: opacity_value as u8
                },
                ctx
            );
        }
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .background_opacity
                .set_value(opacity_value as u8, ctx));
        });
        ctx.notify();
    }

    fn set_blur(
        &mut self,
        blur_value: f32,
        should_set_defaults: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if should_set_defaults {
            send_telemetry_from_ctx!(
                TelemetryEvent::SetBlurRadius {
                    blur_radius: blur_value as u8
                },
                ctx
            );
        }

        ctx.windows()
            .set_all_windows_background_blur_radius(blur_value as u8);

        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .background_blur_radius
                .set_value(blur_value as u8, ctx));
        });
        ctx.notify()
    }

    fn reset_line_height_ratio(&mut self, ctx: &mut ViewContext<Self>) {
        self.line_height_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&format!("{DEFAULT_UI_LINE_HEIGHT_RATIO}"), ctx);
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::SetLineHeight {
                new_value: DEFAULT_UI_LINE_HEIGHT_RATIO
            },
            ctx
        );

        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings
                .line_height_ratio
                .set_value(DEFAULT_UI_LINE_HEIGHT_RATIO, ctx));
        });
    }

    fn set_line_height_ratio(&mut self, ctx: &mut ViewContext<Self>) {
        let user_input = self.line_height_editor.as_ref(ctx).buffer_text(ctx);
        let Ok(new_line_height) = user_input.parse::<f32>() else {
            return;
        };

        let appearance = Appearance::as_ref(ctx);
        let current_line_height = appearance.ui_builder().line_height_ratio();

        if (current_line_height - new_line_height).abs() > f32::EPSILON {
            send_telemetry_from_ctx!(
                TelemetryEvent::SetLineHeight {
                    new_value: new_line_height
                },
                ctx
            );

            if (MIN_LINE_SPACING..=MAX_LINE_SPACING).contains(&new_line_height) {
                FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
                    report_if_error!(font_settings
                        .line_height_ratio
                        .set_value(new_line_height, ctx));
                });
            }
        }
    }

    pub fn toggle_open_windows_at_custom_size(&mut self, ctx: &mut ViewContext<Self>) {
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            let current_val = window_settings.open_windows_at_custom_size.value();
            let new_val: bool = !current_val;
            send_telemetry_from_ctx!(
                TelemetryEvent::ToggleNewWindowsAtCustomSize { enabled: new_val },
                ctx
            );
            report_if_error!(window_settings
                .open_windows_at_custom_size
                .set_value(new_val, ctx));
        });
        ctx.notify();
    }

    fn set_new_windows_num_columns(&mut self, columns: u16, ctx: &mut ViewContext<Self>) {
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            send_telemetry_from_ctx!(TelemetryEvent::SetNewWindowsAtCustomSize, ctx);
            report_if_error!(window_settings
                .new_windows_num_columns
                .set_value(columns, ctx));
        });
    }

    fn set_new_windows_num_rows(&mut self, rows: u16, ctx: &mut ViewContext<Self>) {
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            send_telemetry_from_ctx!(TelemetryEvent::SetNewWindowsAtCustomSize, ctx);
            report_if_error!(window_settings.new_windows_num_rows.set_value(rows, ctx));
        });
    }

    fn update_font_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let monospace_font_family = Appearance::as_ref(ctx).monospace_font_family();
        let ai_font_family = Appearance::as_ref(ctx).ai_font_family();

        self.font_family_dropdown.update(ctx, |dropdown, ctx| {
            // Get the family name of the current monospace font.
            // We check the font_cache for the current monospace family.
            // We also make sure that
            // - If the current monospace family is in our available_families map,
            //   we update its entry to ensure it has the correct family_id
            // - Otherwise, we add a new entry for the current monospace family
            let font_name = ctx
                .font_cache()
                .load_family_name_from_id(monospace_font_family);

            if let Some(font_name) = &font_name {
                self.available_families
                    .entry(font_name.clone())
                    .and_modify(|entry| entry.0 = Some(monospace_font_family))
                    .or_insert((Some(monospace_font_family), FontType::Monospace));
            }
            let font_name = font_name.unwrap_or_default();

            let mut items = self
                .available_families
                .iter()
                .filter_map(|(name, (family, font_type))| {
                    // Only add the item if it's not a default font and then check whether we want
                    // to see it depending on it being monospace and user's preference. We also
                    // want to include the currently selected font, even if it's non-monospace and
                    // user hasn't chosen to view all system fonts.
                    let include_in_dropdown = name != &MonospaceFontName::default_value()
                        && (matches!(self.view_font_type, FontType::Any)
                            || matches!(font_type, FontType::Monospace)
                            || *name == font_name);
                    if include_in_dropdown {
                        let name_move = name.clone();
                        let mut dropdown =
                            DropdownItem::new(name, AppearancePageAction::SetFontFamily(name_move));

                        // If we're on a non-Linux platform, render the dropdown item in the
                        // actual font.  We currently don't do this on Linux because
                        // pre-loading all of the fonts is too expensive.
                        if cfg!(not(any(target_os = "linux", target_os = "freebsd"))) {
                            if let Some(family_id) = family {
                                dropdown = dropdown.with_font_override(*family_id)
                            }
                        }

                        Some(dropdown)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            // Sort the font names by alphabetical order.
            items.sort_by(|a, b| a.display_text.cmp(&b.display_text));
            // Prepend the default item
            items.insert(0, Self::default_font_item(ctx, false));
            dropdown.set_items(items, ctx);

            if !font_name.is_empty() {
                let default_label;
                let label = if font_name == MonospaceFontName::default_value() {
                    default_label = default_font_label(ctx, false);
                    &default_label
                } else {
                    &font_name
                };
                dropdown.set_selected_by_name(label, ctx);
            }
        });

        self.ai_font_family_dropdown.update(ctx, |dropdown, ctx| {
            // Get the family name of the current agent mode font.
            // We check the font_cache for the current agent mode family.
            // We also make sure that
            // - If the current family is in our available_families map,
            //   we update its entry to ensure it has the correct family_id
            // - Otherwise, we add a new entry for the current agent mode family
            let font_name = ctx.font_cache().load_family_name_from_id(ai_font_family);

            if let Some(font_name) = &font_name {
                self.available_families
                    .entry(font_name.clone())
                    .and_modify(|entry| entry.0 = Some(ai_font_family))
                    .or_insert((Some(ai_font_family), FontType::Any));
            }
            let font_name = font_name.unwrap_or_default();

            let mut items = self
                .available_families
                .iter()
                .filter_map(|(name, (family, _font_type))| {
                    if name == &AIFontName::default_value() {
                        return None;
                    }

                    let name_move = name.clone();
                    let mut dropdown =
                        DropdownItem::new(name, AppearancePageAction::SetAIFontFamily(name_move));

                    // If we're on a non-Linux platform, render the dropdown item in the
                    // actual font.  We currently don't do this on Linux because
                    // pre-loading all of the fonts is too expensive.
                    if cfg!(not(any(target_os = "linux", target_os = "freebsd"))) {
                        if let Some(family_id) = family {
                            dropdown = dropdown.with_font_override(*family_id)
                        }
                    }

                    Some(dropdown)
                })
                .collect::<Vec<_>>();

            // Sort the font names by alphabetical order.
            items.sort_by(|a, b| a.display_text.cmp(&b.display_text));
            // Prepend the default item
            items.insert(0, Self::default_font_item(ctx, true));
            dropdown.set_items(items, ctx);

            if !font_name.is_empty() {
                let default_label;
                let label = if font_name == AIFontName::default_value() {
                    default_label = default_font_label(ctx, true);
                    &default_label
                } else {
                    &font_name
                };
                dropdown.set_selected_by_name(label, ctx);
            }
        });

        ctx.notify();
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn set_system_fonts(
        &mut self,
        available_families: Vec<(Option<FamilyId>, FontInfo)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.available_families = available_families
            .into_iter()
            .map(|(family_id, font_info)| {
                let font_type = if font_info.is_monospace {
                    FontType::Monospace
                } else {
                    FontType::Any
                };
                (font_info.family_name, (family_id, font_type))
            })
            .collect();

        // Add Hack font to the available monospace families so user could switch back to it.
        if let Some(family_id) = ctx
            .font_cache()
            .family_id_for_name(DEFAULT_MONOSPACE_FONT_NAME)
        {
            self.available_families.insert(
                String::from(DEFAULT_MONOSPACE_FONT_NAME),
                (Some(family_id), FontType::Monospace),
            );
        }

        self.update_font_dropdown(ctx);
    }

    pub fn set_font_family(&mut self, name: &str, ctx: &mut ViewContext<Self>) {
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings
                .monospace_font_name
                .set_value(name.to_string(), ctx));
            if *font_settings.match_ai_font_to_terminal_font.value() {
                report_if_error!(font_settings.ai_font_name.set_value(name.to_string(), ctx))
            }
        });
    }

    pub fn toggle_match_ai_font_to_terminal_font(&mut self, ctx: &mut ViewContext<Self>) {
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings
                .match_ai_font_to_terminal_font
                .toggle_and_save_value(ctx));
            if *font_settings.match_ai_font_to_terminal_font.value() {
                let font_name = font_settings.monospace_font_name.value().clone();
                self.ai_font_family_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.clear_filter(ctx);
                });
                report_if_error!(font_settings.ai_font_name.set_value(font_name, ctx))
            }
        });
    }

    pub fn set_ai_font_family(&mut self, name: &str, ctx: &mut ViewContext<Self>) {
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings.ai_font_name.set_value(name.to_string(), ctx))
        });
    }

    fn set_thin_strokes(&mut self, value: &ThinStrokes, ctx: &mut ViewContext<Self>) {
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            match font_settings.use_thin_strokes.set_value(*value, ctx) {
                Ok(_) => {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::ThinStrokesSettingChanged { new_value: *value },
                        ctx
                    );
                }
                Err(e) => {
                    report_error!(e);
                }
            }
        });
    }

    pub fn toggle_jump_to_bottom_of_block_button(&mut self, ctx: &mut ViewContext<Self>) {
        let block_list_settings = BlockListSettings::handle(ctx);
        let new_value = {
            !*block_list_settings
                .as_ref(ctx)
                .show_jump_to_bottom_of_block_button
                .value()
        };
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleJumpToBottomofBlockButton { enabled: new_value },
            ctx
        );
        ctx.update_model(&block_list_settings, move |block_list_settings, ctx| {
            report_if_error!(block_list_settings
                .show_jump_to_bottom_of_block_button
                .set_value(new_value, ctx));
        });
    }

    pub fn toggle_show_block_dividers(&mut self, ctx: &mut ViewContext<Self>) {
        let block_list_settings = BlockListSettings::handle(ctx);
        let new_value = { !*block_list_settings.as_ref(ctx).show_block_dividers.value() };
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleShowBlockDividers { enabled: new_value },
            ctx
        );
        ctx.update_model(&block_list_settings, move |block_list_settings, ctx| {
            report_if_error!(block_list_settings
                .show_block_dividers
                .set_value(new_value, ctx));
        });
    }

    pub fn toggle_compact_mode(&mut self, ctx: &mut ViewContext<Self>) {
        TerminalSettings::handle(ctx).update(ctx, |terminal_settings, ctx| {
            let current_value = *terminal_settings.spacing_mode.value();
            report_if_error!(terminal_settings
                .spacing_mode
                .set_value(current_value.other_mode(), ctx));
        });
    }

    pub fn toggle_cursor_blink(&mut self, ctx: &mut ViewContext<Self>) {
        AppEditorSettings::handle(ctx).update(ctx, |me, ctx| {
            me.toggle_cursor_blink(ctx);
        })
    }

    pub fn toggle_respect_system_theme(&mut self, ctx: &mut ViewContext<Self>) {
        ThemeSettings::handle(ctx).update(ctx, |theme_settings, ctx| {
            report_if_error!(theme_settings.use_system_theme.toggle_and_save_value(ctx));
        });
        ctx.notify();
    }

    pub fn toggle_dim_inactive_panes(&mut self, ctx: &mut ViewContext<Self>) {
        PaneSettings::handle(ctx).update(ctx, |pane_settings, ctx| {
            match pane_settings
                .should_dim_inactive_panes
                .toggle_and_save_value(ctx)
            {
                Ok(new_value) => {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::ToggleDimInactivePanes { enabled: new_value },
                        ctx
                    );
                }
                Err(e) => {
                    report_error!(e);
                }
            }
        });
    }

    pub fn toggle_blur_texture(&mut self, ctx: &mut ViewContext<Self>) {
        let blur_enabled = WindowSettings::handle(ctx).read(ctx, |window_settings, _ctx| {
            *window_settings.background_blur_texture.value()
        });
        ctx.windows()
            .set_all_windows_background_blur_texture(!blur_enabled);
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .background_blur_texture
                .toggle_and_save_value(ctx));
        });
        ctx.notify();
    }

    pub fn toggle_left_panel_visibility(&mut self, ctx: &mut ViewContext<Self>) {
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .left_panel_visibility_across_tabs
                .toggle_and_save_value(ctx));
        });
        ctx.notify();
    }

    pub fn set_input_mode(
        &mut self,
        new_mode: InputMode,
        from_binding: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let old_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        send_telemetry_from_ctx!(TelemetryEvent::InputModeChanged { old_mode, new_mode }, ctx);
        InputModeSettings::handle(ctx).update(ctx, |input_mode, ctx| {
            report_if_error!(input_mode.input_mode.set_value(new_mode, ctx));
        });
        let item_name = Self::input_mode_dropdown_item_label(ctx, new_mode);

        if from_binding {
            // If this update is from a command palette action, we need to update the dropdown
            // If not, we can't update it because there is a circular view reference, but the dropdown
            // will update it itself.  Not great state management - I think ideally the dropdowns would have
            // a model they are listening to.
            self.input_mode_dropdown.update(ctx, |input_dropdown, ctx| {
                input_dropdown.set_selected_by_name(item_name, ctx);
                ctx.notify();
            });
        }
    }

    fn set_input_type(&mut self, new_type: InputBoxType, ctx: &mut ViewContext<Self>) {
        let old_type = InputSettings::as_ref(ctx).input_type(ctx);

        if old_type != new_type {
            InputSettings::handle(ctx).update(ctx, |input_type_settings, ctx| {
                report_if_error!(input_type_settings.input_box_type.set_value(new_type, ctx));
            });
            self.input_type_radio_state
                .set_selected_idx(new_type as usize);

            let is_udi_enabled = new_type == InputBoxType::Universal;
            send_telemetry_from_ctx!(
                TelemetryEvent::InputUXModeChanged {
                    is_udi_enabled,
                    origin: InputUXChangeOrigin::Settings
                },
                ctx
            );

            // Selecting classic mode must also enable honor_ps1 so the mode takes
            // effect immediately (input_type() requires honor_ps1 to return classic).
            SessionSettings::handle(ctx).update(ctx, |session_settings, ctx| {
                report_if_error!(session_settings
                    .honor_ps1
                    .set_value(new_type == InputBoxType::Classic, ctx));
            });

            ctx.notify();
        }
    }

    fn set_display_language(&mut self, language: DisplayLanguage, ctx: &mut ViewContext<Self>) {
        LanguageSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.display_language.set_value(language, ctx));
        });
        ctx.notify();
    }

    fn update_language_dropdown(&self, ctx: &mut ViewContext<Self>) {
        let current_value = *LanguageSettings::as_ref(ctx).display_language.value();
        self.language_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(Self::language_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_action(
                AppearancePageAction::SetDisplayLanguage(current_value),
                ctx,
            );
        });
    }

    fn update_localized_dropdowns(&self, ctx: &mut ViewContext<Self>) {
        self.input_mode_dropdown.update(ctx, |dropdown, ctx| {
            let current_value = *InputModeSettings::as_ref(ctx).input_mode.value();
            dropdown.set_items(Self::input_mode_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_action(
                AppearancePageAction::SetInputMode {
                    new_mode: current_value,
                    from_binding: false,
                },
                ctx,
            );
        });

        self.thin_strokes_dropdown.update(ctx, |dropdown, ctx| {
            let current_value = ctx.rendering_config().glyphs.use_thin_strokes;
            dropdown.set_items(Self::thin_strokes_dropdown_items(ctx), ctx);
            dropdown
                .set_selected_by_action(AppearancePageAction::SetThinStrokes(current_value), ctx);
        });

        self.enforce_min_contrast_dropdown
            .update(ctx, |dropdown, ctx| {
                let current_value = *FontSettings::as_ref(ctx).enforce_minimum_contrast;
                dropdown.set_items(Self::enforce_minimum_contrast_dropdown_items(ctx), ctx);
                dropdown.set_selected_by_action(
                    AppearancePageAction::SetEnforceMinimumContrast(current_value),
                    ctx,
                );
            });

        self.workspace_decorations_dropdown
            .update(ctx, |dropdown, ctx| {
                let current_value = TabSettings::as_ref(ctx).workspace_decoration_visibility;
                dropdown.set_items(
                    Self::workspace_decoration_visibility_dropdown_items(ctx),
                    ctx,
                );
                dropdown.set_selected_by_action(
                    AppearancePageAction::SetWorkspaceDecorationVisibility(current_value),
                    ctx,
                );
            });

        self.tab_close_button_position_dropdown
            .update(ctx, |dropdown, ctx| {
                let current_value = TabSettings::as_ref(ctx).close_button_position;
                dropdown.set_items(Self::tab_close_button_position_dropdown_items(ctx), ctx);
                dropdown.set_selected_by_action(
                    AppearancePageAction::SetTabCloseButtonPosition(current_value),
                    ctx,
                );
            });

        self.app_icon_dropdown.update(ctx, |dropdown, ctx| {
            let current_value = *AppIconSettings::as_ref(ctx).app_icon.value();
            dropdown.set_items(Self::app_icon_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_action(AppearancePageAction::SetAppIcon(current_value), ctx);
        });
    }

    fn set_app_icon(&mut self, new_icon: AppIcon, ctx: &mut ViewContext<Self>) {
        AppIconSettings::handle(ctx).update(ctx, |app_icon_settings, ctx| {
            report_if_error!(app_icon_settings.app_icon.set_value(new_icon, ctx));
            send_telemetry_from_ctx!(
                TelemetryEvent::AppIconSelection {
                    icon: new_icon.to_string(),
                },
                ctx
            );
        });
    }

    fn set_cursor_type(&mut self, new_cursor_type: CursorDisplayType, ctx: &mut ViewContext<Self>) {
        AppEditorSettings::handle(ctx).update(ctx, |app_editor_settings, ctx| {
            report_if_error!(app_editor_settings
                .cursor_display_type
                .set_value(new_cursor_type, ctx));
            send_telemetry_from_ctx!(
                TelemetryEvent::CursorDisplayType {
                    cursor: new_cursor_type.to_string(),
                },
                ctx
            );
        });
    }

    fn toggle_all_available_fonts(&mut self, ctx: &mut ViewContext<Self>) {
        self.view_font_type = self.view_font_type.toggle();
        self.update_font_dropdown(ctx);
    }

    fn toggle_tab_indicators(&mut self, ctx: &mut ViewContext<Self>) {
        let tab_settings = TabSettings::handle(ctx);
        let new_value = { !*tab_settings.as_ref(ctx).show_indicators.value() };

        ctx.update_model(&tab_settings, move |tab_settings, ctx| {
            report_if_error!(tab_settings.show_indicators.set_value(new_value, ctx));
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleTabIndicators { enabled: new_value },
            ctx
        );
    }

    fn toggle_show_code_review_button(&mut self, ctx: &mut ViewContext<Self>) {
        let tab_settings = TabSettings::handle(ctx);
        let new_value = !*tab_settings.as_ref(ctx).show_code_review_button.value();

        ctx.update_model(&tab_settings, move |tab_settings, ctx| {
            report_if_error!(tab_settings
                .show_code_review_button
                .set_value(new_value, ctx));
        });
    }

    fn toggle_preserve_active_tab_color(&mut self, ctx: &mut ViewContext<Self>) {
        let tab_settings = TabSettings::handle(ctx);
        let new_value = !*tab_settings.as_ref(ctx).preserve_active_tab_color.value();

        ctx.update_model(&tab_settings, move |tab_settings, ctx| {
            report_if_error!(tab_settings
                .preserve_active_tab_color
                .set_value(new_value, ctx));
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::TogglePreserveActiveTabColor { enabled: new_value },
            ctx
        );
    }

    fn toggle_vertical_tabs(&mut self, ctx: &mut ViewContext<Self>) {
        let tab_settings = TabSettings::handle(ctx);
        let new_value = !*tab_settings.as_ref(ctx).use_vertical_tabs.value();

        ctx.update_model(&tab_settings, move |tab_settings, ctx| {
            report_if_error!(tab_settings.use_vertical_tabs.set_value(new_value, ctx));
        });
    }

    fn toggle_show_vertical_tab_panel_in_restored_windows(&mut self, ctx: &mut ViewContext<Self>) {
        TabSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .show_vertical_tab_panel_in_restored_windows
                .toggle_and_save_value(ctx));
        });
    }

    fn toggle_use_latest_user_prompt_as_conversation_title_in_tab_names(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        TabSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .use_latest_user_prompt_as_conversation_title_in_tab_names
                .toggle_and_save_value(ctx));
        });
    }

    /// Set the workspace decoration visibility to a particular value.
    fn set_workspace_decoration_visibility(
        &mut self,
        new_value: WorkspaceDecorationVisibility,
        ctx: &mut ViewContext<Self>,
    ) {
        let previous_value = TabSettings::handle(ctx).update(ctx, |tab_settings, ctx| {
            let prev_value = *tab_settings.workspace_decoration_visibility.value();
            report_if_error!(tab_settings
                .workspace_decoration_visibility
                .set_value(new_value, ctx));
            prev_value
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleWorkspaceDecorationVisibility {
                previous_value,
                new_value
            },
            ctx
        );
    }

    /// Toggle among the supported workspace decoration visibility values.
    fn toggle_workspace_decoration_visiblity(&mut self, ctx: &mut ViewContext<Self>) {
        let (new_value, previous_value) =
            TabSettings::handle(ctx).update(ctx, |tab_settings, ctx| {
                let previous_value = *tab_settings.workspace_decoration_visibility.value();
                let new_value = previous_value.toggled();
                report_if_error!(tab_settings
                    .workspace_decoration_visibility
                    .set_value(new_value, ctx));
                (new_value, previous_value)
            });
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleWorkspaceDecorationVisibility {
                previous_value,
                new_value
            },
            ctx
        );
    }

    fn build_workspace_decoration_visibility_dropdown(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Dropdown<AppearancePageAction>> {
        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);

            let values = [
                WorkspaceDecorationVisibility::AlwaysShow,
                WorkspaceDecorationVisibility::OnHover,
                WorkspaceDecorationVisibility::HideFullscreen,
            ];

            let current_value = TabSettings::as_ref(ctx).workspace_decoration_visibility;
            let selected_index = values.iter().position(|val| *val == current_value).unwrap_or_else(|| {
                log::error!("Could not find current WorkspaceDecorationVisibility value in dropdown option list");
                0
            });

            dropdown.set_items(
                Self::workspace_decoration_visibility_dropdown_items(ctx),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        })
    }

    fn build_language_dropdown(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Dropdown<AppearancePageAction>> {
        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(INPUT_MODE_DROPDOWN_WIDTH);
            dropdown.set_menu_width(INPUT_MODE_DROPDOWN_WIDTH, ctx);
            dropdown.set_items(Self::language_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_action(
                AppearancePageAction::SetDisplayLanguage(
                    *LanguageSettings::as_ref(ctx).display_language.value(),
                ),
                ctx,
            );
            dropdown
        })
    }

    fn build_tab_close_button_position_dropdown(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Dropdown<AppearancePageAction>> {
        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);

            let values = [
                TabCloseButtonPosition::Right,
                TabCloseButtonPosition::Left,
            ];

            let current_value = TabSettings::as_ref(ctx).close_button_position;
            let selected_index = values.iter().position(|val| *val == current_value).unwrap_or_else(|| {
                log::error!("Could not find current TabCloseButtonPosition value in dropdown option list");
                0
            });

            dropdown.set_items(Self::tab_close_button_position_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        })
    }

    fn build_zoom_level_dropdown(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Dropdown<AppearancePageAction>> {
        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);

            dropdown.set_items(
                crate::window_settings::ZoomLevel::VALUES
                    .iter()
                    .map(|&value| {
                        DropdownItem::new(
                            format!("{value}%"),
                            AppearancePageAction::SetZoomLevel(value),
                        )
                    })
                    .collect(),
                ctx,
            );

            let current_value = *WindowSettings::as_ref(ctx).zoom_level.value();
            dropdown.set_selected_by_action(AppearancePageAction::SetZoomLevel(current_value), ctx);

            dropdown
        })
    }

    fn handle_directory_color_add_picker_event(
        &mut self,
        event: &DirectoryColorAddPickerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            DirectoryColorAddPickerEvent::Selected(path) => {
                add_directory_tab_color_path(path.clone(), ctx);
            }
            DirectoryColorAddPickerEvent::RequestAddFromFilePicker => {
                open_directory_tab_color_folder_picker(ctx);
            }
        }
    }

    fn handle_tab_settings_event(
        &mut self,
        event: &TabSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let TabSettingsChangedEvent::WorkspaceDecorationVisibility { .. } = event {
            let value = TabSettings::as_ref(ctx).workspace_decoration_visibility;
            let name = Self::workspace_decoration_visibility_dropdown_item_label(ctx, value);
            self.workspace_decorations_dropdown
                .update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_name(name, ctx);
                });
        }
        if let TabSettingsChangedEvent::DirectoryTabColors { .. } = event {
            let count = directory_tab_colors(ctx).len();
            self.color_picker_dot_states.resize_with(count, || {
                (0..TAB_COLOR_OPTIONS.len() + 1)
                    .map(|_| MouseStateHandle::default())
                    .collect()
            });
            self.directory_tab_color_delete_buttons = build_directory_delete_buttons(ctx);
        }
        ctx.notify();
    }

    fn toggle_ligature_rendering(&mut self, ctx: &mut ViewContext<Self>) {
        if FeatureFlag::Ligatures.is_enabled() {
            let ligature_settings = LigatureSettings::handle(ctx);
            let new_value = !*ligature_settings
                .as_ref(ctx)
                .ligature_rendering_enabled
                .value();

            ligature_settings.update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .ligature_rendering_enabled
                    .set_value(new_value, ctx));
            });

            send_telemetry_from_ctx!(
                TelemetryEvent::ToggleLigatureRendering { enabled: new_value },
                ctx
            );
        }
    }

    pub fn toggle_input_mode(&mut self, ctx: &mut ViewContext<Self>) {
        // Get the current input type
        let current_type = InputSettings::as_ref(ctx).input_type(ctx);

        // Toggle between Universal and Classic
        let new_type = match current_type {
            InputBoxType::Universal => InputBoxType::Classic,
            InputBoxType::Classic => InputBoxType::Universal,
        };

        // Update the setting
        self.set_input_type(new_type, ctx);
    }

    pub fn update_tab_close_button_position(
        &mut self,
        position: TabCloseButtonPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        TabSettings::handle(ctx).update(ctx, |tab_settings, ctx| {
            report_if_error!(tab_settings.close_button_position.set_value(position, ctx));
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::TabCloseButtonPositionUpdated { position },
            ctx
        );
        ctx.notify();
    }
}

fn render_group(
    children: impl IntoIterator<Item = Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let bar = Container::new(
        ConstrainedBox::new(Empty::new().finish())
            .with_width(4.)
            .finish(),
    )
    .with_background(appearance.theme().outline())
    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
    .with_margin_right(8.)
    .with_margin_left(8.)
    .finish();

    Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(bar)
            .with_child(
                Shrinkable::new(1., Flex::column().with_children(children).finish()).finish(),
            )
            .finish(),
    )
    .with_margin_top(-4.)
    .with_margin_bottom(HEADER_PADDING)
    .finish()
}

#[derive(Default)]
struct CreateCustomThemeWidget {
    mouse_state: MouseStateHandle,
}

impl SettingsWidget for CreateCustomThemeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "create theme create custom theme"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .link(
                    i18n::tr(app, I18nKey::AppearanceThemeCreateCustom).to_string(),
                    Some("https://docs.warp.dev/terminal/appearance/custom-themes".to_string()),
                    None,
                    self.mouse_state.clone(),
                )
                .soft_wrap(false)
                .build()
                .with_margin_bottom(10.)
                .finish(),
        )
        .left()
        .finish()
    }
}

#[derive(Default)]
struct ThemeSelectWidget {
    sync_os_switch_state: SwitchStateHandle,
    open_theme_chooser_button_mouse_state: MouseStateHandle,
    open_theme_chooser_button_mouse_state_light: MouseStateHandle,
    open_theme_chooser_button_mouse_state_dark: MouseStateHandle,
}

impl ThemeSelectWidget {
    fn render_theme_option(
        &self,
        appearance: &Appearance,
        theme_kind: ThemeKind,
        theme_chooser_mode: ThemeChooserMode,
        state: MouseStateHandle,
        is_selected: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme: WarpTheme = WarpConfig::as_ref(app).theme_config().theme(&theme_kind);
        let mode_ui_label = match theme_chooser_mode {
            ThemeChooserMode::SystemLight => i18n::tr(app, I18nKey::AppearanceThemeLight),
            ThemeChooserMode::SystemDark => i18n::tr(app, I18nKey::AppearanceThemeDark),
            ThemeChooserMode::SystemAgnostic => i18n::tr(app, I18nKey::AppearanceThemeCurrent),
        };

        ConstrainedBox::new(
            Hoverable::new(state, |hover_state| {
                let (border_color, border_width) = match hover_state.is_hovered() {
                    true => (theme.accent(), 1.0),
                    false => (theme.accent(), 0.0),
                };

                let mut container = Container::new(
                    Flex::row()
                        .with_child(
                            appearance
                                .ui_builder()
                                .span(mode_ui_label.to_owned())
                                .with_style(
                                    UiComponentStyles::default()
                                        .set_font_weight(Weight::Bold)
                                        .set_margin(Coords {
                                            top: 20.,
                                            bottom: 20.,
                                            left: 10.,
                                            right: 20.,
                                        }),
                                )
                                .build()
                                .finish(),
                        )
                        .with_child(theme::render_preview(
                            &theme,
                            appearance.monospace_font_family(),
                            Some(0.6),
                        ))
                        .with_child(
                            appearance
                                .ui_builder()
                                .span(theme_kind.to_string())
                                .with_style(UiComponentStyles::default().set_margin(Coords {
                                    top: 20.,
                                    bottom: 20.,
                                    left: 20.,
                                    right: 10.,
                                }))
                                .build()
                                .finish(),
                        )
                        .finish(),
                )
                .with_border(Border::all(border_width).with_border_fill(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
                .with_padding_left(10. - border_width)
                .with_padding_top(5. - border_width)
                .with_padding_bottom(5. - border_width);

                if is_selected {
                    container = container
                        .with_background(internal_colors::fg_overlay_1(appearance.theme()));
                }
                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::ShowThemeChooser(theme_chooser_mode));
            })
            .finish(),
        )
        .with_min_height(70.)
        .with_min_width(450.)
        .finish()
    }
}

impl SettingsWidget for ThemeSelectWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "sync with os theme themes background backgrounds color colors customize"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme_settings = ThemeSettings::handle(app);
        let theme_picker = match respect_system_theme(theme_settings.as_ref(app)) {
            RespectSystemTheme::Off => self.render_theme_option(
                appearance,
                active_theme_kind(theme_settings.as_ref(app), app),
                ThemeChooserMode::SystemAgnostic,
                self.open_theme_chooser_button_mouse_state.clone(),
                true, // is selected
                app,
            ),
            RespectSystemTheme::On(SelectedSystemThemes { light, dark }) => Flex::column()
                .with_child(self.render_theme_option(
                    appearance,
                    light.clone(),
                    ThemeChooserMode::SystemLight,
                    self.open_theme_chooser_button_mouse_state_light.clone(),
                    app.system_theme() == SystemTheme::Light,
                    app,
                ))
                .with_child(self.render_theme_option(
                    appearance,
                    dark.clone(),
                    ThemeChooserMode::SystemDark,
                    self.open_theme_chooser_button_mouse_state_dark.clone(),
                    app.system_theme() == SystemTheme::Dark,
                    app,
                ))
                .finish(),
        };

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(render_body_item::<AppearancePageAction>(
                i18n::tr(app, I18nKey::AppearanceThemeSyncWithOs).into(),
                None,
                LocalOnlyIconState::for_setting(
                    UseSystemTheme::storage_key(),
                    UseSystemTheme::sync_to_cloud(),
                    &mut view.local_only_icon_tooltip_states.borrow_mut(),
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                appearance
                    .ui_builder()
                    .switch(self.sync_os_switch_state.clone())
                    .check(matches!(
                        respect_system_theme(ThemeSettings::as_ref(app)),
                        RespectSystemTheme::On { .. }
                    ))
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(AppearancePageAction::ToggleRespectSystemTheme);
                    })
                    .finish(),
                None,
            ))
            .with_child(
                appearance
                    .ui_builder()
                    .span(i18n::tr(app, I18nKey::AppearanceThemeSyncWithOsDescription).to_string())
                    .with_style(
                        UiComponentStyles::default().set_margin(Coords::default().bottom(10.)),
                    )
                    .build()
                    .finish(),
            )
            .with_child(
                Container::new(theme_picker)
                    .with_margin_bottom(25.)
                    .finish(),
            )
            .finish()
    }
}

#[derive(Default)]
struct LanguageWidget {}

impl SettingsWidget for LanguageWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "language display locale localization internationalization i18n chinese simplified chinese 中文 简体中文"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceLanguageLabel),
            Some(i18n::tr(app, I18nKey::AppearanceLanguageDescription)),
            None,
            LocalOnlyIconState::for_setting(
                DisplayLanguageSetting::storage_key(),
                DisplayLanguageSetting::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.language_dropdown,
        )
    }
}

#[derive(Default)]
struct CustomAppIconWidget {}

impl SettingsWidget for CustomAppIconWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "customize custom app icon icons"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        #[allow(unused_mut)]
        let show_bundle_warning = {
            #[cfg(target_os = "macos")]
            #[allow(deprecated)]
            {
                use cocoa::base::id;
                use objc::{class, msg_send, sel, sel_impl};
                unsafe {
                    let running_app: id =
                        msg_send![class!(NSRunningApplication), currentApplication];
                    let bundle_id: id = msg_send![running_app, bundleIdentifier];
                    bundle_id.is_null()
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        };

        let dropdown = render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceAppIconLabel),
            show_bundle_warning.then_some(i18n::tr(app, I18nKey::AppearanceAppIconBundleWarning)),
            None,
            LocalOnlyIconState::Hidden,
            None,
            &view.app_icon_dropdown,
        );

        #[cfg(target_os = "macos")]
        {
            use crate::appearance::AppearanceManager;

            let app_icon_at_startup = AppearanceManager::as_ref(app).app_icon_at_startup();
            let current_icon = *AppIconSettings::as_ref(app).app_icon;
            if current_icon == AppIcon::Default
                && ChannelState::channel() != Channel::Local
                && app_icon_at_startup != AppIcon::Default
            {
                let theme = appearance.theme();
                return Flex::column()
                    .with_child(dropdown)
                    .with_child(
                        appearance
                            .ui_builder()
                            .wrappable_text(
                                i18n::tr(app, I18nKey::AppearanceAppIconRestartWarning),
                                true,
                            )
                            .with_style(UiComponentStyles {
                                font_color: Some(
                                    theme.sub_text_color(theme.background()).into_solid(),
                                ),
                                margin: Some(Coords::default().bottom(8.)),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .finish();
            }
        }

        dropdown
    }
}

#[derive(Default)]
struct CustomWindowSizeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CustomWindowSizeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "open windows with custom size"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let window_settings = WindowSettings::as_ref(app);
        let column_border_color: Option<Fill> =
            (!view.valid_new_window_columns).then(|| themes::theme::Fill::error().into());
        let row_border_color: Option<Fill> =
            (!view.valid_new_window_rows).then(|| themes::theme::Fill::error().into());
        let mut column = Flex::column().with_child(render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceWindowCustomSizeLabel).into(),
            None,
            LocalOnlyIconState::for_setting(
                OpenWindowsAtCustomSize::storage_key(),
                OpenWindowsAtCustomSize::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*window_settings.open_windows_at_custom_size.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleOpenWindowsAtCustomSize);
                })
                .finish(),
            None,
        ));
        if *window_settings.open_windows_at_custom_size.value() {
            column.add_child(
                Container::new(render_body_item::<AppearancePageAction>(
                    i18n::tr(app, I18nKey::AppearanceWindowColumns).into(),
                    None,
                    // We show the local-only icon for this with the toggle, not the individual inputs.
                    LocalOnlyIconState::Hidden,
                    ToggleState::Enabled,
                    appearance,
                    Dismiss::new(
                        appearance
                            .ui_builder()
                            .text_input(view.new_window_columns_editor.clone())
                            .with_style(UiComponentStyles {
                                width: Some(60.),
                                padding: Some(Coords {
                                    top: 4.,
                                    bottom: 4.,
                                    left: 6.,
                                    right: 6.,
                                }),
                                background: Some(appearance.theme().surface_2().into()),
                                border_color: column_border_color,
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(AppearancePageAction::SetNewWindowsCustomColumns)
                    })
                    .finish(),
                    None,
                ))
                .with_margin_left(10.)
                .finish(),
            );
            column.add_child(
                Container::new(render_body_item::<AppearancePageAction>(
                    i18n::tr(app, I18nKey::AppearanceWindowRows).into(),
                    None,
                    // We show the local-only icon for this with the toggle, not the individual inputs.
                    LocalOnlyIconState::Hidden,
                    ToggleState::Enabled,
                    appearance,
                    Dismiss::new(
                        appearance
                            .ui_builder()
                            .text_input(view.new_window_rows_editor.clone())
                            .with_style(UiComponentStyles {
                                width: Some(60.),
                                padding: Some(Coords {
                                    top: 4.,
                                    bottom: 4.,
                                    left: 6.,
                                    right: 6.,
                                }),
                                background: Some(appearance.theme().surface_2().into()),
                                border_color: row_border_color,
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(AppearancePageAction::SetNewWindowsCustomRows)
                    })
                    .finish(),
                    None,
                ))
                .with_margin_left(10.)
                .finish(),
            );
        }
        column.finish()
    }
}

#[derive(Default)]
struct WindowOpacityWidget {
    slider_state: SliderStateHandle,
}

impl SettingsWidget for WindowOpacityWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "window opacity transparency"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let window_settings = WindowSettings::as_ref(app);
        if !window_settings
            .background_opacity
            .is_configurable(view.window_id, app)
        {
            return Flex::column()
                .with_child(
                    Container::new(render_body_item_label::<AppearancePageAction>(
                        format!("{}:", i18n::tr(app, I18nKey::AppearanceWindowOpacityLabel)),
                        None,
                        None,
                        LocalOnlyIconState::Hidden,
                        ToggleState::Disabled,
                        appearance,
                    ))
                    .finish(),
                )
                .with_child(
                    Container::new(
                        FormattedTextElement::from_str(
                            i18n::tr(app, I18nKey::AppearanceWindowTransparencyUnsupported),
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(appearance.theme().disabled_ui_text_color().into_solid())
                        .finish(),
                    )
                    .with_margin_bottom(8.0)
                    .finish(),
                )
                .finish();
        }

        let opacity_value = *window_settings.background_opacity;
        let mut col = Flex::column().with_child(render_body_item::<AppearancePageAction>(
            format!(
                "{}: {opacity_value}",
                i18n::tr(app, I18nKey::AppearanceWindowOpacityLabel)
            ),
            // TODO(CORE-3384) add AdditionalInfo here.
            None,
            LocalOnlyIconState::for_setting(
                BackgroundOpacity::storage_key(),
                BackgroundOpacity::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .slider(self.slider_state.clone())
                .with_range(BackgroundOpacity::MIN as f32..BackgroundOpacity::MAX as f32)
                .with_default_value(opacity_value as f32)
                .with_style(UiComponentStyles {
                    width: Some(OPACITY_SLIDER_WIDTH),
                    // Margin is 3. to add up with 7. padding on slider for a total of 10.
                    margin: Some(Coords::default().top(3.).bottom(3.)),
                    ..Default::default()
                })
                .on_drag(|ctx, _, val| {
                    ctx.dispatch_typed_action(AppearancePageAction::OpacitySliderDragged(val))
                })
                .on_change(|ctx, _, val| {
                    ctx.dispatch_typed_action(AppearancePageAction::SetOpacity(val))
                })
                .build()
                .finish(),
            None,
        ));
        if let Some(window) = app.windows().platform_window(view.window_id) {
            // Skip showing the warning for OpenGL since WGPU often incorrectly reports it as not
            // supporting alpha.
            if !window.supports_transparency() && window.graphics_backend() != GraphicsBackend::Gl {
                let mut message =
                    Cow::Borrowed(i18n::tr(app, I18nKey::AppearanceWindowTransparencyWarning));
                let gpu_settings = GPUSettings::as_ref(app);
                if (gpu_settings
                    .prefer_low_power_gpu
                    .is_supported_on_current_platform()
                    && GPUState::as_ref(app).is_low_power_gpu_available())
                    || gpu_settings
                        .preferred_backend
                        .is_supported_on_current_platform()
                {
                    message.to_mut().push_str(i18n::tr(
                        app,
                        I18nKey::AppearanceWindowTransparencySettingsSuggestion,
                    ));
                }

                col.add_child(
                    Container::new(
                        FormattedTextElement::from_str(
                            message,
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(appearance.theme().disabled_ui_text_color().into_solid())
                        .finish(),
                    )
                    .with_margin_bottom(8.0)
                    .finish(),
                );
            }
        }
        col.finish()
    }
}

#[derive(Default)]
struct WindowBlurWidget {
    slider_state: SliderStateHandle,
    info_button: MouseStateHandle,
}

impl SettingsWidget for WindowBlurWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "window blur radius"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let window_settings = WindowSettings::as_ref(app);
        let blur_value = *window_settings.background_blur_radius;
        let label_info = AdditionalInfo {
            mouse_state: self.info_button.clone(),
            on_click_action: Some(AppearancePageAction::OpenUrl(
                "https://docs.warp.dev/terminal/appearance/size-opacity-blurring".into(),
            )),
            secondary_text: None,
            tooltip_override_text: None,
        };

        Flex::column()
            .with_child(render_body_item::<AppearancePageAction>(
                format!(
                    "{}: {blur_value}",
                    i18n::tr(app, I18nKey::AppearanceWindowBlurRadiusLabel)
                ),
                Some(label_info),
                LocalOnlyIconState::for_setting(
                    BackgroundBlurRadius::storage_key(),
                    BackgroundBlurRadius::sync_to_cloud(),
                    &mut view.local_only_icon_tooltip_states.borrow_mut(),
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                appearance
                    .ui_builder()
                    .slider(self.slider_state.clone())
                    .with_range(BackgroundBlurRadius::MIN as f32..BackgroundBlurRadius::MAX as f32)
                    .with_default_value(blur_value as f32)
                    .with_style(UiComponentStyles {
                        width: Some(OPACITY_SLIDER_WIDTH),
                        // Margin is 3. to add up with 7. padding on slider
                        // for a total of 10.
                        margin: Some(Coords::default().top(3.).bottom(3.)),
                        ..Default::default()
                    })
                    .on_drag(|ctx, _, val| {
                        ctx.dispatch_typed_action(AppearancePageAction::BlurSliderDragged(val))
                    })
                    .on_change(|ctx, _, val| {
                        ctx.dispatch_typed_action(AppearancePageAction::SetBlur(val))
                    })
                    .build()
                    .finish(),
                None,
            ))
            .finish()
    }
}

#[derive(Default)]
struct WindowBlurTextureWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for WindowBlurTextureWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "window blur texture acrylic"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let window_settings = WindowSettings::as_ref(app);
        let use_blur_texture = *window_settings.background_blur_texture;
        let mut col = Flex::column().with_child(render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceWindowBlurTextureLabel).to_string(),
            None,
            LocalOnlyIconState::for_setting(
                BackgroundBlurTexture::storage_key(),
                BackgroundBlurTexture::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(use_blur_texture)
                .build()
                .on_click(|evt_ctx, _app, _v2f| {
                    evt_ctx.dispatch_typed_action(AppearancePageAction::ToggleBlurTexture);
                })
                .finish(),
            None,
        ));
        if let Some(window) = app.windows().platform_window(view.window_id) {
            if !window.supports_transparency() && window.graphics_backend() != GraphicsBackend::Gl {
                col.add_child(
                    Container::new(
                        FormattedTextElement::from_str(
                            i18n::tr(app, I18nKey::AppearanceWindowHardwareTransparencyWarning),
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(appearance.theme().disabled_ui_text_color().into_solid())
                        .finish(),
                    )
                    .with_margin_bottom(8.0)
                    .finish(),
                );
            }
        }
        col.finish()
    }
}

#[derive(Default)]
struct ToolsPanelStateScopeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ToolsPanelStateScopeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "left tools panel open closed across tabs file tree project explorer global search warp drive conversation list"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let window_settings = WindowSettings::as_ref(app);
        let is_enabled = *window_settings.left_panel_visibility_across_tabs;

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceToolsPanelConsistentAcrossTabs).to_string(),
            None,
            LocalOnlyIconState::for_setting(
                LeftPanelVisibilityAcrossTabs::storage_key(),
                LeftPanelVisibilityAcrossTabs::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(is_enabled)
                .build()
                .on_click(|evt_ctx, _app, _v2f| {
                    evt_ctx.dispatch_typed_action(AppearancePageAction::ToggleLeftPanelVisibility);
                })
                .finish(),
            None,
        )
    }
}

struct InputTypeWidget {
    radio_buttons_states: Vec<MouseStateHandle>,
}

impl Default for InputTypeWidget {
    fn default() -> Self {
        Self {
            radio_buttons_states: vec![MouseStateHandle::default(), MouseStateHandle::default()],
        }
    }
}

impl SettingsWidget for InputTypeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "input type warp universal classic style prompt terminal ai developer mode interface shell chips ps1"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let input_type = InputSettings::as_ref(app).input_type(app);
        let radio_buttons = appearance
            .ui_builder()
            .radio_buttons(
                self.radio_buttons_states.clone(),
                vec![
                    RadioButtonItem::text(i18n::tr(app, I18nKey::AppearanceInputRadioWarp)),
                    RadioButtonItem::text(i18n::tr(app, I18nKey::AppearanceInputRadioShellPs1)),
                ],
                view.input_type_radio_state.clone(),
                Some(input_type as usize),
                appearance.ui_font_size(),
                RadioButtonLayout::Row,
            )
            .on_change(Rc::new(move |ctx, _, index| {
                if let Some(index) = index {
                    let input_type = match index {
                        0 => InputBoxType::Universal,
                        _ => InputBoxType::Classic,
                    };
                    ctx.dispatch_typed_action(AppearancePageAction::SetInputType(input_type));
                }
            }))
            .build()
            .finish();

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceInputTypeLabel).into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            radio_buttons,
            None,
        )
    }
}

#[derive(Default)]
struct InputModeWidget {}

impl SettingsWidget for InputModeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "input mode input position pinned top bottom classic waterfall reverse"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceInputPositionLabel),
            None,
            None,
            LocalOnlyIconState::for_setting(
                InputModeState::storage_key(),
                InputModeState::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.input_mode_dropdown,
        )
    }
}

#[derive(Default)]
struct PromptWidget {
    button_mouse_state: MouseStateHandle,
}

impl SettingsWidget for PromptWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "prompt ps1 terminal warp shell custom"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let session_settings = SessionSettings::as_ref(app);
        let honor_ps1 = *session_settings.honor_ps1;
        let background = internal_colors::fg_overlay_1(appearance.theme());

        let body = if honor_ps1 {
            // TODO: we should render something else when the grid info isn't available.
            if let Some((grid, size_info)) = &view.ps1_grid_info {
                let left_padding = size_info.padding_x_px();
                let prompt_grid = BlockGridElement::new(
                    grid,
                    appearance,
                    *FontSettings::as_ref(app).enforce_minimum_contrast,
                    ObfuscateSecrets::No,
                    *size_info,
                )
                .finish();

                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(
                        Clipped::new(
                            Container::new(prompt_grid)
                                // Remove any left-padding built into the prompt to make sure it's
                                // left-aligned with the title.
                                .with_padding_left(-left_padding.as_f32())
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish()
            } else {
                Empty::new().finish()
            }
        } else {
            Wrap::row()
                .with_children(view.context_chips.iter().map(|renderer| {
                    Container::new(renderer.render_unused(ChipDragState::Undraggable, appearance))
                        .with_margin_right(4.)
                        .finish()
                }))
                .with_run_spacing(4.)
                .finish()
        };

        Hoverable::new(self.button_mouse_state.clone(), |hover_state| {
            let (border_color, border_width) = match hover_state.is_hovered() {
                true => (appearance.theme().accent(), 1.0),
                false => (appearance.theme().accent(), 0.0),
            };

            Container::new(body)
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(border_width).with_border_fill(border_color))
                .with_horizontal_padding(24. - border_width)
                .with_vertical_padding(12. - border_width)
                .with_margin_right(4.)
                .with_margin_bottom(16.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::OpenPromptEditor {
                open_source: PromptEditorOpenSource::AppearancePage,
            })
        })
        .finish()
    }
}

#[derive(Default)]
struct DimInactivePanesWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for DimInactivePanesWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "dim inactive panes"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearancePanesDimInactive).into(),
            None,
            LocalOnlyIconState::for_setting(
                ShouldDimInactivePanes::storage_key(),
                ShouldDimInactivePanes::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*PaneSettings::as_ref(app).should_dim_inactive_panes)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleDimInactivePanes);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct FocusFollowsMouseWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for FocusFollowsMouseWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "focus follows mouse"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearancePanesFocusFollowsMouse).into(),
            None,
            LocalOnlyIconState::for_setting(
                FocusPaneOnHover::storage_key(),
                FocusPaneOnHover::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*PaneSettings::as_ref(app).focus_panes_on_hover)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleFocusPaneOnHover);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct CompactModeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CompactModeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "compact mode spacing padding"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_compact_mode = matches!(
            TerminalSettings::as_ref(app).spacing_mode.value(),
            SpacingMode::Compact
        );

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceBlocksCompactMode).into(),
            None,
            LocalOnlyIconState::for_setting(
                Spacing::storage_key(),
                Spacing::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(is_compact_mode)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleCompactMode);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct JumpToBottomOfBlockWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for JumpToBottomOfBlockWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "jump to bottom of block button"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let block_list_settings = BlockListSettings::as_ref(app);
        let enabled = block_list_settings
            .show_jump_to_bottom_of_block_button
            .value();
        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceBlocksJumpToBottomButton).into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowJumpToBottomOfBlockButton::storage_key(),
                ShowJumpToBottomOfBlockButton::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        AppearancePageAction::ToggleJumpToBottomOfBlockButton,
                    );
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ShowBlockDividersWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowBlockDividersWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "show block dividers"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let block_list_settings = BlockListSettings::as_ref(app);
        let enabled = block_list_settings.show_block_dividers.value();
        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceBlocksShowDividers).into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowBlockDividers::storage_key(),
                ShowBlockDividers::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleShowBlockDividers);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct AIFontWidget {
    checkbox_state: MouseStateHandle,
}

impl SettingsWidget for AIFontWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "text agent ai font family font size monospace"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let font_settings = FontSettings::as_ref(app);
        let mut ai_font_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let mut ai_font = Flex::column();
        ai_font.add_child(render_body_item_label::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTextAgentFont).to_string(),
            None,
            None,
            LocalOnlyIconState::for_setting(
                AIFontName::storage_key(),
                AIFontName::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
        ));
        ai_font.add_child(
            Container::new(ChildView::new(&view.ai_font_family_dropdown).finish())
                .with_margin_bottom(10.)
                .finish(),
        );

        ai_font_row
            .add_child(Shrinkable::new(1., Align::new(ai_font.finish()).left().finish()).finish());
        ai_font_row.add_child(
            appearance
                .ui_builder()
                .checkbox(self.checkbox_state.clone(), None)
                .check(*font_settings.match_ai_font_to_terminal_font)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        AppearancePageAction::ToggleMatchAIToTerminalFontFamily,
                    )
                })
                .finish(),
        );
        ai_font_row.add_child(
            appearance
                .ui_builder()
                .span(i18n::tr(app, I18nKey::AppearanceTextMatchTerminal).to_string())
                .build()
                .with_margin_left(2.)
                .with_margin_right(16.)
                .finish(),
        );

        ai_font_row.finish()
    }
}

#[derive(Default)]
struct TerminalFontWidget {
    line_height_button_state: MouseStateHandle,
    fonts_checkbox_state: MouseStateHandle,
}

impl TerminalFontWidget {
    fn render_line_height_editor(
        &self,
        view: &AppearanceSettingsPageView,
        appearance: &Appearance,
        app: &AppContext,
        row: &mut Flex,
    ) {
        let mut line_height = Flex::column();
        line_height.add_child(
            appearance
                .ui_builder()
                .label(i18n::tr(app, I18nKey::AppearanceTextLineHeight).to_string())
                .with_style(UiComponentStyles {
                    margin: Some(Coords {
                        left: 12.,
                        ..Default::default()
                    }),
                    font_size: Some(CONTENT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        line_height.add_child(
            Container::new(
                Dismiss::new(
                    appearance
                        .ui_builder()
                        .text_input(view.line_height_editor.clone())
                        .with_style(UiComponentStyles {
                            width: Some(LINE_HEIGHT_INPUT_BOX_WIDTH),
                            padding: Some(Coords {
                                top: 7.,
                                bottom: 7.,
                                left: 12.,
                                right: 12.,
                            }),
                            margin: Some(Coords {
                                top: 2.,
                                left: 12.,
                                ..Default::default()
                            }),
                            background: Some(appearance.theme().surface_2().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(AppearancePageAction::SetLineHeight)
                })
                .finish(),
            )
            .with_padding_top(4.)
            .finish(),
        );
        line_height.add_child({
            let button = appearance
                .ui_builder()
                .reset_button(
                    ButtonVariant::Text,
                    self.line_height_button_state.clone(),
                    appearance.line_height_ratio() != DEFAULT_UI_LINE_HEIGHT_RATIO,
                    appearance
                        .theme()
                        .disabled_text_color(appearance.theme().surface_2())
                        .into(),
                )
                .with_style(UiComponentStyles {
                    padding: Some(Coords::default().bottom(HEADER_PADDING).top(4.)),
                    margin: Some(Coords {
                        top: 2.,
                        left: 8.,
                        ..Default::default()
                    }),
                    font_size: Some(appearance.ui_font_size() * 0.8),
                    ..Default::default()
                })
                .with_text_label(i18n::tr(app, I18nKey::AppearanceTextResetToDefault).to_string());

            button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::LineHeightEditorResetRatio);
                })
                .finish()
        });
        row.add_child(line_height.finish());
    }
}

impl SettingsWidget for TerminalFontWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "text terminal font family font size line height monospace"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut terminal_font_row = Flex::row();

        // Terminal Font
        let mut terminal_font = Flex::column();
        terminal_font.add_child(render_body_item_label::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTextTerminalFont).to_string(),
            None,
            None,
            LocalOnlyIconState::for_setting(
                MonospaceFontName::storage_key(),
                MonospaceFontName::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
        ));

        terminal_font.add_child(
            Container::new(ChildView::new(&view.font_family_dropdown).finish())
                .with_margin_bottom(10.)
                .finish(),
        );
        terminal_font.add_child(
            Container::new(
                Flex::row()
                    .with_child(
                        Container::new(
                            appearance
                                .ui_builder()
                                .checkbox(self.fonts_checkbox_state.clone(), None)
                                .check(view.view_font_type == FontType::Any)
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        AppearancePageAction::ToggleAllAvailableFonts,
                                    )
                                })
                                .finish(),
                        )
                        .with_margin_left(-7.)
                        .finish(),
                    )
                    .with_child(
                        Shrinkable::new(
                            1.,
                            appearance
                                .ui_builder()
                                .span(
                                    i18n::tr(app, I18nKey::AppearanceTextViewAllFonts).to_string(),
                                )
                                .build()
                                .with_margin_left(2.)
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        terminal_font_row.add_child(Shrinkable::new(1., terminal_font.finish()).finish());

        // Font Weight
        let mut font_weight = Flex::column();
        font_weight.add_child(
            appearance
                .ui_builder()
                .label(i18n::tr(app, I18nKey::AppearanceTextFontWeight).to_string())
                .with_style(UiComponentStyles {
                    font_size: Some(CONTENT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .with_margin_left(12.)
                .finish(),
        );

        font_weight.add_child(
            Container::new(ChildView::new(&view.font_weight_dropdown).finish())
                .with_margin_left(12.)
                .finish(),
        );

        terminal_font_row.add_child(Container::new(font_weight.finish()).finish());

        // Font Size
        let mut font_size = Flex::column();
        font_size.add_child(
            appearance
                .ui_builder()
                .label(i18n::tr(app, I18nKey::AppearanceTextFontSizePx).to_string())
                .with_style(UiComponentStyles {
                    margin: Some(Coords {
                        left: 2.,
                        ..Default::default()
                    }),
                    font_size: Some(CONTENT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        font_size.add_child(
            Container::new(
                Dismiss::new(
                    appearance
                        .ui_builder()
                        .text_input(view.font_size_editor.clone())
                        .with_style(UiComponentStyles {
                            width: Some(FONT_SIZE_INPUT_BOX_WIDTH),
                            padding: Some(Coords {
                                top: 7.,
                                bottom: 7.,
                                left: 12.,
                                right: 12.,
                            }),
                            margin: Some(Coords {
                                top: 2.,
                                left: 2.,
                                ..Default::default()
                            }),
                            background: Some(appearance.theme().surface_2().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(AppearancePageAction::SetFontSize)
                })
                .finish(),
            )
            .with_padding_top(4.)
            .finish(),
        );
        terminal_font_row.add_child(
            Container::new(font_size.finish())
                .with_margin_left(12.)
                .finish(),
        );

        self.render_line_height_editor(view, appearance, app, &mut terminal_font_row);
        terminal_font_row.finish()
    }
}

#[derive(Default)]
struct NotebookFontSizeWidget {
    checkbox_state: MouseStateHandle,
}

impl SettingsWidget for NotebookFontSizeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "text notebook font size"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let font_settings = FontSettings::as_ref(app);
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(
                        1.0,
                        Align::new(
                            appearance
                                .ui_builder()
                                .span(
                                    i18n::tr(app, I18nKey::AppearanceTextNotebookFontSize)
                                        .to_string(),
                                )
                                .build()
                                .with_margin_right(16.)
                                .finish(),
                        )
                        .left()
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    appearance
                        .ui_builder()
                        .checkbox(self.checkbox_state.clone(), None)
                        .check(*font_settings.match_notebook_to_monospace_font_size)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(
                                AppearancePageAction::ToggleMatchNotebookToMonospaceFontSize,
                            )
                        })
                        .finish(),
                )
                .with_child(
                    appearance
                        .ui_builder()
                        .span(i18n::tr(app, I18nKey::AppearanceTextMatchTerminal).to_string())
                        .build()
                        .with_margin_left(2.)
                        .with_margin_right(16.)
                        .finish(),
                )
                .with_child(
                    Container::new(
                        Dismiss::new(
                            appearance
                                .ui_builder()
                                .text_input(view.notebook_font_size_editor.clone())
                                .with_style(UiComponentStyles {
                                    width: Some(NOTEBOOK_FONT_SIZE_INPUT_BOX_WIDTH),
                                    padding: Some(Coords {
                                        top: 7.,
                                        bottom: 7.,
                                        left: 16.,
                                        right: 16.,
                                    }),
                                    background: Some(appearance.theme().surface_2().into()),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .on_dismiss(|ctx, _app| {
                            ctx.dispatch_typed_action(AppearancePageAction::SetNotebookFontSize)
                        })
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_margin_bottom(10.)
        .finish()
    }
}

#[derive(Default)]
struct ThinStrokesWidget {}

impl SettingsWidget for ThinStrokesWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "text thin strokes high dpi"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceTextUseThinStrokes),
            None,
            None,
            LocalOnlyIconState::for_setting(
                UseThinStrokes::storage_key(),
                UseThinStrokes::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.thin_strokes_dropdown,
        )
    }
}

#[derive(Default)]
struct MinimumContrastWidget {}

impl SettingsWidget for MinimumContrastWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "text minimum contrast high"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceTextMinimumContrast),
            None,
            None,
            LocalOnlyIconState::for_setting(
                crate::settings::font::EnforceMinimumContrast::storage_key(),
                crate::settings::font::EnforceMinimumContrast::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.enforce_min_contrast_dropdown,
        )
    }
}

#[derive(Default)]
struct LigaturesWidget {
    switch_state: SwitchStateHandle,
    info_mouse_state: MouseStateHandle,
}

impl SettingsWidget for LigaturesWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "text font ligatures"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ligature_rendering = &LigatureSettings::as_ref(app).ligature_rendering_enabled;
        let ligature_rendering_enabled = ligature_rendering.value();

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTextShowLigatures).into(),
            Some(AdditionalInfo {
                mouse_state: self.info_mouse_state.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    i18n::tr(app, I18nKey::AppearanceTextLigaturesTooltip).to_string(),
                ),
            }),
            LocalOnlyIconState::for_setting(
                LigatureRenderingEnabled::storage_key(),
                LigatureRenderingEnabled::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*ligature_rendering_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleLigatureRendering);
                })
                .finish(),
            None,
        )
    }
}

struct CursorTypeWidget {
    radio_state: RadioButtonStateHandle,
    radio_buttons_states: Vec<MouseStateHandle>,
}

impl Default for CursorTypeWidget {
    fn default() -> Self {
        Self {
            radio_state: Default::default(),
            radio_buttons_states: all::<CursorDisplayType>()
                .map(|_| Default::default())
                .collect(),
        }
    }
}

impl SettingsWidget for CursorTypeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "cursor shape cursor type block bar beam underline"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = AppEditorSettings::as_ref(app);
        let cursor_display_type = &settings.cursor_display_type;
        let is_vim_mode_enabled = *settings.vim_mode.value();

        let cursor_display_types: Vec<CursorDisplayType> = all::<CursorDisplayType>().collect();

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceCursorType).into(),
            None,
            LocalOnlyIconState::for_setting(
                CursorBlinkEnabled::storage_key(),
                CursorBlinkEnabled::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            match is_vim_mode_enabled {
                true => Flex::column()
                    .with_child(
                        appearance
                            .ui_builder()
                            .span(
                                i18n::tr(app, I18nKey::AppearanceCursorTypeDisabledVim).to_string(),
                            )
                            .build()
                            .finish(),
                    )
                    .finish(),
                false => appearance
                    .ui_builder()
                    .radio_buttons(
                        self.radio_buttons_states.clone(),
                        cursor_display_types
                            .iter()
                            .map(|value| {
                                RadioButtonItem::text(
                                    AppearanceSettingsPageView::cursor_display_type_label(
                                        app, *value,
                                    ),
                                )
                            })
                            .collect(),
                        self.radio_state.clone(),
                        Some(cursor_display_type.value().to_index()),
                        appearance.ui_font_size(),
                        RadioButtonLayout::Row,
                    )
                    .on_change(Rc::new(move |ctx, _, index| {
                        if let Some(index) = index {
                            ctx.dispatch_typed_action(AppearancePageAction::SetCursorType(
                                CursorDisplayType::nth(index).expect("Cursor does not exist"),
                            ));
                        }
                    }))
                    .build()
                    .finish(),
            },
            None,
        )
    }
}

#[derive(Default)]
struct BlinkingCursorWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for BlinkingCursorWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "blinking cursor"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = AppEditorSettings::as_ref(app);
        let cursor_blink = &settings.cursor_blink;
        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceCursorBlinking).into(),
            None,
            LocalOnlyIconState::for_setting(
                CursorBlinkEnabled::storage_key(),
                CursorBlinkEnabled::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(cursor_blink.value() == &CursorBlink::Enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleCursorBlink);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct TabCloseButtonPositionWidget {}

impl SettingsWidget for TabCloseButtonPositionWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "tab bar close button position left right"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceTabsCloseButtonPosition),
            None,
            None,
            LocalOnlyIconState::for_setting(
                TabCloseButtonPosition::storage_key(),
                TabCloseButtonPosition::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.tab_close_button_position_dropdown,
        )
    }
}

#[derive(Default)]
struct TabIndicatorWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for TabIndicatorWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "tab indicator"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsShowIndicators).into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowIndicatorsButton::storage_key(),
                ShowIndicatorsButton::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.show_indicators)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleTabIndicators);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct CodeReviewButtonWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CodeReviewButtonWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "code review button tab bar"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsShowCodeReviewButton).into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowCodeReviewButton::storage_key(),
                ShowCodeReviewButton::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.show_code_review_button)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleShowCodeReviewButton);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct PreserveActiveTabColorWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for PreserveActiveTabColorWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "tab color preserve new inherit active"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsPreserveActiveColor).into(),
            None,
            LocalOnlyIconState::for_setting(
                PreserveActiveTabColor::storage_key(),
                PreserveActiveTabColor::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.preserve_active_tab_color)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::TogglePreserveActiveTabColor);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct VerticalTabsWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for VerticalTabsWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "vertical tabs sidebar layout"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsUseVerticalLayout).into(),
            None,
            LocalOnlyIconState::for_setting(
                UseVerticalTabs::storage_key(),
                UseVerticalTabs::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.use_vertical_tabs)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AppearancePageAction::ToggleVerticalTabs);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ShowVerticalTabPanelInRestoredWindowsWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowVerticalTabPanelInRestoredWindowsWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "vertical tabs panel restore window session snapshot"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsShowVerticalPanelRestored).into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowVerticalTabPanelInRestoredWindows::storage_key(),
                ShowVerticalTabPanelInRestoredWindows::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.show_vertical_tab_panel_in_restored_windows)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        AppearancePageAction::ToggleShowVerticalTabPanelInRestoredWindows,
                    );
                })
                .finish(),
            Some(
                i18n::tr(
                    app,
                    I18nKey::AppearanceTabsShowVerticalPanelRestoredDescription,
                )
                .to_string(),
            ),
        )
    }
}

#[derive(Default)]
struct UseLatestUserPromptAsConversationTitleInTabNamesWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for UseLatestUserPromptAsConversationTitleInTabNamesWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "latest user prompt conversation title tab names vertical tabs oz third-party agent"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsUseLatestPromptTitle).into(),
            None,
            LocalOnlyIconState::for_setting(
                UseLatestUserPromptAsConversationTitleInTabNames::storage_key(),
                UseLatestUserPromptAsConversationTitleInTabNames::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(
                    *tab_settings
                        .use_latest_user_prompt_as_conversation_title_in_tab_names,
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        AppearancePageAction::ToggleUseLatestUserPromptAsConversationTitleInTabNames,
                    );
                })
                .finish(),
            Some(
                i18n::tr(app, I18nKey::AppearanceTabsUseLatestPromptTitleDescription)
                    .to_string(),
            ),
        )
    }
}

#[derive(Default)]
struct EditToolbarWidget;

impl SettingsWidget for EditToolbarWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "edit toolbar header panel buttons configure arrange layout chip chips rearrange re-arrange customize"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let label = render_body_item_label::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceTabsHeaderToolbarLayout).to_string(),
            None,
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
        );
        let editor = Container::new(ChildView::new(&view.header_toolbar_inline_editor).finish())
            .with_padding_bottom(HEADER_PADDING)
            .finish();

        Flex::column()
            .with_child(Container::new(label).with_margin_bottom(4.).finish())
            .with_child(editor)
            .finish()
    }
}

/// Returns the visible directory tab colors from the persisted setting.
fn build_directory_delete_buttons(
    ctx: &mut ViewContext<AppearanceSettingsPageView>,
) -> Vec<ViewHandle<ActionButton>> {
    directory_tab_colors(ctx)
        .into_iter()
        .map(|(dir_path, _)| {
            let delete_path = PathBuf::from(&dir_path);
            ctx.add_typed_action_view(move |_| {
                ActionButton::new("", NakedTheme)
                    .with_icon(Icon::X)
                    .with_size(ButtonSize::XSmall)
                    .on_click(move |ctx| {
                        ctx.dispatch_typed_action(
                            AppearancePageAction::RemoveDefaultDirectoryTabColor {
                                path: delete_path.clone(),
                            },
                        );
                    })
            })
        })
        .collect()
}

fn add_directory_tab_color_path(path: PathBuf, ctx: &mut ViewContext<AppearanceSettingsPageView>) {
    TabSettings::handle(ctx).update(ctx, |settings, ctx| {
        let current = settings.directory_tab_colors.value();
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let key = canonical.to_string_lossy().to_string();
        let dominated_by_existing = current
            .0
            .get(&key)
            .is_some_and(|c| *c != DirectoryTabColor::Suppressed);
        if !dominated_by_existing {
            let new_value = current.with_color(&path, DirectoryTabColor::Unassigned);
            let _ = settings.directory_tab_colors.set_value(new_value, ctx);
        }
    });
}

fn open_directory_tab_color_folder_picker(ctx: &mut ViewContext<AppearanceSettingsPageView>) {
    let file_picker_config = FilePickerConfiguration::new().folders_only();
    ctx.open_file_picker(
        move |result, ctx| {
            if let Ok(paths) = result {
                if let Some(directory_path) = paths.first() {
                    add_directory_tab_color_path(PathBuf::from(directory_path), ctx);
                }
            }
        },
        file_picker_config,
    );
}

fn directory_tab_colors(app: &AppContext) -> Vec<(String, DirectoryTabColor)> {
    let configured = &TabSettings::as_ref(app).directory_tab_colors.value().0;
    let mut sorted: Vec<_> = configured
        .iter()
        .filter(|(_, color)| !matches!(color, DirectoryTabColor::Suppressed))
        .map(|(path, color)| (path.clone(), *color))
        .collect();
    sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
    sorted
}

struct DirectoryTabColorsWidget {
    add_picker: ViewHandle<DirectoryColorAddPicker>,
}

impl SettingsWidget for DirectoryTabColorsWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "directory tab color folder codebase repo"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut content = Flex::column().with_spacing(8.);
        let header_text = Flex::column()
            .with_spacing(4.)
            .with_child(
                Text::new(
                    i18n::tr(app, I18nKey::AppearanceTabsDirectoryColors),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.active_ui_text_color().into())
                .soft_wrap(false)
                .finish(),
            )
            .with_child(
                Text::new(
                    i18n::tr(app, I18nKey::AppearanceTabsDirectoryColorsDescription),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .finish(),
            )
            .finish();
        content.add_child(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(Shrinkable::new(1., header_text).finish())
                .with_child(ChildView::new(&self.add_picker).finish())
                .finish(),
        );

        let home_dir =
            dirs::home_dir().and_then(|home_dir| home_dir.to_str().map(|s| s.to_owned()));
        for (idx, (dir_path, current_color)) in directory_tab_colors(app).into_iter().enumerate() {
            let Some(dot_mouse_states) = view.color_picker_dot_states.get(idx).cloned() else {
                log::error!("Missing color picker dot states for directory index {idx}");
                continue;
            };

            let friendly_path = user_friendly_path(&dir_path, home_dir.as_deref()).to_string();
            let path_label = Shrinkable::new(
                1.,
                Text::new(
                    friendly_path,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .soft_wrap(false)
                .finish(),
            )
            .finish();

            let mut dots_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            // None = default (no-color) dot, Some = ANSI color dot
            let color_options =
                std::iter::once(None).chain(TAB_COLOR_OPTIONS.iter().copied().map(Some));
            for (ansi_id, mouse_state) in color_options.zip(dot_mouse_states.iter().cloned()) {
                let tab_color = match ansi_id {
                    None => DirectoryTabColor::Unassigned,
                    Some(id) => DirectoryTabColor::Color(id),
                };
                let dot_color = match ansi_id {
                    None => pathfinder_color::ColorU::transparent_black(),
                    Some(id) => id.to_ansi_color(&theme.terminal_colors().normal).into(),
                };
                let is_selected = current_color == tab_color;
                let tooltip_text = match ansi_id {
                    None => i18n::tr(app, I18nKey::AppearanceTabsDefaultNoColor).to_string(),
                    Some(id) => id.to_string(),
                };
                let dir_path_clone = PathBuf::from(&dir_path);

                dots_row.add_child(
                    render_color_dot(
                        mouse_state,
                        dot_color,
                        is_selected,
                        theme.accent().into(),
                        ansi_id.is_none(),
                        theme.foreground(),
                        tooltip_text,
                        appearance,
                    )
                    .on_click(move |ctx, _, _| {
                        if !is_selected {
                            ctx.dispatch_typed_action(
                                AppearancePageAction::SetDefaultDirectoryTabColor {
                                    path: dir_path_clone.clone(),
                                    color: tab_color,
                                },
                            );
                        }
                    })
                    .finish(),
                );
            }

            let mut right_controls = Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);
            right_controls.add_child(dots_row.finish());
            if let Some(delete_button) = view.directory_tab_color_delete_buttons.get(idx) {
                right_controls.add_child(ChildView::new(delete_button).finish());
            }
            let right_controls = right_controls.finish();

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(path_label)
                .with_child(right_controls)
                .finish();

            content.add_child(
                Container::new(row)
                    .with_horizontal_padding(16.)
                    .with_vertical_padding(8.)
                    .with_background(theme.surface_1())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            );
        }

        Container::new(content.finish())
            .with_padding_bottom(HEADER_PADDING)
            .finish()
    }
}

#[derive(Default)]
struct ZenModeWidget {}

impl SettingsWidget for ZenModeWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "zen mode minimal tab bar window decoration"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceTabsShowTabBar),
            None,
            None,
            LocalOnlyIconState::for_setting(
                WorkspaceDecorationVisibility::storage_key(),
                WorkspaceDecorationVisibility::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.workspace_decorations_dropdown,
        )
    }
}

#[derive(Default)]
struct AltScreenPaddingWidget {
    switch_state: SwitchStateHandle,
    additional_info_mouse_state: MouseStateHandle,
}

impl SettingsWidget for AltScreenPaddingWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "alt screen padding border space vim"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let terminal_settings = &TerminalSettings::as_ref(app);
        let theme = appearance.theme();
        let mut column = Flex::column().with_child(render_body_item::<AppearancePageAction>(
            i18n::tr(app, I18nKey::AppearanceFullscreenUseCustomPadding).into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_mouse_state.clone(),
                on_click_action: Some(AppearancePageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/more-features/full-screen-apps#padding".into(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                AltScreenPadding::storage_key(),
                AltScreenPadding::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(matches!(
                    terminal_settings.alt_screen_padding.value(),
                    AltScreenPaddingMode::Custom { .. }
                ))
                .build()
                .on_click(move |ctx, app, _| {
                    let new_mode = TerminalSettings::as_ref(app).alt_screen_padding.toggled();
                    ctx.dispatch_typed_action(AppearancePageAction::UpdateAltScreenPaddingMode(
                        new_mode,
                    ));
                })
                .finish(),
            None,
        ));

        if matches!(
            terminal_settings.alt_screen_padding.value(),
            AltScreenPaddingMode::Custom { .. }
        ) {
            let buffer_text = view.alt_screen_padding_editor.as_ref(app).buffer_text(app);
            let border_color = match buffer_text.parse::<f32>() {
                Ok(p) if p >= 0. => None,
                _ => Some(themes::theme::Fill::error().into()),
            };

            let editor_style = UiComponentStyles {
                width: Some(40.),
                padding: Some(Coords::uniform(5.)),
                background: Some(theme.surface_2().into()),
                border_color,
                ..Default::default()
            };

            let editor_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(
                    Container::new(
                        Align::new(
                            Text::new(
                                i18n::tr(app, I18nKey::AppearanceFullscreenUniformPaddingPx),
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    appearance
                        .ui_builder()
                        .text_input(view.alt_screen_padding_editor.clone())
                        .with_style(editor_style)
                        .build()
                        .finish(),
                )
                .finish();

            column.add_child(render_group([editor_row], appearance));
        }
        column.finish()
    }
}

struct ZoomLevelWidget;

impl SettingsWidget for ZoomLevelWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "zoom level zoom size scale"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let current_zoom_level = *WindowSettings::as_ref(app).zoom_level.value();
        let changed_from_default = current_zoom_level != ZoomLevel::default_value();

        let reset_button = build_reset_button(
            appearance,
            app,
            view.zoom_reset_button_mouse_state.clone(),
            changed_from_default,
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AppearancePageAction::ResetZoomLevel);
        })
        .finish();

        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AppearanceZoomLabel),
            Some(i18n::tr(app, I18nKey::AppearanceZoomDescription)),
            Some(reset_button),
            LocalOnlyIconState::for_setting(
                crate::window_settings::ZoomLevel::storage_key(),
                crate::window_settings::ZoomLevel::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            None,
            &view.zoom_level_dropdown,
        )
    }
}

impl SettingsPageMeta for AppearanceSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Appearance
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<AppearanceSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<AppearanceSettingsPageView>) -> Self {
        SettingsPageViewHandle::Appearance(view_handle)
    }
}
