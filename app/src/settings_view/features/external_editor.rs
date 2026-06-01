use std::cell::RefCell;
use std::collections::HashMap;

use settings::{Setting, ToggleableSetting};
use warp_core::features::FeatureFlag;
use warpui::elements::{Flex, MouseStateHandle, ParentElement};
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::appearance::Appearance;
use crate::server::telemetry::TelemetryEvent;
use crate::settings_view::settings_page::{
    render_body_item, render_dropdown_item, AdditionalInfo, LocalOnlyIconState, ToggleState,
};
use crate::util::file::external_editor::settings::{
    EditorChoice, EditorLayout, OpenCodePanelsFileEditor, OpenFileEditor, OpenFileLayout,
    PreferMarkdownViewer, PreferTabbedEditorView,
};
use crate::util::file::external_editor::{EditorSettings, SUPPORTED_EDITORS};
use crate::view_components::{Dropdown, DropdownItem};
use crate::{report_if_error, send_telemetry_from_ctx};

const TABBED_FILE_VIEWER_TOGGLE_HEADER: &str = "Group files into single editor pane";
const TABBED_FILE_VIEWER_TOGGLE_DESCRIPTION: &str = "When this setting is on, any files opened in the same tab will be automatically grouped into a single editor pane.";

#[derive(Debug, Clone, PartialEq)]
pub enum ExternalEditorAction {
    SetEditor(EditorChoice),
    SetCodePanelsEditor(EditorChoice),
    SetLayout(EditorLayout),
    TogglePreferMarkdownViewer,
    ToggleTabbedEditorView,
    OpenUrl(String),
}

pub struct ExternalEditorView {
    editor_dropdown: ViewHandle<Dropdown<ExternalEditorAction>>,
    code_panels_editor_dropdown: ViewHandle<Dropdown<ExternalEditorAction>>,
    layout_dropdown: ViewHandle<Dropdown<ExternalEditorAction>>,
    tabbed_editor_view_mouse_state: SwitchStateHandle,
    prefer_markdown_viewer_switch: SwitchStateHandle,
    markdown_viewer_mouse_state: MouseStateHandle,
    local_only_icon_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl ExternalEditorView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let settings = EditorSettings::handle(ctx);
        let editor_to_open_files = *settings.as_ref(ctx).open_file_editor;
        let code_panels_editor_to_open_files = *settings.as_ref(ctx).open_code_panels_file_editor;
        let layout_to_open_files = *settings.as_ref(ctx).open_file_layout;

