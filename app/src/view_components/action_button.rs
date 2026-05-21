use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::{borrow::Cow, sync::Arc};
use warp_core::ui::{
    appearance::Appearance,
    color::{coloru_with_opacity, contrast::MinimumAllowedContrast, ContrastingColor},
    theme::{color::internal_colors, AnsiColorIdentifier, Fill},
};
use warpui::{elements::MainAxisAlignment, Gradient};
use warpui::{elements::MainAxisSize, text_layout::ClipConfig};
use warpui::{
    elements::{
        Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
        Hoverable, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor, ParentElement as _,
        ParentOffsetBounds, Radius, Stack, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
    },
    fonts::{Properties, Weight},
    keymap::Keystroke,
    platform::Cursor,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, BlurContext, Element, Entity, EventContext, FocusContext, SingletonEntity as _,
    TypedActionView, View, ViewContext,
};

use crate::{
    settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier},
    terminal::input::{MenuPositioning, MenuPositioningProvider},
    ui_components::icons::Icon,
    util::bindings::keybinding_name_to_keystroke,
};

/// Maximum width of a tooltip before it soft-wraps.
const TOOLTIP_MAX_WIDTH: f32 = 300.;

/// A consistent Button component.
///
/// This corresponds to the Figma [`button` component](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=GRYXipD0INVmDupA-0).
/// It's named `ActionButton` to not conflict with the existing `Button` `UiComponent`.
pub struct ActionButton {
    /// If `true`, this button is active, so we reuse the hover styles.
    active: bool,
    /// If `true`, this button is focused, so we reuse the hover styles.
    focused: bool,
    /// If `true`, this button is disabled, so we use the disabled theme.
    disabled: bool,
    /// An icon to show to the left of the label.
    icon: Option<Icon>,
    /// Optional override for icon color derived from the current theme's ANSI palette.
    icon_ansi_color: Option<AnsiColorIdentifier>,
    /// The text of this button.
    label: Cow<'static, str>,
    /// An optional tooltip to show on hover.
    tooltip: Option<String>,
    tooltip_sublabel: Option<String>,
    /// Maximum height of a tooltip before it truncates.
    tooltip_max_height: Option<f32>,
    size: ButtonSize,
    /// If set, the button text will be clipped to this width.
    max_label_width: Option<f32>,
    /// If set, applies a maximum width to the button.
    width: Option<f32>,
    /// Optional custom height that overrides the default height from ButtonSize.
    custom_height: Option<f32>,
    callout: Option<Callout>,
    theme: Arc<dyn ActionButtonTheme>,

    /// If `true`, this button triggers a dropdown menu, so we add a chevron icon to the right.
    has_menu: bool,

    click_handler: Option<Arc<ClickHandler>>,

    cached_keystroke: Option<Keystroke>,
    mouse_state_handle: MouseStateHandle,
    tooltip_positioning_provider: Option<Arc<dyn MenuPositioningProvider>>,
    /// Controls how tooltips are aligned
    tooltip_alignment: TooltipAlignment,
    /// Custom theme to use when disabled, if None uses DisabledTheme
    disabled_theme: Option<Arc<dyn ActionButtonTheme>>,

    /// If true, expands the internal row to max width to center contents without
    /// requiring an explicit width. The actual button width remains governed by the parent.
    full_width: bool,

    /// If set, the button is joined with another element on that side.
    /// Corner radius and border is removed on the side that is joined.
    adjoined_side: Option<AdjoinedSide>,

    /// If true, renders the keybinding as plain text without individual key boxes.
    compact_keybinding: bool,

    /// If true, renders the keybinding before the label (but after the icon).
    keybinding_before_label: bool,
}

pub type ClickHandler = Box<dyn Fn(&mut EventContext) + 'static>;

/// Theming delegate for a button.
pub trait ActionButtonTheme {
    /// The background fill for the button.
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill>;

    /// The color to use for text and icons, given the current background color.
    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU;

    /// The color to use for the border on the adjoined side.
    /// Will take precedence over the border color.
    fn adjoined_side_border(&self, _appearance: &Appearance) -> Option<ColorU> {
        None
    }

    fn border(&self, _appearance: &Appearance) -> Option<ColorU> {
        None
    }

    fn border_gradient(&self, _appearance: &Appearance) -> Option<(Vector2F, Vector2F, Gradient)> {
        None
    }

    fn keyboard_shortcut_border(
        &self,
        _text_color: ColorU,
        _appearance: &Appearance,
    ) -> Option<ColorU> {
        None
    }

    fn keyboard_shortcut_background(&self, _appearance: &Appearance) -> Option<ColorU> {
        None
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        false
    }

    fn font_properties(&self) -> Option<Properties> {
        None
    }
}

/// Alignment options for tooltips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TooltipAlignment {
    Left,
    Center,
    /// Right alignment is the default for historical reasons.
    #[default]
    Right,
}

/// A special callout that may be attached to buttons, usually for announcing new features.
pub enum Callout {
    /// This button is for a beta feature.
    Beta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdjoinedSide {
    Left,
    Right,
}

/// The [`ButtonSize`] enum constants aren't named after traditional sizes because there's no simple
/// linear ordering of the sizes. Instead, each [`ButtonSize`] is named after its most prominent
/// use case within the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonSize {
    #[default]
    Default,
    Small,
    XSmall,
    InlineActionHeader,
    InputPrompt,
    /// Sizing for buttons at the bottom of the UDI.
    UDIButton,
    /// Sizing for prompt chips in the UDI.
    UDIPromptChip,
    /// Sizing for buttons in the AgentView input, e.g. when `FeatureFlag::AgentView` is enabled.
    AgentInputButton,
}

