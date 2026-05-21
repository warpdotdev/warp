use super::{
    fork_label_for_query, mark_feature_used_and_write_to_user_defaults, AIAgentExchangeId,
    AIConversationId, AgentModeRewindEntrypoint, AppContext, BlocklistAIHistoryModel, ChannelState,
    ClipboardContent, ContextMenuAction, ContextMenuInfo, ContextMenuState, ContextMenuType,
    EntityId, FeatureFlag, ForkAIConversationParams, ForkFromExchange,
    ForkedConversationDestination, MenuItem, MenuItemFields, RichContentLink,
    ServerConversationToken, ServerOutputId, ShareableObject, TelemetryEvent, TerminalAction,
    TerminalModel, TerminalView, Tip, TipHint, Vector2F, ViewContext, CONTEXT_MENU_WIDTH,
};
use warp_core::send_telemetry_from_ctx;
use warpui::{SingletonEntity, UpdateView};

impl TerminalView {
    pub(super) fn ai_block_copying_menu_items(
        &self,
        ai_block_view_id: EntityId,
        ai_conversation_id: AIConversationId,
        hovered_link: Option<RichContentLink>,
        model: &TerminalModel,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<TerminalAction>> {
        let mut items = vec![
            MenuItemFields::new("Copy")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::CopyAIBlock { ai_block_view_id },
                ))
                .into_item(),
            MenuItemFields::new("Copy prompt")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::CopyAIBlockQuery { ai_block_view_id },
                ))
                .into_item(),
            MenuItemFields::new("Copy output as Markdown")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::CopyAIBlockOutput { ai_block_view_id },
                ))
                .into_item(),
        ];

        if let Some(link) = hovered_link {
            match link {
                RichContentLink::Url(url) => {
                    items.push(
                        MenuItemFields::new("Copy URL")
                            .with_on_select_action(TerminalAction::ContextMenu(
                                ContextMenuAction::CopyUrl { url_content: url },
                            ))
                            .into_item(),
                    );
                }
                #[cfg(feature = "local_fs")]
                RichContentLink::FilePath { absolute_path, .. } => {
                    items.push(
                        MenuItemFields::new("Copy path")
                            .with_on_select_action(TerminalAction::ContextMenu(
                                ContextMenuAction::CopyUrl {
                                    url_content: absolute_path.to_string_lossy().into_owned(),
                                },
                            ))
                            .into_item(),
                    );
                }
            }
        }

        let num_requested_commands = self
            .rich_content_views
            .iter()
            .find_map(|rich_content| {
                let ai_metadata = rich_content.ai_block_metadata()?;
                if ai_metadata.ai_block_handle.id() == ai_block_view_id {
                    return Some(ai_metadata.ai_block_handle.as_ref(ctx));
                }
                None
            })
            .map_or_else(|| 0, |ai_block| ai_block.num_requested_commands());

        if num_requested_commands > 0 {
            items.push(
                MenuItemFields::new(String::from("Copy command"))
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::CopyAgentCommand { ai_block_view_id },
                    ))
                    .into_item(),
            );
        }

        let action_ids: Vec<_> = self
            .rich_content_views
            .iter()
            .find_map(|rich_content| {
                let ai_metadata = rich_content.ai_block_metadata()?;
                if ai_metadata.ai_block_handle.id() == ai_block_view_id {
                    return Some(ai_metadata.ai_block_handle.as_ref(ctx));
                }
                None
            })
            .map(|ai_block| {
                ai_block
                    .requested_commands_iter()
                    .map(|(action_id, _)| action_id)
                    .collect()
            })
            .unwrap_or_default();

        let has_git_branch = action_ids.iter().any(|action_id| {
            model
                .block_list()
                .block_for_ai_action_id(action_id)
                .is_some_and(|block| block.git_branch().is_some())
        });
        if has_git_branch {
            items.push(
                MenuItemFields::new(String::from("Copy git branch"))
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::CopyAgentGitBranch { ai_block_view_id },
                    ))
                    .into_item(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(
            MenuItemFields::new("Save as prompt")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::SavePromptAsAgentModeWorkflow { ai_block_view_id },
                ))
                .into_item(),
        );
        items.push(MenuItem::Separator);

        if FeatureFlag::CloudConversations.is_enabled() {
            let history_model = BlocklistAIHistoryModel::as_ref(ctx);
            if history_model.can_conversation_be_shared(&ai_conversation_id) {
                items.push(
                    MenuItemFields::new("Copy share link")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::CopyConversationShareLink {
                                conversation_id: ai_conversation_id,
                            },
                        ))
                        .into_item(),
                );
                items.push(
                    MenuItemFields::new("Share conversation")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::OpenConversationShareDialog {
                                conversation_id: ai_conversation_id,
                            },
                        ))
                        .into_item(),
                );
            }
        }

        items.push(
            MenuItemFields::new("Copy conversation text")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::CopyAIBlockConversation { ai_block_view_id },
                ))
                .into_item(),
        );

        items
    }

    fn conversation_text(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<String> {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            log::warn!("No conversation found for conversation ID {conversation_id}");
            return None;
        };

        let mut result = Vec::new();
        for exchange in conversation.root_task_exchanges() {
            let formatted_exchange =
                exchange.format_for_copy(Some(self.ai_action_model.as_ref(ctx)));
            if !formatted_exchange.is_empty() {
                result.push(formatted_exchange);
            }
        }

        if result.is_empty() {
            log::warn!("No copyable conversation text found for conversation ID {conversation_id}");
            return None;
        }

        Some(result.join("\n\n"))
    }

    pub(super) fn copy_conversation_text(
        &self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(conversation_text) = self.conversation_text(conversation_id, ctx) {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(conversation_text));
        }
    }

    pub(super) fn fork_ai_conversation(
        &self,
        conversation_id: AIConversationId,
        fork_from_exchange: Option<ForkFromExchange>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.dispatch_global_action(
            "workspace:fork_ai_conversation",
            ForkAIConversationParams {
                conversation_id,
                fork_from_exchange,
                summarize_after_fork: false,
                summarization_prompt: None,
                initial_prompt: None,
                destination: ForkedConversationDestination::SplitPane,
            },
        );
    }

    fn conversation_server_token(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<ServerConversationToken> {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        // Prefer loaded conversation data when available.
        history_model
            .conversation(&conversation_id)
            .and_then(|conversation| {
                conversation
                    .server_conversation_token()
                    .or_else(|| conversation.forked_from_server_conversation_token())
                    .cloned()
            })
            .or_else(|| {
                // Restored entries may only have server metadata loaded.
                history_model
                    .get_server_conversation_metadata(&conversation_id)
                    .map(|metadata| metadata.server_conversation_token.clone())
            })
    }

    fn conversation_debug_request_id(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<ServerOutputId> {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|conversation| conversation.root_task_exchanges().last())
            .and_then(|exchange| exchange.output_status.server_output_id())
    }

    fn copy_debugging_menu_items(
        &self,
        conversation_token: ServerConversationToken,
        server_output_id: Option<ServerOutputId>,
    ) -> Vec<(String, ContextMenuAction)> {
        if ChannelState::channel().is_dogfood() {
            vec![
                (
                    "Copy debugging link".to_string(),
                    ContextMenuAction::CopyAIDebuggingLink {
                        conversation_token: conversation_token.clone(),
                        request_id: server_output_id,
                    },
                ),
                (
                    "Copy conversation ID".to_string(),
                    ContextMenuAction::CopyConversationId {
                        conversation_id: conversation_token,
                    },
                ),
            ]
        } else {
            vec![(
                "Copy debugging ID".to_string(),
                ContextMenuAction::CopyExternalDebuggingId {
                    request_id: server_output_id,
                    conversation_id: conversation_token,
                },
            )]
        }
    }

    pub(super) fn create_copy_debugging_menu_item(
        &self,
        ai_exchange_id: AIAgentExchangeId,
        ai_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<(String, ContextMenuAction)> {
        let conversation_token = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&ai_conversation_id)
            .and_then(|convo| {
                convo
                    .server_conversation_token()
                    .or_else(|| convo.forked_from_server_conversation_token())
            });

        let Some(conversation_token) = conversation_token else {
            return Vec::new();
        };

        let server_output_id = self
            .ai_block_for_exchange(&ai_exchange_id)
            .and_then(|ai_block_handle| ai_block_handle.as_ref(ctx).server_output_id(ctx));
        self.copy_debugging_menu_items(conversation_token.clone(), server_output_id)
    }

    fn conversation_menu_items(
        &self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<TerminalAction>> {
        let mut items = Vec::new();

        if FeatureFlag::CloudConversations.is_enabled()
            && ShareableObject::AIConversation(conversation_id)
                .link(ctx)
                .is_some()
        {
            items.push(
                MenuItemFields::new("Copy share link")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::CopyConversationShareLink { conversation_id },
                    ))
                    .into_item(),
            );
        }

        items.push(
            MenuItemFields::new("Copy conversation text")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::CopyConversationText { conversation_id },
                ))
                .into_item(),
        );

        items.push(
            MenuItemFields::new("Fork")
                .with_on_select_action(TerminalAction::ContextMenu(
                    ContextMenuAction::ForkAIConversation { conversation_id },
                ))
                .into_item(),
        );

        if let Some(conversation_token) = self.conversation_server_token(conversation_id, ctx) {
            let server_output_id = self.conversation_debug_request_id(conversation_id, ctx);
            let debugging_items =
                self.copy_debugging_menu_items(conversation_token, server_output_id);
            if !debugging_items.is_empty() {
                if !items.is_empty() {
                    items.push(MenuItem::Separator);
                }
                for (button_text, action) in debugging_items {
                    items.push(
                        MenuItemFields::new(button_text)
                            .with_on_select_action(TerminalAction::ContextMenu(action))
                            .into_item(),
                    );
                }
            }
        }

        items
    }

    pub(super) fn open_agent_view_entry_context_menu(
        &mut self,
        conversation_id: AIConversationId,
        agent_view_entry_block_id: EntityId,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        self.show_context_menu(
            ContextMenuState {
                menu_type: ContextMenuType::AgentViewEntryConversation {
                    agent_view_entry_block_id,
                    position,
                },
            },
            self.conversation_menu_items(conversation_id, ctx),
            ctx,
        );
    }

    pub(super) fn open_ai_block_overflow_context_menu(
        &mut self,
        ai_block_view_id: EntityId,
        ai_exchange_id: AIAgentExchangeId,
        ai_conversation_id: AIConversationId,
        is_restored: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut menu_items = {
            let model = self.model.lock();
            self.ai_block_copying_menu_items(
                ai_block_view_id,
                ai_conversation_id,
                None,
                &model,
                ctx,
            )
        };

        if !cfg!(target_family = "wasm") {
            let fork_label = fork_label_for_query(
                &self
                    .rich_content_views
                    .iter()
                    .find_map(|rc| {
                        let meta = rc.ai_block_metadata()?;
                        (meta.ai_block_handle.id() == ai_block_view_id).then(|| {
                            meta.ai_block_handle
                                .as_ref(ctx)
                                .get_preceding_user_query(ctx)
                        })
                    })
                    .unwrap_or_default(),
            );
            menu_items.push(
                MenuItemFields::new(fork_label)
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::ForkAIConversationFromBlock {
                            ai_block_view_id,
                            exchange_id: ai_exchange_id,
                            conversation_id: ai_conversation_id,
                        },
                    ))
                    .into_item(),
            );

            if ChannelState::channel().is_dogfood() {
                menu_items.push(
                    MenuItemFields::new("Fork from here")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::ForkAIConversationFromExactExchange {
                                ai_block_view_id,
                                exchange_id: ai_exchange_id,
                                conversation_id: ai_conversation_id,
                            },
                        ))
                        .into_item(),
                );
            }
        }

        // We can't revert restored blocks since we don't restore the full diff
        if FeatureFlag::RevertToCheckpoints.is_enabled() && !is_restored {
            menu_items.push(
                MenuItemFields::new("Rewind to before here")
                    .with_on_select_action(TerminalAction::RewindAIConversation {
                        ai_block_view_id,
                        exchange_id: ai_exchange_id,
                        conversation_id: ai_conversation_id,
                        entrypoint: AgentModeRewindEntrypoint::ContextMenu,
                    })
                    .into_item(),
            );
        }

        let debugging_items =
            self.create_copy_debugging_menu_item(ai_exchange_id, ai_conversation_id, ctx);
        if !debugging_items.is_empty() {
            if !menu_items.is_empty() {
                menu_items.push(MenuItem::Separator);
            }
            for (button_text, action) in debugging_items {
                menu_items.push(
                    MenuItemFields::new(button_text)
                        .with_on_select_action(TerminalAction::ContextMenu(action))
                        .into_item(),
                );
            }
        }

        self.show_context_menu(
            ContextMenuState {
                menu_type: ContextMenuType::AIBlockOverflowMenu { ai_block_view_id },
            },
            menu_items,
            ctx,
        );
    }

    pub(super) fn show_context_menu(
        &mut self,
        menu_state: ContextMenuState,
        items: Vec<MenuItem<TerminalAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.update_view(&self.context_menu, |context_menu, view_ctx| {
            context_menu.set_origin(menu_state.menu_type.origin());
            context_menu.set_width(CONTEXT_MENU_WIDTH);
            // This will also reset the selection.
            context_menu.set_items(items, view_ctx);
        });

        self.context_menu_state = Some(menu_state);
        ctx.focus(&self.context_menu);
        ctx.notify();

        send_telemetry_from_ctx!(
            TelemetryEvent::OpenContextMenu {
                context_menu_info: ContextMenuInfo {
                    menu_type: menu_state.menu_type,
                }
            },
            ctx
        );
        self.tips_completed.update(ctx, |tips, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Hint(TipHint::BlockAction),
                tips,
                ctx,
            );
            ctx.notify();
        });
    }
}
