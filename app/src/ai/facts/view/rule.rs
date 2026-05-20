use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{
    CloudObject, GenericStringObjectFormat, JsonObjectType, Owner, Revision,
};
use crate::drive::CloudObjectTypeAndId;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::network::NetworkStatus;
use crate::search_bar::SearchBar;
use crate::server::cloud_objects::update_manager::{UpdateManager, UpdateManagerEvent};
use crate::server::ids::{ClientId, SyncId};
use crate::server::sync_queue::SyncQueue;
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::ui_components::icons::Icon;
use crate::view_components::{
    action_button::{ActionButton, NakedTheme},
    DismissibleToast,
};
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;
use ai::project_context::model::{ProjectContextModel, ProjectContextModelEvent};
use markdown_parser::{
    weight::CustomWeight, FormattedText, FormattedTextFragment, FormattedTextLine,
};
use std::fmt::Debug;
use std::path::PathBuf;
use warp_core::ui::{
    appearance::{Appearance, AppearanceEvent},
    theme::color::internal_colors,
};
use warpui::elements::Shrinkable;
use warpui::platform::FilePickerConfiguration;
use warpui::ui_components::button::ButtonVariant;
use warpui::{
    elements::{
        Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Expanded, Flex, FormattedTextElement, HighlightedHyperlink, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, ParentElement,
    },
    platform::Cursor,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::{is_edit_allowed, is_syncing, style, AIFact, CloudAIFact, CloudAIFactModel};
use crate::ai::facts::AIMemory;

pub const HEADER_TEXT: &str = "Rules";
const DESCRIPTION_TEXT: &str = "Rules enhance the agent by providing structured guidelines that help maintain consistency, enforce best practices, and adapt to specific workflows, including codebases or broader tasks.";

const SEARCH_PLACEHOLDER_TEXT: &str = "Search rules";
const ZERO_STATE_TEXT: &str =
    "Add a rule above, or drop one at ~/.agents/AGENTS.md to apply it across every project.";
const ZERO_STATE_TEXT_PROJECT: &str =
    "Once you generate a WARP.md rules file for a project, it will appear here.";

const DISABLED_BANNER_TEXT: &str =
    "Your rules are disabled and won't be used as context in sessions. You can ";
const DISABLED_BANNER_LINK_TEXT: &str = "turn it back on";
const DISABLED_BANNER_TEXT_2: &str = " anytime.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    Global,
    ProjectBased,
}

#[derive(Debug, Clone)]
pub enum RuleViewEvent {
    AddRule,
    Edit(SyncId),
    OpenSettings,
    OpenFile(PathBuf),
    InitializeProject(PathBuf),
}

#[derive(Debug, Clone)]
pub enum RuleViewAction {
    AddRule,
    InitializeProject,
    Edit(SyncId),
    OpenSettings,
    SelectScope(RuleScope),
    OpenFile(PathBuf),
}

#[derive(Default, Debug, Clone)]
pub struct MouseStateHandles {
    pub hover: MouseStateHandle,
    pub sync_status_hover: MouseStateHandle,
    pub sync_status_icon: MouseStateHandle,
}

#[derive(Debug, Clone)]
struct CloudRuleRow {
    fact: CloudAIFact,
    mouse_states: MouseStateHandles,
}

/// A rule row backed by a file on disk — used for both project-scoped rules
/// (e.g. `<repo>/WARP.md`) and file-based global rules (e.g.
/// `~/.agents/AGENTS.md`). The render path is identical for both: a path label
/// plus an "Open file" button.
#[derive(Debug, Clone)]
struct FileBackedRow {
    file_path: PathBuf,
    mouse_state: MouseStateHandle,
}

#[derive(Debug, Clone)]
enum RuleRow {
    Global(Box<CloudRuleRow>),
    FileBacked(FileBackedRow),
}