/// Source for the keystrokes associated with an [`ActionButton`].
#[derive(Clone)]
pub enum KeystrokeSource {
    /// A fixed keybinding.
    Fixed(Keystroke),
    /// A dynamic/editable binding, referenced by name.
    Binding(&'static str),
}

impl KeystrokeSource {
    pub fn displayed(&self, app: &AppContext) -> Option<String> {
        match self {
            KeystrokeSource::Fixed(keybinding) => Some(keybinding.displayed()),
            KeystrokeSource::Binding(name) => {
                keybinding_name_to_keystroke(name, app).map(|keybinding| keybinding.displayed())
            }
        }
    }
}

impl ActionButton {
    pub fn new(
        label: impl Into<Cow<'static, str>>,
        theme: impl ActionButtonTheme + 'static,
    ) -> Self {
        Self::new_with_boxed_theme(label, Arc::new(theme))
    }

    pub fn new_with_boxed_theme(
        label: impl Into<Cow<'static, str>>,
        theme: Arc<dyn ActionButtonTheme>,
    ) -> Self {
        Self {
            active: false,
            focused: false,
            disabled: false,
            icon: None,
            icon_ansi_color: None,
            label: label.into(),
            tooltip: None,
            tooltip_sublabel: None,
            tooltip_max_height: None,
            has_menu: false,
            size: Default::default(),
            max_label_width: None,
            width: None,
            custom_height: None,
            theme,
            click_handler: None,
            cached_keystroke: None,
            mouse_state_handle: Default::default(),
            callout: None,
            tooltip_positioning_provider: None,
            tooltip_alignment: TooltipAlignment::default(),
            disabled_theme: None,
            full_width: false,
            adjoined_side: None,
            compact_keybinding: false,
            keybinding_before_label: false,
        }
    }

    // `with_*` methods are for chained configuration when first creating an ActionButton.
    // They consume `self`, and can only be called before the ActionButton view has been added to
    // the UI framework (thus, no ctx.notify).

    /// Set the icon shown to the left of this button.
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set an icon color derived from the current theme's ANSI palette.
    pub fn with_icon_ansi_color(mut self, icon_ansi_color: AnsiColorIdentifier) -> Self {
        self.icon_ansi_color = Some(icon_ansi_color);
        self
    }

    /// Set the tooltip text shown on hover.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn with_tooltip_sublabel(mut self, tooltip_sublabel: impl Into<String>) -> Self {
        self.tooltip_sublabel = Some(tooltip_sublabel.into());
        self
    }

    pub fn with_tooltip_max_height(mut self, tooltip_max_height: f32) -> Self {
        self.tooltip_max_height = Some(tooltip_max_height);
        self
    }

    /// Set the size class of the button.
    pub fn with_size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Set the keybinding associated with this button's action. This may only be called once.
    pub fn with_keybinding(
        mut self,
        keybinding: KeystrokeSource,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        self.setup_keybinding(keybinding, ctx);
        self
    }

    pub fn with_adjoined_side(mut self, adjoined_side: AdjoinedSide) -> Self {
        self.adjoined_side = Some(adjoined_side);
        self
    }

    /// Renders the keybinding as plain text without individual key boxes.
    pub fn with_compact_keybinding(mut self, compact: bool) -> Self {
        self.compact_keybinding = compact;
        self
    }

    /// Renders the keybinding before the label (but after the icon).
    pub fn with_keybinding_before_label(mut self, before_label: bool) -> Self {
        self.keybinding_before_label = before_label;
        self
    }

    pub fn set_adjoined_side(&mut self, adjoined_side: AdjoinedSide, ctx: &mut ViewContext<Self>) {
        self.adjoined_side = Some(adjoined_side);
        ctx.notify();
    }

    pub fn clear_adjoined_side(&mut self, ctx: &mut ViewContext<Self>) {
        self.adjoined_side = None;
        ctx.notify();
    }

    pub fn with_menu(mut self, has_menu: bool) -> Self {
        self.has_menu = has_menu;
        self
    }
    pub fn set_has_menu(&mut self, has_menu: bool, ctx: &mut ViewContext<Self>) {
        self.has_menu = has_menu;
        ctx.notify();
    }

    #[allow(dead_code)]
    pub fn with_callout(mut self, callout: Callout) -> Self {
        self.callout = Some(callout);
        self
    }

    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: 'static + Fn(&mut EventContext),
    {
        self.click_handler = Some(Arc::new(Box::new(handler)));
        self
    }

    pub fn set_on_click<F>(&mut self, handler: F, ctx: &mut ViewContext<Self>)
    where
        F: 'static + Fn(&mut EventContext),
    {
        self.click_handler = Some(Arc::new(Box::new(handler)));
        ctx.notify();
    }

    /// Sets a provider to determine how tooltips should be positioned.
    pub fn with_tooltip_positioning_provider(
        mut self,
        provider: Arc<dyn MenuPositioningProvider>,
    ) -> Self {
        self.tooltip_positioning_provider = Some(provider);
        self
    }

    /// Configure tooltip alignment. If not specified, defaults to Right for historical reasons.
    pub fn with_tooltip_alignment(mut self, alignment: TooltipAlignment) -> Self {
        self.tooltip_alignment = alignment;
        self
    }

    pub fn with_max_label_width(mut self, max_label_width: f32) -> Self {
        self.max_label_width = Some(max_label_width);
        self
    }