        let editor_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            Self::init_editor_dropdown(
                &editor_to_open_files,
                &mut dropdown,
                ExternalEditorAction::SetEditor,
                ctx,
            );
            dropdown
        });
        let code_panels_editor_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            Self::init_editor_dropdown(
                &code_panels_editor_to_open_files,
                &mut dropdown,
                ExternalEditorAction::SetCodePanelsEditor,
                ctx,
            );
            dropdown
        });
        let layout_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            Self::init_layout_dropdown(&layout_to_open_files, &mut dropdown, ctx);
            dropdown
        });
        ctx.subscribe_to_model(
            &EditorSettings::handle(ctx),
            |me, editor_settings, _, ctx| {
                me.editor_dropdown.update(ctx, |dropdown, ctx| {
                    let editor = *editor_settings.as_ref(ctx).open_file_editor;
                    Self::init_editor_dropdown(
                        &editor,
                        dropdown,
                        ExternalEditorAction::SetEditor,
                        ctx,
                    );
                });
                me.code_panels_editor_dropdown.update(ctx, |dropdown, ctx| {
                    let editor = *editor_settings.as_ref(ctx).open_code_panels_file_editor;
                    Self::init_editor_dropdown(
                        &editor,
                        dropdown,
                        ExternalEditorAction::SetCodePanelsEditor,
                        ctx,
                    );
                });
                ctx.notify()
            },
        );

        Self {
            editor_dropdown,
            code_panels_editor_dropdown,
            layout_dropdown,
            tabbed_editor_view_mouse_state: Default::default(),
            prefer_markdown_viewer_switch: Default::default(),
            markdown_viewer_mouse_state: Default::default(),
            local_only_icon_states: Default::default(),
        }
    }

    fn init_layout_dropdown(
        layout_to_open_files: &EditorLayout,
        dropdown: &mut Dropdown<ExternalEditorAction>,
        ctx: &mut ViewContext<Dropdown<ExternalEditorAction>>,
    ) {
        let default_option_text = "Split Pane";
        let default_app = DropdownItem::new(
            default_option_text,
            ExternalEditorAction::SetLayout(EditorLayout::SplitPane),
        );

        let mut items = vec![default_app];
        items.push(DropdownItem::new(
            "New Tab",
            ExternalEditorAction::SetLayout(EditorLayout::NewTab),
        ));

        dropdown.set_items(items, ctx);
        match layout_to_open_files {
            EditorLayout::SplitPane => dropdown.set_selected_by_name(default_option_text, ctx),
            EditorLayout::NewTab => dropdown.set_selected_by_name("New Tab", ctx),
        };
    }

    fn init_editor_dropdown(
        editor_to_open_files: &EditorChoice,
        dropdown: &mut Dropdown<ExternalEditorAction>,
        mut make_action: impl FnMut(EditorChoice) -> ExternalEditorAction,
        ctx: &mut ViewContext<Dropdown<ExternalEditorAction>>,
    ) {
        let default_option_text = "Default App";
        let default_app = DropdownItem::new(
            default_option_text,
            make_action(EditorChoice::SystemDefault),
        );

        let mut items = vec![default_app];

        items.push(DropdownItem::new("Warp", make_action(EditorChoice::Warp)));
        if FeatureFlag::AllowOpeningFileLinksUsingEditorEnv.is_enabled() {
            items.push(DropdownItem::new(
                "$EDITOR",
                make_action(EditorChoice::EnvEditor),
            ));
        }
        for editor in SUPPORTED_EDITORS {
            if editor.is_installed(ctx) {
                let editor_name = format!("{editor}");
                items.push(DropdownItem::new(
                    editor_name,
                    make_action(EditorChoice::ExternalEditor(*editor)),
                ));
            }
        }

        dropdown.set_items(items, ctx);
        match editor_to_open_files {
            EditorChoice::ExternalEditor(editor) => {
                dropdown.set_selected_by_name(format!("{editor}"), ctx)
            }
            EditorChoice::Warp => dropdown.set_selected_by_name("Warp", ctx),
            EditorChoice::EnvEditor => dropdown.set_selected_by_name("$EDITOR", ctx),
            EditorChoice::SystemDefault => dropdown.set_selected_by_name(default_option_text, ctx),
        };
    }

    /// Handles [`ExternalEditorAction::SetEditor`] by updating the external editor settings.
    fn set_editor(&mut self, editor: &EditorChoice, ctx: &mut ViewContext<Self>) {
        EditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.open_file_editor.set_value(*editor, ctx));
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::FeaturesPageAction {
                action: "SetEditor".to_string(),
                value: format!("{editor:?}")
            },
            ctx
        );
    }

    fn set_code_panels_editor(&mut self, editor: &EditorChoice, ctx: &mut ViewContext<Self>) {
        EditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .open_code_panels_file_editor
                .set_value(*editor, ctx));
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::FeaturesPageAction {
                action: "SetCodePanelsEditor".to_string(),
                value: format!("{editor:?}")
            },
            ctx
        );
    }

    // Handles [`ExternalEditorAction::SetLayout`] by updating the external editor layout settings.
    fn set_layout(&mut self, layout: &EditorLayout, ctx: &mut ViewContext<Self>) {
        EditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.open_file_layout.set_value(*layout, ctx));
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::FeaturesPageAction {
                action: "SetLayout".to_string(),
                value: format!("{layout:?}")
            },
            ctx
        );
    }

    /// Handles [`ExternalEditorAction::TogglePreferMarkdownViewer`]
    /// preference.
    fn toggle_prefer_markdown_viewer(&mut self, ctx: &mut ViewContext<Self>) {
        let new_value = EditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            let new_value = settings.prefer_markdown_viewer.toggle_and_save_value(ctx);
            report_if_error!(new_value);
            new_value.unwrap_or(PreferMarkdownViewer::default_value())
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::FeaturesPageAction {
                action: "TogglePreferMarkdownViewer".to_string(),
                value: new_value.to_string()
            },
            ctx
        );
    }

    /// Handles [`ExternalEditorAction::TogglePreferTabbedEditorView`] by updating the tabbed file viewer preference.
    fn toggle_prefer_tabbed_editor_view(&mut self, ctx: &mut ViewContext<Self>) {
        let new_value = EditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            let new_value = settings
                .prefer_tabbed_editor_view
                .toggle_and_save_value(ctx);
            report_if_error!(new_value);
            new_value.unwrap_or(PreferTabbedEditorView::default_value())
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::FeaturesPageAction {
                action: "ToggleTabbedEditorView".to_string(),
                value: new_value.to_string()
            },
            ctx
        );
    }
}

