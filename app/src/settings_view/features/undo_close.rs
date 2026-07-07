use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use settings::{Setting, ToggleableSetting};
use warpui::elements::{
    Container, CrossAxisAlignment, Flex, MainAxisAlignment, MouseStateHandle, ParentElement, Text,
};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{self, EditorView, SingleLineEditorOptions, TextOptions};
use crate::report_if_error;
use crate::settings_view::features_page::render_group;
use crate::settings_view::settings_page::{render_body_item, LocalOnlyIconState, ToggleState};
use crate::undo_close::settings::UndoCloseEnabled;
use crate::undo_close::UndoCloseSettings;

#[derive(Debug, Clone, Copy)]
pub enum Action {
    ToggleUndoCloseEnabled,
    UpdateGracePeriod,
}

/// A view containing settings relating to the undo close feature.
pub struct UndoCloseView {
    /// State for the enable/disable toggle switch.
    switch_state: SwitchStateHandle,
    /// An editor for modifying the undo close grace period.
    grace_period_editor: ViewHandle<EditorView>,
    /// Whether or not the grace period value is valid.
    is_grace_period_valid: bool,
    /// State for the local only icon tooltip.
    local_only_icon_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl UndoCloseView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let grace_period_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
                    ..Default::default()
                },
                ctx,
            )
        });

        ctx.subscribe_to_model(&UndoCloseSettings::handle(ctx), |me, _, _, ctx| {
            // Update the value of the grace period input to match the new setting
            me.grace_period_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(
                    &format!(
                        "{}",
                        UndoCloseSettings::handle(ctx)
                            .as_ref(ctx)
                            .grace_period
                            .as_secs_f32()
                    ),
                    ctx,
                );
            });
            ctx.notify()
        });

        ctx.subscribe_to_view(&grace_period_editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let grace_period = UndoCloseSettings::as_ref(ctx).grace_period.as_secs();
        grace_period_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&grace_period.to_string(), ctx);
        });
        Self {
            switch_state: Default::default(),
            local_only_icon_states: Default::default(),
            grace_period_editor,
            is_grace_period_valid: true,
        }
    }

    /// This callback updates the undo close setting based on user input. If the
    /// user hits Enter or the input loses focus, the new setting is saved (they
    /// can also save it by clicking outside of the text field).
    fn handle_editor_event(&mut self, event: &editor::Event, ctx: &mut ViewContext<Self>) {
        use editor::Event;
        match event {
            Event::Edited(_) => {
                let buffer_text = self.grace_period_editor.as_ref(ctx).buffer_text(ctx);
                let new_validity = Self::parse_grace_period(&buffer_text).is_some();
                if new_validity != self.is_grace_period_valid {
                    self.is_grace_period_valid = new_validity;
                    ctx.notify();
                }
            }
            Event::Blurred | Event::Enter => {
                self.handle_action(&Action::UpdateGracePeriod, ctx);
            }
            _ => (),
        }
    }

    /// Parses user-entered text into a grace period duration, returning
    /// None if the text isn't a valid grace period.
    fn parse_grace_period(text: &str) -> Option<Duration> {
        text.parse::<u64>().ok().map(Duration::from_secs)
    }

    /// Renders the editor for the grace period duration.
    fn render_grace_period_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let border_color = if self.is_grace_period_valid {
            None
        } else {
            Some(crate::themes::theme::Fill::error().into())
        };

        let editor_style = UiComponentStyles {
            border_color,
            width: Some(40.),
            padding: Some(Coords::uniform(5.)),
            background: Some(theme.surface_2().into()),
            ..Default::default()
        };
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(
                Container::new(
                    Text::new_inline(
                        "Grace period (seconds)",
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
                )
                .with_padding_right(8.5)
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .text_input(self.grace_period_editor.clone())
                    .with_style(editor_style)
                    .build()
                    .finish(),
            )
            .finish()
    }
}

impl Entity for UndoCloseView {
    type Event = ();
}

impl View for UndoCloseView {
    fn ui_name() -> &'static str {
        "UndoCloseView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let settings = UndoCloseSettings::as_ref(app);
        let enabled = *settings.enabled;

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(render_body_item::<Action>(
                "Enable reopening of closed sessions".into(),
                None,
                LocalOnlyIconState::for_setting(
                    UndoCloseEnabled::storage_key(),
                    UndoCloseEnabled::sync_to_cloud(),
                    &mut self.local_only_icon_states.borrow_mut(),
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                ui_builder
                    .switch(self.switch_state.clone())
                    .check(enabled)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::ToggleUndoCloseEnabled);
                    })
                    .finish(),
                None,
            ));

        if enabled {
            column.add_child(render_group(
                [self.render_grace_period_editor(appearance)],
                appearance,
            ));
        }

        column.finish()
    }
}

impl TypedActionView for UndoCloseView {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut warpui::ViewContext<Self>) {
        match action {
            Action::ToggleUndoCloseEnabled => {
                UndoCloseSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.enabled.toggle_and_save_value(ctx));
                })
            }
            Action::UpdateGracePeriod => {
                let grace_period_secs = self
                    .grace_period_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx));
                let Some(grace_period) = Self::parse_grace_period(&grace_period_secs) else {
                    self.is_grace_period_valid = false;
                    return;
                };

                self.is_grace_period_valid = true;
                UndoCloseSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.grace_period.set_value(grace_period, ctx));
                });
            }
        }
    }
}