    // TODO: Remove when we have use cases for with_height outside find bar (not compiled for WASM)
    #[allow(dead_code)]
    // TODO(varoon): If the total width of child elements exceeds the button width, layout issues can occur. Fix this in a follow-on PR.
    // When using this API, ensure the width is large enough to accommodate all child elements with some buffer.
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Expand the internal row to the available width so contents are centered.
    /// The button still respects parent width constraints; no explicit width is set.
    pub fn with_full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    // TODO: Remove when we have use cases for with_height outside find bar (not compiled for WASM)
    #[allow(dead_code)]
    pub fn with_height(mut self, height: f32) -> Self {
        self.custom_height = Some(height);
        self
    }

    /// Set a custom theme to use when the button is disabled
    pub fn with_disabled_theme(mut self, theme: impl ActionButtonTheme + 'static) -> Self {
        self.disabled_theme = Some(Arc::new(theme));
        self
    }

    pub fn set_active(&mut self, active: bool, ctx: &mut ViewContext<Self>) {
        self.active = active;
        ctx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, ctx: &mut ViewContext<Self>) {
        self.disabled = disabled;
        ctx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns the height of the button.
    pub fn height(&self, app: &AppContext) -> f32 {
        let appearance = Appearance::as_ref(app);
        self.custom_height
            .unwrap_or_else(|| self.size.button_height(appearance, app))
    }

    pub fn set_icon(&mut self, icon: Option<Icon>, ctx: &mut ViewContext<Self>) {
        self.icon = icon;
        ctx.notify();
    }

    pub fn set_label(&mut self, label: impl Into<Cow<'static, str>>, ctx: &mut ViewContext<Self>) {
        self.label = label.into();
        ctx.notify();
    }

    pub fn set_tooltip(&mut self, tooltip: Option<impl Into<String>>, ctx: &mut ViewContext<Self>) {
        self.tooltip = tooltip.map(|t| t.into());
        ctx.notify();
    }

    pub fn clear_tooltip(&mut self, ctx: &mut ViewContext<Self>) {
        self.tooltip = None;
        ctx.notify();
    }

    pub fn set_tooltip_sublabel(
        &mut self,
        tooltip_sublabel: Option<impl Into<String>>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.tooltip_sublabel = tooltip_sublabel.map(|t| t.into());
        ctx.notify();
    }

    pub fn set_theme(
        &mut self,
        theme: impl ActionButtonTheme + 'static,
        ctx: &mut ViewContext<Self>,
    ) {
        self.theme = Arc::new(theme);
        ctx.notify();
    }

    pub fn set_disabled_theme(
        &mut self,
        disabled_theme: impl ActionButtonTheme + 'static,
        ctx: &mut ViewContext<Self>,
    ) {
        self.disabled_theme = Some(Arc::new(disabled_theme));
        ctx.notify();
    }

    /// Change the keybinding associated with this button.
    pub fn set_keybinding(&mut self, source: Option<KeystrokeSource>, ctx: &mut ViewContext<Self>) {
        // Remove any subscription from a previous keybinding.
        ctx.unsubscribe_to_model(&KeybindingChangedNotifier::handle(ctx));
        match source {
            Some(source) => self.setup_keybinding(source, ctx),
            None => {
                self.cached_keystroke = None;
            }
        }
        ctx.notify();
    }

    pub fn set_callout(&mut self, callout: Option<Callout>, ctx: &mut ViewContext<Self>) {
        self.callout = callout;
        ctx.notify();
    }

    /// Set up a new keybinding associated with this button. The caller is responsible for resetting
    /// any previous keybinding state.
    fn setup_keybinding(&mut self, source: KeystrokeSource, ctx: &mut ViewContext<Self>) {
        match source {
            KeystrokeSource::Fixed(keybinding) => {
                self.cached_keystroke = Some(keybinding);
            }
            KeystrokeSource::Binding(name) => {
                self.cached_keystroke = keybinding_name_to_keystroke(name, ctx);
                ctx.subscribe_to_model(
                    &KeybindingChangedNotifier::handle(ctx),
                    move |me, _, event, ctx| {
                        let KeybindingChangedEvent::BindingChanged {
                            binding_name,
                            new_trigger,
                        } = event;
                        if binding_name == name {
                            me.cached_keystroke = new_trigger.clone();
                            ctx.notify();
                        }
                    },
                );
            }
        }
    }

    /// Returns true if the button only displays an icon (no label, no keybinding).
    fn is_icon_only(&self) -> bool {
        self.icon.is_some() && self.label.is_empty() && self.cached_keystroke.is_none()
    }

    fn maybe_render_callout(
        &self,
        has_preceding_element: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        match self.callout.as_ref()? {
            Callout::Beta => {
                // The beta callout should overall be the same height as an icon, so we set the font size accordingly.
                let padding = self.size.callout_padding();
                let overall_height = self.size.icon_size(appearance, app);

                Some(
                    Container::new(
                        Text::new_inline(
                            "Beta",
                            appearance.ui_font_family(),
                            overall_height - padding.top() - padding.bottom(),
                        )
                        .with_color(appearance.theme().background().into_solid())
                        .finish(),
                    )
                    .with_padding(padding)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                    .with_background_color(
                        appearance.theme().terminal_colors().bright.magenta.into(),
                    )
                    .with_margin_left(if has_preceding_element { 4. } else { 0. })
                    .finish(),
                )
            }
        }
    }

    fn maybe_render_tooltip(&self, appearance: &Appearance, app: &AppContext, stack: &mut Stack) {
        let Some(tooltip) = self.tooltip.clone() else {
            return;
        };

        let tooltip_element = if let Some(tooltip_sublabel) = self.tooltip_sublabel.clone() {
            appearance
                .ui_builder()
                .tool_tip_with_sublabel(tooltip, tooltip_sublabel)
                .build()
        } else {
            appearance.ui_builder().tool_tip(tooltip).build()
        };
        let mut tooltip_box =
            ConstrainedBox::new(tooltip_element.finish()).with_max_width(TOOLTIP_MAX_WIDTH);

        if let Some(tooltip_max_height) = self.tooltip_max_height {
            tooltip_box = tooltip_box.with_max_height(tooltip_max_height);
        }

        let tooltip_element = tooltip_box.finish();

        // In the input, buttons have a negative margin to ensure they stay within size
        // constraints of the prompt. We have to propagate this to the tooltip to ensure
        // it's still positioned correctly.
        let y_offset =
            self.size.tooltip_offset() + self.size.negative_vertical_margin().unwrap_or_default();

        let positioning = match self
            .tooltip_positioning_provider
            .as_ref()
            .map(|provider| provider.menu_position(app))
            .unwrap_or_default()
        {
            MenuPositioning::AboveInputBox => match self.tooltip_alignment {
                TooltipAlignment::Left => OffsetPositioning::offset_from_parent(
                    vec2f(0., y_offset),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
                TooltipAlignment::Center => OffsetPositioning::offset_from_parent(
                    vec2f(0., y_offset),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                ),
                TooltipAlignment::Right => OffsetPositioning::offset_from_parent(
                    vec2f(0., y_offset),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::BottomRight,
                ),
            },
            MenuPositioning::BelowInputBox => match self.tooltip_alignment {
                TooltipAlignment::Left => OffsetPositioning::offset_from_parent(
                    vec2f(0., -y_offset),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
                TooltipAlignment::Center => OffsetPositioning::offset_from_parent(
                    vec2f(0., -y_offset),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                ),
                TooltipAlignment::Right => OffsetPositioning::offset_from_parent(
                    vec2f(0., -y_offset),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            },
        };

        stack.add_positioned_overlay_child(tooltip_element, positioning);
    }
}

impl Entity for ActionButton {
    type Event = ();
}

impl View for ActionButton {
    fn ui_name() -> &'static str {
        "ActionButton"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        static DEFAULT_DISABLED_THEME: DisabledTheme = DisabledTheme;
        let theme: &dyn ActionButtonTheme = if self.disabled {
            self.disabled_theme
                .as_deref()
                .unwrap_or(&DEFAULT_DISABLED_THEME)
        } else {
            &*self.theme
        };
        let mut hoverable = Hoverable::new(self.mouse_state_handle.clone(), |mouse_state| {
            let show_hover =
                !self.disabled && (mouse_state.is_hovered() || self.active || self.focused);
            let background = theme.background(show_hover, appearance);
            let mut text_color = theme.text_color(show_hover, background, appearance);

            if !theme.should_opt_out_of_contrast_adjustment() {
                // Ensures that the action button text is always rendered with sufficient contrast.
                // For hovered states that use a semi-transparent background, we apply the contrast adjustment using the base background.
                if let Some(base_bg) = theme.background(false, appearance) {
                    text_color = text_color
                        .on_background(base_bg.into_solid(), MinimumAllowedContrast::Text);
                }
            }

            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Center);
            // Set once an element has been added to the row, to simplify spacing with conditional elements.
            let mut has_preceding_element = false;

            // Helper closure to render the keybinding
            let render_keybinding = |row: &mut Flex, has_preceding: bool| {
                if let Some(shortcut) = self.cached_keystroke.as_ref() {
                    let mut margin = Coords::default();
                    if has_preceding {
                        margin.left = self.size.keystroke_left_spacing();
                    }

                    if self.compact_keybinding {
                        // Compact mode: render as plain text without boxes
                        let shortcut_color = appearance
                            .theme()
                            .disabled_text_color(appearance.theme().surface_1())
                            .into_solid();
                        let shortcut_styles = UiComponentStyles {
                            margin: Some(margin),
                            font_size: Some(self.size.font_size(appearance)),
                            font_color: Some(shortcut_color),
                            font_family_id: Some(appearance.ui_font_family()),
                            ..Default::default()
                        };
                        row.add_child(
                            appearance
                                .ui_builder()
                                .keyboard_shortcut(shortcut)
                                .text_only()
                                .with_style(shortcut_styles)
                                .with_line_height_ratio(1.0)
                                .build()
                                .finish(),
                        );
                    } else {
                        // Standard mode: render with individual key boxes
                        let shortcut_styles = UiComponentStyles {
                            margin: Some(margin),
                            padding: Some(Coords::uniform(1.)),
                            border_width: Some(1.),
                            border_color: theme
                                .keyboard_shortcut_border(text_color, appearance)
                                .map(Into::into),
                            background: theme
                                .keyboard_shortcut_background(appearance)
                                .map(Into::into),
                            font_color: Some(text_color),
                            font_family_id: Some(appearance.ui_font_family()),
                            ..Default::default()
                        };
                        row.add_child(
                            appearance
                                .ui_builder()
                                .keyboard_shortcut(shortcut)
                                .with_space_between_keys(4.)
                                .with_style(self.size.keystroke_sizing(appearance))
                                .with_style(shortcut_styles)
                                .with_line_height_ratio(1.0)
                                .build()
                                .finish(),
                        );
                    }
                    true
                } else {
                    false
                }
            };

            if let Some(icon) = self.icon {
                let icon_size = self.size.icon_size(appearance, app);
                let icon_fill = self
                    .icon_ansi_color
                    .map(|ansi| {
                        Fill::Solid(appearance.theme().ansi_fg(
                            ansi.to_ansi_color(&appearance.theme().terminal_colors().normal),
                        ))
                    })
                    .unwrap_or(Fill::Solid(text_color));
                row.add_child(
                    ConstrainedBox::new(icon.to_warpui_icon(icon_fill).finish())
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                );
                has_preceding_element = true;
            }

            // Render keybinding before label if configured
            if self.keybinding_before_label && render_keybinding(&mut row, has_preceding_element) {
                has_preceding_element = true;
            }

            if !self.label.is_empty() {
                let font_properties = theme
                    .font_properties()
                    .unwrap_or_else(|| self.size.font_properties());
                let mut text = Text::new_inline(
                    self.label.clone(),
                    appearance.ui_font_family(),
                    self.size.font_size(appearance),
                )
                .with_color(text_color)
                .with_style(font_properties)
                .with_selectable(false)
                .with_clip(ClipConfig::ellipsis())
                .finish();

                // Determine the maximum width constraint for the button label:
                // - If both max_label_width and width are set, use the smaller value to ensure the label fits
                // - If only one constraint is set, use that value
                // - If no constraints are set, don't set a width constraint
                let max_label_width = match (self.max_label_width, self.width) {
                    (Some(w1), Some(w2)) => Some(w1.min(w2)),
                    (Some(w), None) => Some(w),
                    (None, Some(w)) => Some(w),
                    (None, None) => None,
                };
                if let Some(max_label_width) = max_label_width {
                    text = ConstrainedBox::new(text)
                        .with_max_width(max_label_width)
                        .finish();
                }

                row.add_child(
                    Container::new(text)
                        .with_margin_left(if has_preceding_element { 4. } else { 0. })
                        .finish(),
                );
                has_preceding_element = true;
            }

            if let Some(callout) = self.maybe_render_callout(has_preceding_element, appearance, app)
            {
                row.add_child(callout);
                has_preceding_element = true;
            }

            // Render keybinding after label (default position) if not already rendered before
            if !self.keybinding_before_label && render_keybinding(&mut row, has_preceding_element) {
                has_preceding_element = true;
            }

            if self.has_menu {
                let icon_size = self.size.icon_size(appearance, app);
                row.add_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::ChevronDown.to_warpui_icon(text_color.into()).finish(),
                        )
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                    )
                    .with_margin_left(if has_preceding_element { 4. } else { 0. })
                    .finish(),
                )
            }

            let corner_radius = match self.adjoined_side {
                Some(AdjoinedSide::Left) => CornerRadius::with_right(Radius::Pixels(4.)),
                Some(AdjoinedSide::Right) => CornerRadius::with_left(Radius::Pixels(4.)),
                None => CornerRadius::with_all(Radius::Pixels(4.)),
            };

            let border = if let Some(adjoined_side_border) = theme.adjoined_side_border(appearance)
            {
                match self.adjoined_side {
                    Some(AdjoinedSide::Left) => {
                        // Add border on the left side to avoid a double border against right-joined buttons
                        Some(Border::left(1.).with_border_color(adjoined_side_border))
                    }
                    Some(AdjoinedSide::Right) | None => None,
                }
            } else if let Some((gradient_start, gradient_end, gradient)) =
                theme.border_gradient(appearance)
            {
                match self.adjoined_side {
                    Some(AdjoinedSide::Left) | None => Some(Border::all(1.).with_border_gradient(
                        gradient_start,
                        gradient_end,
                        gradient,
                    )),
                    Some(AdjoinedSide::Right) => Some(
                        Border::new(1.)
                            // Remove border on the right side to avoid a double border against left-joined buttons
                            .with_sides(true, true, true, false)
                            .with_border_gradient(gradient_start, gradient_end, gradient),
                    ),
                }
            } else if let Some(border) = theme.border(appearance) {
                match self.adjoined_side {
                    Some(AdjoinedSide::Left) | None => {
                        Some(Border::all(1.).with_border_color(border))
                    }
                    Some(AdjoinedSide::Right) => Some(
                        Border::new(1.)
                            // Remove border on the right side to avoid a double border against left-joined buttons
                            .with_sides(true, true, true, false)
                            .with_border_color(border),
                    ),
                }
            } else {
                None
            };

            let mut container = {
                // Get the added border height so we can subtract it from the overall container
                // height, since the border gets added to the outside of the `ConstrainedBox`
                let border_height = match border {
                    Some(border) => border.top_width() + border.bottom_width(),
                    None => 0.,
                };

                let button_height = self
                    .custom_height
                    .unwrap_or_else(|| self.size.button_height(appearance, app))
                    - border_height;

                let mut constrained_box = ConstrainedBox::new(
                    if self.width.is_some() || self.is_icon_only() || self.full_width {
                        row.with_main_axis_size(MainAxisSize::Max).finish()
                    } else {
                        row.finish()
                    },
                )
                .with_height(button_height);

                // buttons that only have an icon should always be square
                if self.is_icon_only() {
                    let width = button_height;
                    constrained_box = constrained_box.with_width(width).with_max_width(width);
                } else if let Some(width) = self.width {
                    constrained_box = constrained_box.with_max_width(width);
                }

                Container::new(constrained_box.finish())
            }
            .with_corner_radius(corner_radius);

            if !self.is_icon_only() {
                container =
                    container.with_horizontal_padding(self.size.button_horizontal_padding());
            }

            if let Some(background) = background {
                container = container.with_background(background);
            }

            if let Some(border) = border {
                container = container.with_border(border);
            }

            let button_element = container.finish();

            // Only wrap in a Stack if we need to show a tooltip.
            // This avoids creating unnecessary layers which can interfere with
            // z-ordering when the button is inside an overlay context.
            if mouse_state.is_hovered() && self.tooltip.is_some() {
                let mut stack = Stack::new().with_child(button_element);
                self.maybe_render_tooltip(appearance, app, &mut stack);
                stack.finish()
            } else {
                button_element
            }
        });

