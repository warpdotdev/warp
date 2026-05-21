use warpui::{
    elements::{CrossAxisAlignment, Fill, Flex, ParentElement, Shrinkable},
    presenter::ChildView,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    editor::{EditorView, Event, SingleLineEditorOptions, TextOptions},
    report_if_error, send_telemetry_from_ctx,
    server::telemetry::TelemetryEvent,
    terminal::{
        available_shells::{AvailableShell, AvailableShells},
        local_tty::shell::is_valid_path_or_command_for_supported_shell,
        session_settings::{SessionSettings, SessionSettingsChangedEvent},
    },
    view_components::{dropdown::TOP_MENU_BAR_HEIGHT, Dropdown, DropdownItem},
};

/// A view for configuring the initial shell for new sessions. This can be the
/// user's login shell, the default installed version of zsh, bash, or fish,
/// or an arbitrary user-provided path.
pub struct StartupShellView {
    /// This dropdown is for selecting between the login shell, supported shells,
    /// and a custom shell.
    shell_dropdown: ViewHandle<Dropdown<NewSessionShellAction>>,
    /// This flags whether or not to show the custom path editor. It's toggled
    /// when the user chooses different dropdown options.
    should_display_editor: bool,
    /// If the user chose a custom shell path, they enter it in this editor.
    custom_path_editor: ViewHandle<EditorView>,
    /// This holds the current validity of the user's custom shell path, for
    /// drawing an error border if it's invalid.
    is_custom_path_valid: bool,
}

#[derive(Debug, Clone)]
pub enum NewSessionShellAction {
    /// Changes the user's startup shell to the given option. This also hides
    /// the custom shell path editor if a non-custom shell was chosen.
    Set(AvailableShell),
    /// Displays the custom shell path editor.
    ShowCustomPathInput,
}

impl NewSessionShellAction {
    /// Produces a [`TelemetryEvent`] that corresponds to this UI action.
    ///
    /// This tracks both high-level information about which shells users select
    /// and when they switch to the custom path UI (so we can see if they're
    /// trying to use a custom shell but are unable to).
    fn telemetry_event(&self) -> TelemetryEvent {
        match self {
            NewSessionShellAction::Set(option) => TelemetryEvent::FeaturesPageAction {
                action: "NewSessionShellOverride".to_string(),
                value: option.telemetry_value(),
            },
            NewSessionShellAction::ShowCustomPathInput => TelemetryEvent::FeaturesPageAction {
                action: "ShowCustomPathInput".to_string(),
                value: String::new(),
            },
        }
    }
}

