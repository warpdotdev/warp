use warpui::{AppContext, ViewHandle};

use super::AIBlock;
use crate::terminal::model::blocks::BlockList;

pub(crate) trait BlockListAiViewExt {
    fn last_non_hidden_ai_block_handle(&self, app: &AppContext) -> Option<ViewHandle<AIBlock>>;
    fn has_active_ai_block(&self, app: &AppContext) -> bool;
}

impl BlockListAiViewExt for BlockList {
    fn last_non_hidden_ai_block_handle(&self, app: &AppContext) -> Option<ViewHandle<AIBlock>> {
        let rich_content_view_id = self
            .last_non_hidden_rich_content_block_after_block(None)?
            .1
            .view_id;
        let active_window_id = app.windows().active_window()?;
        app.view_with_id::<AIBlock>(active_window_id, rich_content_view_id)
    }

    fn has_active_ai_block(&self, app: &AppContext) -> bool {
        self.last_non_hidden_ai_block_handle(app)
            .is_some_and(|handle| !handle.as_ref(app).is_finished())
    }
}