        if let Some(on_click) = &self.click_handler {
            if !self.disabled {
                let on_click = on_click.clone();
                let mouse_state_handle = self.mouse_state_handle.clone();
                hoverable =
                    hoverable
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            on_click(ctx);
                            if let Ok(mut state) = mouse_state_handle.lock() {
                                state.reset_interaction_state();
                            }
                        });
            } else {
                // Use arrow cursor when disabled to avoid inheriting text cursor from children.
                // Register a no-op click handler so the Hoverable still consumes the
                // event, preventing it from propagating to parent elements.
                hoverable = hoverable.with_cursor(Cursor::Arrow).on_click(|_, _, _| {});
            }
        }

        match self.size.negative_vertical_margin() {
            None => hoverable.finish(),
            Some(negative_margin) => Container::new(hoverable.finish())
                .with_vertical_margin(negative_margin)
                .finish(),
        }
    }
}

// ActionButton currently has no typed actions - this is a placeholder so that callers aren't affected if we add any.
impl TypedActionView for ActionButton {
    type Action = ();
}

/// "DangerPrimary" buttons have a red fill for destructive actions.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=c27DwGHWevMlisVN-0)
pub struct DangerPrimaryTheme;

impl ActionButtonTheme for DangerPrimaryTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        let red_color = appearance.theme().ansi_fg_red();
        if hovered {
            Some(Fill::Solid(ColorU::new(255, 130, 114, 255)))
        } else {
            Some(Fill::Solid(red_color))
        }
    }

    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        PrimaryTheme.text_color(hovered, background, appearance)
    }

    fn keyboard_shortcut_border(
        &self,
        text_color: ColorU,
        _appearance: &Appearance,
    ) -> Option<ColorU> {
        Some(coloru_with_opacity(text_color, 60))
    }
}