impl Entity for ExternalEditorView {
    type Event = ();
}

impl View for ExternalEditorView {
    fn ui_name() -> &'static str {
        "ExternalEditorView"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);

        let default_editor = render_dropdown_item(
            appearance,
            "Choose an editor to open file links",
            None,
            None,
            LocalOnlyIconState::for_setting(
                OpenFileEditor::storage_key(),
                OpenFileEditor::sync_to_cloud(),
                &mut self.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &self.editor_dropdown,
        );

        let code_panels_editor = render_dropdown_item(
            appearance,
            "Choose an editor to open files from the code review panel, project explorer, and global search",
            None,
            None,
            LocalOnlyIconState::for_setting(
                OpenCodePanelsFileEditor::storage_key(),
                OpenCodePanelsFileEditor::sync_to_cloud(),
                &mut self.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &self.code_panels_editor_dropdown,
        );

        let default_layout = render_dropdown_item(
            appearance,
            "Choose a layout to open files in Warp",
            None,
            None,
            LocalOnlyIconState::for_setting(
                OpenFileLayout::storage_key(),
                OpenFileLayout::sync_to_cloud(),
                &mut self.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &self.layout_dropdown,
        );

        let mut column = Flex::column()
            .with_child(default_editor)
            .with_child(code_panels_editor)
            .with_child(default_layout);

        if FeatureFlag::TabbedEditorView.is_enabled() {
            column.add_child(render_body_item::<ExternalEditorAction>(
                TABBED_FILE_VIEWER_TOGGLE_HEADER.into(),
                None,
                LocalOnlyIconState::for_setting(
                    PreferTabbedEditorView::storage_key(),
                    PreferTabbedEditorView::sync_to_cloud(),
                    &mut self.local_only_icon_states.borrow_mut(),
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                appearance
                    .ui_builder()
                    .switch(self.tabbed_editor_view_mouse_state.clone())
                    .check(
                        *EditorSettings::as_ref(app)
                            .prefer_tabbed_editor_view
                            .value(),
                    )
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ExternalEditorAction::ToggleTabbedEditorView);
                    })
                    .finish(),
                Some(TABBED_FILE_VIEWER_TOGGLE_DESCRIPTION.into()),
            ));
        }

        column.add_child(render_body_item::<ExternalEditorAction>(
            "Open Markdown files in Warp's Markdown Viewer by default".to_string(),
            Some(AdditionalInfo {
                mouse_state: self.markdown_viewer_mouse_state.clone(),
                on_click_action: Some(ExternalEditorAction::OpenUrl(
                    "https://docs.warp.dev/terminal/more-features/markdown-viewer".to_string(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                PreferMarkdownViewer::storage_key(),
                PreferMarkdownViewer::sync_to_cloud(),
                &mut self.local_only_icon_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.prefer_markdown_viewer_switch.clone())
                .check(*EditorSettings::as_ref(app).prefer_markdown_viewer.value())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ExternalEditorAction::TogglePreferMarkdownViewer);
                })
                .finish(),
            None,
        ));

        column.finish()
    }
}

impl TypedActionView for ExternalEditorView {
    type Action = ExternalEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ExternalEditorAction::SetEditor(editor) => self.set_editor(editor, ctx),
            ExternalEditorAction::SetCodePanelsEditor(editor) => {
                self.set_code_panels_editor(editor, ctx)
            }
            ExternalEditorAction::SetLayout(layout) => self.set_layout(layout, ctx),
            ExternalEditorAction::TogglePreferMarkdownViewer => {
                self.toggle_prefer_markdown_viewer(ctx)
            }
            ExternalEditorAction::ToggleTabbedEditorView => {
                self.toggle_prefer_tabbed_editor_view(ctx);
            }
            ExternalEditorAction::OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
        }
    }
}