impl StartupShellView {
    /// Creates a new `StartupShellView`. The UI is initialized with the user's
    /// current startup shell setting.
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let custom_shell_text = AvailableShells::handle(ctx).read(ctx, |shells, ctx| {
            shells.get_user_preferred_shell(ctx).get_custom_path()
        });

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                SessionSettingsChangedEvent::StartupShellOverride { .. }
            ) {
                Self::update_dropdown_state(me.shell_dropdown.clone(), ctx);
                me.maybe_update_editor_state(ctx);
            }
            ctx.notify()
        });

        let shell_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(200.);
            dropdown
        });

        Self::update_dropdown_state(shell_dropdown.clone(), ctx);

        let shell_editor = ctx.add_typed_action_view(|ctx| {
            let appearance_handle = Appearance::handle(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_font_size(appearance_handle.as_ref(ctx)),
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(crate::i18n::tr_static(ctx, "Executable path"), ctx);

            if let Some(shell) = custom_shell_text.as_ref() {
                editor.set_buffer_text(shell, ctx);
            }

            editor
        });

        ctx.subscribe_to_view(&shell_editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        Self {
            shell_dropdown,
            custom_path_editor: shell_editor,
            is_custom_path_valid: true,
            should_display_editor: custom_shell_text.is_some(),
        }
    }

    fn maybe_update_editor_state(&mut self, ctx: &mut ViewContext<Self>) {
        let custom_shell_path = AvailableShells::handle(ctx).read(ctx, |shells, ctx| {
            shells.get_user_preferred_shell(ctx).get_custom_path()
        });
        if let Some(custom_shell_path) = custom_shell_path {
            self.should_display_editor = true;
            self.custom_path_editor.update(ctx, |editor_view, ctx| {
                editor_view.set_buffer_text(&custom_shell_path, ctx);
            });
        }
    }

    fn update_dropdown_state(
        dropdown: ViewHandle<Dropdown<NewSessionShellAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown.update(ctx, |dropdown, ctx| {
            let mut items = vec![DropdownItem::new(
                "Default",
                NewSessionShellAction::Set(AvailableShell::default()),
            )];
            let shell_to_index = AvailableShells::handle(ctx).read(ctx, |model, _| {
                let mut shell_to_index = std::collections::HashMap::new();
                // Iterate over each shell in the model and add it to the dropdown if it's valid.
                for shell_entry in model.get_available_shells() {
                    items.push(DropdownItem::new(
                        model.display_name_for_shell(shell_entry),
                        NewSessionShellAction::Set(shell_entry.clone()),
                    ));
                    shell_to_index.insert(shell_entry.clone(), items.len() - 1);
                }

                shell_to_index
            });

            items.push(DropdownItem::new(
                "Custom",
                NewSessionShellAction::ShowCustomPathInput,
            ));
            let custom_index = items.len() - 1;
            dropdown.set_items(items, ctx);

            let selected_shell = AvailableShells::as_ref(ctx).get_user_preferred_shell(ctx);

            let selected_index = if selected_shell.get_custom_path().is_some() {
                custom_index
            } else {
                shell_to_index.get(&selected_shell).copied().unwrap_or(0)
            };

            dropdown.set_selected_by_index(selected_index, ctx);
        });
    }

    /// This callback updates the startup shell override setting based on user
    /// input. If the user hits Enter or the input loses focus, the new setting
    /// is saved (they can also save it by clicking outside of the text field).
    fn handle_editor_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        match event {
            Event::Edited(_) => {
                let buffer_text = self.custom_path_editor.as_ref(ctx).buffer_text(ctx);
                let new_validity = is_valid_path_or_command_for_supported_shell(&buffer_text);
                if new_validity != self.is_custom_path_valid {
                    self.is_custom_path_valid = new_validity;
                    ctx.notify();
                }
            }
            Event::Blurred | Event::Enter => {
                let buffer_text = self.custom_path_editor.as_ref(ctx).buffer_text(ctx);
                if let Ok(shell) = AvailableShell::try_from(buffer_text.as_str()) {
                    self.handle_action(&NewSessionShellAction::Set(shell), ctx);
                }
            }
            _ => (),
        }
    }
}

impl Entity for StartupShellView {
    type Event = ();
}

impl View for StartupShellView {
    fn ui_name() -> &'static str {
        "StartupShellView"
    }

    /// Renders controls to change the default shell for new sessions.
    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        row.add_child(ChildView::new(&self.shell_dropdown).finish());

        if self.should_display_editor {
            let border_color: Option<Fill> = if self.is_custom_path_valid {
                None
            } else {
                Some(crate::themes::theme::Fill::error().into())
            };

            row.add_child(
                Shrinkable::new(
                    1.,
                    ui_builder
                        .text_input(self.custom_path_editor.clone())
                        .with_style(UiComponentStyles {
                            border_color,
                            // Make sure the editor is the same height as the dropdown it's next to.
                            height: Some(TOP_MENU_BAR_HEIGHT),
                            padding: Some(Coords::uniform(7.)),
                            margin: Some(Coords::default().left(8.).right(8.)),
                            font_size: Some(appearance.ui_font_size()),
                            background: Some(theme.surface_2().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .finish(),
            )
        }

        row.finish()
    }
}

impl TypedActionView for StartupShellView {
    type Action = NewSessionShellAction;

    /// Handles a `NewSessionShellAction`, either triggered by the shell dropdown
    /// or by custom path editor events.
    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            NewSessionShellAction::ShowCustomPathInput => {
                self.should_display_editor = true;
                ctx.notify();
            }
            NewSessionShellAction::Set(shell) => {
                if shell.get_custom_path().is_none() && self.should_display_editor {
                    self.should_display_editor = false;
                    ctx.notify();
                }
                AvailableShells::handle(ctx).update(ctx, |shells, ctx| {
                    report_if_error!(shells.set_user_preferred_shell(shell.clone(), ctx));
                });
            }
        }
        send_telemetry_from_ctx!(action.telemetry_event(), ctx);
    }
}
