use std::path::PathBuf;

use warp_core::ui::appearance::Appearance;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::{
    Align, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CrossAxisAlignment, Expanded, Flex, MainAxisAlignment, MainAxisSize, ParentElement,
    ScrollbarWidth,
};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::{AIFact, CloudAIFact, CloudAIFactModel};
use crate::cloud_object::{
    CloudObject, CloudObjectSyncStatus, GenericStringObjectFormat, JsonObjectType,
};
use crate::drive::CloudObjectTypeAndId;
use crate::network::NetworkStatus;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::server::ids::SyncId;
use crate::server::sync_queue::SyncQueue;
use crate::ui_components::icons::Icon;

pub mod rule;
pub mod rule_editor;
mod style;
use rule::*;
use rule_editor::*;

const OFFLINE_TEXT: &str = "You are offline. Some rules will be read only.";

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub enum AIFactPage {
    #[default]
    Rules,
    RuleEditor {
        sync_id: Option<SyncId>,
    },
}

impl std::fmt::Display for AIFactPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIFactPage::Rules => write!(f, "Rules"),
            AIFactPage::RuleEditor { .. } => write!(f, "Rule Editor"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AIFactViewEvent {
    Pane(PaneEvent),
    OpenSettings,
    OpenFile(LocalOrRemotePath),
    InitializeProject(PathBuf),
}

#[derive(Debug, Clone)]
pub enum AIFactViewAction {
    AddRule,
    UpdatePage(AIFactPage),
}

pub struct AIFactView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    current_page: AIFactPage,
    rule_view: ViewHandle<RuleView>,
    rule_editor_view: ViewHandle<RuleEditorView>,
    clipped_scroll_state: ClippedScrollStateHandle,
}

impl AIFactView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(HEADER_TEXT));

        let rule_view = ctx.add_typed_action_view(RuleView::new);
        ctx.subscribe_to_view(&rule_view, |me, _, event, ctx| {
            me.handle_rule_view_event(event, ctx);
        });

        let rule_editor_view = ctx.add_typed_action_view(RuleEditorView::new);
        ctx.subscribe_to_view(&rule_editor_view, |me, _, event, ctx| {
            me.handle_rule_editor_view_event(event, ctx);
        });

        Self {
            pane_configuration,
            focus_handle: None,
            rule_editor_view,
            rule_view,
            current_page: AIFactPage::default(),
            clipped_scroll_state: Default::default(),
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn current_page(&self) -> AIFactPage {
        self.current_page
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        match self.current_page {
            AIFactPage::Rules => ctx.focus(&self.rule_view),
            AIFactPage::RuleEditor { .. } => ctx.focus(&self.rule_editor_view),
        }
    }

    fn handle_rule_view_event(&mut self, event: &RuleViewEvent, ctx: &mut ViewContext<Self>) {
        match event {
            RuleViewEvent::AddRule => {
                self.update_page(AIFactPage::RuleEditor { sync_id: None }, ctx);
            }
            RuleViewEvent::Edit(sync_id) => {
                self.update_page(
                    AIFactPage::RuleEditor {
                        sync_id: Some(*sync_id),
                    },
                    ctx,
                );
            }
            RuleViewEvent::OpenSettings => {
                ctx.emit(AIFactViewEvent::OpenSettings);
            }
            RuleViewEvent::OpenFile(path) => {
                ctx.emit(AIFactViewEvent::OpenFile(path.clone()));
            }
            RuleViewEvent::InitializeProject(path) => {
                ctx.emit(AIFactViewEvent::InitializeProject(path.clone()));
            }
        }
    }

    fn handle_rule_editor_view_event(
        &mut self,
        event: &RuleEditorViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        self.update_page(AIFactPage::Rules, ctx);
        match event {
            RuleEditorViewEvent::Add { name, content } => {
                self.rule_view.update(ctx, |rule_view, ctx| {
                    rule_view.add_ai_rule(name.clone(), content.clone(), ctx);
                });
            }
            RuleEditorViewEvent::Edit {
                name,
                content,
                sync_id,
                revision_ts,
            } => {
                self.rule_view.update(ctx, |rule_view, ctx| {
                    rule_view.edit_ai_rule(
                        name.clone(),
                        content.clone(),
                        *sync_id,
                        revision_ts.clone(),
                        ctx,
                    );
                });
            }
            RuleEditorViewEvent::Delete { sync_id } => {
                self.rule_view.update(ctx, |rule_view, ctx| {
                    rule_view.delete_ai_rule(*sync_id, ctx);
                });
            }
            _ => {}
        }
    }

    pub fn update_page(&mut self, page: AIFactPage, ctx: &mut ViewContext<Self>) {
        self.current_page = page;
        if let AIFactPage::RuleEditor { sync_id } = page {
            self.rule_editor_view.update(ctx, |rule_editor_view, ctx| {
                rule_editor_view.set_ai_rule(sync_id, ctx);
            });
        }
        self.focus(ctx);
        ctx.notify();
    }

    fn render_offline_banner(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_child(
                    ConstrainedBox::new(
                        Icon::CloudOffline
                            .to_warpui_icon(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().surface_2()),
                            )
                            .finish(),
                    )
                    .with_width(style::ICON_SIZE)
                    .with_height(style::ICON_SIZE)
                    .finish(),
                )
                .with_child(
                    Expanded::new(
                        1.,
                        Container::new(
                            appearance
                                .ui_builder()
                                .wrappable_text(OFFLINE_TEXT, true)
                                .build()
                                .finish(),
                        )
                        .with_margin_left(style::ICON_MARGIN)
                        .finish(),
                    )
                    .finish(),
                )
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_background(appearance.theme().surface_2())
        .with_vertical_padding(4.)
        .with_horizontal_padding(style::PANE_PADDING)
        .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
        .finish()
    }
}

