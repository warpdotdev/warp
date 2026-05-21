use crate::ai::agent::SuggestedRule;
use crate::ai::facts::CloudAIFactModel;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::Owner;
use crate::drive::CloudObjectTypeAndId;
use crate::editor::{
    EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent, InteractionState,
    PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::modal::{Modal, ModalEvent};
use crate::network::NetworkStatus;
use crate::send_telemetry_from_ctx;
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, OperationSuccessType, UpdateManagerEvent,
};
use crate::server::ids::SyncId;
use crate::server::telemetry::TelemetryEvent;
use crate::view_components::action_button::{ActionButton, PrimaryTheme};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    ai::facts::{AIFact, AIMemory},
    server::cloud_objects::update_manager::UpdateManager,
    ui_components::blended_colors,
};
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    ChildAnchor, OffsetPositioning, PositionedElementAnchor, PositionedElementOffsetBounds,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::{
    elements::ClippedScrollStateHandle,
    ui_components::components::{Coords, UiComponentStyles},
};
use warpui::{
    elements::{
        Align, Border, ChildView, ClippedScrollable, ConstrainedBox, Container, CornerRadius, Flex,
        ParentElement, Radius, ScrollbarWidth,
    },
    ui_components::components::UiComponent,
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const HEADER_TEXT: &str = "Suggested rule";
const MAX_EDITOR_HEIGHT: f32 = 240.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        SuggestedRuleDialogAction::Close,
        id!(SuggestedRuleView::ui_name()),
    )]);
}

#[derive(Debug, Clone, Copy)]
enum EditorType {
    Name,
    Content,
}

#[derive(Debug, Clone)]
pub enum SuggestedRuleModalEvent {
    AddNewRule { rule: SuggestedRule },
    OpenRuleForEditing { rule: SuggestedRule },
    Close,
}

/// A modal component for displaying and managing suggested rules.
/// This component wraps a SuggestedRuleView in a modal dialog with proper styling
/// and event handling.
///
/// # Focus Management
/// - The modal automatically focuses the rule editor when opened
/// - Key events (like Escape) are handled for dismissing the modal
/// - Focus returns to the terminal/AI block when the modal is closed
/// - Tab navigation works between name and content editors
///
/// # Positioning
/// - The modal positions itself relative to the chip that triggered it
/// - Uses `OffsetPositioning` with the chip's saved position as the anchor
/// - The position is calculated based on the rule's unique identifier to maintain consistency
///
pub struct SuggestedRuleModal {
    modal: ViewHandle<Modal<SuggestedRuleView>>,
    view: ViewHandle<SuggestedRuleView>,
}

impl SuggestedRuleModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let background = blended_colors::neutral_2(appearance.theme());

        let view = ctx.add_typed_action_view(SuggestedRuleView::new);
        ctx.subscribe_to_view(&view, |me, _, event, ctx| {
            me.handle_view_event(event, ctx);
        });

        let view_handle = view.clone();
        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some(HEADER_TEXT.to_string()), view, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(510.),
                    background: Some(background.into()),
                    ..Default::default()
                })
                .with_header_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 8.,
                        bottom: 0.,
                        left: 24.,
                        right: 24.,
                    }),
                    font_size: Some(16.),
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 0.,
                        bottom: 24.,
                        left: 24.,
                        right: 24.,
                    }),
                    ..Default::default()
                })
                .with_background_opacity(100)
                .with_dismiss_on_click()
        });

        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_modal_event(event, ctx);
        });

        Self {
            modal,
            view: view_handle,
        }
    }

    pub fn set_rule_and_id(
        &mut self,
        rule_and_id: &SuggestedRuleAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.view.update(ctx, |view, ctx| {
            view.set_rule_and_id(rule_and_id, ctx);
        });
        self.modal.update(ctx, |modal, ctx| {
            modal.set_offset_positioning(OffsetPositioning::offset_from_save_position_element(
                format!("rule_position_{}", rule_and_id.rule.logging_id),
                vec2f(0., 0.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ));
            ctx.notify();
        });
        ctx.notify();
    }

    fn handle_view_event(&mut self, event: &SuggestedRuleDialogEvent, ctx: &mut ViewContext<Self>) {
        match event {
            SuggestedRuleDialogEvent::AddNewRule { rule } => {
                ctx.emit(SuggestedRuleModalEvent::AddNewRule { rule: rule.clone() })
            }
            SuggestedRuleDialogEvent::OpenRuleForEditing { rule } => {
                ctx.emit(SuggestedRuleModalEvent::OpenRuleForEditing { rule: rule.clone() })
            }
            SuggestedRuleDialogEvent::Close => ctx.emit(SuggestedRuleModalEvent::Close),
        }
    }

    fn handle_modal_event(&mut self, event: &ModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ModalEvent::Close => {
                ctx.emit(SuggestedRuleModalEvent::Close);
            }
        }
    }
}

