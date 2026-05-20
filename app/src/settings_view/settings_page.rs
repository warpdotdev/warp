use crate::ui_components::blended_colors;
use core::fmt::{self, Display};
use itertools::Itertools as _;
use pathfinder_color::ColorU;
use std::borrow::Cow;
use std::collections::HashMap;

use super::{
    about_page::AboutPageView,
    ai_page::{AISettingsPageAction, AISettingsPageView},
    appearance_page::AppearanceSettingsPageView,
    billing_and_usage_page::BillingAndUsagePageView,
    billing_and_usage_page_v2::BillingAndUsagePageV2View,
    code_page::CodeSettingsPageView,
    environments_page::EnvironmentsPageView,
    features_page::FeaturesPageView,
    keybindings::KeybindingsView,
    main_page::MainSettingsPageView,
    mcp_servers_page::MCPServersSettingsPageView,
    privacy_page::PrivacyPageView,
    referrals_page::ReferralsPageView,
    show_blocks_view::ShowBlocksView,
    teams_page::TeamsPageView,
    warp_drive_page::WarpDriveSettingsPageView,
    warpify_page::WarpifyPageView,
    SettingsSection,
};
use crate::{
    appearance::Appearance,
    settings::CloudPreferencesSettings,
    themes::theme::Fill,
    ui_components::icons::Icon,
    view_components::{Dropdown, SubmittableTextInput},
};
use pathfinder_geometry::vector::vec2f;
use settings::Setting;
use warp_core::{
    settings::SyncToCloud,
    ui::{color::blend::Blend, theme::color::internal_colors},
};
use warpui::{
    elements::{
        new_scrollable::{ClippedAxisConfiguration, DualAxisConfig, SingleAxisConfig},
        Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Element, Empty, Expanded, Flex, Hoverable,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, NewScrollable, OffsetPositioning,
        ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition, ScrollTarget,
        ScrollToPositionMode, Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Stack,
        Text,
    },
    fonts::{Properties, Weight},
    platform::Cursor,
    ui_components::{
        button::{Button, ButtonVariant},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    units::Pixels,
    Action, AppContext, SingletonEntity, ViewContext, ViewHandle,
};

pub const TOGGLE_BUTTON_RIGHT_PADDING: f32 = 5.;
pub const HEADER_PADDING: f32 = 15.;
pub const CONTENT_FONT_SIZE: f32 = 12.;
pub const SUBHEADER_MARGIN_BOTTOM: f32 = 4.;
pub const PAGE_TITLE_MARGIN_BOTTOM: f32 = 4.;
pub(super) const PAGE_PADDING: f32 = 28.;
pub(super) const HEADER_FONT_SIZE: f32 = 23.;
pub const SUBHEADER_FONT_SIZE: f32 = 16.;
const ALTERNATING_LIST_CLOSE_BUTTON_DIAMETER: f32 = 20.0;
const ALTERNATING_LIST_ITEM_PADDING: f32 = 8.0;
const GREY_TEXT_OPACITY: u8 = 60;
const MIN_PAGE_WIDTH: f32 = 520.;
const MAX_PAGE_WIDTH: f32 = 800.;
const INFO_TOOLTIP_MAX_WIDTH: f32 = 320.;

/// Left margin for top-level sidebar nav items (pages and umbrella labels).
pub(super) const NAV_ITEM_LEFT_MARGIN: f32 = 12.;

pub struct SettingsPage {
    pub section: SettingsSection,
    pub view_handle: SettingsPageViewHandle,
    button_state_handle: MouseStateHandle,
}

pub trait SettingsPageMeta {
    fn section() -> SettingsSection;

    /// Performs any work necessary to set up the page when it is selected by the user. Pages
    /// should respect the `allow_steal_focus` parameter, abstaining from focusing the page if it's
    /// false.
    fn on_page_selected(&mut self, _allow_steal_focus: bool, _ctx: &mut ViewContext<Self>) {
        log::info!("No updates for the selected view handle.");
    }

    fn should_render(&self, _ctx: &AppContext) -> bool;

    fn on_tab_pressed(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData;

    fn scroll_to_widget(&mut self, widget_id: &'static str);

    fn clear_highlighted_widget(&mut self);
}

/// Page enum lists all the pages that we want to support.
/// It is required to allow for SettingsPage struct be put in the collection (ie. vector).
#[derive(Clone)]
pub enum SettingsPageViewHandle {
    Main(ViewHandle<MainSettingsPageView>),
    Appearance(ViewHandle<AppearanceSettingsPageView>),
    Features(ViewHandle<FeaturesPageView>),
    SharedBlocks(ViewHandle<ShowBlocksView>),
    Keybindings(ViewHandle<KeybindingsView>),
    About(ViewHandle<AboutPageView>),
    Code(ViewHandle<CodeSettingsPageView>),
    Teams(ViewHandle<TeamsPageView>),
    OzCloudAPIKeys(ViewHandle<super::platform_page::PlatformPageView>),
    Privacy(ViewHandle<PrivacyPageView>),
    Warpify(ViewHandle<WarpifyPageView>),
    Referrals(ViewHandle<ReferralsPageView>),
    AI(ViewHandle<AISettingsPageView>),
    CloudEnvironments(ViewHandle<EnvironmentsPageView>),
    BillingAndUsage(ViewHandle<BillingAndUsagePageView>),
    BillingAndUsageV2(ViewHandle<BillingAndUsagePageV2View>),
    MCPServers(ViewHandle<MCPServersSettingsPageView>),
    WarpDrive(ViewHandle<WarpDriveSettingsPageView>),
}

impl SettingsPageViewHandle {
    pub fn child_view(&self) -> Box<dyn Element> {
        use SettingsPageViewHandle::*;
        match self {
            Main(view_handle) => ChildView::new(view_handle).finish(),
            Appearance(view_handle) => ChildView::new(view_handle).finish(),
            Features(view_handle) => ChildView::new(view_handle).finish(),
            SharedBlocks(view_handle) => ChildView::new(view_handle).finish(),
            Keybindings(view_handle) => ChildView::new(view_handle).finish(),
            About(view_handle) => ChildView::new(view_handle).finish(),
            Code(view_handle) => ChildView::new(view_handle).finish(),
            Teams(view_handle) => ChildView::new(view_handle).finish(),
            OzCloudAPIKeys(view_handle) => ChildView::new(view_handle).finish(),
            Privacy(view_handle) => ChildView::new(view_handle).finish(),
            Warpify(view_handle) => ChildView::new(view_handle).finish(),
            Referrals(view_handle) => ChildView::new(view_handle).finish(),
            AI(view_handle) => ChildView::new(view_handle).finish(),
            CloudEnvironments(view_handle) => ChildView::new(view_handle).finish(),
            BillingAndUsage(view_handle) => ChildView::new(view_handle).finish(),
            BillingAndUsageV2(view_handle) => ChildView::new(view_handle).finish(),
            MCPServers(view_handle) => ChildView::new(view_handle).finish(),
            WarpDrive(view_handle) => ChildView::new(view_handle).finish(),
        }
    }
}

impl From<ViewHandle<MCPServersSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<MCPServersSettingsPageView>) -> Self {
        SettingsPageViewHandle::MCPServers(view_handle)
    }
}

impl SettingsPage {
    pub fn new<V>(view_handle: ViewHandle<V>) -> Self
    where
        V: SettingsPageMeta,
        ViewHandle<V>: Into<SettingsPageViewHandle>,
    {
        SettingsPage {
            section: V::section(),
            view_handle: view_handle.into(),
            button_state_handle: MouseStateHandle::default(),
        }
    }

    pub fn render_page_button(
        &self,
        appearance: &Appearance,
        match_data: MatchData,
        clicked: bool,
    ) -> Hoverable {
        appearance
            .ui_builder()
            .button(
                if clicked {
                    ButtonVariant::Accent
                } else {
                    ButtonVariant::Text
                },
                self.button_state_handle.clone(),
            )
            .with_text_label(self.section.to_string() + &match_data.to_string())
            .with_style(
                UiComponentStyles::default()
                    .set_border_width(0.)
                    .set_margin(Coords::default().left(NAV_ITEM_LEFT_MARGIN))
                    .set_padding(Coords::uniform(8.)),
            )
            .build()
    }
}

#[derive(PartialEq, Eq)]
pub enum SettingsPageEvent {
    FocusModal,
    Pane(PaneEventWrapper),
    EnvironmentSetupModeSelectorToggled { is_open: bool },
    AgentAssistedEnvironmentModalToggled { is_open: bool },
}

/// Wrapper for pane events to avoid circular dependency with pane module.
/// The actual handling converts this to the real PaneEvent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneEventWrapper {
    Close,
}

