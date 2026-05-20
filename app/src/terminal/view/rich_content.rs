use warpui::{prelude::ChildView, Element, EntityId, View, ViewContext, ViewHandle};

use crate::{
    ai::{
        agent::{conversation::AIConversationId, AIAgentExchangeId},
        blocklist::{
            agent_view::AgentViewEntryOrigin, block::PendingUserQueryBlock,
            telemetry_banner::TelemetryBanner, AIBlock,
        },
    },
    env_vars::env_var_collection_block::EnvVarCollectionBlock,
    terminal::{
        block_list_viewport::ScrollPositionUpdate,
        model::{
            blocks::RichContentItem, rich_content::RichContentType, terminal_model::BlockIndex,
        },
        ssh::{error::SshErrorBlock, install_tmux::SshInstallTmuxBlock, warpify::SshWarpifyBlock},
        view::{
            ambient_agent::AmbientAgentEntryBlock,
            block_onboarding::onboarding_agentic_suggestions_block::OnboardingAgenticSuggestionsBlock,
            init_environment::InitEnvironmentBlock,
            ssh_remote_server_choice_view::SshRemoteServerChoiceView,
            ssh_remote_server_failed_banner::SshRemoteServerFailedBanner,
        },
        warpify::success_block::WarpifySuccessBlock,
        TerminalView,
    },
};

use super::{InitStepBlock, InitStepKind};

/// Specifies where to insert rich content in the blocklist.
#[derive(Clone, Copy, Debug)]
pub enum RichContentInsertionPosition {
    /// Append to the end of the blocklist. If `insert_below_long_running_block` is true
    /// and there is a long-running block, the content is inserted after that block.
    Append {
        insert_below_long_running_block: bool,
    },
    /// Insert before the block at the given index.
    BeforeBlockIndex(BlockIndex),
    /// Pin to the bottom of the blocklist. The BlockList will automatically
    /// keep this item at the end by reordering it after any subsequent insertions.
    /// Only one item can be pinned at a time.
    PinToBottom,
}

/// Metadata for an AI block rich content.
#[derive(Clone, Debug)]
pub struct AIBlockMetadata {
    /// The ID corresponding to the `AIAgentExchange` represented in this block.
    pub exchange_id: AIAgentExchangeId,
    /// The ID of the conversation to which this block belongs.
    pub conversation_id: AIConversationId,
    /// The ViewHandle for the AI block.
    pub ai_block_handle: ViewHandle<AIBlock>,
}

/// Metadata for an agent view entry rich content.
#[derive(Clone, Debug)]
pub struct AgentViewEntryMetadata {
    pub conversation_id: AIConversationId,
    /// The origin when this block was created (not the current session origin).
    pub origin: AgentViewEntryOrigin,
}

/// Wrapper type to hold rich content views and allow generating typed `ChildView` instances
/// on-demand. The `ChildView`s are then passed to the `BlockListElement` to be used when
/// displaying rich content.
pub struct RichContent {
    view_id: EntityId,
    element_builder: Box<dyn Fn() -> Box<dyn Element>>,

    /// Optional rich content view-specific metadata to be passed to the `BlocklistElement` for
    /// rendering.
    metadata: Option<RichContentMetadata>,

    /// The conversation ID of the active agent view when this rich content was created, if any.
    /// This is used to determine visibility when switching between agent view conversations.
    /// Rich content created within an agent view should only be visible when that conversation
    /// is active.
    agent_view_conversation_id: Option<AIConversationId>,
}

impl RichContent {
    /// Create a new `RichContent` using a ViewHandle. The RichContent type will continue to own
    /// the ViewHandle for its lifetime, ensuring that the underlying View remains active.
    ///
    /// `ai_conversation_id` should be the active agent view conversation ID if this content is
    /// being created within an agent view, or `None` if created in terminal mode.
    pub fn new<V: View>(
        handle: ViewHandle<V>,
        agent_view_conversation_id: Option<AIConversationId>,
    ) -> Self {
        let view_id = handle.id();
        // By `move`ing the handle into the closure, the closure will own the handle and keep it
        // alive for the duration. This also allows us to generate any number of necessary
        // `ChildView` instances
        let element_builder = Box::new(move || ChildView::new(&handle).finish());

        Self {
            view_id,
            element_builder,
            metadata: None,
            agent_view_conversation_id,
        }
    }

