#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::sync::Arc;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::document::ai_document_model::{AIDocumentSaveStatus, AIDocumentUserEditStatus};
use crate::ai::document::orchestration_config_block::OrchestrationConfigBlockView;
use crate::appearance::Appearance;
use crate::drive::{items::WarpDriveItemId, sharing::ShareableObject, CloudObjectTypeAndId};
use crate::notebooks::editor::view::RichTextEditorConfig;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::header::components::{
    render_pane_header_buttons, render_pane_header_title_text, render_three_column_header,
    CenteredHeaderEdgeWidth,
};
use crate::pane_group::pane::view::header::toolbelt_button_position_id;
use crate::pane_group::pane::view::header::PaneHeaderAction;
use crate::send_telemetry_from_ctx;
use crate::settings::FontSettings;
use crate::terminal::input::MenuPositioning;
use crate::terminal::view::TerminalView;
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::view_components::action_button::{
    ButtonSize, NakedTheme, SecondaryTheme, TooltipAlignment,
};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use crate::BlocklistAIHistoryModel;
use crate::{
    ai::document::ai_document_model::{
        AIDocumentId, AIDocumentInstance, AIDocumentModel, AIDocumentModelEvent,
        AIDocumentUpdateSource, AIDocumentVersion,
    },
    editor::InteractionState,
    menu::{Menu, MenuItem, MenuItemFields},
    notebooks::{
        editor::{
            model::NotebooksEditorModel,
            rich_text_styles,
            view::{EditorViewEvent, RichTextEditorView},
        },
        link::{NotebookLinks, SessionSource},
    },
    pane_group::{pane::view, BackingView, PaneConfiguration, PaneEvent},
    server::telemetry::TelemetryEvent,
    ui_components::buttons::icon_button,
    ui_components::icons::Icon,
    view_components::action_button::{ActionButton, PrimaryTheme},
};
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::icons;
use warp_core::ui::icons::ICON_DIMENSIONS;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::clipboard::ClipboardContent;
use warpui::elements::CrossAxisAlignment;
use warpui::elements::MainAxisAlignment;
use warpui::elements::MainAxisSize;
use warpui::elements::{ChildAnchor, PositionedElementAnchor, PositionedElementOffsetBounds};
use warpui::keymap::EditableBinding;
use warpui::keymap::FixedBinding;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonTooltipPosition;
use warpui::ui_components::components::UiComponent;
use warpui::{
    elements::{
        ChildView, ConstrainedBox, Container, Flex, Hoverable, MouseStateHandle, OffsetPositioning,
        ParentElement, SavePosition, Stack,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};
use warpui::{id, EntityId};

pub fn init(app: &mut AppContext) {
    app.register_editable_bindings([EditableBinding::new(
        // Reuse the save file keybinding name and description
        // so that there's only one entry in settings reused for both cases.
        SAVE_FILE_BINDING_NAME,
        SAVE_FILE_BINDING_DESCRIPTION,
        AIDocumentAction::SendUpdatedPlan,
    )
    .with_context_predicate(id!("AIDocumentView") & !id!("IMEOpen"))
    .with_key_binding("cmdorctrl-s")]);

    // Allow closing the AI document pane with the same toggle keybinding used in Terminal
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-alt-p",
        AIDocumentAction::Close,
        id!("AIDocumentView") & !id!("IMEOpen"),
    )]);
}

#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::settings::EditorLayout;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;
#[cfg(feature = "local_fs")]
use warp_util::path::LineAndColumnArg;

// Import keybinding constants from code view to ensure consistency
use crate::code::view::{SAVE_FILE_BINDING_DESCRIPTION, SAVE_FILE_BINDING_NAME};
use crate::notebooks::file::MarkdownDisplayMode;

#[derive(Debug, Clone, PartialEq)]
pub enum AIDocumentAction {
    Close,
    SelectVersion(AIDocumentVersion),
    Export,
    OpenVersionMenu,
    CreateWarpDriveNotebook,
    RevertToDocumentVersion,
    SendUpdatedPlan,
    CopyLink(String),
    CopyPlanId,
    ShowInWarpDrive,
    AttachToActiveSession,
}

#[derive(Debug, Clone)]
pub enum AIDocumentEvent {
    Pane(PaneEvent),
    CloseRequested,
    ViewInWarpDrive(WarpDriveItemId),
    #[cfg(feature = "local_fs")]
    OpenCodeInWarp {
        source: CodeSource,
        layout: EditorLayout,
        line_col: Option<LineAndColumnArg>,
    },
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: std::path::PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    AttachPlanAsContext(AIDocumentId),
}

impl From<PaneEvent> for AIDocumentEvent {
    fn from(event: PaneEvent) -> Self {
        AIDocumentEvent::Pane(event)
    }
}

pub const DEFAULT_PLANNING_DOCUMENT_TITLE: &str = "Planning document";

/// Entry for the version history dropdown menu.
struct VersionMenuEntry {
    version: AIDocumentVersion,
    created_at: chrono::DateTime<chrono::Local>,
    restored_from: Option<AIDocumentVersion>,
}

