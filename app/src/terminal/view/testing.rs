//! Module for test-only convenience methods on `TerminalView`.
use warpui::ModelHandle;

use crate::terminal::find::TerminalFindModel;
cfg_if::cfg_if! {
    if #[cfg(test)] {
        use std::sync::Arc;

        use parking_lot::FairMutex;
        use warpui::{ViewContext};

        use crate::{
            ai::blocklist::SerializedBlockListItem, pane_group::TerminalViewResources,
            resource_center::TipsCompleted,
        };
        use crate::terminal::model::session::Sessions;
        use crate::terminal::model_events::ModelEventDispatcher;
        use crate::terminal::view::WARP_PROMPT_HEIGHT_LINES;
        use crate::terminal::{SizeInfo, TerminalModel};

        use crate::context_chips::prompt_type::PromptType;
        use crate::terminal::color::List;
    }
}

use super::TerminalView;

impl TerminalView {
    #[cfg(test)]
    pub fn new_for_test(
        tips_model: ModelHandle<TipsCompleted>,
        restored_blocks: Option<&[SerializedBlockListItem]>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_for_test_with_cloud_mode(tips_model, restored_blocks, false, ctx)
    }

    #[cfg(test)]
    pub fn new_for_test_with_cloud_mode(
        tips_model: ModelHandle<TipsCompleted>,
        restored_blocks: Option<&[SerializedBlockListItem]>,
        is_cloud_mode: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        use pathfinder_geometry::vector::vec2f;
        use warpui::units::{IntoPixels as _, Pixels};

        use crate::server::server_api::ServerApiProvider;
        use crate::terminal::BlockPadding;
        use crate::terminal::event_listener::ChannelEventListener;
        use crate::terminal::model::block::BlockSize;
        use crate::themes::default_themes::dark_theme;
        let size_info = SizeInfo::new(
            vec2f(7., 10.5),
            1.0.into_pixels(),
            1.0.into_pixels(),
            Pixels::zero(),
            Pixels::zero(),
        );
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let event_proxy = ChannelEventListener::builder_for_test()
            .with_terminal_events_tx(events_tx)
            .with_wakeups_tx(wakeups_tx)
            .build();

        let colors = List::from(&dark_theme().into());

        let block_padding = BlockPadding {
            padding_top: 0.5,
            command_padding_top: 0.5,
            middle: 0.5,
            bottom: 0.5,
        };
        let max_block_scroll_limit = 1000;
        let sizes = BlockSize {
            block_padding,
            size: size_info,
            max_block_scroll_limit,
            warp_prompt_height_lines: WARP_PROMPT_HEIGHT_LINES,
        };

        let server_api = ServerApiProvider::new_for_test().get();
        let terminal_view_resources = TerminalViewResources {
            tips_completed: tips_model,
            server_api: server_api.clone(),
            model_event_sender: None,
        };

        let model = Arc::new(FairMutex::new(TerminalModel::new_for_test(
            sizes,
            colors,
            event_proxy,
            ctx.background_executor().clone(),
            false,
            restored_blocks,
            false, /* honor_ps1 */
            false, /* is_inverted */
            None,  /* startup_directory */
        )));

        let sessions = ctx.add_model(|_| Sessions::new_for_test());
        let model_events =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
        let prompt_type =
            ctx.add_model(|ctx| PromptType::new_dynamic_from_sessions(sessions.clone(), ctx));

        Self::new(
            terminal_view_resources,
            wakeups_rx,
            model_events,
            model,
            sessions,
            size_info,
            colors,
            None,
            prompt_type,
            None,
            None, // conversation_restoration - not used for test
            None, // inactive_pty_reads_rx - not used for test
            is_cloud_mode,
            ctx,
        )
    }

    pub fn find_model(&self) -> &ModelHandle<TerminalFindModel> {
        &self.find_model
    }

    #[cfg(test)]
    pub fn rich_content_view_count_for_test(&self) -> usize {
        self.rich_content_views.len()
    }
}