impl Entity for AIFactView {
    type Event = AIFactViewEvent;
}

impl View for AIFactView {
    fn ui_name() -> &'static str {
        "AIFactView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            match self.current_page {
                AIFactPage::Rules => ctx.focus(&self.rule_view),
                AIFactPage::RuleEditor { .. } => ctx.focus(&self.rule_editor_view),
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut col = Flex::column().with_main_axis_size(MainAxisSize::Min);
        if !is_online(app) {
            col.add_child(self.render_offline_banner(appearance));
        }
        match self.current_page {
            AIFactPage::Rules => col.add_child(ChildView::new(&self.rule_view).finish()),
            AIFactPage::RuleEditor { .. } => {
                col.add_child(ChildView::new(&self.rule_editor_view).finish())
            }
        }

        ClippedScrollable::vertical(
            self.clipped_scroll_state.clone(),
            Align::new(
                Container::new(
                    ConstrainedBox::new(col.finish())
                        .with_max_width(style::PANE_WIDTH)
                        .finish(),
                )
                .with_uniform_padding(style::PANE_PADDING)
                .finish(),
            )
            .top_center()
            .finish(),
            ScrollbarWidth::Auto,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish()
    }
}

impl TypedActionView for AIFactView {
    type Action = AIFactViewAction;

    fn handle_action(&mut self, action: &AIFactViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            AIFactViewAction::AddRule => {
                self.rule_editor_view.update(ctx, |rule_editor_view, ctx| {
                    rule_editor_view.set_ai_rule(None, ctx);
                });
                self.update_page(AIFactPage::RuleEditor { sync_id: None }, ctx);
            }
            AIFactViewAction::UpdatePage(page) => self.update_page(*page, ctx),
        }
    }
}

impl BackingView for AIFactView {
    type PaneHeaderOverflowMenuAction = AIFactViewAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut warpui::ViewContext<Self>,
    ) {
        self.handle_action(_action, _ctx)
    }

    fn close(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        ctx.emit(AIFactViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple(HEADER_TEXT)
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

pub fn is_online(app: &AppContext) -> bool {
    NetworkStatus::as_ref(app).is_online()
}

pub fn is_delete_allowed(ai_fact: CloudAIFact, app: &AppContext) -> bool {
    let cloud_object_type_and_id = CloudObjectTypeAndId::GenericStringObject {
        object_type: GenericStringObjectFormat::Json(JsonObjectType::AIFact),
        id: ai_fact.sync_id(),
    };
    is_online(app)
        && cloud_object_type_and_id.has_server_id()
        && !ai_fact.metadata().has_pending_online_only_change()
}

pub fn is_edit_allowed(ai_fact: CloudAIFact, app: &AppContext) -> bool {
    let cloud_object_type_and_id = CloudObjectTypeAndId::GenericStringObject {
        object_type: GenericStringObjectFormat::Json(JsonObjectType::AIFact),
        id: ai_fact.sync_id(),
    };
    is_online(app) || !cloud_object_type_and_id.has_server_id()
}

pub fn is_syncing(ai_fact: CloudAIFact, app: &AppContext) -> bool {
    let sync_queue_is_dequeueing = SyncQueue::as_ref(app).is_dequeueing();
    let sync_status = &ai_fact.metadata().pending_changes_statuses;
    let has_in_flight_requests = matches!(
        &sync_status.content_sync_status,
        CloudObjectSyncStatus::InFlight(reqs) if reqs.0 > 0
    );
    (has_in_flight_requests && sync_queue_is_dequeueing)
        || sync_status.has_pending_metadata_change
        || sync_status.has_pending_permissions_change
        || sync_status.pending_untrash
}