pub struct AIDocumentView {
    document_id: AIDocumentId,
    document_version: AIDocumentVersion,
    links: ModelHandle<NotebookLinks>,
    editor: ViewHandle<RichTextEditorView>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    original_terminal_view: Option<ViewHandle<TerminalView>>,
    // Version menu state
    version_menu: ViewHandle<Menu<AIDocumentAction>>,
    sync_button_mouse_state: MouseStateHandle,
    update_plan_button: ViewHandle<ActionButton>,
    restore_button: ViewHandle<ActionButton>,
    is_version_menu_open: bool,
    version_button_position_id: String,
    synced_status_mouse_state: MouseStateHandle,
    view_position_id: String,
    version_button: ViewHandle<ActionButton>,
    orchestration_config_block: Option<ViewHandle<OrchestrationConfigBlockView>>,
}

impl AIDocumentView {
    pub fn new(
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let window_id = ctx.window_id();

        let links = ctx.add_model(|ctx| NotebookLinks::new(SessionSource::Active(window_id), ctx));

        // Subscribe to document model updates so we can react to agent changes
        ctx.subscribe_to_model(
            &AIDocumentModel::handle(ctx),
            move |me, _, event, ctx| match event {
                AIDocumentModelEvent::DocumentUpdated {
                    document_id,
                    version,
                    source,
                } => {
                    // Only handle updates for our document.
                    // If the agent created a new version, auto update this view to the newest version.
                    if document_id == &me.document_id {
                        match source {
                            AIDocumentUpdateSource::Agent => {
                                me.document_version = *version;
                                me.refresh(ctx);
                            }
                            // Restoration is used for both persisted restore and
                            // shared-session viewer mirroring.
                            AIDocumentUpdateSource::Restoration => {
                                let is_shared_session_view = AIDocumentModel::as_ref(ctx)
                                    .get_conversation_id_for_document_id(document_id)
                                    .and_then(|conv_id| {
                                        BlocklistAIHistoryModel::as_ref(ctx)
                                            .conversation(&conv_id)
                                            .map(|c| c.is_viewing_shared_session())
                                    })
                                    .unwrap_or(false);

                                if is_shared_session_view {
                                    // For shared-session viewers mirrored updates represent the live truth,
                                    // so we always follow the latest version.
                                    me.document_version = *version;
                                    me.refresh(ctx);
                                } else if *version == me.document_version {
                                    // For normal persisted restoration, only refresh when restoration
                                    // targets the version this pane was opened for.
                                    me.document_version = *version;
                                    me.refresh(ctx);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                AIDocumentModelEvent::DocumentSaveStatusUpdated(id) => {
                    if *id != document_id {
                        return;
                    }
                    me.update_header_buttons(ctx);
                }
                AIDocumentModelEvent::DocumentUserEditStatusUpdated {
                    document_id: id,
                    status,
                } => {
                    if *id != document_id {
                        return;
                    }

                    // Auto-set pending document ID when document becomes dirty
                    if status.is_dirty() {
                        if let Some(terminal_view) = &me.original_terminal_view {
                            terminal_view.update(ctx, |terminal_view, ctx| {
                                terminal_view.ai_context_model().update(
                                    ctx,
                                    |context_model, ctx| {
                                        context_model.set_pending_document(Some(document_id), ctx);
                                    },
                                );
                            });
                        }
                    }

                    me.update_header_buttons(ctx);
                }
                AIDocumentModelEvent::StreamingDocumentsCleared(_) => {
                    me.refresh(ctx);
                }
                AIDocumentModelEvent::DocumentVisibilityChanged(_) => {}
            },
        );

        // Subscribe to conversation status changes to update buttons
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |me, _, event, ctx| {
                use crate::ai::blocklist::BlocklistAIHistoryEvent;
                match event {
                    BlocklistAIHistoryEvent::UpdatedConversationStatus {
                        terminal_view_id, ..
                    } => {
                        // Check if this is our terminal view
                        if let Some(tv) = &me.original_terminal_view {
                            if tv.id() == *terminal_view_id {
                                me.update_header_buttons(ctx);
                            }
                        }
                    }
                    BlocklistAIHistoryEvent::RestoredConversations {
                        terminal_view_id,
                        conversation_ids,
                    } => {
                        // Try to populate terminal view if conversations were restored
                        me.maybe_populate_terminal_view(*terminal_view_id, conversation_ids, ctx);
                    }
                    BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                        conversation_id: cid,
                        from_restore,
                    } => {
                        let our_conv = AIDocumentModel::as_ref(ctx)
                            .get_conversation_id_for_document_id(&document_id);
                        if our_conv.as_ref() == Some(cid) {
                            // Lazily create the config block view if the
                            // plan sidebar opened before the orchestration
                            // config arrived.
                            let was_freshly_created = if me.orchestration_config_block.is_none() {
                                let conv_id = *cid;
                                // TODO: introduce DocumentId / PlanId newtypes to make this
                                // conversion type-safe.
                                let plan_id = document_id.to_string();
                                me.orchestration_config_block =
                                    Some(ctx.add_typed_action_view(move |ctx| {
                                        OrchestrationConfigBlockView::new(conv_id, plan_id, ctx)
                                    }));
                                true
                            } else {
                                false
                            };
                            // Arm auto-pop for live agent dispatches but
                            // not for restore-hydrated events.
                            if was_freshly_created && !*from_restore {
                                if let Some(block) = &me.orchestration_config_block {
                                    block.update(ctx, |block, ctx| {
                                        block.arm_for_fresh_dispatch(ctx);
                                    });
                                }
                            }
                            ctx.notify();
                        }
                    }
                    _ => {}
                }
            },
        );

        let view_position_id = format!("ai_document_view_{}", ctx.view_id());

        // Get the initial editor model, or create an empty one if the document isn't found
        let initial_editor_model = AIDocumentModel::as_ref(ctx)
            .get_document(&document_id, document_version)
            .map(|doc| doc.get_editor())
            .unwrap_or_else(|| {
                log::warn!("AI document {document_id} version {document_version} not found in model, using empty editor");
                ctx.add_model(|ctx| {
                    let appearance = Appearance::as_ref(ctx);
                    let font_settings = FontSettings::as_ref(ctx);
                    let styles = rich_text_styles(appearance, font_settings);
                    let mut model = NotebooksEditorModel::new_unbound(styles, ctx);
                    model.set_default_mermaid_display_mode(MarkdownDisplayMode::Rendered, ctx);
                    model
                })
            });

        let editor = ctx.add_typed_action_view(|ctx| {
            RichTextEditorView::new(
                view_position_id.clone(),
                initial_editor_model.clone(),
                links.clone(),
                RichTextEditorConfig::default(),
                ctx,
            )
        });

        editor.update(ctx, |editor_view, ctx| {
            editor_view
                .model()
                .update(ctx, |model, ctx| model.set_window_id(window_id, ctx));
        });

        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let document_title = AIDocumentModel::as_ref(ctx)
            .get_document(&document_id, document_version)
            .map(|doc| doc.get_title())
            .unwrap_or_else(|| DEFAULT_PLANNING_DOCUMENT_TITLE.to_string());
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(document_title));

        // Create version menu view and subscribe to close events to hide overlay
        let version_menu = ctx.add_typed_action_view(|_| Menu::new().with_width(220.));
        ctx.subscribe_to_view(&version_menu, |me, _, event, ctx| {
            if let crate::menu::Event::Close { .. } = event {
                me.is_version_menu_open = false;
                me.version_menu
                    .update(ctx, |menu, ctx| menu.reset_selection(ctx));
                ctx.notify();
            }
        });

        // Anchor overlay to the toolbelt "Show version history" button
        let version_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(icons::Icon::History)
                .with_size(ButtonSize::Small)
                .with_tooltip(crate::i18n::tr_static(ctx, "Show version history"))
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        PaneHeaderAction::<AIDocumentAction, AIDocumentAction>::CustomAction(
                            AIDocumentAction::OpenVersionMenu,
                        ),
                    );
                })
        });
        let version_button_position_id =
            toolbelt_button_position_id(&pane_configuration, version_button.id());

        pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.refresh_pane_header_overflow_menu_items(ctx)
        });

        // Create sync button mouse state (for Warp Drive syncing)
        let sync_button_mouse_state = MouseStateHandle::default();

        // Create Update Agent button
        // Read the actual configured keybinding for the save action
        let save_action = keybinding_name_to_keystroke(SAVE_FILE_BINDING_NAME, ctx)
            .map(|k| k.displayed())
            .unwrap_or("Click".to_string());
        let tooltip_text = format!("This plan has changes the agent isn't aware of. {save_action} to stop the agent's current task and send the updated plan");
        let update_plan_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Update Agent"), PrimaryTheme)
                .with_size(ButtonSize::Small)
                .with_tooltip(tooltip_text)
                .with_tooltip_alignment(TooltipAlignment::Right)
                .with_tooltip_positioning_provider(Arc::new(MenuPositioning::BelowInputBox))
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        PaneHeaderAction::<AIDocumentAction, AIDocumentAction>::CustomAction(
                            AIDocumentAction::SendUpdatedPlan,
                        ),
                    );
                })
        });

        // Create restore button
        let restore_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Restore"), SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        PaneHeaderAction::<AIDocumentAction, AIDocumentAction>::CustomAction(
                            AIDocumentAction::RevertToDocumentVersion,
                        ),
                    );
                })
        });

        // Create the orchestration config block if there's an active config
        // for this document's conversation.
        let doc_conversation_id =
            AIDocumentModel::as_ref(ctx).get_conversation_id_for_document_id(&document_id);
        let has_orchestration_config = doc_conversation_id.and_then(|cid| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&cid)
                .and_then(|conv| {
                    let plan_id_str = document_id.to_string();
                    conv.orchestration_config_for_plan(&plan_id_str)
                        .map(|_| cid)
                })
        });
        let doc_id_for_block = document_id;
        let orchestration_config_block = has_orchestration_config.map(|conv_id| {
            let plan_id = doc_id_for_block.to_string();
            ctx.add_typed_action_view(move |ctx| {
                OrchestrationConfigBlockView::new(conv_id, plan_id, ctx)
            })
        });

        let mut me = Self {
            document_id,
            document_version,
            links,
            editor,
            pane_configuration,
            focus_handle: None,
            original_terminal_view: None,
            version_menu,
            sync_button_mouse_state,
            update_plan_button,
            restore_button,
            is_version_menu_open: false,
            version_button_position_id,
            synced_status_mouse_state: MouseStateHandle::default(),
            view_position_id,
            version_button,
            orchestration_config_block,
        };
        // Force update the editor view based on the initial document version
        me.refresh(ctx);

        me
    }

    pub fn pane_configuration(&self) -> &ModelHandle<PaneConfiguration> {
        &self.pane_configuration
    }

    pub fn document_id(&self) -> &AIDocumentId {
        &self.document_id
    }

    pub fn document_version(&self) -> AIDocumentVersion {
        self.document_version
    }

    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        let selected_text = self
            .editor
            .as_ref(ctx)
            .model()
            .as_ref(ctx)
            .selected_text(ctx);
        if selected_text.is_empty() {
            return None;
        }
        Some(selected_text)
    }

    pub fn set_original_terminal_view(&mut self, terminal_view: Option<ViewHandle<TerminalView>>) {
        self.original_terminal_view = terminal_view;
    }

    /// Get the terminal view for this document. Returns None if no terminal is associated.
    pub fn terminal_view(&self) -> Option<ViewHandle<TerminalView>> {
        self.original_terminal_view.clone()
    }

    /// Attempts to populate the terminal view reference if not already set.
    /// This is called when conversations are restored, allowing us to find the
    /// terminal view associated with this document's conversation.
    fn maybe_populate_terminal_view(
        &mut self,
        terminal_view_id: EntityId,
        conversation_ids: &[AIConversationId],
        ctx: &mut ViewContext<Self>,
    ) {
        // If we already have a terminal view, no need to update
        if self.original_terminal_view.is_some() {
            return;
        }

        // Get conversation ID from document
        let Some(document_conversation_id) =
            AIDocumentModel::as_ref(ctx).get_conversation_id_for_document_id(&self.document_id)
        else {
            return;
        };

        // Check if our document's conversation is in the restored conversations
        if !conversation_ids.contains(&document_conversation_id) {
            return;
        }

        // Search for the terminal view by ID
        let window_id = ctx.window_id();
        if let Some(terminal_views) = ctx.views_of_type::<TerminalView>(window_id) {
            if let Some(terminal_view) = terminal_views
                .into_iter()
                .find(|tv| tv.id() == terminal_view_id)
            {
                self.original_terminal_view = Some(terminal_view);
                ctx.notify();
            }
        }
    }

    /// Returns true if the conversation associated with this document is actively streaming.
    fn is_conversation_streaming(&self, ctx: &AppContext) -> bool {
        let document_model = AIDocumentModel::handle(ctx);
        let Some(conversation_id) = document_model
            .as_ref(ctx)
            .get_conversation_id_for_document_id(&self.document_id)
        else {
            return false;
        };

        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(conversation) = history_model.conversation(&conversation_id) else {
            return false;
        };

        conversation.status().is_in_progress()
    }

    /// Refresh the view to match the current state of the document model
    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(document) =
            AIDocumentModel::as_ref(ctx).get_document(&self.document_id, self.document_version)
        else {
            return;
        };

        // Document is read-only if it's an earlier version OR if we're still streaming the plan.
        // Once streaming completes, the current version becomes editable.
        let is_earlier_version = matches!(document, AIDocumentInstance::Earlier(_));
        let is_document_creation_streaming =
            AIDocumentModel::as_ref(ctx).is_document_creation_streaming(&self.document_id);
        let is_read_only = is_earlier_version || is_document_creation_streaming;

        self.set_editor_model(document.get_editor(), is_read_only, ctx);
        self.update_header_buttons(ctx);

        // Update pane title to the document title
        let title = document.get_title();
        self.pane_configuration.update(ctx, |pc, ctx| {
            pc.set_title(title, ctx);
        });
    }

    fn update_header_buttons(&mut self, ctx: &mut ViewContext<Self>) {
        let server_id = AIDocumentModel::as_ref(ctx)
            .get_current_document(&self.document_id)
            .and_then(|doc| doc.sync_id)
            .and_then(|sync_id| sync_id.into_server());

        self.pane_configuration.update(ctx, |pc, ctx| {
            pc.set_shareable_object(server_id.map(ShareableObject::WarpDriveObject), ctx);
            pc.refresh_pane_header_overflow_menu_items(ctx);
        });
        ctx.notify();
    }

    fn render_header_buttons(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let document = AIDocumentModel::as_ref(app).get_current_document(&self.document_id);

        let is_current_version = document
            .as_ref()
            .map(|doc| doc.version == self.document_version)
            .unwrap_or(true);

        // If viewing an older version, show restore button
        if !is_current_version {
            let restore_button = self.restore_button.clone();
            return Some(
                Container::new(ChildView::new(&restore_button).finish())
                    .with_margin_right(8.)
                    .finish(),
            );
        }

        let user_edit_status = document
            .as_ref()
            .map(|doc| doc.user_edit_status)
            .unwrap_or(AIDocumentUserEditStatus::UpToDate);

        let save_status = AIDocumentModel::as_ref(app).get_document_save_status(&self.document_id);

        let is_streaming = self.is_conversation_streaming(app);

        let sync_element = self.render_sync_element(save_status, app);

        if is_streaming && user_edit_status.is_dirty() {
            let update_plan_button = self.update_plan_button.clone();
            Some(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(ChildView::new(&update_plan_button).finish())
                    .with_child(Container::new(sync_element).with_margin_left(4.).finish())
                    .finish(),
            )
        } else {
            Some(sync_element)
        }
    }

    /// Renders the sync/save status element based on save status.
    fn render_sync_element(
        &self,
        save_status: AIDocumentSaveStatus,
        app: &AppContext,
    ) -> Box<dyn Element> {
        match save_status {
            AIDocumentSaveStatus::NotSaved => {
                let appearance = Appearance::as_ref(app);
                let ui_builder = appearance.ui_builder().clone();
                let tooltip = ui_builder
                    .tool_tip(
                        crate::i18n::tr_static(
                            app,
                            "Save and auto-sync this plan to your Warp Drive",
                        )
                        .to_string(),
                    )
                    .build()
                    .finish();
                let sync_button_mouse_state = self.sync_button_mouse_state.clone();
                icon_button(
                    appearance,
                    Icon::RefreshCw04,
                    false,
                    sync_button_mouse_state,
                )
                .with_tooltip(move || tooltip)
                .with_tooltip_position(ButtonTooltipPosition::BelowRight)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(
                        PaneHeaderAction::<AIDocumentAction, AIDocumentAction>::CustomAction(
                            AIDocumentAction::CreateWarpDriveNotebook,
                        ),
                    )
                })
                .finish()
            }
            AIDocumentSaveStatus::Saving => {
                let appearance = Appearance::as_ref(app);
                let theme = appearance.theme();
                let color = theme.nonactive_ui_detail().into_solid();
                Container::new(
                    ConstrainedBox::new(
                        Container::new(
                            ConstrainedBox::new(
                                Icon::RefreshCw04
                                    .to_warpui_icon(ThemeFill::Solid(color))
                                    .finish(),
                            )
                            .with_width(16.)
                            .with_height(16.)
                            .finish(),
                        )
                        .with_uniform_padding(4.)
                        .finish(),
                    )
                    .with_width(24.)
                    .with_height(24.)
                    .finish(),
                )
                .finish()
            }
            AIDocumentSaveStatus::Saved => {
                let appearance = Appearance::as_ref(app);
                let theme = appearance.theme();
                let color = theme.nonactive_ui_detail().into_solid();
                let ui_builder = appearance.ui_builder().clone();
                let tooltip_text =
                    "This plan is synced to your Warp Drive and will auto save any edits you make."
                        .to_string();
                let synced_status_mouse_state = self.synced_status_mouse_state.clone();
                Container::new(
                    ConstrainedBox::new(
                        Container::new(
                            Hoverable::new(synced_status_mouse_state, move |state| {
                                let icon = {
                                    let icon_elem = Icon::RefreshCw04
                                        .to_warpui_icon(ThemeFill::Solid(color))
                                        .finish();
                                    ConstrainedBox::new(icon_elem)
                                        .with_width(16.)
                                        .with_height(16.)
                                        .finish()
                                };

                                if state.is_hovered() {
                                    let tooltip =
                                        ui_builder.tool_tip(tooltip_text.clone()).build().finish();
                                    let mut stack = Stack::new().with_child(icon);
                                    stack.add_positioned_overlay_child(
                                        tooltip,
                                        OffsetPositioning::offset_from_parent(
                                            vec2f(0., 4.),
                                            warpui::elements::ParentOffsetBounds::WindowByPosition,
                                            warpui::elements::ParentAnchor::BottomRight,
                                            ChildAnchor::TopRight,
                                        ),
                                    );
                                    stack.finish()
                                } else {
                                    icon
                                }
                            })
                            .finish(),
                        )
                        .with_uniform_padding(4.)
                        .finish(),
                    )
                    .with_width(24.)
                    .with_height(24.)
                    .finish(),
                )
                .finish()
            }
        }
    }

    fn render_plan_header(
        &self,
        header_ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let title = AIDocumentModel::as_ref(app)
            .get_current_document(&self.document_id)
            .map(|doc| doc.title.clone())
            .unwrap_or_else(|| DEFAULT_PLANNING_DOCUMENT_TITLE.to_string());

        let version_button = SavePosition::new(
            ChildView::new(&self.version_button).finish(),
            &self.version_button_position_id,
        )
        .finish();
        let left_row = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(version_button)
            .finish();

        let mut right_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);
        if let Some(sharing) = header_ctx.sharing_controls(app, None, None) {
            right_row.add_child(sharing);
        }
        if let Some(header_buttons) = self.render_header_buttons(app) {
            right_row.add_child(header_buttons);
        }

        let should_show_close_button = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_in_split_pane(app));
        right_row.add_child(render_pane_header_buttons::<
            AIDocumentAction,
            AIDocumentAction,
        >(
            header_ctx,
            appearance,
            should_show_close_button,
            None,
            None,
        ));

        let button_count = should_show_close_button as u32 + header_ctx.has_overflow_items as u32;
        render_three_column_header(
            left_row,
            render_pane_header_title_text(title, appearance, ClipConfig::start()),
            right_row.finish(),
            CenteredHeaderEdgeWidth {
                min: button_count as f32 * ICON_DIMENSIONS,
                max: 180.0,
            },
            header_ctx.header_left_inset,
            header_ctx.draggable_state.is_dragging(),
        )
    }

    fn set_editor_model(
        &mut self,
        editor_model: ModelHandle<NotebooksEditorModel>,
        is_read_only: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.unsubscribe_to_view(&self.editor);

        let view_position_id = format!("ai_document_view_{}", ctx.view_id());
        let links = self.links.clone();
        let new_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = RichTextEditorView::new(
                view_position_id.clone(),
                editor_model.clone(),
                links,
                RichTextEditorConfig::default(),
                ctx,
            );
            editor.set_interaction_state(
                if is_read_only {
                    InteractionState::Selectable
                } else {
                    InteractionState::Editable
                },
                ctx,
            );
            editor
        });

        ctx.subscribe_to_view(&new_editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        self.editor = new_editor;

        // Ensure model is bound to the window for rendering/events
        let window_id = ctx.window_id();
        self.bind_window(window_id, ctx);

        ctx.notify();
    }

    fn open_version_menu(&mut self, ctx: &mut ViewContext<Self>) {
        // Build menu items: current version first, then earlier versions (newest to oldest)
        let model = AIDocumentModel::handle(ctx);
        let model = model.as_ref(ctx);

        let mut versions: Vec<VersionMenuEntry> = Vec::new();

        if let Some(current) = model.get_current_document(&self.document_id) {
            versions.push(VersionMenuEntry {
                version: current.version,
                created_at: current.created_at,
                restored_from: current.restored_from,
            });
        }

        if let Some(earlier_versions) = model.get_earlier_document_versions(&self.document_id) {
            versions.extend(earlier_versions.iter().rev().map(|v| VersionMenuEntry {
                version: v.version,
                created_at: v.created_at,
                restored_from: v.restored_from,
            }));
        }

        if versions.is_empty() {
            self.version_menu.update(ctx, |menu, ctx| {
                menu.set_items(Vec::new(), ctx);
            });
            return;
        }

        let selected_index = versions
            .iter()
            .position(|entry| entry.version == self.document_version);

        let items: Vec<MenuItem<AIDocumentAction>> = versions
            .iter()
            .map(|entry| {
                let label = if let Some(from_version) = entry.restored_from {
                    format!("{} (restored from {})", entry.version, from_version)
                } else {
                    entry.version.to_string()
                };
                MenuItemFields::new(label)
                    .with_timestamp(entry.created_at)
                    .with_on_select_action(AIDocumentAction::SelectVersion(entry.version))
                    .into_item()
            })
            .collect();

        self.version_menu.update(ctx, |menu, ctx| {
            menu.set_items(items, ctx);
            if let Some(idx) = selected_index {
                menu.set_selected_by_index(idx, ctx);
            }
            // Ensure selection matches the currently viewed version even if index computation changes
            menu.set_selected_by_action(
                &AIDocumentAction::SelectVersion(self.document_version),
                ctx,
            );
        });

        self.is_version_menu_open = true;
        ctx.focus(&self.version_menu);
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorViewEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorViewEvent::Edited => {
                ctx.notify();
            }
            EditorViewEvent::Focused => {
                ctx.emit(AIDocumentEvent::Pane(PaneEvent::FocusSelf));
            }
            #[cfg(feature = "local_fs")]
            EditorViewEvent::OpenFile {
                path,
                line_and_column_num,
                force_open_in_warp,
            } => {
                use crate::util::file::external_editor::EditorSettings;
                use crate::util::openable_file_type::{
                    is_supported_image_file, resolve_file_target,
                };

                if *force_open_in_warp {
                    let layout = *EditorSettings::as_ref(ctx).open_file_layout;
                    let source = CodeSource::Link {
                        path: path.clone(),
                        range_start: *line_and_column_num,
                        range_end: None,
                    };
                    ctx.emit(AIDocumentEvent::OpenCodeInWarp {
                        source,
                        layout,
                        line_col: *line_and_column_num,
                    });
                } else {
                    let settings = EditorSettings::as_ref(ctx);
                    let target = if is_supported_image_file(path) {
                        FileTarget::SystemGeneric
                    } else {
                        resolve_file_target(path, settings, None)
                    };
                    ctx.emit(AIDocumentEvent::OpenFileWithTarget {
                        path: path.clone(),
                        target,
                        line_col: *line_and_column_num,
                    });
                }
            }
            _ => (),
        }
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.editor);
        ctx.emit(AIDocumentEvent::Pane(PaneEvent::FocusSelf));
    }

    /// Bind the underlying editor model to the given window, enabling render/event processing.
    pub fn bind_window(&self, window_id: warpui::WindowId, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor_view, ctx| {
            editor_view
                .model()
                .update(ctx, |model, ctx| model.set_window_id(window_id, ctx));
        });
    }

    fn create_warp_drive_notebook(&self, ctx: &mut ViewContext<Self>) {
        let success = AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
            model.sync_to_warp_drive(self.document_id, ctx)
        });
        if !success {
            log::error!("Failed to create Warp Drive notebook");
        }
    }

    /// Export the current content as a markdown file.
    #[cfg(feature = "local_fs")]
    fn export(&self, ctx: &mut ViewContext<Self>) {
        use crate::drive::export::safe_filename;
        use warpui::platform::SaveFilePickerConfiguration;
        let markdown = self.editor.as_ref(ctx).markdown_unescaped(ctx);

        // Get the document title from the model
        let title = AIDocumentModel::as_ref(ctx)
            .get_current_document(&self.document_id)
            .map(|doc| doc.title.clone())
            .unwrap_or_else(|| "Untitled".to_string());

        // Sanitize the title for use as a filename
        let sanitized_title = safe_filename(&title);
        let filename = if sanitized_title.is_empty() {
            "Untitled.md".to_string()
        } else {
            format!("{sanitized_title}.md")
        };

        // Get the default directory from the associated terminal view's pwd
        let default_directory = self
            .original_terminal_view
            .as_ref()
            .and_then(|terminal_view| terminal_view.as_ref(ctx).pwd().map(PathBuf::from));

        let mut config = SaveFilePickerConfiguration::new().with_default_filename(filename);
        if let Some(directory) = default_directory {
            config = config.with_default_directory(directory);
        }

        ctx.open_save_file_picker(
            move |path_opt: Option<String>, _me: &mut Self, _ctx: &mut ViewContext<Self>| {
                if let Some(path) = path_opt {
                    if let Err(e) = std::fs::write(&path, &markdown) {
                        log::error!("Failed to export AI document: {e}");
                    }
                }
            },
            config,
        );
    }

    /// Export the current content as a markdown file (WASM stub).
    #[cfg(not(feature = "local_fs"))]
    fn export(&self, _ctx: &mut ViewContext<Self>) {
        // No-op for WASM target
    }
}