impl Entity for SuggestedRuleModal {
    type Event = SuggestedRuleModalEvent;
}

impl View for SuggestedRuleModal {
    fn ui_name() -> &'static str {
        "SuggestedRuleModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.modal).finish()
    }
}

impl TypedActionView for SuggestedRuleModal {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

#[derive(Debug, Clone)]
enum SuggestedRuleDialogAction {
    Add,
    Edit,
    Close,
}

#[derive(Debug, Clone)]
pub enum SuggestedRuleDialogEvent {
    AddNewRule { rule: SuggestedRule },
    OpenRuleForEditing { rule: SuggestedRule },
    Close,
}

#[derive(Debug, Clone)]
pub struct SuggestedRuleAndId {
    pub rule: SuggestedRule,
    pub sync_id: SyncId,
}

struct SuggestedRuleView {
    rule_and_id: Option<SuggestedRuleAndId>,
    owner: Option<Owner>,
    is_saved: bool,
    current_editor: EditorType,
    name_editor: ViewHandle<EditorView>,
    content_editor: ViewHandle<EditorView>,
    add_button: ViewHandle<ActionButton>,
    edit_button: ViewHandle<ActionButton>,
    clipped_scroll_state: ClippedScrollStateHandle,
}

impl SuggestedRuleView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let update_manager = UpdateManager::handle(ctx);
        ctx.subscribe_to_model(&update_manager, |me, _, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });

        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, _, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        let owner = UserWorkspaces::as_ref(ctx).personal_drive(ctx);

        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, |me, _, _event, ctx| {
            let is_edit_allowed = me.is_edit_allowed(ctx);
            let tooltip = if !is_edit_allowed {
                Some("Editing is disabled while offline.".to_string())
            } else {
                None
            };
            me.edit_button.update(ctx, |edit_button, ctx| {
                edit_button.set_disabled(!is_edit_allowed, ctx);
                edit_button.set_tooltip(tooltip, ctx);
            });
            ctx.notify();
        });

        let appearance = Appearance::as_ref(ctx);
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let text = TextOptions {
            font_size_override: Some(font_size),
            font_family_override: Some(font_family),
            ..Default::default()
        };

        let name_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: text.clone(),
                    soft_wrap: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            )
        });
        ctx.subscribe_to_view(&name_editor, |me, _editor, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let content_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::new(
                EditorOptions {
                    text,
                    soft_wrap: true,
                    autogrow: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    supports_vim_mode: false,
                    single_line: false,
                    enter_settings: EnterSettings {
                        shift_enter: EnterAction::InsertNewLineIfMultiLine,
                        enter: EnterAction::InsertNewLineIfMultiLine,
                        alt_enter: EnterAction::InsertNewLineIfMultiLine,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ctx,
            )
        });
        ctx.subscribe_to_view(&content_editor, |me, _editor, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let add_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Add rule"), PrimaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(SuggestedRuleDialogAction::Add))
        });

        let edit_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Edit rule"), PrimaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(SuggestedRuleDialogAction::Edit))
        });

        Self {
            rule_and_id: None,
            owner,
            is_saved: false,
            current_editor: EditorType::Name,
            name_editor,
            content_editor,
            add_button,
            edit_button,
            clipped_scroll_state: Default::default(),
        }
    }

    pub fn set_rule_and_id(
        &mut self,
        rule_and_id: &SuggestedRuleAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.rule_and_id = Some(rule_and_id.clone());
        self.reset_rule(ctx);
        ctx.focus_self();
        ctx.notify();
    }

    pub fn is_edit_allowed(&self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(SuggestedRuleAndId { sync_id, .. }) = &self.rule_and_id else {
            return false;
        };

        let is_online = NetworkStatus::as_ref(ctx).is_online();
        is_online || sync_id.into_server().is_none()
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        let (current_editor, next_editor, next_editor_type) = match self.current_editor {
            EditorType::Name => (&self.name_editor, &self.content_editor, EditorType::Content),
            EditorType::Content => (&self.content_editor, &self.name_editor, EditorType::Name),
        };

        match event {
            EditorEvent::Escape => {
                ctx.emit(SuggestedRuleDialogEvent::Close);
            }
            EditorEvent::Focused => {
                self.current_editor = if self.name_editor.is_focused(ctx) {
                    EditorType::Name
                } else {
                    EditorType::Content
                };
            }
            EditorEvent::Edited(_) => {
                // todo this seems noisy?
                if let Some(SuggestedRuleAndId { rule, .. }) = &self.rule_and_id {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AISuggestedRuleContentChanged {
                            rule_id: rule.logging_id.clone(),
                            is_saved: self.is_saved
                        },
                        ctx
                    );
                }
            }
            EditorEvent::Navigate(NavigationKey::Tab)
            | EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.current_editor = next_editor_type;
                ctx.focus(next_editor);
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                current_editor.update(ctx, |editor, ctx| {
                    editor.move_up(ctx);
                });
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                current_editor.update(ctx, |editor, ctx| {
                    editor.move_down(ctx);
                });
            }
            _ => {}
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if let (ObjectOperation::Create { .. }, OperationSuccessType::Success) =
            (&result.operation, &result.success_type)
        {
            if let Some(rule_and_id) = &self.rule_and_id {
                if rule_and_id.sync_id.into_client() == result.client_id {
                    if let Some(server_id) = result.server_id {
                        self.rule_and_id = Some(SuggestedRuleAndId {
                            rule: rule_and_id.rule.clone(),
                            sync_id: SyncId::ServerId(server_id),
                        });
                        // Reload the rule from the cloud model.
                        self.load_rule(ctx);
                    }
                }
            }
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectUpdated {
                type_and_id: CloudObjectTypeAndId::GenericStringObject { id, .. },
                ..
            } => {
                if let Some(rule_and_id) = &self.rule_and_id {
                    if rule_and_id.sync_id.into_client() == id.into_client() {
                        self.load_rule(ctx);
                    }
                }
            }
            CloudModelEvent::ObjectTrashed {
                type_and_id: CloudObjectTypeAndId::GenericStringObject { id, .. },
                ..
            }
            | CloudModelEvent::ObjectDeleted {
                type_and_id: CloudObjectTypeAndId::GenericStringObject { id, .. },
                ..
            } => {
                // If the rule has been deleted, then we should reset the rule such that
                // the suggestion can be added again.
                if let Some(rule_and_id) = &self.rule_and_id {
                    if rule_and_id.sync_id == *id {
                        self.reset_rule(ctx);
                    }
                }
            }
            _ => {}
        }
    }

    /// Resets the rule to its initial state.
    fn reset_rule(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_saved = false;
        let name = self
            .rule_and_id
            .as_ref()
            .map(|rule_and_id| rule_and_id.rule.name.clone())
            .unwrap_or("".to_string());
        self.name_editor.update(ctx, |name_editor, ctx| {
            name_editor.set_buffer_text(&name, ctx);
            name_editor.set_interaction_state(InteractionState::Editable, ctx);
        });
        let content = self
            .rule_and_id
            .as_ref()
            .map(|rule_and_id| rule_and_id.rule.content.clone())
            .unwrap_or("".to_string());
        self.content_editor.update(ctx, |content_editor, ctx| {
            content_editor.set_buffer_text(&content, ctx);
            content_editor.set_interaction_state(InteractionState::Editable, ctx);
        });
        ctx.notify();
    }

    /// Fetches the rule from the cloud model, and updates the UI to reflect that.
    fn load_rule(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(SuggestedRuleAndId { sync_id, .. }) = &self.rule_and_id else {
            return;
        };

        let cloud_model = CloudModel::handle(ctx);
        if let Some(rule) = cloud_model
            .as_ref(ctx)
            .get_object_of_type::<GenericStringObjectId, CloudAIFactModel>(sync_id)
        {
            let AIFact::Memory(AIMemory { name, content, .. }) = rule.model().string_model.clone();
            self.name_editor.update(ctx, |name_editor, ctx| {
                name_editor.set_buffer_text(&name.unwrap_or("Untitled".to_string()), ctx);
            });
            self.content_editor.update(ctx, |content_editor, ctx| {
                content_editor.set_buffer_text(&content, ctx);
            });
            ctx.notify();
        }
    }

    pub fn add_rule(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(SuggestedRuleAndId { rule, sync_id }) = self.rule_and_id.clone() else {
            log::warn!("No rule to add in suggested rule dialog");
            return;
        };

        // Add rule as a WD object.
        let update_manager = UpdateManager::handle(ctx);
        let name = if self.name_editor.as_ref(ctx).buffer_text(ctx).is_empty() {
            None
        } else {
            Some(self.name_editor.as_ref(ctx).buffer_text(ctx).clone())
        };
        let content = self.content_editor.as_ref(ctx).buffer_text(ctx);
        if let Some(owner) = self.owner {
            let ai_fact = AIFact::Memory(AIMemory {
                is_autogenerated: false,
                name,
                content,
                suggested_logging_id: Some(rule.logging_id.clone()),
            });
            update_manager.update(ctx, |update_manager, ctx| {
                if let Some(client_id) = sync_id.into_client() {
                    update_manager.create_ai_fact(ai_fact, client_id, owner, ctx);
                }
            });
        }
        self.on_add_rule(ctx);
        ctx.emit(SuggestedRuleDialogEvent::AddNewRule { rule });
    }

    /// Updates the UI state to reflect that a rule has been added.
    fn on_add_rule(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_saved = true;
        self.name_editor.update(ctx, |name_editor, ctx| {
            name_editor.set_interaction_state(InteractionState::Disabled, ctx);
        });
        self.content_editor.update(ctx, |content_editor, ctx| {
            content_editor.set_interaction_state(InteractionState::Disabled, ctx);
        });
        ctx.notify();
    }

    fn render_label(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(appearance.ui_builder().span(text).build().finish())
            .with_margin_bottom(8.)
            .finish()
    }

    fn render_rule_form(&self, appearance: &Appearance) -> Box<dyn Element> {
        let editor_bg = blended_colors::neutral_4(appearance.theme());
        let editor_border =
            Border::all(1.).with_border_fill(blended_colors::neutral_2(appearance.theme()));
        let editor_corner_radius = CornerRadius::with_all(Radius::Pixels(4.));
        let editor_horizontal_padding = 16.;
        let editor_vertical_padding = 12.;
        let editor_margin = 16.;

        Flex::column()
            .with_child(self.render_label("Name".to_string(), appearance))
            .with_child(
                Container::new(ChildView::new(&self.name_editor).finish())
                    .with_background(editor_bg)
                    .with_border(editor_border)
                    .with_corner_radius(editor_corner_radius)
                    .with_vertical_padding(editor_vertical_padding)
                    .with_horizontal_padding(editor_horizontal_padding)
                    .with_margin_bottom(editor_margin)
                    .finish(),
            )
            .with_child(self.render_label("Rule".to_string(), appearance))
            .with_child(
                ConstrainedBox::new(
                    Container::new(
                        ClippedScrollable::vertical(
                            self.clipped_scroll_state.clone(),
                            ChildView::new(&self.content_editor).finish(),
                            ScrollbarWidth::Auto,
                            appearance.theme().nonactive_ui_detail().into(),
                            appearance.theme().active_ui_detail().into(),
                            warpui::elements::Fill::None,
                        )
                        .finish(),
                    )
                    .with_background(editor_bg)
                    .with_border(editor_border)
                    .with_corner_radius(editor_corner_radius)
                    .with_padding_left(editor_horizontal_padding)
                    .with_vertical_padding(editor_vertical_padding)
                    .with_margin_bottom(editor_margin)
                    .finish(),
                )
                .with_max_height(MAX_EDITOR_HEIGHT)
                .finish(),
            )
            .finish()
    }
}