/// "DangerSecondary" buttons have no fill and a colorful border.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=c27DwGHWevMlisVN-0)
pub struct DangerSecondaryTheme;

impl ActionButtonTheme for DangerSecondaryTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(
                appearance
                    .theme()
                    .ansi_overlay_2(
                        AnsiColorIdentifier::Red
                            .to_ansi_color(&appearance.theme().terminal_colors().normal),
                    )
                    .into(),
            )
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().ansi_fg_red()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(appearance.theme().ansi_fg_red())
    }

    fn keyboard_shortcut_border(
        &self,
        _text_color: ColorU,
        appearance: &Appearance,
    ) -> Option<ColorU> {
        Some(appearance.theme().ansi_fg_red())
    }
}

/// "Disabled" buttons have a disabled fill and text color.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=c27DwGHWevMlisVN-0)
pub struct DisabledTheme;

impl ActionButtonTheme for DisabledTheme {
    fn background(&self, _hovered: bool, appearance: &Appearance) -> Option<Fill> {
        Some(internal_colors::neutral_4(appearance.theme()).into())
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        internal_colors::neutral_5(appearance.theme())
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }
}

/// "Naked" buttons have no fill or border, only their contents.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=c27DwGHWevMlisVN-0)
pub struct NakedTheme;

impl ActionButtonTheme for NakedTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(internal_colors::fg_overlay_2(appearance.theme()))
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().foreground().into_solid()
    }
}