impl Entity for AIDocumentView {
    type Event = AIDocumentEvent;
}

impl View for AIDocumentView {
    fn ui_name() -> &'static str {
        "AIDocumentView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let has_orchestration_config = AIDocumentModel::as_ref(app)
            .get_conversation_id_for_document_id(&self.document_id)
            .and_then(|cid| {
                let plan_id_str = self.document_id.to_string();
                BlocklistAIHistoryModel::as_ref(app)
                    .conversation(&cid)
                    .and_then(|conv| conv.orchestration_config_for_plan(&plan_id_str).map(|_| ()))
            })
            .is_some();

        let mut content_column =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Orchestration config block — shown above the editor when the
        // conversation has an active OrchestrationConfigSnapshot.
        if has_orchestration_config {
            if let Some(config_block) = &self.orchestration_config_block {
                content_column.add_child(
                    Container::new(ChildView::new(config_block).finish())
                        .with_horizontal_padding(16.)
                        .with_padding_bottom(12.)
                        .with_padding_top(8.)
                        .finish(),
                );
            }
        }

        let editor = Container::new(ChildView::new(&self.editor).finish())
            .with_padding_left(8.)
            .with_padding_right(8.)
            .finish();
        content_column.add_child(warpui::elements::Expanded::new(1.0, editor).finish());

