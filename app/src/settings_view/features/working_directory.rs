use itertools::Itertools;
use warpui::{
    elements::{Container, CrossAxisAlignment, Flex, ParentElement, Shrinkable},
    presenter::ChildView,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions},
    report_if_error, send_telemetry_from_ctx,
    server::telemetry::TelemetryEvent,
    settings_view::features_page::render_group,
    terminal::session_settings::*,
    view_components::{dropdown::TOP_MENU_BAR_HEIGHT, Dropdown, DropdownItem},
};

#[derive(Clone, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum WorkingDirectoryAction {
    /// Sets the mode that should be used for all new sessions, independent of
    /// source.  A value of None indicates that the mode should be configured
    /// per-source instead of globally (i.e.: "advanced" mode).
    SetGlobalWorkingDirectoryMode(Option<WorkingDirectoryMode>),
    /// Sets the mode that should be used for new sessions spawned from the
    /// given source (e.g.: new tab/window/split pane).
    SetPerSourceWorkingDirectoryMode(NewSessionSource, WorkingDirectoryMode),
    /// Sets the path that will be used for [`WorkingDirectoryMode::CustomDir`]
    /// for the given source (where None represents global configuration).
    SetCustomWorkingDirectoryValue(Option<NewSessionSource>, String),
}

/// A view for configuring the initial working directory for new sessions,
/// either globally or on a per-source (new tab/window/split pane) basis.
pub struct WorkingDirectoryView {
    working_directory_dropdown: ViewHandle<Dropdown<WorkingDirectoryAction>>,
    working_directory_editor: ViewHandle<EditorView>,
    new_window_working_directory_dropdown: ViewHandle<Dropdown<WorkingDirectoryAction>>,
    new_window_working_directory_editor: ViewHandle<EditorView>,
    new_tab_working_directory_dropdown: ViewHandle<Dropdown<WorkingDirectoryAction>>,
    new_tab_working_directory_editor: ViewHandle<EditorView>,
    split_pane_working_directory_dropdown: ViewHandle<Dropdown<WorkingDirectoryAction>>,
    split_pane_working_directory_editor: ViewHandle<EditorView>,
}

impl WorkingDirectoryView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let working_directory_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            init_top_level_dropdown(&mut dropdown, ctx);
            dropdown
        });
        let working_directory_editor = create_editor(None, ctx);

        let new_window_working_directory_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            init_per_source_dropdown(&mut dropdown, NewSessionSource::Window, ctx);
            dropdown
        });
        let new_window_working_directory_editor =
            create_editor(Some(NewSessionSource::Window), ctx);

        let new_tab_working_directory_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            init_per_source_dropdown(&mut dropdown, NewSessionSource::Tab, ctx);
            dropdown
        });
        let new_tab_working_directory_editor = create_editor(Some(NewSessionSource::Tab), ctx);

        let split_pane_working_directory_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            init_per_source_dropdown(&mut dropdown, NewSessionSource::SplitPane, ctx);
            dropdown
        });
        let split_pane_working_directory_editor =
            create_editor(Some(NewSessionSource::SplitPane), ctx);

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                SessionSettingsChangedEvent::WorkingDirectoryConfig { .. }
            ) {
                me.working_directory_dropdown.update(ctx, |dropdown, ctx| {
                    init_top_level_dropdown(dropdown, ctx);
                    ctx.notify();
                });
                me.new_window_working_directory_dropdown
                    .update(ctx, |dropdown, ctx| {
                        init_per_source_dropdown(dropdown, NewSessionSource::Window, ctx);
                        ctx.notify();
                    });
                me.new_tab_working_directory_dropdown
                    .update(ctx, |dropdown, ctx| {
                        init_per_source_dropdown(dropdown, NewSessionSource::Tab, ctx);
                        ctx.notify();
                    });
                me.split_pane_working_directory_dropdown
                    .update(ctx, |dropdown, ctx| {
                        init_per_source_dropdown(dropdown, NewSessionSource::SplitPane, ctx);
                        ctx.notify();
                    });
                ctx.notify();
            }
        });

        Self {
            working_directory_dropdown,
            working_directory_editor,
            new_window_working_directory_dropdown,
            new_window_working_directory_editor,
            new_tab_working_directory_dropdown,
            new_tab_working_directory_editor,
            split_pane_working_directory_dropdown,
            split_pane_working_directory_editor,
        }
    }
}

impl Entity for WorkingDirectoryView {
    type Event = ();
}

