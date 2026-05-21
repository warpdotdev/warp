//! Trigger button + [`CodeReviewDiffMenu`] overlay for picking the diff
//! target in the code review header.
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{
        ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Element, Flex, MouseStateHandle, OffsetPositioning, ParentElement, PositionedElementAnchor,
        PositionedElementOffsetBounds, Radius, SavePosition, Stack, Text,
    },
    fonts::{Properties, Weight},
    id,
    keymap::FixedBinding,
    platform::Cursor,
    text_layout::ClipConfig,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Entity, FocusContext, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    appearance::Appearance,
    code_review::{
        diff_menu::{CodeReviewDiffMenu, CodeReviewDiffMenuEvent},
        diff_state::DiffMode,
    },
    ui_components::icons::Icon,
};

/// A single selectable target in the diff selector menu.
#[derive(Debug, Clone)]
pub struct DiffTarget {
    pub label: String,
    pub mode: DiffMode,
    pub is_selected: bool,
}

impl DiffTarget {
    pub fn new(label: impl Into<String>, mode: DiffMode, is_selected: bool) -> Self {
        Self {
            label: label.into(),
            mode,
            is_selected,
        }
    }
}

const BUTTON_LABEL_MAX_WIDTH: f32 = 240.;
const MENU_OFFSET_Y: f32 = 4.;
const BUTTON_CORNER_RADIUS: f32 = 4.;
const BUTTON_VERTICAL_PADDING: f32 = 5.;
const BUTTON_HORIZONTAL_PADDING: f32 = 8.;

pub struct DiffSelector {
    menu: ViewHandle<CodeReviewDiffMenu>,
    menu_open: bool,
    trigger_mouse_state: MouseStateHandle,
    trigger_position_id: String,
    /// Cached label for the trigger button; mirrors the selected `DiffTarget`.
    trigger_label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiffSelectorAction {
    Toggle,
}

#[derive(Clone, Debug)]
pub enum DiffSelectorEvent {
    SelectMode(DiffMode),
}

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "enter",
        DiffSelectorAction::Toggle,
        id!(DiffSelector::ui_name()),
    )]);
}

impl DiffSelector {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let menu = ctx.add_typed_action_view(CodeReviewDiffMenu::new);

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            CodeReviewDiffMenuEvent::Select(mode) => {
                me.menu_open = false;
                ctx.emit(DiffSelectorEvent::SelectMode(mode.clone()));
                ctx.notify();
            }
            CodeReviewDiffMenuEvent::Close => {
                me.menu_open = false;
                ctx.notify();
            }
        });

        Self {
            trigger_position_id: format!("code_review_diff_selector_trigger_{}", menu.id()),
            menu,
            menu_open: false,
            trigger_mouse_state: MouseStateHandle::default(),
            trigger_label: String::new(),
        }
    }

    pub fn toggle(&mut self, ctx: &mut ViewContext<Self>) {
        if self.menu_open {
            self.close(ctx);
        } else {
            self.menu_open = true;
            self.menu.update(ctx, |menu, ctx| menu.reset(ctx));
            ctx.focus(&self.menu);
            ctx.notify();
        }
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        if self.menu_open {
            self.menu_open = false;
            ctx.notify();
        }
    }

    /// Replace the menu rows. Trigger label follows the selected target.
    pub fn set_targets(&mut self, targets: Vec<DiffTarget>, ctx: &mut ViewContext<Self>) {
        self.trigger_label = targets
            .iter()
            .find(|target| target.is_selected)
            .map(|target| target.label.clone())
            .unwrap_or_default();
        self.menu.update(ctx, |menu, ctx| {
            menu.set_targets(targets, ctx);
        });
        ctx.notify();
    }
}

impl Entity for DiffSelector {
    type Event = DiffSelectorEvent;
}

impl TypedActionView for DiffSelector {
    type Action = DiffSelectorAction;

    fn handle_action(&mut self, action: &DiffSelectorAction, ctx: &mut ViewContext<Self>) {
        match action {
            DiffSelectorAction::Toggle => self.toggle(ctx),
        }
    }
}

impl View for DiffSelector {
    fn ui_name() -> &'static str {
        "CodeReviewDiffSelector"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() && self.menu_open {
            ctx.focus(&self.menu);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.background()).into_solid();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();

        let label = if self.trigger_label.is_empty() {
            "Uncommitted changes".to_string()
        } else {
            self.trigger_label.clone()
        };

        // Build the icon+text row by hand: `with_text_and_icon_label` wraps
        // the text in `Shrinkable<flex=1>`, which makes the button grow to
        // fill available width and paints an oversized hover rectangle even
        // for short labels.
        let icon = ConstrainedBox::new(
            Icon::SwitchHorizontal01
                .to_warpui_icon(Fill::Solid(text_color))
                .finish(),
        )
        .with_width(15.)
        .with_height(15.)
        .finish();

        // Truncate long branch names. Capping the text directly avoids
        // double-constraining padding from an outer wrapper.
        let label_text = ConstrainedBox::new(
            Text::new_inline(label, font_family, font_size)
                .with_color(text_color)
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_clip(ClipConfig::ellipsis())
                .finish(),
        )
        .with_max_width(BUTTON_LABEL_MAX_WIDTH)
        .finish();

        let custom_label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(Container::new(label_text).with_margin_left(6.).finish())
            .finish();

        // Shared hover + active styles so the fill persists when the pointer
        // moves into the menu's search input.
        let hover_or_active_styles = UiComponentStyles {
            background: Some(theme.surface_2().into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_CORNER_RADIUS))),
            ..Default::default()
        };

        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.trigger_mouse_state.clone())
            .with_custom_label(custom_label)
            .with_style(UiComponentStyles {
                font_family_id: Some(font_family),
                font_size: Some(font_size),
                font_color: Some(text_color),
                font_weight: Some(Weight::Semibold),
                padding: Some(Coords {
                    top: BUTTON_VERTICAL_PADDING,
                    bottom: BUTTON_VERTICAL_PADDING,
                    left: BUTTON_HORIZONTAL_PADDING,
                    right: BUTTON_HORIZONTAL_PADDING,
                }),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_CORNER_RADIUS))),
                ..Default::default()
            })
            .with_hovered_styles(hover_or_active_styles)
            .with_active_styles(hover_or_active_styles);

        if self.menu_open {
            button = button.active();
        }

        let trigger = button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(DiffSelectorAction::Toggle);
            })
            .finish();
        let trigger = SavePosition::new(trigger, &self.trigger_position_id).finish();

        let mut stack = Stack::new().with_child(trigger);

        if self.menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.trigger_position_id.as_str(),
                    vec2f(0., MENU_OFFSET_Y),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        Container::new(stack.finish()).finish()
    }
}