impl RuleRow {
    fn matches_search_term(&self, search_term: &str) -> bool {
        let search_term = search_term.to_lowercase();
        let search_term = search_term.as_str();
        match self {
            RuleRow::Global(row) => {
                let AIFact::Memory(AIMemory { name, content, .. }) =
                    row.fact.model().string_model.clone();
                name.unwrap_or_default()
                    .to_lowercase()
                    .contains(search_term)
                    || content.to_lowercase().contains(search_term)
            }
            RuleRow::FileBacked(row) => row
                .file_path
                .to_str()
                .map(|s| s.to_lowercase().contains(search_term))
                .unwrap_or(false),
        }
    }

    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (RuleRow::Global(a), RuleRow::Global(b)) => {
                b.fact.metadata().revision.cmp(&a.fact.metadata().revision)
            }
            (RuleRow::FileBacked(a), RuleRow::FileBacked(b)) => a.file_path.cmp(&b.file_path),
            _ => std::cmp::Ordering::Equal,
        }
    }
}

pub struct RuleView {
    owner: Option<Owner>,
    cloud_global_rules: Vec<CloudRuleRow>,
    /// File-based global rules (e.g. `~/.agents/AGENTS.md`). Surfaced in the
    /// Global tab alongside cloud rules. Sourced from
    /// `ProjectContextModel::global_rule_paths()`.
    file_backed_global_rules: Vec<FileBackedRow>,
    project_rules: Vec<FileBackedRow>,
    search_editor: ViewHandle<EditorView>,
    search_bar: ViewHandle<SearchBar>,
    add_button: ViewHandle<ActionButton>,
    initialize_button: ViewHandle<ActionButton>,
    disabled_banner_highlight_index: HighlightedHyperlink,
    current_scope: RuleScope,
    global_tab_mouse_state: MouseStateHandle,
    project_tab_mouse_state: MouseStateHandle,
}