pub fn render_customer_type_badge(appearance: &Appearance, text: String) -> Box<dyn Element> {
    Container::new(
        Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(
                appearance
                    .theme()
                    .background()
                    .blend(
                        &appearance
                            .theme()
                            .foreground()
                            .with_opacity(GREY_TEXT_OPACITY),
                    )
                    .into(),
            )
            .with_style(Properties::default().weight(Weight::Medium))
            .finish(),
    )
    .with_uniform_padding(4.)
    .with_background(
        appearance
            .theme()
            .background()
            .blend(&appearance.theme().foreground().with_opacity(25)),
    )
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
    .with_margin_left(10.)
    .finish()
}

/// Adds padding to the sub header
pub fn render_sub_header(
    appearance: &Appearance,
    text_name: impl Into<Cow<'static, str>>,
    local_only_icon_state: Option<LocalOnlyIconState>,
) -> Box<dyn Element> {
    let mut sub_header = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Shrinkable::new(
                1.,
                build_sub_header(appearance, text_name, None)
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
            )
            .finish(),
        );
    if let Some(LocalOnlyIconState::Visible {
        mouse_state,
        custom_tooltip,
    }) = local_only_icon_state
    {
        sub_header.add_child(
            Container::new(render_local_only_icon(
                appearance,
                mouse_state,
                custom_tooltip,
            ))
            .with_padding_top(3.)
            .finish(),
        );
    }
    sub_header.finish()
}

/// Contains only the sub header
pub fn build_sub_header(
    appearance: &Appearance,
    text_name: impl Into<Cow<'static, str>>,
    color_override: Option<Fill>,
) -> Container {
    let color = color_override.unwrap_or(appearance.theme().active_ui_text_color());
    Container::new(
        Align::new(
            Text::new_inline(text_name, appearance.ui_font_family(), SUBHEADER_FONT_SIZE)
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(color.into())
                .finish(),
        )
        .left()
        .finish(),
    )
    .with_margin_bottom(SUBHEADER_MARGIN_BOTTOM)
}

pub fn render_sub_header_with_description(
    appearance: &Appearance,
    text_name: impl Into<Cow<'static, str>>,
    description: impl Into<Cow<'static, str>>,
) -> Box<dyn Element> {
    Container::new(
        Flex::column()
            .with_child(build_sub_header(appearance, text_name, None).finish())
            .with_child(
                Align::new(
                    Text::new(description, appearance.ui_font_family(), CONTENT_FONT_SIZE)
                        .with_color(appearance.theme().nonactive_ui_text_color().into())
                        .finish(),
                )
                .left()
                .finish(),
            )
            .finish(),
    )
    .with_padding_bottom(HEADER_PADDING)
    .finish()
}

#[cfg_attr(target_family = "wasm", allow(unused))]
pub fn render_sub_sub_header(
    appearance: &Appearance,
    text_name: impl Into<Cow<'static, str>>,
    local_only_icon_state: Option<LocalOnlyIconState>,
) -> Box<dyn Element> {
    let mut sub_sub_header = Flex::row().with_child(
        Container::new(
            Align::new(
                Text::new_inline(text_name, appearance.ui_font_family(), CONTENT_FONT_SIZE)
                    .with_style(Properties::default().weight(Weight::Bold))
                    .with_color(appearance.theme().active_ui_text_color().into())
                    .finish(),
            )
            .left()
            .finish(),
        )
        .with_padding_bottom(4.)
        .finish(),
    );
    if let Some(LocalOnlyIconState::Visible {
        mouse_state,
        custom_tooltip,
    }) = local_only_icon_state
    {
        sub_sub_header.add_child(render_local_only_icon(
            appearance,
            mouse_state.clone(),
            custom_tooltip,
        ));
    }
    sub_sub_header.finish()
}

pub fn render_custom_size_header(
    appearance: &Appearance,
    text_name: impl Into<Cow<'static, str>>,
    font_size: f32,
    color_override: Option<Fill>,
) -> Box<dyn Element> {
    Flex::row()
        .with_child(
            Container::new(
                Align::new(
                    Text::new_inline(text_name, appearance.ui_font_family(), font_size)
                        .with_style(Properties::default().weight(Weight::Bold))
                        .with_color(
                            color_override
                                .unwrap_or(appearance.theme().active_ui_text_color())
                                .into(),
                        )
                        .finish(),
                )
                .left()
                .finish(),
            )
            .with_padding_bottom(4.)
            .finish(),
        )
        .finish()
}

pub fn render_separator(appearance: &Appearance) -> Box<dyn Element> {
    Container::new(Empty::new().finish())
        .with_border(Border::bottom(2.).with_border_fill(appearance.theme().outline()))
        .with_margin_bottom(HEADER_PADDING)
        .finish()
}