/// Like [`NakedTheme`] but uses `sub_text_color` instead of `foreground` for
/// text and icon color, matching the muted style of pane header buttons.
pub struct PaneHeaderTheme;

impl ActionButtonTheme for PaneHeaderTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        NakedTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid()
    }
}

/// The "Danger Naked" button variant.
///
/// [Figma Spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=MQvgxvZWjcapwzkK-11).
pub struct DangerNakedTheme;

impl ActionButtonTheme for DangerNakedTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(
                appearance
                    .theme()
                    .ansi_overlay_1(
                        AnsiColorIdentifier::Red
                            .to_ansi_color(&appearance.theme().terminal_colors().normal),
                    )
                    .into(),
            )
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().ansi_fg_red()
    }
}

/// "Secondary" buttons have no fill and a border.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=L1sS5Nxu1zzpWPYp-0)
pub struct SecondaryTheme;

impl ActionButtonTheme for SecondaryTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(internal_colors::fg_overlay_2(appearance.theme()))
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().foreground().into()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_4(appearance.theme()))
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }
}

/// Custom disabled theme for SecondaryTheme that preserves border.
/// This avoids unwanted width changes when toggling between enabled/disabled states.
pub struct DisabledSecondaryTheme;

impl ActionButtonTheme for DisabledSecondaryTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        DisabledTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        DisabledTheme.text_color(hovered, background, appearance)
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_4(appearance.theme()))
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        DisabledTheme.keyboard_shortcut_background(appearance)
    }
}

/// "Primary" buttons have a colorful fill.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=GRYXipD0INVmDupA-0)
pub struct PrimaryTheme;

