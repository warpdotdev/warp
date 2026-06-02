use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement,
};
use warpui::{Action, AppContext, Element, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, AdjoinedSide, ButtonSize, KeystrokeSource,
};
const BUTTON_MARGIN: f32 = 8.;

// Size switch thresholds for responsive button behavior
pub const SMALL_SIZE_SWITCH_THRESHOLD: f32 = 400.0;
pub const MEDIUM_SIZE_SWITCH_THRESHOLD: f32 = 500.0;
pub const LARGE_SIZE_SWITCH_THRESHOLD: f32 = 600.0;
pub const XLARGE_SIZE_SWITCH_THRESHOLD: f32 = 650.0;

/// Stores normal and compact (i.e. without a keybinding display) versions of action buttons
/// for use in views that need to display buttons in different modes.
#[derive(Clone)]
pub struct CompactibleActionButton {
    compact_button: ViewHandle<ActionButton>,
    expanded_button: ViewHandle<ActionButton>,
}

pub trait RenderCompactibleActionButton {
    fn render_expanded_button(&self) -> Box<dyn Element>;
    fn render_compact_button(&self) -> Box<dyn Element>;
}

impl CompactibleActionButton {
    /// Creates a new button pair with compact and regular variants.
    pub fn new<T, A>(
        label: String,
        keybinding: Option<KeystrokeSource>,
        size: ButtonSize,
        action: A,
        compact_icon: Icon,
        theme: Arc<dyn ActionButtonTheme>,
        ctx: &mut ViewContext<'_, T>,
    ) -> Self
    where
        T: TypedActionView<Action = A> + View,
        A: Action + Clone + 'static,
    {
        let action_for_compact = action.clone();
        let compact_button = ctx.add_typed_action_view(|ctx| {
            let mut compact_button =
                ActionButton::new_with_boxed_theme(String::new(), Arc::clone(&theme))
                    .with_size(size)
                    .with_icon(compact_icon)
                    .with_tooltip(label.clone())
                    .on_click(move |ctx| ctx.dispatch_typed_action(action_for_compact.clone()));

            if let Some(ref kb) = keybinding {
                if let Some(tooltip_sublabel) = kb.displayed(ctx) {
                    compact_button = compact_button.with_tooltip_sublabel(tooltip_sublabel);
                }
            }

            compact_button
        });

        let expanded_button = ctx.add_typed_action_view(move |ctx| {
            let mut button = ActionButton::new_with_boxed_theme(label.clone(), theme)
                .with_size(size)
                .on_click(move |ctx| ctx.dispatch_typed_action(action.clone()));

            if let Some(kb) = keybinding {
                button = button.with_keybinding(kb, ctx);
            }

            button
        });

        Self {
            compact_button,
            expanded_button,
        }
    }

    pub fn set_label<T: View>(&mut self, label: String, ctx: &mut ViewContext<T>) {
        self.expanded_button.update(ctx, |button, ctx| {
            button.set_label(label.clone(), ctx);
        });
        self.compact_button.update(ctx, |button, ctx| {
            button.set_tooltip(Some(label), ctx);
        });
    }

    pub fn set_keybinding<T: View>(
        &mut self,
        keybinding: Option<KeystrokeSource>,
        ctx: &mut ViewContext<T>,
    ) {
        self.expanded_button.update(ctx, |button, ctx| {
            button.set_keybinding(keybinding.clone(), ctx);
        });

        self.compact_button.update(ctx, |button, ctx| {
            if let Some(keybinding) = keybinding {
                button.set_tooltip_sublabel(keybinding.displayed(ctx), ctx);
            } else {
                button.set_tooltip_sublabel(None::<String>, ctx);
            }
        });
    }

    pub fn set_adjoined_side<T: View>(
        &mut self,
        adjoined_side: AdjoinedSide,
        ctx: &mut ViewContext<T>,
    ) {
        self.compact_button.update(ctx, |button, ctx| {
            button.set_adjoined_side(adjoined_side, ctx);
        });
        self.expanded_button.update(ctx, |button, ctx| {
            button.set_adjoined_side(adjoined_side, ctx);
        });
    }

    /// Sets the disabled state on both the compact and expanded buttons.
    pub fn set_disabled<T: View>(&mut self, disabled: bool, ctx: &mut ViewContext<T>) {
        self.compact_button.update(ctx, |button, ctx| {
            button.set_disabled(disabled, ctx);
        });
        self.expanded_button.update(ctx, |button, ctx| {
            button.set_disabled(disabled, ctx);
        });
    }

    /// Sets the tooltip on both buttons. Passing `None` clears the compact
    /// button's hover affordance; use [`set_label`](Self::set_label) to
    /// preserve the label tooltip.
    pub fn set_tooltip<T: View>(&mut self, tooltip: Option<String>, ctx: &mut ViewContext<T>) {
        let compact_tooltip = tooltip.clone();
        let expanded_tooltip = tooltip;
        self.compact_button.update(ctx, |button, ctx| {
            button.set_tooltip(compact_tooltip, ctx);
        });
        self.expanded_button.update(ctx, |button, ctx| {
            button.set_tooltip(expanded_tooltip, ctx);
        });
    }

    pub fn compact_button(&self) -> &ViewHandle<ActionButton> {
        &self.compact_button
    }

    pub fn expanded_button(&self) -> &ViewHandle<ActionButton> {
        &self.expanded_button
    }
}

impl RenderCompactibleActionButton for CompactibleActionButton {
    fn render_expanded_button(&self) -> Box<dyn Element> {
        ChildView::new(self.expanded_button()).finish()
    }
    fn render_compact_button(&self) -> Box<dyn Element> {
        ChildView::new(self.compact_button()).finish()
    }
}

/// Render both compact and expanded button rows
/// and then switch between them based on the container width.
pub fn render_compact_and_regular_button_rows(
    buttons: Vec<&dyn RenderCompactibleActionButton>,
    // None when we don't want to show the expansion icon at all.
    expansion_icon_state: Option<bool>,
    appearance: &Appearance,
    app: &AppContext,
) -> (Box<dyn Element>, Box<dyn Element>) {
    let (full_buttons, compact_buttons) = buttons
        .iter()
        .map(|button| {
            (
                button.render_expanded_button(),
                button.render_compact_button(),
            )
        })
        .unzip();

    let mut full_row = render_button_row(full_buttons);
    let mut compact_row = render_button_row(compact_buttons);

    if let Some(expansion_icon_state) = expansion_icon_state {
        full_row.add_child(render_expansion_icon(
            expansion_icon_state,
            false,
            appearance,
            app,
        ));
        compact_row.add_child(render_expansion_icon(
            expansion_icon_state,
            false,
            appearance,
            app,
        ));
    }

    (full_row.finish(), compact_row.finish())
}

fn render_button_row(buttons: Vec<Box<dyn Element>>) -> Flex {
    let mut row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min);

    for (index, element) in buttons.into_iter().enumerate() {
        let mut container = Container::new(element);
        if index != 0 {
            container = container.with_margin_left(BUTTON_MARGIN);
        }
        row.add_child(container.finish());
    }

    row
}

pub fn render_expansion_icon(
    expanded: bool,
    expands_upwards: bool,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    ConstrainedBox::new(
        warpui::elements::Icon::new(
            if expanded {
                if expands_upwards {
                    Icon::ChevronUp.into()
                } else {
                    Icon::ChevronDown.into()
                }
            } else {
                Icon::ChevronRight.into()
            },
            appearance.theme().foreground(),
        )
        .finish(),
    )
    .with_width(icon_size(app))
    .with_height(icon_size(app))
    .finish()
}