pub fn render_full_pane_width_ai_button(
    text: &str,
    is_any_ai_enabled: bool,
    mouse_state: MouseStateHandle,
    action: AISettingsPageAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let (text_color, bg, icon_bg) = if is_any_ai_enabled {
        (
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
                .into(),
            internal_colors::neutral_3(appearance.theme()),
            appearance.theme().background(),
        )
    } else {
        (
            appearance.theme().disabled_ui_text_color().into(),
            internal_colors::neutral_2(appearance.theme()),
            appearance.theme().disabled_ui_text_color(),
        )
    };

    let mut button = Hoverable::new(mouse_state, |_| {
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(
                    Expanded::new(
                        1.,
                        appearance
                            .ui_builder()
                            .wrappable_text(text.to_string(), true)
                            .with_style(UiComponentStyles {
                                font_size: Some(CONTENT_FONT_SIZE),
                                font_color: Some(text_color),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    ConstrainedBox::new(
                        Icon::ChevronRight
                            .to_warpui_icon(appearance.theme().main_text_color(icon_bg))
                            .finish(),
                    )
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
                )
                .finish(),
        )
        .with_background(bg)
        .with_border(
            Border::new(1.).with_border_fill(internal_colors::neutral_4(appearance.theme())),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_horizontal_padding(16.)
        .with_vertical_padding(11.)
        .with_margin_bottom(12.)
        .finish()
    });

    if is_any_ai_enabled {
        button = button
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            })
            .with_cursor(Cursor::PointingHand);
    }

    button.finish()
}

#[derive(Default)]
pub struct AdditionalInfo<T> {
    pub mouse_state: MouseStateHandle,
    pub on_click_action: Option<T>,
    pub secondary_text: Option<String>,
    pub tooltip_override_text: Option<String>,
}

#[derive(Default)]
pub enum ToggleState {
    #[default]
    Enabled,
    Disabled,
}

impl From<bool> for ToggleState {
    fn from(value: bool) -> Self {
        if value {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

/// Whether to show an icon indicating a setting is not cloud-synced
#[derive(Default, Clone)]
pub enum LocalOnlyIconState {
    #[default]
    Hidden,
    Visible {
        mouse_state: MouseStateHandle,
        custom_tooltip: Option<String>,
    },
}

impl LocalOnlyIconState {
    /// Creates a `LocalOnlyIconState` for a given setting.
    ///
    /// This function determines whether to show an icon indicating that a setting
    /// is not cloud-synced based on the `SyncToCloud` value of the setting.
    ///
    /// # Arguments
    ///
    /// * `storage_key` - A string slice that holds the storage key for the setting.
    /// * `sync_to_cloud` - The `SyncToCloud` value for the setting.
    /// * `mouse_states` - A mutable reference to a `HashMap` storing `MouseStateHandle`s.
    ///
    /// # Returns
    ///
    /// Returns a `LocalOnlyIconState` enum variant:
    /// - `LocalOnlyIconState::Visible` with a `MouseStateHandle` if the setting is never synced to cloud.
    /// - `LocalOnlyIconState::Hidden` if the setting is synced to cloud.
    pub fn for_setting(
        storage_key: &str,
        sync_to_cloud: SyncToCloud,
        mouse_states: &mut HashMap<String, MouseStateHandle>,
        app: &AppContext,
    ) -> Self {
        if !*CloudPreferencesSettings::as_ref(app).settings_sync_enabled {
            // Only show the local-only icon if settings sync is enabled.
            return Self::Hidden;
        }

        match sync_to_cloud {
            SyncToCloud::Never => {
                let mouse_state = mouse_states
                    .entry(storage_key.to_string())
                    .or_default()
                    .clone();
                Self::Visible {
                    mouse_state,
                    custom_tooltip: None,
                }
            }
            _ => Self::Hidden,
        }
    }
}

pub fn render_info_icon<T: Clone + Action>(
    appearance: &Appearance,
    additional_info: AdditionalInfo<T>,
) -> Box<dyn Element> {
    let tooltip_text = additional_info
        .tooltip_override_text
        .unwrap_or("Click to learn more in docs".to_owned());
    let icon = Container::new(
        ConstrainedBox::new(
            Icon::Info
                .to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_width(13.)
        .with_height(13.)
        .finish(),
    )
    .finish();

    let mut info_button = Hoverable::new(additional_info.mouse_state.clone(), move |state| {
        let mut stack = Stack::new().with_child(icon);
        if state.is_hovered() {
            let tool_tip = ConstrainedBox::new(
                appearance
                    .ui_builder()
                    .tool_tip(tooltip_text)
                    .build()
                    .finish(),
            )
            .with_max_width(INFO_TOOLTIP_MAX_WIDTH)
            .finish();
            stack.add_positioned_child(
                tool_tip,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -3.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
        }
        stack.finish()
    })
    .with_cursor(Cursor::PointingHand);

    if let Some(on_click_action) = additional_info.on_click_action {
        info_button = info_button
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(on_click_action.clone()));
    }

    Container::new(Box::new(info_button))
        .with_margin_left(4.)
        .finish()
}

pub fn render_local_only_icon(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    custom_tooltip: Option<String>,
) -> Box<dyn Element> {
    let info_button = appearance
        .ui_builder()
        .local_only_icon_with_tooltip(
            13.,
            custom_tooltip.unwrap_or("This setting is not synced to your other devices".to_owned()),
            mouse_state.clone(),
        )
        .finish();

    Container::new(info_button).with_margin_left(4.).finish()
}

pub fn render_body_item_label<T: Clone + Action>(
    label_text: String,
    label_color_override: Option<Fill>,
    additional_info: Option<AdditionalInfo<T>>,
    local_only_icon_state: LocalOnlyIconState,
    toggle_state: ToggleState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_body_item_label_internal(
        label_text,
        None,
        label_color_override,
        additional_info,
        local_only_icon_state,
        toggle_state,
        appearance,
    )
}

pub fn render_body_item_label_with_icon<T: Clone + Action>(
    label_text: String,
    icon: Icon,
    label_color_override: Option<Fill>,
    additional_info: Option<AdditionalInfo<T>>,
    local_only_icon_state: LocalOnlyIconState,
    toggle_state: ToggleState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_body_item_label_internal(
        label_text,
        Some(icon),
        label_color_override,
        additional_info,
        local_only_icon_state,
        toggle_state,
        appearance,
    )
}

pub fn render_body_item_label_internal<T: Clone + Action>(
    label_text: String,
    label_icon: Option<Icon>,
    label_color_override: Option<Fill>,
    additional_info: Option<AdditionalInfo<T>>,
    local_only_icon_state: LocalOnlyIconState,
    toggle_state: ToggleState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut label = Flex::row();
    let label_color = match label_color_override {
        Some(color) => color,
        None => match toggle_state {
            ToggleState::Enabled => appearance.theme().active_ui_text_color(),
            ToggleState::Disabled => appearance.theme().disabled_ui_text_color(),
        },
    };
    let label_text = Text::new_inline(label_text, appearance.ui_font_family(), CONTENT_FONT_SIZE)
        .with_color(label_color.into());
    if let Some(icon) = label_icon {
        label.add_child(
            Container::new(
                ConstrainedBox::new(icon.to_warpui_icon(label_color).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );
    }
    label.add_child(label_text.finish());

    let label = label.finish();
    if let Some(additional_info) = additional_info {
        // Construct a child element for the secondary text, if necessary, before
        // `additional_info` gets moved into `render_info_icon()`.
        let secondary_text_child =
            if let Some(secondary_text) = additional_info.secondary_text.clone() {
                let warp_theme = appearance.theme();
                Some(
                    appearance
                        .ui_builder()
                        .span(secondary_text)
                        .with_style(UiComponentStyles {
                            font_color: Some(
                                warp_theme
                                    .sub_text_color(warp_theme.surface_2())
                                    .into_solid(),
                            ),
                            margin: Some(Coords {
                                left: 8.,
                                ..Default::default()
                            }),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
            } else {
                None
            };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label)
            .with_child(render_info_icon(appearance, additional_info));
        if let LocalOnlyIconState::Visible {
            mouse_state,
            custom_tooltip,
        } = local_only_icon_state
        {
            row.add_child(render_local_only_icon(
                appearance,
                mouse_state,
                custom_tooltip,
            ));
        }
        if let Some(child) = secondary_text_child {
            row.add_child(child);
        }
        row.finish()
    } else if let LocalOnlyIconState::Visible {
        mouse_state,
        custom_tooltip,
    } = local_only_icon_state
    {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label)
            .with_child(render_local_only_icon(
                appearance,
                mouse_state,
                custom_tooltip,
            ))
            .finish()
    } else {
        label
    }
}

pub fn render_page_title(text: &str, size: f32, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        Align::new(
            Text::new_inline(text.to_string(), appearance.ui_font_family(), size)
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish(),
        )
        .left()
        .finish(),
    )
    .with_margin_bottom(PAGE_TITLE_MARGIN_BOTTOM)
    .finish()
}

/// Renders a toggle with a label on the left and a toggle on the right,
/// including bottom padding.
pub fn render_body_item<T: Clone + Action>(
    label_text: String,
    additional_info: Option<AdditionalInfo<T>>,
    local_only_icon_state: LocalOnlyIconState,
    toggle_state: ToggleState,
    appearance: &Appearance,
    child_element: Box<dyn Element>,
    description_text: Option<String>,
) -> Box<dyn Element> {
    build_toggle_element(
        render_body_item_label(
            label_text,
            None,
            additional_info,
            local_only_icon_state,
            toggle_state,
            appearance,
        ),
        child_element,
        appearance,
        description_text,
    )
}

/// Builds a custom toggle with a label on the left and a toggle on the right.
pub fn build_toggle_element(
    name_element: Box<dyn Element>,
    toggle_element: Box<dyn Element>,
    appearance: &Appearance,
    description_text: Option<String>,
) -> Box<dyn Element> {
    let mut column = Flex::column();
    let header = Shrinkable::new(
        1.0,
        Container::new(Align::new(name_element).left().finish()).finish(),
    )
    .finish();
    let toggle = Container::new(toggle_element)
        .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
        .finish();

    let mut header_row = Container::new(
        Flex::row()
            .with_child(header)
            .with_child(toggle)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish(),
    );
    if description_text.is_none() {
        header_row = header_row.with_padding_bottom(HEADER_PADDING);
    }
    column.add_child(header_row.finish());
    if let Some(description_text) = description_text {
        let description = appearance
            .ui_builder()
            .paragraph(description_text)
            .with_style(UiComponentStyles {
                font_color: Some(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().surface_1(),
                )),
                font_size: Some(12.),
                margin: Some(Coords {
                    top: 4.,
                    bottom: 0.,
                    left: 0.,
                    right: 0.,
                }),
                ..Default::default()
            })
            .build()
            .finish();
        column.add_child(
            Container::new(description)
                .with_margin_right(100.)
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
        );
    }
    column.finish()
}

pub fn render_dropdown_item_label(
    label_text: String,
    secondary_text: Option<String>,
    local_only_icon_state: LocalOnlyIconState,
    color_override: Option<Fill>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let label = Text::new(label_text, appearance.ui_font_family(), CONTENT_FONT_SIZE)
        .with_color(
            color_override
                .unwrap_or(appearance.theme().active_ui_text_color())
                .into(),
        )
        .finish();
    let label = if let Some(secondary_text) = secondary_text {
        let warp_theme = appearance.theme();
        let secondary_text_child = appearance
            .ui_builder()
            .span(secondary_text)
            .with_style(UiComponentStyles {
                font_color: Some(
                    color_override
                        .unwrap_or(warp_theme.sub_text_color(warp_theme.surface_2()))
                        .into_solid(),
                ),
                margin: Some(Coords {
                    top: 4.,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .with_soft_wrap()
            .build()
            .finish();

        Flex::column()
            .with_child(label)
            .with_child(secondary_text_child)
            .finish()
    } else {
        label
    };

    if let LocalOnlyIconState::Visible {
        mouse_state,
        custom_tooltip,
    } = local_only_icon_state
    {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Shrinkable::new(1.0, label).finish())
            .with_child(render_local_only_icon(
                appearance,
                mouse_state,
                custom_tooltip,
            ))
            .finish()
    } else {
        label
    }
}

pub(crate) fn render_dropdown_item<T: Clone + Action>(
    appearance: &Appearance,
    label: &str,
    secondary_text: Option<&str>,
    dropdown_subtext: Option<Box<dyn Element>>,
    local_only_icon_state: LocalOnlyIconState,
    color_override: Option<Fill>,
    handle: &ViewHandle<Dropdown<T>>,
) -> Box<dyn Element> {
    let row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    let dropdown_item_label = Align::new(render_dropdown_item_label(
        label.to_string(),
        secondary_text.map(|secondary_text| secondary_text.to_string()),
        local_only_icon_state,
        color_override,
        appearance,
    ))
    .left()
    .finish();

    let mut dropdown = Flex::column().with_child(ChildView::new(handle).finish());
    if let Some(dropdown_subtext) = dropdown_subtext {
        dropdown.add_child(dropdown_subtext);
    }

    row.with_child(
        Shrinkable::new(
            1.0,
            Container::new(dropdown_item_label)
                .with_margin_bottom(4.)
                .with_padding_right(16.)
                .finish(),
        )
        .finish(),
    )
    .with_child(dropdown.finish())
    .finish()
}

pub(crate) fn render_settings_info_banner(
    text: &str,
    subtext: Option<&str>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let icon = Container::new(
        ConstrainedBox::new(
            Icon::AlertCircle
                .to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish(),
    )
    .with_margin_right(8.)
    .finish();

    let text = {
        let mut children = vec![Container::new(
            Text::new(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish(),
        )
        .finish()];

        if let Some(subtext) = subtext {
            children.push(
                Container::new(
                    Text::new(
                        subtext.to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size() - 1.,
                    )
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().background())
                            .into(),
                    )
                    .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );
        }

        Shrinkable::new(1.0, Flex::column().with_children(children).finish()).finish()
    };

    Container::new(
        Flex::row()
            .with_children(vec![icon, text])
            .with_main_axis_size(MainAxisSize::Max)
            .finish(),
    )
    .with_background_color(appearance.theme().accent_overlay().into())
    .with_uniform_padding(12.)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .finish()
}

const WORKSPACE_OVERRIDE_TOOLTIP_TEXT: &str =
    "This option is enforced by your organization's settings and cannot be customized.";

pub struct InputListItem<SettingsPageAction: Action + Clone> {
    pub item: String,
    pub mouse_state_handle: MouseStateHandle,
    pub on_remove_action: SettingsPageAction,
    pub is_disabled: bool,
    /// Must be pre-created (not inline during render) to preserve mouse tracking.
    pub tooltip_mouse_state: Option<MouseStateHandle>,
}
/// Renders a title, an input field to add new items and a list of already
/// added items.
///
/// TODO: standardize this and remove [`render_alternating_color_list`].
pub fn render_input_list<SettingsPageAction: Action + Clone>(
    title: Option<&str>,
    items: impl IntoIterator<Item = InputListItem<SettingsPageAction>>,
    handle: Option<&ViewHandle<SubmittableTextInput>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut column = Flex::column();

    if let Some(title) = title {
        column.add_child(
            appearance
                .ui_builder()
                .span(title.to_string())
                .with_style(UiComponentStyles {
                    font_size: Some(CONTENT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
    }

    if let Some(handle) = handle {
        column.add_child(ChildView::new(handle).finish());
    }

    let background = appearance.theme().surface_1();
    let peekable = items.into_iter().peekable();
    for item in peekable {
        let disabled = item.is_disabled;
        let row_element = render_alternating_color_list_item(
            background,
            item.item,
            item.mouse_state_handle,
            item.on_remove_action,
            disabled,
            appearance,
        );
        let row_element = if let Some(tooltip_mouse_state) = item.tooltip_mouse_state {
            render_workspace_override_row_tooltip(row_element, tooltip_mouse_state, appearance)
        } else {
            row_element
        };
        let container = Container::new(row_element).with_margin_bottom(4.);
        column.add_child(container.finish());
    }

    column.finish()
}

pub fn render_alternating_color_list<
    ListItem: Display,
    SettingsPageAction: Action + Clone,
    F: Fn(usize) -> SettingsPageAction,
>(
    body: &mut Flex,
    patterns: &[ListItem],
    mouse_states: &[MouseStateHandle],
    create_action: F,
    appearance: &Appearance,
) {
    debug_assert!(
        mouse_states.len() >= patterns.len(),
        "mouse_states length ({}) is less than patterns length ({})",
        mouse_states.len(),
        patterns.len()
    );
    for (i, pattern) in patterns.iter().enumerate() {
        let background = if i % 2 == 0 {
            internal_colors::fg_overlay_1(appearance.theme())
        } else {
            Fill::Solid(ColorU::transparent_black())
        };

        body.add_child(render_alternating_color_list_item::<SettingsPageAction>(
            background,
            pattern.to_string(),
            mouse_states[i].clone(),
            create_action(i),
            false,
            appearance,
        ));
    }
}

fn render_workspace_override_row_tooltip(
    child: Box<dyn Element>,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Hoverable::new(mouse_state, |state| {
        let mut stack = Stack::new().with_child(child);
        if state.is_hovered() {
            let tooltip = appearance
                .ui_builder()
                .tool_tip(WORKSPACE_OVERRIDE_TOOLTIP_TEXT.to_string())
                .build()
                .finish();
            stack.add_positioned_child(
                tooltip,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -4.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}

fn render_alternating_color_list_item<SettingsPageAction: Action + Clone>(
    background: impl Into<Fill>,
    item_label: String,
    mouse_state: MouseStateHandle,
    action: SettingsPageAction,
    disabled: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut remove_button = appearance
        .ui_builder()
        .close_button(ALTERNATING_LIST_CLOSE_BUTTON_DIAMETER, mouse_state);

    if disabled {
        remove_button = remove_button.disabled();
    }

    let mut remove_button = remove_button.build();
    if !disabled {
        remove_button =
            remove_button.on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));
    }
    let remove_button = remove_button.finish();

    let background = background.into();
    let font_color = if disabled {
        appearance.theme().disabled_text_color(background)
    } else {
        appearance.theme().foreground()
    };

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                Shrinkable::new(
                    1.,
                    Align::new(
                        appearance
                            .ui_builder()
                            .wrappable_text(item_label, true)
                            .with_style(UiComponentStyles {
                                font_color: Some(font_color.into_solid()),
                                font_family_id: Some(appearance.monospace_font_family()),
                                font_size: Some(appearance.ui_font_size()),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
                Container::new(remove_button)
                    .with_margin_left(ALTERNATING_LIST_ITEM_PADDING)
                    .finish(),
            ])
            .finish(),
    )
    .with_background(background)
    .with_uniform_padding(ALTERNATING_LIST_ITEM_PADDING)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    // The bottom has a bit of extra padding b/c lines of text have more space above the text
    // than below. This visually balances that to make it look vertically centered.
    .with_padding_bottom(ALTERNATING_LIST_ITEM_PADDING + 2.)
    .finish()
}

/// Adds a setting (e.g., "Background opacity") to the parent flex if it is supported on the current platform. Returns
/// true if the setting was added to the flex, false if not.
///
/// This is the default method to use when rendering a setting in the settings menu, across all pages
/// (Appearance, Features, etc).
pub fn add_setting<F>(
    parent_flex: &mut Flex,
    setting_model: &impl Setting,
    setting_element: F,
) -> bool
where
    F: FnOnce() -> Box<dyn Element>,
{
    if setting_model.is_supported_on_current_platform() {
        parent_flex.add_child(setting_element());
        true
    } else {
        false
    }
}

/// Structured contents of a settings tab page. This type breaks all the content into
/// [`SettingsWidget`]s.
pub(super) enum PageType<V: warpui::View> {
    /// A page where the contents cannot be separated for showing search results. If any part
    /// matches the search query, the whole page must show. The whole page is one big
    /// [`SettingsWidget`].
    ///
    /// The vertical and horizontal scroll states are optional to let Monolith pages
    /// handle and render their own scrollable elements.
    Monolith {
        widget: Box<dyn SettingsWidget<View = V>>,
        title: Option<&'static str>,
        filter: bool,
        vertical_scroll_state: Option<ClippedScrollStateHandle>,
        horizontal_scroll_state: Option<ClippedScrollStateHandle>,
        min_page_width: f32,
    },
    /// A page which is a series of [`SettingsWidget`]s that don't fall under sub-categories.
    Uncategorized {
        widgets: Vec<Box<dyn SettingsWidget<View = V>>>,
        title: Option<&'static str>,
        filter: Vec<usize>,
        vertical_scroll_state: ClippedScrollStateHandle,
        horizontal_scroll_state: ClippedScrollStateHandle,
        highlighted_widget_id: Option<&'static str>,
        min_page_width: f32,
    },
    /// A page which is a series of [`SettingsWidget`]s that fall under sub-categories.
    Categorized {
        categories: Vec<Category<V>>,
        title: Option<&'static str>,
        filter: Vec<Vec<usize>>,
        vertical_scroll_state: ClippedScrollStateHandle,
        horizontal_scroll_state: ClippedScrollStateHandle,
        highlighted_widget_id: Option<&'static str>,
        min_page_width: f32,
    },
}

/// Some settings pages break down into a collection of smaller widgets while others are
/// "monoliths". The way the matches are presented differs between them.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MatchData {
    /// The monoliths use the Uncounted variant to indicate that they match a search
    /// term. Alternatively, we may use this variant for non-monolithic pages if we shouldn't
    /// bother counting the number of matches, say if the search query has become empty.
    Uncounted(bool),
    /// Used for non-monolithic pages when we want to display a specific count for the number of
    /// matches to a search query.
    Countable(usize),
}

impl MatchData {
    pub(crate) fn is_truthy(&self) -> bool {
        match self {
            MatchData::Countable(n) => *n > 0,
            MatchData::Uncounted(flag) => *flag,
        }
    }
}

impl Display for MatchData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MatchData::Countable(n) => write!(f, " ({n})"),
            MatchData::Uncounted(_) => write!(f, ""),
        }
    }
}

impl From<bool> for MatchData {
    fn from(value: bool) -> Self {
        MatchData::Uncounted(value)
    }
}

impl From<usize> for MatchData {
    fn from(value: usize) -> Self {
        MatchData::Countable(value)
    }
}

impl<V: warpui::View> PageType<V> {
    /// A page where the contents cannot be separated for showing search results. If any part
    /// matches the search query, the whole page must show. The whole page is one big
    /// [`SettingsWidget`].
    pub(super) fn new_monolith(
        widget: impl SettingsWidget<View = V> + 'static,
        title: Option<&'static str>,
        is_dual_scrollable: bool,
    ) -> Self {
        let (vertical_scroll_state, horizontal_scroll_state) = if is_dual_scrollable {
            (
                Some(ClippedScrollStateHandle::default()),
                Some(ClippedScrollStateHandle::default()),
            )
        } else {
            (None, None)
        };

        Self::Monolith {
            filter: true,
            widget: Box::new(widget),
            title,
            vertical_scroll_state,
            horizontal_scroll_state,
            min_page_width: MIN_PAGE_WIDTH,
        }
    }

    /// A page which is a series of [`SettingsWidget`]s that don't fall under sub-categories.
    pub(super) fn new_uncategorized(
        widgets: Vec<Box<dyn SettingsWidget<View = V>>>,
        title: Option<&'static str>,
    ) -> Self {
        Self::Uncategorized {
            filter: widgets.iter().enumerate().map(|(i, _)| i).collect(),
            widgets,
            title,
            vertical_scroll_state: Default::default(),
            horizontal_scroll_state: Default::default(),
            highlighted_widget_id: Default::default(),
            min_page_width: MIN_PAGE_WIDTH,
        }
    }

    /// A page which is a series of [`SettingsWidget`]s that fall under sub-categories.
    pub(super) fn new_categorized(
        categories: Vec<Category<V>>,
        title: Option<&'static str>,
    ) -> Self {
        Self::Categorized {
            filter: categories
                .iter()
                .map(|category| {
                    category
                        .widgets
                        .iter()
                        .enumerate()
                        .map(|(i, _)| i)
                        .collect()
                })
                .collect(),
            categories,
            title,
            vertical_scroll_state: Default::default(),
            horizontal_scroll_state: Default::default(),
            highlighted_widget_id: Default::default(),
            min_page_width: MIN_PAGE_WIDTH,
        }
    }

    /// Apply the search query by matching against all the widgets and storing the results.
    /// Uses all-words matching: every word in the query must appear somewhere in the
    /// widget's search terms (but not necessarily contiguously).
    pub(super) fn update_filter(&mut self, query: &str, app: &AppContext) -> MatchData {
        /// Returns true if every whitespace-delimited word in `query` appears
        /// somewhere in `terms` (case-insensitive). An empty query matches everything.
        fn search_terms_match(terms: &str, query: &str) -> bool {
            if query.is_empty() {
                return true;
            }
            let terms_lower = terms.to_lowercase();
            query
                .to_lowercase()
                .split_whitespace()
                .all(|word| terms_lower.contains(word))
        }
        match self {
            Self::Monolith { widget, filter, .. } => {
                *filter =
                    widget.should_render(app) && search_terms_match(widget.search_terms(), query);
                (*filter).into()
            }
            Self::Uncategorized {
                widgets, filter, ..
            } => {
                *filter = widgets
                    .iter()
                    .enumerate()
                    .filter_map(|(i, widget)| {
                        (widget.should_render(app)
                            && search_terms_match(widget.search_terms(), query))
                        .then_some(i)
                    })
                    .collect();
                if query.is_empty() {
                    MatchData::Uncounted(true)
                } else {
                    filter.len().into()
                }
            }
            Self::Categorized {
                categories, filter, ..
            } => {
                *filter = categories
                    .iter()
                    .map(|category| {
                        category
                            .widgets
                            .iter()
                            .enumerate()
                            .filter_map(|(i, widget)| {
                                (widget.should_render(app)
                                    && search_terms_match(widget.search_terms(), query))
                                .then_some(i)
                            })
                            .collect_vec()
                    })
                    .collect();
                if query.is_empty() {
                    MatchData::Uncounted(true)
                } else {
                    filter
                        .iter()
                        .map(|indices| indices.len())
                        .sum::<usize>()
                        .into()
                }
            }
        }
    }

    pub fn scroll_to_widget(&mut self, widget_id: &'static str) {
        match self {
            Self::Monolith { .. } => {}
            Self::Uncategorized {
                vertical_scroll_state: scrollable_state,
                highlighted_widget_id,
                ..
            }
            | Self::Categorized {
                vertical_scroll_state: scrollable_state,
                highlighted_widget_id,
                ..
            } => {
                *highlighted_widget_id = Some(widget_id);
                scrollable_state.scroll_to_position(ScrollTarget {
                    position_id: widget_id.to_string(),
                    mode: ScrollToPositionMode::FullyIntoView,
                })
            }
        }
    }

    pub fn clear_highlighted_widget(&mut self) {
        match self {
            Self::Monolith { .. } => {}
            Self::Uncategorized {
                highlighted_widget_id,
                ..
            }
            | Self::Categorized {
                highlighted_widget_id,
                ..
            } => {
                *highlighted_widget_id = None;
            }
        }
    }

    /// Set the minimum page width for narrow panes.
    pub fn set_min_page_width(&mut self, width: f32) {
        match self {
            Self::Monolith { min_page_width, .. }
            | Self::Uncategorized { min_page_width, .. }
            | Self::Categorized { min_page_width, .. } => {
                *min_page_width = width;
            }
        }
    }

    /// Apply the filter we saved from the last matching of the search query to return only the
    /// relevant results.
    pub(super) fn get_filtered(&self) -> FilteredPageType<'_, V> {
        match self {
            Self::Monolith {
                widget,
                filter,
                title,
                vertical_scroll_state,
                horizontal_scroll_state,
                ..
            } => FilteredPageType::Monolith {
                widget: filter.then_some(widget.as_ref()),
                title: *title,
                vertical_scroll_state: vertical_scroll_state.clone(),
                horizontal_scroll_state: horizontal_scroll_state.clone(),
            },
            Self::Uncategorized {
                widgets,
                filter,
                title,
                vertical_scroll_state,
                horizontal_scroll_state,
                highlighted_widget_id,
                ..
            } => FilteredPageType::Uncategorized {
                widgets: filter.iter().map(|i| widgets[*i].as_ref()).collect(),
                title: *title,
                vertical_scroll_state: vertical_scroll_state.clone(),
                horizontal_scroll_state: horizontal_scroll_state.clone(),
                highlighted_widget_id: *highlighted_widget_id,
            },
            Self::Categorized {
                categories,
                filter,
                title,
                vertical_scroll_state,
                horizontal_scroll_state,
                highlighted_widget_id,
                ..
            } => FilteredPageType::Categorized {
                categories: filter
                    .iter()
                    .enumerate()
                    .filter(|(_, indices)| !indices.is_empty())
                    .map(|(i, indices)| {
                        let category = &categories[i];
                        FilteredCategory {
                            title: category.title,
                            subtitle: category.subtitle,
                            widgets: indices
                                .iter()
                                .map(|i| category.widgets[*i].as_ref())
                                .collect(),
                        }
                    })
                    .collect(),
                title: *title,
                vertical_scroll_state: vertical_scroll_state.clone(),
                horizontal_scroll_state: horizontal_scroll_state.clone(),
                highlighted_widget_id: *highlighted_widget_id,
            },
        }
    }

    fn get_scroll_states(
        &self,
    ) -> (
        Option<ClippedScrollStateHandle>,
        Option<ClippedScrollStateHandle>,
    ) {
        match self.get_filtered() {
            FilteredPageType::Monolith {
                vertical_scroll_state,
                horizontal_scroll_state,
                ..
            } => (vertical_scroll_state, horizontal_scroll_state),
            FilteredPageType::Uncategorized {
                vertical_scroll_state,
                horizontal_scroll_state,
                ..
            } => (Some(vertical_scroll_state), Some(horizontal_scroll_state)),
            FilteredPageType::Categorized {
                vertical_scroll_state,
                horizontal_scroll_state,
                ..
            } => (Some(vertical_scroll_state), Some(horizontal_scroll_state)),
        }
    }

    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    pub fn scroll_by(&self, delta: Pixels) {
        match self {
            PageType::Monolith {
                vertical_scroll_state: Some(scrollable_state),
                ..
            }
            | PageType::Uncategorized {
                vertical_scroll_state: scrollable_state,
                ..
            }
            | PageType::Categorized {
                vertical_scroll_state: scrollable_state,
                ..
            } => scrollable_state.scroll_by(delta),
            _ => {}
        }
    }

    pub(super) fn render_page(&self, view: &V, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let page = match self.get_filtered() {
            FilteredPageType::Monolith { widget, title, .. } => {
                let mut page = Empty::new().finish();
                if let Some(widget) = widget {
                    if widget.should_render(app) {
                        if let Some(title) = title {
                            let col = Flex::column()
                                .with_child(render_page_title(title, HEADER_FONT_SIZE, appearance))
                                .with_child(widget.render_widget(view, false, appearance, app));
                            page = col.finish();
                        } else {
                            page = widget.render_widget(view, false, appearance, app);
                        }
                    }
                }
                page
            }
            FilteredPageType::Uncategorized {
                widgets,
                title,
                highlighted_widget_id,
                ..
            } => {
                let mut page = Flex::column();
                if let Some(title) = title {
                    page.add_child(render_page_title(title, HEADER_FONT_SIZE, appearance));
                }
                for widget in widgets {
                    let highlighted =
                        highlighted_widget_id.is_some_and(|id| id == widget.widget_id());
                    if widget.should_render(app) {
                        page.add_child(widget.render_widget(view, highlighted, appearance, app));
                    }
                }
                page.finish()
            }
            FilteredPageType::Categorized {
                categories,
                title,
                highlighted_widget_id,
                ..
            } => {
                let mut page = Flex::column();
                if let Some(title) = title {
                    page.add_child(render_page_title(title, HEADER_FONT_SIZE, appearance));
                }
                let num_categories = categories.len();
                for (i, category) in categories.into_iter().enumerate() {
                    if !category.title.is_empty() {
                        if let Some(subtitle) = category.subtitle {
                            page.add_child(render_sub_header_with_description(
                                appearance,
                                category.title,
                                subtitle,
                            ));
                        } else {
                            page.add_child(render_sub_header(appearance, category.title, None));
                        }
                    }
                    for widget in &category.widgets {
                        let highlighted =
                            highlighted_widget_id.is_some_and(|id| id == widget.widget_id());
                        if widget.should_render(app) {
                            page.add_child(widget.render_widget(
                                view,
                                highlighted,
                                appearance,
                                app,
                            ));
                        }
                    }
                    if i < num_categories - 1 {
                        page.add_child(render_separator(appearance));
                    }
                }
                page.finish()
            }
        };

        Container::new(
            Align::new(
                ConstrainedBox::new(page)
                    .with_max_width(MAX_PAGE_WIDTH)
                    .finish(),
            )
            .top_center()
            .finish(),
        )
        .with_uniform_padding(PAGE_PADDING)
        .finish()
    }

    fn wrap_dual_scrollable(
        &self,
        view: &V,
        horizontal_scroll_state: ClippedScrollStateHandle,
        vertical_scroll_state: ClippedScrollStateHandle,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Get the minimum page width from the PageType configuration
        let min_width = match self {
            Self::Monolith { min_page_width, .. }
            | Self::Uncategorized { min_page_width, .. }
            | Self::Categorized { min_page_width, .. } => *min_page_width,
        };

        // Use SizeConstraintSwitch to add horizontal scrolling only when width < min_width
        let switch = SizeConstraintSwitch::new(
            NewScrollable::vertical(
                SingleAxisConfig::Clipped {
                    handle: vertical_scroll_state.clone(),
                    child: Align::new(self.render_page(view, app))
                        .top_center()
                        .finish(),
                },
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                warpui::elements::Fill::None,
            )
            .finish(),
            vec![(
                SizeConstraintCondition::WidthLessThan(min_width),
                NewScrollable::horizontal_and_vertical(
                    DualAxisConfig::Clipped {
                        horizontal: ClippedAxisConfiguration {
                            handle: horizontal_scroll_state,
                            max_size: None,
                            stretch_child: true,
                        },
                        vertical: ClippedAxisConfiguration {
                            handle: vertical_scroll_state,
                            max_size: None,
                            stretch_child: false,
                        },
                        child: Align::new(
                            ConstrainedBox::new(self.render_page(view, app))
                                .with_max_width(min_width)
                                .finish(),
                        )
                        .top_center()
                        .finish(),
                    },
                    theme.nonactive_ui_detail().into(),
                    theme.active_ui_detail().into(),
                    warpui::elements::Fill::None,
                )
                .finish(),
            )],
        )
        .finish();

        Flex::column()
            .with_child(Expanded::new(1., switch).finish())
            .finish()
    }

    pub fn render(&self, view: &V, app: &AppContext) -> Box<dyn Element> {
        if let (Some(vertical_scroll_state), Some(horizontal_scroll_state)) =
            self.get_scroll_states()
        {
            self.wrap_dual_scrollable(view, horizontal_scroll_state, vertical_scroll_state, app)
        } else {
            self.render_page(view, app)
        }
    }
}

/// The results from a [`PageType`] with only matching [`SettingsWidget`]s.
pub(super) enum FilteredPageType<'a, V: warpui::View> {
    Monolith {
        widget: Option<&'a dyn SettingsWidget<View = V>>,
        title: Option<&'static str>,
        vertical_scroll_state: Option<ClippedScrollStateHandle>,
        horizontal_scroll_state: Option<ClippedScrollStateHandle>,
    },
    Uncategorized {
        widgets: Vec<&'a dyn SettingsWidget<View = V>>,
        title: Option<&'static str>,
        vertical_scroll_state: ClippedScrollStateHandle,
        horizontal_scroll_state: ClippedScrollStateHandle,
        highlighted_widget_id: Option<&'static str>,
    },
    Categorized {
        categories: Vec<FilteredCategory<'a, V>>,
        title: Option<&'static str>,
        vertical_scroll_state: ClippedScrollStateHandle,
        horizontal_scroll_state: ClippedScrollStateHandle,
        highlighted_widget_id: Option<&'static str>,
    },
}

/// A grouping of related [`SettingsWidget`]s that fall under the same sub-header.
pub(super) struct Category<V: warpui::View> {
    title: &'static str,
    subtitle: Option<&'static str>,
    widgets: Vec<Box<dyn SettingsWidget<View = V>>>,
}

impl<V: warpui::View> Category<V> {
    pub(super) fn new(
        title: &'static str,
        widgets: Vec<Box<dyn SettingsWidget<View = V>>>,
    ) -> Self {
        Self {
            title,
            subtitle: None,
            widgets,
        }
    }

    pub(super) fn with_subtitle(mut self, subtitle: &'static str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }
}

/// A [`Category`] with only the results which match a search query.
pub(super) struct FilteredCategory<'a, V: warpui::View> {
    pub(super) title: &'static str,
    pub(super) subtitle: Option<&'static str>,
    pub(super) widgets: Vec<&'a dyn SettingsWidget<View = V>>,
}

/// Widgets are pieces of renderable settings modal content which can be associated with search
/// content to match against.
pub(super) trait SettingsWidget {
    /// Which View (settings page) this widget belongs to.
    type View: warpui::View;

    fn static_widget_id() -> &'static str
    where
        Self: Sized,
    {
        std::any::type_name::<Self>()
    }

    fn widget_id(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// The terms to match search queries against.
    fn search_terms(&self) -> &str;

    fn should_render(&self, _app: &AppContext) -> bool {
        true
    }

    fn render_widget(
        &self,
        view: &Self::View,
        highlighted: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut content = self.render(view, appearance, app);
        if highlighted {
            content = Container::new(content)
                .with_border(Border::all(1.).with_border_fill(appearance.theme().accent()))
                .with_background(internal_colors::accent_overlay_1(appearance.theme()))
                .with_horizontal_padding(8.)
                .finish()
        }
        SavePosition::new(content, self.widget_id()).finish()
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element>;
}

/// Builds a standardized button for resetting a setting to its default value.
/// Callers should add an `on_click` handler and add the button to the UI below
/// the setting.
pub(super) fn build_reset_button(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    changed_from_default: bool,
) -> Button {
    let theme = appearance.theme();
    appearance
        .ui_builder()
        .reset_button(
            ButtonVariant::Text,
            mouse_state,
            changed_from_default,
            theme.disabled_text_color(theme.background()).into(),
        )
        .with_style(UiComponentStyles {
            padding: Some(Coords::default().bottom(HEADER_PADDING).top(5.)),
            font_size: Some(appearance.ui_font_size() * 0.8),
            ..Default::default()
        })
        .with_text_label("Reset to default".to_owned())
}