impl ActionButtonTheme for PrimaryTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(internal_colors::accent_overlay_4(appearance.theme()))
        } else {
            Some(appearance.theme().accent())
        }
    }

    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        let theme = appearance.theme();
        let effective_background = background
            .or_else(|| self.background(hovered, appearance))
            .unwrap_or(theme.background());
        theme.font_color(effective_background).into_solid()
    }

    fn adjoined_side_border(&self, _appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::fg_overlay_4(_appearance.theme()).into())
    }

    fn keyboard_shortcut_border(
        &self,
        text_color: ColorU,
        _appearance: &Appearance,
    ) -> Option<ColorU> {
        Some(coloru_with_opacity(text_color, 60))
    }
}

/// Variant of PrimaryTheme that "solidifies" horizontal gradient accents by
/// using the right side color of the gradient. This is useful for adjoined
/// menu buttons that should visually match the gradient's right edge.
pub struct PrimaryRightBiasedTheme;

impl ActionButtonTheme for PrimaryRightBiasedTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        let accent = appearance.theme().accent();
        match accent {
            Fill::HorizontalGradient(_) => {
                if hovered {
                    let hover_fill = internal_colors::accent_overlay_4(appearance.theme());
                    Some(Fill::Solid(hover_fill.into_solid_bias_right_color()))
                } else {
                    Some(Fill::Solid(accent.into_solid_bias_right_color()))
                }
            }
            _ => {
                if hovered {
                    Some(internal_colors::accent_overlay_4(appearance.theme()))
                } else {
                    Some(accent)
                }
            }
        }
    }

    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        PrimaryTheme.text_color(hovered, background, appearance)
    }

    fn adjoined_side_border(&self, appearance: &Appearance) -> Option<ColorU> {
        PrimaryTheme.adjoined_side_border(appearance)
    }

    fn keyboard_shortcut_border(
        &self,
        text_color: ColorU,
        appearance: &Appearance,
    ) -> Option<ColorU> {
        PrimaryTheme.keyboard_shortcut_border(text_color, appearance)
    }
}

impl ButtonSize {
    pub fn icon_size(&self, appearance: &Appearance, app: &AppContext) -> f32 {
        match self {
            ButtonSize::Default => 16.,
            ButtonSize::Small => 14.,
            ButtonSize::XSmall => 14.,
            ButtonSize::InlineActionHeader => appearance.monospace_font_size(),
            ButtonSize::InputPrompt => appearance.monospace_font_size(),
            ButtonSize::UDIButton => appearance.monospace_font_size() - 1.0,
            ButtonSize::UDIPromptChip => appearance.monospace_font_size() - 1.0,
            ButtonSize::AgentInputButton => app.font_cache().line_height(
                appearance.monospace_font_size(),
                DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4,
            ),
        }
    }

    fn font_size(&self, appearance: &Appearance) -> f32 {
        match self {
            ButtonSize::Default => 14.,
            ButtonSize::Small => 12.,
            ButtonSize::XSmall => 12.,
            ButtonSize::InlineActionHeader => appearance.monospace_font_size() - 2.,
            ButtonSize::InputPrompt => appearance.monospace_font_size(),
            ButtonSize::UDIButton => appearance.monospace_font_size() - 1.0,
            ButtonSize::UDIPromptChip => appearance.monospace_font_size() - 1.0,
            ButtonSize::AgentInputButton => appearance.monospace_font_size() - 1.0,
        }
    }

    fn font_properties(&self) -> Properties {
        match self {
            ButtonSize::Default => Properties::default().weight(Weight::Semibold),
            ButtonSize::Small => Properties::default().weight(Weight::Semibold),
            ButtonSize::XSmall => Properties::default().weight(Weight::Normal),
            ButtonSize::InlineActionHeader => Properties::default().weight(Weight::Semibold),
            ButtonSize::InputPrompt => Properties::default(),
            ButtonSize::UDIButton => Properties::default(),
            ButtonSize::UDIPromptChip => Properties::default().weight(Weight::Semibold),
            ButtonSize::AgentInputButton => Properties::default(),
        }
    }

    /// Left spacing between the keystroke and button label.
    fn keystroke_left_spacing(&self) -> f32 {
        match self {
            ButtonSize::Default => 6.,
            ButtonSize::Small => 4.,
            ButtonSize::XSmall => 4.,
            ButtonSize::InlineActionHeader => 6.,
            ButtonSize::InputPrompt => 5.,
            ButtonSize::UDIButton => 5.,
            ButtonSize::UDIPromptChip => 4.,
            ButtonSize::AgentInputButton => 4.,
        }
    }