impl RuleView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let update_manager = UpdateManager::handle(ctx);
        ctx.subscribe_to_model(&update_manager, |me, _, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });

        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, _, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, |_me, _, _event, ctx| {
            ctx.notify();
        });

        let owner = UserWorkspaces::as_ref(ctx).personal_drive(ctx);

        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::MemoryEnabled { .. }
                    | AISettingsChangedEvent::IsAnyAIEnabled { .. }
            ) {
                ctx.notify();
            }
        });

        let ai_rules: Vec<CloudAIFact> = {
            let cloud_model = CloudModel::handle(ctx);
            cloud_model
                .as_ref(ctx)
                .get_all_objects_of_type::<GenericStringObjectId, CloudAIFactModel>()
                .cloned()
                .collect()
        };
        let ai_rules: Vec<CloudRuleRow> = ai_rules
            .into_iter()
            .map(|fact| CloudRuleRow {
                fact,
                mouse_states: Default::default(),
            })
            .collect();

        let project_context = ProjectContextModel::handle(ctx);
        let project_rules = project_context
            .as_ref(ctx)
            .indexed_rules()
            .map(|p| FileBackedRow {
                file_path: p,
                mouse_state: Default::default(),
            })
            .collect();
        let file_backed_global_rules = project_context
            .as_ref(ctx)
            .global_rule_paths()
            .map(|p| FileBackedRow {
                file_path: p,
                mouse_state: Default::default(),
            })
            .collect();

        ctx.subscribe_to_model(
            &project_context,
            |me, context_model, event, ctx| match event {
                // Upon indexing a new path, update project rules.
                ProjectContextModelEvent::PathIndexed => {
                    me.project_rules = context_model
                        .as_ref(ctx)
                        .indexed_rules()
                        .map(|p| FileBackedRow {
                            file_path: p,
                            mouse_state: Default::default(),
                        })
                        .collect();
                    ctx.notify();
                }
                // On detecting a change to global rule files, update file-backed global rules.
                ProjectContextModelEvent::GlobalRulesChanged(_) => {
                    me.file_backed_global_rules = context_model
                        .as_ref(ctx)
                        .global_rule_paths()
                        .map(|p| FileBackedRow {
                            file_path: p,
                            mouse_state: Default::default(),
                        })
                        .collect();
                    ctx.notify();
                }
                // No action needed for other ProjectContextModelEvent variants.
                // PathIndexed is emitted last and indicates it's time to refresh the UI.
                ProjectContextModelEvent::KnownRulesChanged(_) => {}
            },
        );

        let appearance = Appearance::handle(ctx);
        ctx.subscribe_to_model(&appearance, move |me, _, event, ctx| {
            if let AppearanceEvent::ThemeChanged = event {
                let appearance = Appearance::as_ref(ctx);
                let search_bar_styles = style::search_bar(appearance);
                me.search_bar.update(ctx, |search_bar, _| {
                    search_bar.with_style(search_bar_styles)
                });
            }
        });

        let search_editor_text = TextOptions::ui_text(None, appearance.as_ref(ctx));
        let search_editor = {
            let options = SingleLineEditorOptions {
                text: search_editor_text,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            ctx.add_typed_action_view(|ctx| EditorView::single_line(options, ctx))
        };
        ctx.subscribe_to_view(&search_editor, move |me, _, event, ctx| {
            me.handle_search_editor_event(event, ctx);
        });

        search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text(SEARCH_PLACEHOLDER_TEXT, ctx);
        });
        let search_bar = ctx.add_typed_action_view(|_| SearchBar::new(search_editor.clone()));

        let add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add", NakedTheme)
                .with_icon(Icon::Plus)
                .on_click(|ctx| ctx.dispatch_typed_action(RuleViewAction::AddRule))
        });

        let initialize_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Initialize Project", NakedTheme)
                .with_icon(Icon::Plus)
                .on_click(|ctx| ctx.dispatch_typed_action(RuleViewAction::InitializeProject))
        });

        Self {
            owner,
            cloud_global_rules: ai_rules,
            file_backed_global_rules,
            project_rules,
            search_editor,
            search_bar,
            add_button,
            initialize_button,
            disabled_banner_highlight_index: Default::default(),
            current_scope: RuleScope::Global,
            global_tab_mouse_state: Default::default(),
            project_tab_mouse_state: Default::default(),
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let UpdateManagerEvent::ObjectOperationComplete { .. } = event {
            self.fetch_ai_rules(ctx);
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectUpdated { .. }
            | CloudModelEvent::ObjectTrashed { .. }
            | CloudModelEvent::ObjectUntrashed { .. }
            | CloudModelEvent::ObjectCreated { .. }
            | CloudModelEvent::ObjectDeleted { .. } => {
                self.fetch_ai_rules(ctx);
            }
            _ => {}
        }
    }

    fn handle_search_editor_event(&mut self, _event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn fetch_ai_rules(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_rules: Vec<CloudAIFact> = {
            let cloud_model = CloudModel::handle(ctx);
            cloud_model
                .as_ref(ctx)
                .get_all_objects_of_type::<GenericStringObjectId, CloudAIFactModel>()
                .cloned()
                .collect()
        };
        self.cloud_global_rules = ai_rules
            .into_iter()
            .map(|ai_fact| CloudRuleRow {
                fact: ai_fact,
                mouse_states: Default::default(),
            })
            .collect();
        ctx.notify();
    }

    fn select_scope(&mut self, scope: RuleScope, ctx: &mut ViewContext<Self>) {
        self.current_scope = scope;
        ctx.notify();
    }

    fn get_filtered_rules(&self) -> Vec<RuleRow> {
        match self.current_scope {
            RuleScope::Global => self
                .cloud_global_rules
                .iter()
                .cloned()
                .map(|rule| RuleRow::Global(Box::new(rule)))
                .chain(
                    self.file_backed_global_rules
                        .iter()
                        .cloned()
                        .map(RuleRow::FileBacked),
                )
                .collect(),
            RuleScope::ProjectBased => self
                .project_rules
                .iter()
                .cloned()
                .map(RuleRow::FileBacked)
                .collect(),
        }
    }

    pub fn add_ai_rule(
        &mut self,
        name: Option<String>,
        content: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let update_manager = UpdateManager::handle(ctx);
        if let Some(owner) = self.owner {
            let ai_fact = AIFact::Memory(AIMemory {
                is_autogenerated: false,
                name,
                content,
                suggested_logging_id: None,
            });
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.create_ai_fact(ai_fact, ClientId::default(), owner, ctx);
            });
        }
    }

    pub fn edit_ai_rule(
        &mut self,
        name: Option<String>,
        content: String,
        sync_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ViewContext<Self>,
    ) {
        let update_manager = UpdateManager::handle(ctx);
        let (is_autogenerated, suggested_logging_id) = CloudModel::as_ref(ctx)
            .get_object_of_type::<GenericStringObjectId, CloudAIFactModel>(&sync_id)
            .map(|ai_fact| {
                let AIFact::Memory(AIMemory {
                    is_autogenerated,
                    suggested_logging_id,
                    ..
                }) = ai_fact.model().string_model.clone();
                (is_autogenerated, suggested_logging_id)
            })
            .unwrap_or((false, None));
        update_manager.update(ctx, |update_manager, ctx| {
            let ai_fact = AIFact::Memory(AIMemory {
                is_autogenerated,
                name,
                content,
                suggested_logging_id,
            });
            update_manager.update_ai_fact(ai_fact, sync_id, revision_ts, ctx);
        });
    }

    pub fn delete_ai_rule(&mut self, id: SyncId, ctx: &mut ViewContext<Self>) {
        let update_manager = UpdateManager::handle(ctx);
        update_manager.update(ctx, |update_manager, ctx| {
            update_manager.delete_object_by_user(
                CloudObjectTypeAndId::GenericStringObject {
                    object_type: GenericStringObjectFormat::Json(JsonObjectType::AIFact),
                    id,
                },
                ctx,
            );
        });
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        warpui::elements::Icon::new(
                            Icon::BookOpen.into(),
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().background()),
                        )
                        .finish(),
                    )
                    .with_width(style::ICON_SIZE)
                    .with_height(style::ICON_SIZE)
                    .finish(),
                )
                .with_margin_right(style::ICON_MARGIN)
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .wrappable_text(HEADER_TEXT, true)
                    .with_style(style::header_text())
                    .build()
                    .finish(),
            )
            .finish()
    }

    fn render_description(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .wrappable_text(DESCRIPTION_TEXT, true)
                .with_style(style::description_text(appearance))
                .build()
                .finish(),
        )
        .with_vertical_margin(style::ITEM_BOTTOM_MARGIN)
        .finish()
    }

    fn render_scope_tabs(&self, appearance: &Appearance) -> Box<dyn Element> {
        let global_tab = Container::new(self.render_scope_tab(
            "Global",
            RuleScope::Global,
            appearance,
            self.global_tab_mouse_state.clone(),
        ))
        .with_padding_right(4.)
        .finish();
        let project_tab = self.render_scope_tab(
            "Project based",
            RuleScope::ProjectBased,
            appearance,
            self.project_tab_mouse_state.clone(),
        );

        Container::new(
            Flex::row()
                .with_child(global_tab)
                .with_child(project_tab)
                .finish(),
        )
        .with_margin_bottom(style::SECTION_MARGIN)
        .finish()
    }

    fn render_scope_tab(
        &self,
        title: &str,
        scope: RuleScope,
        appearance: &Appearance,
        mouse_state: MouseStateHandle,
    ) -> Box<dyn Element> {
        let is_selected = self.current_scope == scope;
        let text_color = if is_selected {
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
        } else {
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
        };
        let title_owned = title.to_string();

        Hoverable::new(mouse_state, move |state| {
            let mut container = Container::new(
                appearance
                    .ui_builder()
                    .wrappable_text(title_owned.clone(), true)
                    .with_style(UiComponentStyles {
                        font_size: Some(style::TEXT_FONT_SIZE),
                        font_color: Some(text_color.into()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_horizontal_padding(style::ROW_HORIZONTAL_PADDING)
            .with_vertical_padding(8.);

            if is_selected {
                container = container
                    .with_background(appearance.theme().surface_2())
                    .with_corner_radius(CornerRadius::with_all(warpui::elements::Radius::Pixels(
                        4.,
                    )));
            } else if state.is_hovered() {
                container = container
                    .with_background(appearance.theme().surface_1())
                    .with_corner_radius(CornerRadius::with_all(warpui::elements::Radius::Pixels(
                        4.,
                    )));
            }

            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(RuleViewAction::SelectScope(scope));
        })
        .finish()
    }

    fn render_add_button(&self) -> Box<dyn Element> {
        Container::new(
            ChildView::new(if self.current_scope == RuleScope::ProjectBased {
                &self.initialize_button
            } else {
                &self.add_button
            })
            .finish(),
        )
        .with_margin_left(style::SECTION_MARGIN)
        .finish()
    }

    fn render_disabled_banner(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut link = FormattedTextFragment::hyperlink(DISABLED_BANNER_LINK_TEXT, "Settings > AI");
        link.styles.weight = Some(CustomWeight::Bold);

        let formatted_text = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::bold(DISABLED_BANNER_TEXT),
                link,
                FormattedTextFragment::bold(DISABLED_BANNER_TEXT_2),
            ])]),
            style::SUBTEXT_FONT_SIZE,
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into(),
            self.disabled_banner_highlight_index.clone(),
        )
        .with_hyperlink_font_color(internal_colors::accent_fg_strong(appearance.theme()).into())
        .register_default_click_handlers(|_, ctx, _| {
            ctx.dispatch_typed_action(RuleViewAction::OpenSettings);
        });

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::Info
                                .to_warpui_icon(
                                    appearance
                                        .theme()
                                        .sub_text_color(appearance.theme().background()),
                                )
                                .finish(),
                        )
                        .with_width(style::BANNER_ICON_SIZE)
                        .with_height(style::BANNER_ICON_SIZE)
                        .finish(),
                    )
                    .with_margin_right(style::ROW_ICON_MARGIN)
                    .finish(),
                )
                .with_child(Expanded::new(1., formatted_text.finish()).finish())
                .finish(),
        )
        .with_background(appearance.theme().accent_overlay())
        .with_corner_radius(CornerRadius::with_all(warpui::elements::Radius::Pixels(4.)))
        .with_uniform_padding(style::BANNER_PADDING)
        .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
        .finish()
    }

    fn render_search_bar_row(&self, filtered_rules: &[RuleRow]) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Expanded::new(1., ChildView::new(&self.search_bar).finish()).finish());

        if !filtered_rules.is_empty() {
            row.add_child(self.render_add_button());
        }
        Container::new(row.finish())
            .with_margin_bottom(style::SECTION_MARGIN)
            .finish()
    }

    fn render_sync_status_icon(
        &self,
        ai_row: CloudRuleRow,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        // Don't show icon if the syncing is in progress.
        if is_syncing(ai_row.fact.clone(), app) {
            return None;
        }

        let item = ai_row.fact.to_warp_drive_item(appearance)?;
        let icon = item.sync_status_icon(
            SyncQueue::as_ref(app).is_dequeueing(),
            ai_row.mouse_states.sync_status_icon.clone(),
            appearance,
        )?;

        Some(
            Hoverable::new(ai_row.mouse_states.sync_status_hover.clone(), |state| {
                let mut container = Container::new(icon)
                    .with_border(Border::all(1.))
                    .with_uniform_padding(4.);
                if state.is_hovered() {
                    container = container
                        .with_background(appearance.theme().surface_2())
                        .with_border(
                            Border::all(1.).with_border_fill(appearance.theme().surface_3()),
                        );
                }
                container.with_margin_right(style::ROW_ICON_MARGIN).finish()
            })
            .finish(),
        )
    }

    fn render_file_backed_row(
        &self,
        project_row: FileBackedRow,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let row_name = project_row.file_path.to_str().map(|s| s.to_string())?;
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        row.add_child(
            Shrinkable::new(
                1.,
                appearance
                    .ui_builder()
                    .wrappable_text(row_name, true)
                    .with_style(style::fact_project_based_row_text(appearance))
                    .build()
                    .finish(),
            )
            .finish(),
        );

        let file_path = project_row.file_path.clone();
        row.add_child(
            appearance
                .ui_builder()
                .button(ButtonVariant::Outlined, project_row.mouse_state.clone())
                .with_text_label("Open file".to_string())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(RuleViewAction::OpenFile(file_path.clone()));
                })
                .finish(),
        );

        Some(
            Container::new(row.finish())
                .with_background(internal_colors::neutral_1(appearance.theme()))
                .with_corner_radius(CornerRadius::with_all(warpui::elements::Radius::Pixels(4.)))
                .with_border(
                    Border::all(1.)
                        .with_border_color(internal_colors::neutral_2(appearance.theme())),
                )
                .with_horizontal_padding(style::ROW_HORIZONTAL_PADDING)
                .with_vertical_padding(style::RULE_VERTICAL_PADDING)
                .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
                .finish(),
        )
    }

    fn render_global_rule_row(
        &self,
        ai_row: CloudRuleRow,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let AIFact::Memory(AIMemory { name, content, .. }) =
            ai_row.fact.model().string_model.clone();
        let formatted_name = match name {
            Some(name) => {
                if name.is_empty() {
                    "Untitled".to_string()
                } else {
                    name
                }
            }
            None => "Untitled".to_string(),
        };
        // Truncate content to 3 lines
        let formatted_content = if content.split("\n").count() > 3 {
            content
                .split("\n")
                .take(3)
                .collect::<Vec<&str>>()
                .join("\n")
                + "..."
        } else {
            content
        };

        let fact_text = Flex::column()
            .with_child(
                appearance
                    .ui_builder()
                    .wrappable_text(formatted_name, true)
                    .with_style(style::fact_row_text(appearance))
                    .build()
                    .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .wrappable_text(formatted_content, true)
                    .with_style(style::fact_row_subtext(appearance))
                    .build()
                    .finish(),
            )
            .finish();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

        if let Some(sync_status_icon) =
            self.render_sync_status_icon(ai_row.clone(), appearance, app)
        {
            row.add_child(sync_status_icon);
        }

        row.add_child(Expanded::new(1., fact_text).finish());

        let mut hoverable = Hoverable::new(ai_row.mouse_states.hover.clone(), |state| {
            let mut bg_color = internal_colors::neutral_1(appearance.theme());
            if state.is_hovered() {
                bg_color = internal_colors::neutral_4(appearance.theme());
            }

            Container::new(row.finish())
                .with_background(bg_color)
                .with_corner_radius(CornerRadius::with_all(warpui::elements::Radius::Pixels(4.)))
                .with_border(
                    Border::all(1.)
                        .with_border_color(internal_colors::neutral_2(appearance.theme())),
                )
                .with_horizontal_padding(style::ROW_HORIZONTAL_PADDING)
                .with_vertical_padding(style::RULE_VERTICAL_PADDING)
                .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
                .finish()
        });

        if is_edit_allowed(ai_row.fact.clone(), app) {
            hoverable = hoverable
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(RuleViewAction::Edit(ai_row.fact.sync_id()));
                });
        }

        hoverable.finish()
    }

    fn render_items(
        &self,
        appearance: &Appearance,
        mut filtered_rules: Vec<RuleRow>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut col = Flex::column();

        // Filter the rows based on the search query
        let search_term = self.search_editor.as_ref(app).buffer_text(app);
        if !search_term.is_empty() {
            filtered_rules = filtered_rules
                .iter()
                .filter(|row| row.matches_search_term(search_term.as_str()))
                .cloned()
                .collect();
        }
        // Sort the rows by the last modified timestamp
        filtered_rules.sort_by(|a, b| a.cmp(b));

        for row in filtered_rules {
            let row = match row {
                RuleRow::Global(global_row) => {
                    Some(self.render_global_rule_row(*global_row, appearance, app))
                }
                RuleRow::FileBacked(file_row) => self.render_file_backed_row(file_row, appearance),
            };

            if let Some(row) = row {
                col.add_child(row);
            }
        }
        col.finish()
    }

    fn render_zero_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let text = match self.current_scope {
            RuleScope::Global => ZERO_STATE_TEXT,
            RuleScope::ProjectBased => ZERO_STATE_TEXT_PROJECT,
        };

        let centered_text = appearance
            .ui_builder()
            .wrappable_text(text, true)
            .with_style(style::description_text(appearance))
            .build()
            .finish();

        Container::new(
            ConstrainedBox::new(
                Align::new(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Container::new(Align::new(centered_text).top_center().finish())
                                .with_horizontal_padding(style::ROW_HORIZONTAL_PADDING)
                                .finish(),
                        )
                        .with_child(self.render_add_button())
                        .finish(),
                )
                .finish(),
            )
            .with_height(style::ZERO_STATE_HEIGHT)
            .finish(),
        )
        .with_horizontal_padding(style::ROW_HORIZONTAL_PADDING)
        .with_border(
            Border::all(1.).with_border_color(internal_colors::neutral_2(appearance.theme())),
        )
        .with_margin_bottom(style::SECTION_MARGIN)
        .finish()
    }

    fn render_body(
        &self,
        appearance: &Appearance,
        filtered_rules: Vec<RuleRow>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_child(self.render_search_bar_row(&filtered_rules))
            .with_child(self.render_items(appearance, filtered_rules, app))
            .finish()
    }
}