impl Entity for SuggestedRuleView {
    type Event = SuggestedRuleDialogEvent;
}

impl View for SuggestedRuleView {
    fn ui_name() -> &'static str {
        "SuggestedDialog"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            if self.is_saved {
                ctx.focus_self();
            } else {
                match self.current_editor {
                    EditorType::Name => ctx.focus(&self.name_editor),
                    EditorType::Content => ctx.focus(&self.content_editor),
                }
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let add_edit_button = if self.is_saved {
            &self.edit_button
        } else {
            &self.add_button
        };

        Flex::column()
            .with_child(self.render_rule_form(appearance))
            .with_child(
                Container::new(
                    Align::new(ChildView::new(add_edit_button).finish())
                        .right()
                        .finish(),
                )
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for SuggestedRuleView {
    type Action = SuggestedRuleDialogAction;

    fn handle_action(&mut self, action: &SuggestedRuleDialogAction, ctx: &mut ViewContext<Self>) {
        match action {
            SuggestedRuleDialogAction::Add => {
                self.add_rule(ctx);
            }
            SuggestedRuleDialogAction::Edit => {
                if let Some(SuggestedRuleAndId { rule, .. }) = &self.rule_and_id {
                    ctx.emit(SuggestedRuleDialogEvent::OpenRuleForEditing { rule: rule.clone() });
                } else {
                    log::warn!("No rule to edit in suggested rule dialog");
                }
            }
            SuggestedRuleDialogAction::Close => {
                ctx.emit(SuggestedRuleDialogEvent::Close);
            }
        }
    }
}