        let mut stack = Stack::new().with_child(content_column.finish());

        if self.is_version_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.version_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.version_button_position_id.clone(),
                    vec2f(0., 4.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        SavePosition::new(stack.finish(), &self.view_position_id).finish()
    }
}

impl TypedActionView for AIDocumentView {
    type Action = AIDocumentAction;

    fn handle_action(&mut self, action: &AIDocumentAction, ctx: &mut ViewContext<Self>) {
        match action {
            AIDocumentAction::Close => {
                self.close(ctx);
            }
            AIDocumentAction::SelectVersion(version) => {
                self.document_version = *version;
                self.refresh(ctx);
            }
            AIDocumentAction::Export => self.export(ctx),
            AIDocumentAction::CreateWarpDriveNotebook => self.create_warp_drive_notebook(ctx),
            AIDocumentAction::CopyLink(link) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::ObjectLinkCopied { link: link.clone() },
                    ctx
                );
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(link.to_owned()));

                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::success(
                            crate::i18n::tr_static(ctx, "Link copied to clipboard").to_string(),
                        ),
                        window_id,
                        ctx,
                    );
                });
            }
            AIDocumentAction::CopyPlanId => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(self.document_id.to_string()));

                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::success(
                            crate::i18n::tr_static(ctx, "Plan ID copied to clipboard").to_string(),
                        ),
                        window_id,
                        ctx,
                    );
                });
            }
            AIDocumentAction::OpenVersionMenu => {
                if self.is_version_menu_open {
                    self.is_version_menu_open = false;
                    ctx.notify();
                } else {
                    self.open_version_menu(ctx);
                }
            }
            AIDocumentAction::RevertToDocumentVersion => {
                match AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
                    model.revert_to_document_version(&self.document_id, self.document_version, ctx)
                }) {
                    Ok(new_version) => {
                        self.document_version = new_version;
                        self.refresh(ctx);
                    }
                    Err(e) => {
                        log::error!("Failed to restore previous version: {e}");
                    }
                }
            }
            AIDocumentAction::SendUpdatedPlan => {
                let Some(terminal_view) = &self.original_terminal_view else {
                    log::warn!("Cannot send updated plan: no terminal view associated");
                    return;
                };

                // Get conversation ID directly from the document
                let document_model = AIDocumentModel::handle(ctx);
                let Some(conversation_id) = document_model
                    .as_ref(ctx)
                    .get_conversation_id_for_document_id(&self.document_id)
                else {
                    log::warn!("Cannot send updated plan: no conversation ID found for document");
                    return;
                };

                terminal_view.update(ctx, |terminal_view, ctx| {
                    let history_model = BlocklistAIHistoryModel::handle(ctx);
                    let history_model_ref = history_model.as_ref(ctx);

                    // Get the conversation by ID
                    let Some(conversation) = history_model_ref.conversation(&conversation_id)
                    else {
                        log::warn!("Cannot send updated plan: conversation not found");
                        return;
                    };

                    // Only proceed if conversation is actually streaming
                    if !conversation.status().is_in_progress() {
                        log::warn!(
                            "Skipping sending updated plan: conversation is not in progress"
                        );
                        return;
                    }

                    // Select the conversation in the context model before sending the query
                    terminal_view
                        .ai_context_model()
                        .update(ctx, |context_model, ctx| {
                            context_model.set_pending_query_state_for_existing_conversation(
                                conversation_id,
                                AgentViewEntryOrigin::AIDocument,
                                ctx,
                            );
                        });

                    // Send a user query to inform the agent about the plan update
                    // The document is already marked as Dirty and pending_document_id
                    // is already set in the context model, so the updated plan will be attached.
                    // TODO(roland): don't directly use user query, but send a new input type that can be formatted on the server.
                    terminal_view
                        .ai_controller()
                        .update(ctx, |controller, ctx| {
                            controller.send_user_query_in_conversation(
                                "I've updated the plan.".to_string(),
                                conversation_id,
                                None,
                                ctx,
                            );
                        });
                });

                // Update UI to reflect the new query
                self.update_header_buttons(ctx);
            }
            AIDocumentAction::ShowInWarpDrive => {
                if let Some(document) =
                    AIDocumentModel::as_ref(ctx).get_current_document(&self.document_id)
                {
                    if let Some(sync_id) = document.sync_id {
                        ctx.emit(AIDocumentEvent::ViewInWarpDrive(WarpDriveItemId::Object(
                            CloudObjectTypeAndId::Notebook(sync_id),
                        )));
                    }
                }
            }
            AIDocumentAction::AttachToActiveSession => {
                ctx.emit(AIDocumentEvent::AttachPlanAsContext(self.document_id));
            }
        }
    }
}