impl View for WorkingDirectoryView {
    fn ui_name() -> &'static str {
        "WorkingDirectoryView"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let settings = SessionSettings::as_ref(app);
        let config = &settings.working_directory_config;

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(render_row(
            &self.working_directory_dropdown,
            &self.working_directory_editor,
            config.global.mode == WorkingDirectoryMode::CustomDir && !config.advanced_mode,
            appearance,
        ));

        if config.advanced_mode {
            let items = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_children([
                    ui_builder
                        .label(crate::i18n::tr_static(app, "New window"))
                        .build()
                        .finish(),
                    render_row(
                        &self.new_window_working_directory_dropdown,
                        &self.new_window_working_directory_editor,
                        config.new_window.mode == WorkingDirectoryMode::CustomDir,
                        appearance,
                    ),
                    ui_builder
                        .label(crate::i18n::tr_static(app, "New tab"))
                        .build()
                        .finish(),
                    render_row(
                        &self.new_tab_working_directory_dropdown,
                        &self.new_tab_working_directory_editor,
                        config.new_tab.mode == WorkingDirectoryMode::CustomDir,
                        appearance,
                    ),
                    ui_builder
                        .label(crate::i18n::tr_static(app, "Split pane"))
                        .build()
                        .finish(),
                    render_row(
                        &self.split_pane_working_directory_dropdown,
                        &self.split_pane_working_directory_editor,
                        config.split_pane.mode == WorkingDirectoryMode::CustomDir,
                        appearance,
                    ),
                ])
                .finish();
            column.add_child(
                Container::new(render_group(
                    [Container::new(items)
                        .with_margin_top(4.)
                        .with_margin_bottom(2.)
                        .finish()],
                    appearance,
                ))
                .with_margin_top(8.)
                .finish(),
            );
        }

        column.finish()
    }
}

impl TypedActionView for WorkingDirectoryView {
    type Action = WorkingDirectoryAction;

    fn handle_action(&mut self, action: &WorkingDirectoryAction, ctx: &mut ViewContext<Self>) {
        use WorkingDirectoryAction::*;

        match action {
            SetGlobalWorkingDirectoryMode(mode) => {
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.working_directory_config.update_and_save_value(
                        |config| {
                            if let Some(mode) = mode {
                                config.advanced_mode = false;
                                config.global.mode = *mode;
                            } else {
                                config.advanced_mode = true;
                            }
                        },
                        ctx,
                    ));
                });

                send_telemetry_from_ctx!(
                    TelemetryEvent::InitialWorkingDirectoryConfigurationChanged {
                        advanced_mode_enabled: mode.is_none()
                    },
                    ctx
                );

                // Redraw settings in case we switched in or out of advanced mode.
                ctx.notify();
            }
            SetPerSourceWorkingDirectoryMode(source, mode) => {
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.working_directory_config.update_and_save_value(
                        |config| match source {
                            NewSessionSource::SplitPane => config.split_pane.mode = *mode,
                            NewSessionSource::Tab => config.new_tab.mode = *mode,
                            NewSessionSource::Window => config.new_window.mode = *mode,
                        },
                        ctx,
                    ));
                });
                // Redraw settings in case we changed a mode to/from "custom directory".
                ctx.notify();
            }
            SetCustomWorkingDirectoryValue(source, value) => {
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.working_directory_config.update_and_save_value(
                        |config| match source {
                            Some(NewSessionSource::SplitPane) => {
                                config.split_pane.custom_dir.clone_from(value)
                            }
                            Some(NewSessionSource::Tab) => {
                                config.new_tab.custom_dir.clone_from(value)
                            }
                            Some(NewSessionSource::Window) => {
                                config.new_window.custom_dir.clone_from(value)
                            }
                            None => config.global.custom_dir.clone_from(value),
                        },
                        ctx,
                    ));
                });
            }
        }
    }
}