    pub fn with_metadata(mut self, metadata: RichContentMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Returns the conversation ID of the agent view this content was created in, if any.
    pub fn agent_view_conversation_id(&self) -> Option<AIConversationId> {
        self.agent_view_conversation_id
    }

    /// Updates the associated agent view conversation id with this rich content item.
    pub fn update_agent_view_conversation_id(
        &mut self,
        new_agent_view_conversation_id: AIConversationId,
    ) {
        self.agent_view_conversation_id = Some(new_agent_view_conversation_id);
    }

    /// Sets the associated agent view conversation id for this rich content item.
    pub fn set_agent_view_conversation_id(
        &mut self,
        agent_view_conversation_id: Option<AIConversationId>,
    ) {
        self.agent_view_conversation_id = agent_view_conversation_id;
    }

    /// Build a new `ChildView` element for this rich content
    fn element(&self) -> Box<dyn Element> {
        (self.element_builder)()
    }

    pub fn view_id(&self) -> EntityId {
        self.view_id
    }

    /// Returns a reference to the metadata, if present.
    pub fn metadata(&self) -> Option<&RichContentMetadata> {
        self.metadata.as_ref()
    }

    pub fn metadata_mut(&mut self) -> Option<&mut RichContentMetadata> {
        self.metadata.as_mut()
    }

    pub fn is_ai_block(&self) -> bool {
        matches!(self.metadata, Some(RichContentMetadata::AIBlock(_)))
    }

    pub fn is_usage_footer(&self) -> bool {
        matches!(self.metadata, Some(RichContentMetadata::UsageFooter))
    }

    pub fn is_telemetry_banner(&self) -> bool {
        matches!(
            self.metadata,
            Some(RichContentMetadata::TelemetryBanner { .. })
        )
    }

    pub fn is_agent_view_entry(&self) -> bool {
        matches!(self.metadata, Some(RichContentMetadata::AgentViewEntry(_)))
    }

    pub fn is_inline_agent_view_header(&self) -> bool {
        matches!(
            self.metadata,
            Some(RichContentMetadata::InlineAgentViewHeader)
        )
    }

    pub fn is_agent_view_zero_state(&self) -> bool {
        matches!(self.metadata, Some(RichContentMetadata::AgentViewZeroState))
    }

    pub fn is_pending_user_query(&self) -> bool {
        matches!(
            self.metadata,
            Some(RichContentMetadata::PendingUserQuery { .. })
        )
    }

    pub fn is_init_step(&self) -> bool {
        matches!(self.metadata, Some(RichContentMetadata::InitStep { .. }))
    }

    pub fn init_step_kind(&self) -> Option<InitStepKind> {
        match &self.metadata {
            Some(RichContentMetadata::InitStep { step_kind, .. }) => Some(*step_kind),
            _ => None,
        }
    }

    pub fn init_step_block_handle(&self) -> Option<&ViewHandle<InitStepBlock>> {
        match &self.metadata {
            Some(RichContentMetadata::InitStep { block_handle, .. }) => Some(block_handle),
            _ => None,
        }
    }

    pub fn ai_block_metadata(&self) -> Option<&AIBlockMetadata> {
        match &self.metadata {
            Some(RichContentMetadata::AIBlock(metadata)) => Some(metadata),
            _ => None,
        }
    }

    pub fn agent_view_entry_metadata(&self) -> Option<&AgentViewEntryMetadata> {
        match &self.metadata {
            Some(RichContentMetadata::AgentViewEntry(metadata)) => Some(metadata),
            _ => None,
        }
    }

    pub(super) fn to_block_list_element_render_params(
        &self,
    ) -> (EntityId, Box<dyn Element>, Option<RichContentMetadata>) {
        (self.view_id(), self.element(), self.metadata.clone())
    }
}

/// `RichContent` view-specific metadata required for rendering in the `BlocklistElement`.
#[derive(Clone, Debug)]
pub enum RichContentMetadata {
    AIBlock(AIBlockMetadata),
    AIOnboardingBlock {
        /// The ID corresponding to the `AIAgentExchange` represented in this block.
        exchange_id: AIAgentExchangeId,
    },
    UsageFooter,
    InitStep {
        step_kind: InitStepKind,
        block_handle: ViewHandle<InitStepBlock>,
    },
    InitEnvironment {
        block_handle: ViewHandle<InitEnvironmentBlock>,
    },
    OnboardingAgenticSuggestions {
        agentic_suggestions_block_handle: ViewHandle<OnboardingAgenticSuggestionsBlock>,
    },
    EnvVarCollectionBlock {
        env_var_collection_block_handle: ViewHandle<EnvVarCollectionBlock>,
    },
    SshWarpifyBlock {
        ssh_warpify_block_handle: ViewHandle<SshWarpifyBlock>,
    },
    SshInstallTmuxBlock {
        ssh_install_tmux_block_handle: ViewHandle<SshInstallTmuxBlock>,
    },
    SshErrorBlock {
        ssh_error_block_handle: ViewHandle<SshErrorBlock>,
    },
    SshRemoteServerChoiceBlock {
        handle: ViewHandle<SshRemoteServerChoiceView>,
    },
    SshRemoteServerFailedBanner {
        handle: ViewHandle<SshRemoteServerFailedBanner>,
    },
    WarpifySuccessBlock {
        bootstrap_success_block_handle: ViewHandle<WarpifySuccessBlock>,
    },
    TelemetryBanner {
        telemetry_banner_handle: ViewHandle<TelemetryBanner>,
    },
    AgentViewEntry(AgentViewEntryMetadata),
    AmbientAgentBlock {
        block_handle: ViewHandle<AmbientAgentEntryBlock>,
    },
    InlineAgentViewHeader,
    AgentViewZeroState,
    TerminalViewZeroState,
    PluginInstructionsBlock,
    PendingUserQuery {
        pending_user_query_block_handle: ViewHandle<PendingUserQueryBlock>,
    },
    HarnessSessionHeader,
}

impl TerminalView {
    /// Add a rich content `View` to the block list. This view can contain any content
    /// we want to display, however it must be exactly `height_px` tall. It will take up that much
    /// space in the block list and when it is laid out in the scene, it will be passed that height
    /// as a strict constraint to the `Element::layout` method.
    ///
    /// The `position` parameter controls where the content is inserted:
    /// - `Append`: Adds to the end; if `insert_below_long_running_block` is true and there's a
    ///   long-running block, the content is inserted after that block.
    /// - `BeforeBlockIndex`: Inserts before the specified block index.
    pub fn insert_rich_content<V: View>(
        &mut self,
        content_type: Option<RichContentType>,
        handle: ViewHandle<V>,
        metadata: Option<RichContentMetadata>,
        position: RichContentInsertionPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        // Agent view entry blocks, inline agent view headers, and terminal zero state blocks
        // should not be associated with any conversation, as they always belong in the top-level
        // terminal view and should be hidden while agent view is active.
        let is_agent_view_scoped_terminal_content = matches!(
            metadata,
            Some(
                RichContentMetadata::AgentViewEntry(_)
                    | RichContentMetadata::InlineAgentViewHeader
                    | RichContentMetadata::TerminalViewZeroState
            )
        );
        let is_use_agent_footer = handle.id() == self.use_agent_footer.id();

        let (agent_view_conversation_id, should_hide) = if is_agent_view_scoped_terminal_content {
            (None, self.agent_view_controller.as_ref(ctx).is_active())
        } else if is_use_agent_footer {
            (
                self.agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .fullscreen_conversation_id(),
                false,
            )
        } else {
            (
                self.agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                false,
            )
        };
        let item = RichContentItem::new(
            content_type,
            handle.id(),
            agent_view_conversation_id,
            should_hide,
        );

        match position {
            RichContentInsertionPosition::Append {
                insert_below_long_running_block,
            } => {
                self.model
                    .lock()
                    .block_list_mut()
                    .append_rich_content(item, insert_below_long_running_block);
            }
            RichContentInsertionPosition::BeforeBlockIndex(block_index) => {
                self.model
                    .lock()
                    .block_list_mut()
                    .insert_rich_content_before_block_index(item, block_index);
            }
            RichContentInsertionPosition::PinToBottom => {
                self.model
                    .lock()
                    .block_list_mut()
                    .append_rich_content_pinned_to_bottom(item);
            }
        }

        let mut rich_content = RichContent::new(handle, agent_view_conversation_id);
        if let Some(metadata) = metadata {
            rich_content = rich_content.with_metadata(metadata);
        }
        self.rich_content_views.push(rich_content);

        self.update_input_prompt_suggestions_banner_state(ctx);

        // Scroll to bottom
        self.update_scroll_position_locking(ScrollPositionUpdate::AfterRichBlockInserted, ctx);

        ctx.notify();
    }
}
