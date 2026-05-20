use warp_core::{features::FeatureFlag, send_telemetry_from_ctx, ui::appearance::Appearance};
use warpui::{keymap::Keystroke, EntityId, SingletonEntity, ViewContext};

use crate::{
    ai::{
        agent::conversation::AIConversationId,
        blocklist::{
            agent_view::{
                AgentViewEntryBlock, AgentViewEntryBlockEvent, AgentViewEntryBlockParams,
                AgentViewEntryOrigin, AutoTriggerBehavior, DismissalStrategy, EnterAgentViewError,
                EphemeralMessage, ENTER_OR_EXIT_CONFIRMATION_WINDOW,
            },
            history_model::CloudConversationData,
            BlocklistAIHistoryModel,
        },
    },
    global_resource_handles::GlobalResourceHandlesProvider,
    persistence::ModelEvent,
    server::telemetry::TelemetryAgentViewEntryOrigin,
    terminal::{
        input::message_bar::{Message, MessageItem},
        model::rich_content::RichContentType,
        view::{AgentViewEntryMetadata, RichContentInsertionPosition, RichContentMetadata},
        TerminalView,
    },
    view_components::DismissibleToast,
    workspace::ToastStack,
    TelemetryEvent,
};

pub const ENTER_AGAIN_TO_SEND_MESSAGE_ID: &str = "enter_again_to_send";

impl TerminalView {
    pub fn enter_agent_view(
        &mut self,
        initial_prompt: Option<String>,
        conversation_id: Option<AIConversationId>,
        origin: AgentViewEntryOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(id) = conversation_id {
            self.enter_agent_view_for_conversation(initial_prompt, origin, id, ctx);
        } else {
            self.enter_agent_view_for_new_conversation(initial_prompt, origin, ctx);
        }
    }