/// Render a single row, containing a dropdown view and editor.
///
/// `show_editor` controls whether the editor is currently visible.
fn render_row(
    dropdown: &ViewHandle<Dropdown<WorkingDirectoryAction>>,
    editor: &ViewHandle<EditorView>,
    show_editor: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    row.add_child(ChildView::new(dropdown).finish());
    if show_editor {
        row.add_child(
            Shrinkable::new(
                1.,
                appearance
                    .ui_builder()
                    .text_input(editor.clone())
                    .with_style(UiComponentStyles {
                        height: Some(TOP_MENU_BAR_HEIGHT),
                        font_color: Some(pathfinder_color::ColorU::black()),
                        font_size: Some(appearance.ui_font_size()),
                        padding: Some(Coords::uniform(7.)),
                        margin: Some(Coords::default().left(8.).right(8.)),
                        background: Some(appearance.theme().surface_2().into()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish(),
        );
    }

    row.finish()
}

/// Initializes the top-level dropdown (global configuration vs. advanced mode).
fn init_top_level_dropdown(
    dropdown: &mut Dropdown<WorkingDirectoryAction>,
    ctx: &mut ViewContext<Dropdown<WorkingDirectoryAction>>,
) {
    let mut items = [
        WorkingDirectoryMode::HomeDir,
        WorkingDirectoryMode::PreviousDir,
        WorkingDirectoryMode::CustomDir,
    ]
    .into_iter()
    .map(|mode| {
        DropdownItem::new(
            mode.dropdown_item_label(),
            WorkingDirectoryAction::SetGlobalWorkingDirectoryMode(Some(mode)),
        )
    })
    .collect_vec();
    items.push(DropdownItem::new(
        "Advanced".to_string(),
        WorkingDirectoryAction::SetGlobalWorkingDirectoryMode(None),
    ));
    let advanced_item_index = items.len() - 1;
    dropdown.set_items(items, ctx);
    dropdown.set_top_bar_max_width(200.);

    let config = &SessionSettings::as_ref(ctx).working_directory_config;
    if config.advanced_mode {
        dropdown.set_selected_by_index(advanced_item_index, ctx);
    } else {
        dropdown.set_selected_by_name(config.global.mode.dropdown_item_label(), ctx);
    }
}

/// Initializes a dropdown that relates to a particular new session source.
fn init_per_source_dropdown(
    dropdown: &mut Dropdown<WorkingDirectoryAction>,
    source: NewSessionSource,
    ctx: &mut ViewContext<Dropdown<WorkingDirectoryAction>>,
) {
    let items = [
        WorkingDirectoryMode::HomeDir,
        WorkingDirectoryMode::PreviousDir,
        WorkingDirectoryMode::CustomDir,
    ]
    .into_iter()
    .map(|mode| {
        DropdownItem::new(
            mode.dropdown_item_label(),
            WorkingDirectoryAction::SetPerSourceWorkingDirectoryMode(source, mode),
        )
    })
    .collect_vec();
    dropdown.set_items(items, ctx);
    dropdown.set_top_bar_max_width(200.);

    let config = &SessionSettings::as_ref(ctx).working_directory_config;
    let source_config = match source {
        NewSessionSource::SplitPane => &config.split_pane,
        NewSessionSource::Tab => &config.new_tab,
        NewSessionSource::Window => &config.new_window,
    };
    dropdown.set_selected_by_name(source_config.mode.dropdown_item_label(), ctx);
}

/// Creates a new editor view for entering a custom initial directory path.
fn create_editor(
    source: Option<NewSessionSource>,
    ctx: &mut ViewContext<WorkingDirectoryView>,
) -> ViewHandle<EditorView> {
    let editor = {
        let appearance = Appearance::as_ref(ctx);
        let options = SingleLineEditorOptions {
            text: TextOptions::ui_font_size(appearance),
            ..Default::default()
        };
        ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(crate::i18n::tr_static(ctx, "Directory path"), ctx);
            editor
        })
    };
    let initial_value = {
        let config = &SessionSettings::as_ref(ctx).working_directory_config;
        let source_config = match source {
            None => &config.global,
            Some(NewSessionSource::SplitPane) => &config.split_pane,
            Some(NewSessionSource::Tab) => &config.new_tab,
            Some(NewSessionSource::Window) => &config.new_window,
        };
        source_config.custom_dir.clone()
    };
    editor.update(ctx, |editor, ctx| {
        editor.set_buffer_text(&initial_value, ctx);
    });
    let editor_handle = editor.clone();
    ctx.subscribe_to_view(&editor, move |me, _, event, ctx| match event {
        // If the user presses enter or focus moves out of the editor view,
        // update our configuration to match the current value.
        EditorEvent::Blurred | EditorEvent::Enter => {
            let editor_contents = editor_handle.as_ref(ctx).buffer_text(ctx);
            me.handle_action(
                &WorkingDirectoryAction::SetCustomWorkingDirectoryValue(source, editor_contents),
                ctx,
            );
        }
        _ => {}
    });
    editor
}
