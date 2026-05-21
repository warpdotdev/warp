use std::sync::Arc;

use warpui::elements::{ChildView, Flex, ParentElement, SavePosition};
use warpui::{Action, AppContext, Element, TypedActionView, View, ViewContext, ViewHandle};

use crate::view_components::action_button::AdjoinedSide;
use crate::view_components::compactible_action_button::RenderCompactibleActionButton;
use crate::{
    ui_components::icons::Icon,
    view_components::action_button::{
        ActionButton, ButtonSize, KeystrokeSource, NakedTheme, PrimaryRightBiasedTheme,
        PrimaryTheme,
    },
    view_components::compactible_action_button::CompactibleActionButton,
};

/// A split button composed of a primary CompactibleActionButton and a trailing
/// icon-only menu button (chevron-down). The menu button may be used as an anchor
/// for a dropdown Menu via `with_menu`.
#[derive(Clone)]
#[allow(dead_code)]
pub struct CompactibleSplitActionButton {
    primary_button: CompactibleActionButton,
    menu_button: ViewHandle<ActionButton>,
    save_position_id: Option<String>,
}

impl CompactibleSplitActionButton {
    /// Creates a split button: a primary CompactibleActionButton and an icon-only
    /// chevron menu button that inherits the same theme choice.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub fn new<T, A>(
        label: String,
        keybinding: Option<KeystrokeSource>,
        size: ButtonSize,
        action: A,
        menu_action: A,
        compact_icon: Icon,
        use_primary_theme: bool,
        save_position_id: Option<String>,
        ctx: &mut ViewContext<'_, T>,
    ) -> Self
    where
        T: TypedActionView<Action = A> + View,
        A: Action + Clone + 'static,
    {
        let mut primary_button = CompactibleActionButton::new(
            label,
            keybinding,
            size,
            action,
            compact_icon,
            if use_primary_theme {
                Arc::new(PrimaryTheme)
            } else {
                Arc::new(NakedTheme)
            },
            ctx,
        );

        primary_button.set_adjoined_side(AdjoinedSide::Right, ctx);

        // The down-caret icon-only menu button.
        let menu_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new_with_boxed_theme(
                "",
                if use_primary_theme {
                    Arc::new(PrimaryRightBiasedTheme)
                } else {
                    Arc::new(NakedTheme)
                },
            )
            .with_size(size)
            .with_icon(Icon::ChevronDown)
            .with_adjoined_side(AdjoinedSide::Left)
            .on_click(move |ctx| ctx.dispatch_typed_action(menu_action.clone()))
        });

        Self {
            primary_button,
            menu_button,
            save_position_id,
        }
    }

    fn render_button(&self, is_expanded: bool) -> Box<dyn Element> {
        let button = if is_expanded {
            self.primary_button.expanded_button()
        } else {
            self.primary_button.compact_button()
        };
        let row = Flex::row()
            .with_child(ChildView::new(button).finish())
            .with_child(ChildView::new(&self.menu_button).finish());
        if let Some(save_position_id) = &self.save_position_id {
            SavePosition::new(row.finish(), save_position_id).finish()
        } else {
            row.finish()
        }
    }

    pub fn set_keybinding<T: View>(
        &mut self,
        keybinding: Option<KeystrokeSource>,
        ctx: &mut ViewContext<T>,
    ) {
        self.primary_button.set_keybinding(keybinding, ctx);
    }

    /// Sets the disabled state on both the primary and menu buttons.
    pub fn set_disabled<T: View>(&mut self, disabled: bool, ctx: &mut ViewContext<T>) {
        self.primary_button.set_disabled(disabled, ctx);
        self.menu_button.update(ctx, |button, ctx| {
            button.set_disabled(disabled, ctx);
        });
    }

    /// Sets the tooltip on the primary button.
    pub fn set_tooltip<T: View>(&mut self, tooltip: Option<String>, ctx: &mut ViewContext<T>) {
        self.primary_button.set_tooltip(tooltip, ctx);
    }
}

impl RenderCompactibleActionButton for CompactibleSplitActionButton {
    fn render_expanded_button(&self) -> Box<dyn Element> {
        self.render_button(true)
    }
    fn render_compact_button(&self) -> Box<dyn Element> {
        self.render_button(false)
    }

    fn compact_button_mouse_state_handle(
        &self,
        app: &AppContext,
    ) -> Option<warpui::elements::MouseStateHandle> {
        self.primary_button.compact_button_mouse_state_handle(app)
    }
}