    pub fn enter_agent_view_for_new_conversation(
        &mut self,
        initial_prompt: Option<String>,
        origin: AgentViewEntryOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        // Don't allow starting a new conversation while the agent is in control. 3p cloud
        // viewers enter agent view to wrap an existing run's content and are not starting a
        // new conversation, so they are exempt from this guard.
        if !matches!(origin, AgentViewEntryOrigin::ThirdPartyCloudAgent)
            && !self
                .ai_context_model
                .as_ref(ctx)
                .can_start_new_conversation()
        {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    DismissibleToast::error(
                        "Cannot start a new conversation while agent is monitoring a command."
                            .to_string(),
                    ),
                    window_id,
                    ctx,
                );
            });
            return;
        }

        if let Err(e) = self.try_enter_agent_view(initial_prompt, origin, None, ctx) {
            log::error!(
                "Failed to enter agent view for new conversation from origin {:?}: {:?}",
                origin,
                e
            );
            self.show_error_toast(e.to_string(), ctx);
        }
        self.redetermine_global_focus(ctx);
    }

    // Enters the agent view for a restored CLI agent transcript, setting the title using the
    // restored CLI conversation metadata if we have it.
    pub(crate) fn enter_agent_view_for_restored_cli_agent(
        &mut self,
        fallback_title: String,
        ctx: &mut ViewContext<Self>,
    ) -> Option<AIConversationId> {
        let origin = AgentViewEntryOrigin::ThirdPartyCloudAgent;

        match self.try_enter_agent_view(None, origin, None, ctx) {
            Ok(conversation_id) => {
                let title = fallback_title.trim();
                if !title.is_empty() {
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                        if let Some(conversation) = history.conversation_mut(&conversation_id) {
                            conversation.set_fallback_display_title(title.to_owned());
                        }
                    });
                }
                self.redetermine_global_focus(ctx);
                Some(conversation_id)
            }
            Err(e) => {
                log::error!(
                    "Failed to enter agent view for restored CLI agent from origin {:?}: {:?}",
                    origin,
                    e
                );
                self.show_error_toast(e.to_string(), ctx);
                self.redetermine_global_focus(ctx);
                None
            }
        }
    }

    pub fn enter_agent_view_for_conversation(
        &mut self,
        initial_prompt: Option<String>,
        origin: AgentViewEntryOrigin,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model = BlocklistAIHistoryModel::handle(ctx).as_ref(ctx);

        let is_conversation_in_memory = history_model.conversation(&conversation_id).is_some();
        let is_live = history_model
            .all_live_conversations_for_terminal_view(self.view_id)
            .any(|conversation| conversation.id() == conversation_id);

        if is_conversation_in_memory && is_live {
            if let Err(e) = self.try_enter_agent_view(
                initial_prompt.clone(),
                origin,
                Some(conversation_id),
                ctx,
            ) {
                log::error!(
                    "Failed to enter agent view for existing conversation ({:?}) from origin {:?}: {:?}",
                    conversation_id,
                    origin,
                    e
                );
                self.show_error_toast(e.to_string(), ctx);
            }
        } else {
            let conversation_id_copy = conversation_id;
            let future = history_model.load_conversation_data(conversation_id_copy, ctx);
            ctx.spawn(future, move |me, conversation, ctx| {
                let Some(conversation) = conversation else {
                    me.show_error_toast(
                        format!("Failed to load conversation with id: {conversation_id}"),
                        ctx,
                    );
                    return;
                };
                // For Oz conversations, restore data and then re-enter agent view (the
                // conversation will be in memory after restoration).
                // For CLI agent conversations, restore the block snapshot only. Because we
                // don't update the in-memory model in this case, attempting to re-enter agent
                // view will trigger an infinite loop of fetching and loading conversation data
                // from the server.
                #[allow(clippy::type_complexity)]
                let on_restored: Box<
                    dyn FnOnce(&mut Self, &mut ViewContext<Self>),
                > = if matches!(&conversation, CloudConversationData::Oz(_)) {
                    Box::new(move |me, ctx| {
                        me.enter_agent_view_for_conversation(
                            initial_prompt,
                            origin,
                            conversation_id,
                            ctx,
                        );
                    })
                } else {
                    if !FeatureFlag::AgentHarness.is_enabled() {
                        log::warn!("AgentHarness flag is disabled; ignoring CLI agent conversation {conversation_id}");
                        return;
                    }
                    Box::new(|_, _| {})
                };
                me.restore_conversation_and_directory_context(
                    conversation,
                    false,
                    on_restored,
                    ctx,
                );
            });
        }
    }

    pub(super) fn try_enter_agent_view(
        &mut self,
        initial_prompt: Option<String>,
        origin: AgentViewEntryOrigin,
        conversation_id: Option<AIConversationId>,
        ctx: &mut ViewContext<Self>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        // Capture pending context block IDs before entering agent view.
        let pending_attached_blocks = self
            .ai_context_model
            .as_ref(ctx)
            .pending_context_block_ids()
            .clone();
        let was_in_agent_view_already = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .is_fullscreen();

        let conversation_id = self.agent_view_controller.update(ctx, |controller, ctx| {
            controller.try_enter_agent_view(conversation_id, origin, ctx)
        })?;

        // Associate pending context blocks with the new conversation so they remain
        // visible in the agent view. This must happen after the conversation is created
        // but before any re-filtering of pending context block IDs occurs.
        if !pending_attached_blocks.is_empty() {
            let attached_blocks = self
                .model
                .lock()
                .block_list_mut()
                .associate_blocks_with_conversation(
                    pending_attached_blocks.iter(),
                    conversation_id,
                );

            // Persist the updated visibility for each modified block
            if let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
                .get()
                .model_event_sender
                .as_ref()
            {
                for (block_id, agent_view_visibility) in attached_blocks {
                    if let Err(e) = sender.send(ModelEvent::UpdateBlockAgentViewVisibility {
                        block_id: block_id.to_string(),
                        agent_view_visibility: agent_view_visibility.into(),
                    }) {
                        log::error!("Error sending UpdateBlockAgentViewVisibility event: {e:?}");
                    }
                }
            }
        }

        let mut did_auto_trigger_request = false;
        // Show ephemeral message when entering agent view via input with a prompt
        if let Some(initial_prompt) = initial_prompt {
            let should_auto_submit = match origin.should_autotrigger_request() {
                AutoTriggerBehavior::Always => true,
                AutoTriggerBehavior::InAgentView => was_in_agent_view_already,
                AutoTriggerBehavior::Never => false,
            };
            if should_auto_submit {
                // Clear the "enter again to send" ephemeral message if it's currently showing
                self.ephemeral_message_model.update(ctx, |model, ctx| {
                    if model
                        .current_message()
                        .and_then(|msg| msg.id())
                        .is_some_and(|id| id == ENTER_AGAIN_TO_SEND_MESSAGE_ID)
                    {
                        model.clear_message(ctx);
                    }
                });

                self.ai_controller.update(ctx, |controller, ctx| {
                    controller.send_user_query_in_conversation(
                        initial_prompt,
                        conversation_id,
                        None,
                        ctx,
                    );
                });
                did_auto_trigger_request = true;
            } else {
                let appearance = Appearance::handle(ctx).as_ref(ctx);
                let message = Message::new(vec![
                    MessageItem::keystroke(Keystroke {
                        key: "enter".to_owned(),
                        ..Default::default()
                    }),
                    MessageItem::text("again to send to agent"),
                ])
                .with_text_color(appearance.theme().ansi_fg_magenta());
                self.ephemeral_message_model.update(ctx, |model, ctx| {
                    // Keep this explicit (instead of relying on the default message duration) so
                    // "enter again to send" stays aligned with the broader confirmation cadence.
                    model.show_ephemeral_message(
                        EphemeralMessage::new(
                            message,
                            DismissalStrategy::Timer(ENTER_OR_EXIT_CONFIRMATION_WINDOW),
                        )
                        .with_id(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
                        ctx,
                    );
                });

                self.input.update(ctx, |input, ctx| {
                    input.replace_buffer_content(&initial_prompt, ctx);
                });
            }
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::AgentViewEntered {
                origin: TelemetryAgentViewEntryOrigin::from(origin),
                did_auto_trigger_request,
            },
            ctx
        );

        // Mark all AgentViewEntry rich content as dirty so their heights get
        // re-measured. When the agent view is active, AgentViewEntryBlock renders
        // as Empty (0 height). When exiting, we need to force a re-layout so the
        // block's actual height is restored. The dirty item processing happens
        // before viewport iteration, so this works even for 0-height items at
        // the prefix of the blocklist.
        let mut model = self.model.lock();
        self.mark_all_rich_content_items_dirty_where(&mut model, |metadata| {
            matches!(metadata, RichContentMetadata::AgentViewEntry(_))
        });
        drop(model);

        Ok(conversation_id)
    }

    pub(super) fn insert_agent_view_entry_block(
        &mut self,
        params: AgentViewEntryBlockParams,
        position: RichContentInsertionPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&params.conversation_id)
            .is_some_and(|conversation| conversation.is_entirely_passive())
        {
            return;
        }
        let conversation_id = params.conversation_id;
        let origin = params.origin;
        let agent_view_block =
            ctx.add_typed_action_view(|ctx| AgentViewEntryBlock::new(params, ctx));
        ctx.subscribe_to_view(&agent_view_block, |me, _, event, ctx| match event {
            AgentViewEntryBlockEvent::EnterAgentView { conversation_id } => me
                .enter_agent_view_for_conversation(
                    None,
                    AgentViewEntryOrigin::AgentViewBlock,
                    *conversation_id,
                    ctx,
                ),
            AgentViewEntryBlockEvent::OpenConversationContextMenu {
                conversation_id,
                agent_view_entry_block_id,
                position,
            } => me.open_agent_view_entry_context_menu(
                *conversation_id,
                *agent_view_entry_block_id,
                *position,
                ctx,
            ),
            AgentViewEntryBlockEvent::ForkConversation { conversation_id } => {
                me.fork_ai_conversation(*conversation_id, None, ctx);
            }
        });
        self.insert_rich_content(
            Some(RichContentType::EnterAgentView),
            agent_view_block,
            Some(RichContentMetadata::AgentViewEntry(
                AgentViewEntryMetadata {
                    conversation_id,
                    origin,
                },
            )),
            position,
            ctx,
        );
    }

    /// Retags the rich content view with the given id so it renders under `conversation_id`'s
    /// agent view. Updates both the local `rich_content_views` entry and the block list so
    /// `should_hide_for_agent_view_state` picks up the new association.
    pub(super) fn set_rich_content_agent_view_conversation_id(
        &mut self,
        rich_content_view_id: EntityId,
        conversation_id: AIConversationId,
    ) {
        let Some(rich_content) = self
            .rich_content_views
            .iter_mut()
            .find(|rich_content| rich_content.view_id() == rich_content_view_id)
        else {
            return;
        };

        rich_content.set_agent_view_conversation_id(Some(conversation_id));
        self.model
            .lock()
            .block_list_mut()
            .update_agent_view_conversation_id_for_rich_content(
                rich_content_view_id,
                Some(conversation_id),
            );
    }
}
