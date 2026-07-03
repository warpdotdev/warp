use warpui::{Entity, ModelContext, ViewHandle};

use crate::ai::blocklist::inline_action::code_diff_view::{CodeDiffView, CodeDiffViewEvent};

/// Intermediate model between CodeDiffPane and CodeDiffView.
/// This model is needed because if the PaneGroup subscribes directly to the CodeDiffView,
/// then the PaneGroup could not unsubscribe during detach.
/// This is because unsubscribing does not work when handling an event from the subscribed view.
pub struct CodeDiffPaneModel {}

impl CodeDiffPaneModel {
    pub fn new(view: ViewHandle<CodeDiffView>, ctx: &mut ModelContext<Self>) -> Self {
        // Subscribe to the CodeDiffView events
        ctx.subscribe_to_view(&view, |_model, _, event, ctx| ctx.emit(event.clone()));

        Self {}
    }
}

impl Entity for CodeDiffPaneModel {
    type Event = CodeDiffViewEvent;
}