impl Entity for RuleView {
    type Event = RuleViewEvent;
}

impl View for RuleView {
    fn ui_name() -> &'static str {
        "RuleView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut col = Flex::column()
            .with_child(self.render_header(appearance))
            .with_child(self.render_description(appearance));

        col.add_child(self.render_scope_tabs(appearance));

        let ai_settings = AISettings::as_ref(app);
        if !ai_settings.is_memory_enabled(app) {
            col.add_child(self.render_disabled_banner(appearance));
        }

        let filtered_rules = self.get_filtered_rules();
        if filtered_rules.is_empty() {
            col.add_child(self.render_zero_state(appearance));
        } else {
            col.add_child(self.render_body(appearance, filtered_rules, app));
        };
        col.finish()
    }
}

impl TypedActionView for RuleView {
    type Action = RuleViewAction;

    fn handle_action(&mut self, action: &RuleViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            RuleViewAction::AddRule => {
                ctx.emit(RuleViewEvent::AddRule);
            }
            RuleViewAction::Edit(sync_id) => {
                ctx.emit(RuleViewEvent::Edit(*sync_id));
            }
            RuleViewAction::OpenSettings => {
                ctx.emit(RuleViewEvent::OpenSettings);
            }
            RuleViewAction::SelectScope(scope) => {
                self.select_scope(*scope, ctx);
            }
            RuleViewAction::OpenFile(path) => {
                ctx.emit(RuleViewEvent::OpenFile(path.clone()));
            }
            RuleViewAction::InitializeProject => {
                let file_picker_config = FilePickerConfiguration::new().folders_only();
                let window_id = ctx.window_id();

                ctx.open_file_picker(
                    move |result, ctx| match result {
                        Ok(paths) => {
                            if let Some(directory_path) = paths.first() {
                                let path = PathBuf::from(directory_path);
                                ctx.emit(RuleViewEvent::InitializeProject(path));
                            }
                        }
                        Err(err) => {
                            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                toast_stack.add_ephemeral_toast(
                                    DismissibleToast::error(format!("{err}")),
                                    window_id,
                                    ctx,
                                );
                            });
                        }
                    },
                    file_picker_config,
                );
            }
        }
    }
}