    /// Sizing and padding styles for the keystroke.
    /// This does not include any visual styling (colors, fonts).
    fn keystroke_sizing(&self, appearance: &Appearance) -> UiComponentStyles {
        match self {
            // Some of these values differ a little from Figma, in order to produce the desired
            // results with the KeyboardShortcut component implementation.
            ButtonSize::Default => UiComponentStyles {
                font_size: Some(12.),
                // The default KeyboardShortcut styles unfortunately include a height, so we have
                // to override it with something here.
                height: Some(18.),
                width: Some(18.),
                margin: Some(Coords::default()),
                padding: Some(Coords {
                    top: 0.,
                    bottom: 0.,
                    left: 4.,
                    right: 4.,
                }),
                ..Default::default()
            },
            ButtonSize::Small => UiComponentStyles {
                font_size: Some(10.),
                height: Some(14.),
                width: Some(14.),
                margin: Some(Coords::default()),
                padding: Some(Coords::default()),
                ..Default::default()
            },
            ButtonSize::XSmall => UiComponentStyles {
                font_size: Some(10.),
                height: Some(12.),
                width: Some(12.),
                margin: Some(Coords::default()),
                padding: Some(Coords::default()),
                ..Default::default()
            },
            ButtonSize::InlineActionHeader => UiComponentStyles {
                font_size: Some(appearance.monospace_font_size() - 4.),
                height: Some(appearance.monospace_font_size()),
                width: Some(appearance.monospace_font_size()),
                margin: Some(Coords::default()),
                padding: Some(Coords::default()),
                ..Default::default()
            },
            ButtonSize::InputPrompt => UiComponentStyles {
                font_size: Some(appearance.monospace_font_size() - 4.),
                width: Some(appearance.monospace_font_size() * DEFAULT_UI_LINE_HEIGHT_RATIO),
                height: Some(appearance.monospace_font_size() * DEFAULT_UI_LINE_HEIGHT_RATIO),
                padding: Some(Coords::default()),
                ..Default::default()
            },
            ButtonSize::UDIButton => UiComponentStyles {
                font_size: Some(appearance.monospace_font_size() - 4.),
                width: Some(appearance.monospace_font_size() * DEFAULT_UI_LINE_HEIGHT_RATIO),
                height: Some(appearance.monospace_font_size() * DEFAULT_UI_LINE_HEIGHT_RATIO),
                padding: Some(Coords::default()),
                ..Default::default()
            },
            ButtonSize::UDIPromptChip => UiComponentStyles {
                font_size: Some(appearance.monospace_font_size() - 4.),
                width: Some(appearance.monospace_font_size()),
                height: Some(appearance.monospace_font_size()),
                padding: Some(Coords::default()),
                ..Default::default()
            },
            ButtonSize::AgentInputButton => UiComponentStyles {
                font_size: Some(appearance.monospace_font_size() - 4.),
                width: Some(appearance.monospace_font_size()),
                height: Some(appearance.monospace_font_size()),
                padding: Some(Coords::default()),
                ..Default::default()
            },
        }
    }

    /// The overall button height, including all padding.
    ///
    /// We use this, rather than padding on the [`Container`] element, to ensure a consistent
    /// button height regardless of contents (e.g. whether or not there's an icon).
    pub fn button_height(&self, appearance: &Appearance, app: &AppContext) -> f32 {
        match self {
            ButtonSize::Default => 32.,
            ButtonSize::Small => 24.,
            ButtonSize::XSmall => 20.,
            // Should be 24px high at a 14px font size, and scale accordingly.
            ButtonSize::InlineActionHeader => 10. + appearance.monospace_font_size(),
            // Should be 20px high at a 14px font size, and scale accordingly.
            ButtonSize::InputPrompt => 6. + appearance.monospace_font_size(),
            ButtonSize::UDIButton => 6. + appearance.monospace_font_size(),
            ButtonSize::UDIPromptChip => {
                // Add 1 to the vertical padding to account for the border.
                let vertical_padding =
                    1. + crate::context_chips::spacing::UDI_CHIP_VERTICAL_PADDING;
                2. * vertical_padding + self.font_size(appearance)
            }
            ButtonSize::AgentInputButton => {
                // Add 1 to the vertical padding to account for the border.
                let vertical_padding =
                    1. + crate::context_chips::spacing::UDI_CHIP_VERTICAL_PADDING;
                let line_height = app
                    .font_cache()
                    .line_height(self.font_size(appearance), appearance.line_height_ratio());
                2. * vertical_padding + line_height
            }
        }
    }

    /// An optional negative margin for buttons rendered in small spaces like the prompt.
    fn negative_vertical_margin(&self) -> Option<f32> {
        match self {
            ButtonSize::Default => None,
            ButtonSize::Small => None,
            ButtonSize::XSmall => None,
            ButtonSize::InlineActionHeader => None,
            ButtonSize::InputPrompt => Some(-2.),
            ButtonSize::UDIButton => None,
            ButtonSize::UDIPromptChip => None,
            ButtonSize::AgentInputButton => None,
        }
    }

    /// Horizontal padding around the button contents.
    fn button_horizontal_padding(&self) -> f32 {
        match self {
            ButtonSize::Default => 12.,
            ButtonSize::Small => 8.,
            ButtonSize::XSmall => 6.,
            ButtonSize::InlineActionHeader => 8.,
            ButtonSize::InputPrompt => 4.,
            ButtonSize::UDIButton => 4.,
            ButtonSize::UDIPromptChip | ButtonSize::AgentInputButton => {
                crate::context_chips::spacing::UDI_CHIP_HORIZONTAL_PADDING
            }
        }
    }

    /// Vertical offset for tooltips.
    fn tooltip_offset(&self) -> f32 {
        match self {
            ButtonSize::Default => -4.,
            ButtonSize::Small => -4.,
            ButtonSize::XSmall => -4.,
            ButtonSize::InlineActionHeader => -4.,
            // Account for the negative margin on prompt buttons.
            ButtonSize::InputPrompt => -8.,
            ButtonSize::UDIButton => -8.,
            ButtonSize::UDIPromptChip | ButtonSize::AgentInputButton => -8.,
        }
    }

    /// Padding on callout items.
    fn callout_padding(&self) -> Padding {
        match self {
            ButtonSize::Default => Padding::uniform(2.),
            ButtonSize::Small => Padding::uniform(2.),
            ButtonSize::XSmall => Padding::uniform(2.),
            ButtonSize::InlineActionHeader => Padding::uniform(2.),
            ButtonSize::InputPrompt => Padding::default().with_vertical(1.).with_horizontal(2.),
            ButtonSize::UDIButton => Padding::default().with_vertical(1.).with_horizontal(2.),
            ButtonSize::UDIPromptChip | ButtonSize::AgentInputButton => {
                Padding::default().with_vertical(1.).with_horizontal(2.)
            }
        }
    }
}