impl BackingView for AIDocumentView {
    type PaneHeaderOverflowMenuAction = AIDocumentAction;
    type CustomAction = AIDocumentAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn handle_custom_action(&mut self, action: &Self::CustomAction, ctx: &mut ViewContext<Self>) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(AIDocumentEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.editor);
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }

    fn pane_header_overflow_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<Self::PaneHeaderOverflowMenuAction>> {
        let mut menu_items = vec![];

        // Only show shareable link when the document is synced to Warp Drive
        if let Some(link) =
            AIDocumentModel::as_ref(ctx).get_document_warp_drive_object_link(&self.document_id, ctx)
        {
            menu_items.push(
                MenuItemFields::new(crate::i18n::tr_static(ctx, "Copy link"))
                    .with_on_select_action(AIDocumentAction::CopyLink(link))
                    .with_icon(Icon::Link)
                    .into_item(),
            );
            menu_items.push(
                MenuItemFields::new(crate::i18n::tr_static(ctx, "Show in Warp Drive"))
                    .with_on_select_action(AIDocumentAction::ShowInWarpDrive)
                    .with_icon(Icon::WarpDrive)
                    .into_item(),
            );
        }

        #[cfg(feature = "local_fs")]
        {
            menu_items.push(
                crate::menu::MenuItemFields::new(crate::i18n::tr_static(
                    ctx,
                    "Save as markdown file",
                ))
                .with_on_select_action(AIDocumentAction::Export)
                .with_icon(Icon::Download)
                .into_item(),
            );
        }

        // Add "Attach to active session" menu item
        menu_items.push(
            MenuItemFields::new(crate::i18n::tr_static(ctx, "Attach to active session"))
                .with_on_select_action(AIDocumentAction::AttachToActiveSession)
                .with_icon(Icon::Paperclip)
                .into_item(),
        );

        // Add "Copy plan ID" menu item
        menu_items.push(
            MenuItemFields::new(crate::i18n::tr_static(ctx, "Copy plan ID"))
                .with_on_select_action(AIDocumentAction::CopyPlanId)
                .with_icon(Icon::Copy)
                .into_item(),
        );

        menu_items
    }

    fn render_header_content(
        &self,
        header_ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::Custom {
            element: self.render_plan_header(header_ctx, app),
            has_custom_draggable_behavior: false,
        }
    }
}
