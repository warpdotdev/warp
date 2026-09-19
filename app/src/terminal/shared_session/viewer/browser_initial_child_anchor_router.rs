use std::collections::{HashMap, HashSet};

#[cfg(target_family = "wasm")]
use warpui::ModelHandle;
use warpui::{Entity, ModelContext, SingletonEntity, WeakViewHandle};

#[cfg(target_family = "wasm")]
use super::orchestration_viewer_model::{OrchestrationViewerModel, OrchestrationViewerModelEvent};
use crate::ai::agent::conversation::AIConversationId;
#[cfg(any(target_family = "wasm", test))]
use crate::ai::ambient_agents::AmbientAgentTask;
use crate::ai::ambient_agents::AmbientAgentTaskId;
#[cfg(target_family = "wasm")]
use crate::ai::blocklist::orchestration_event_streamer::{
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::{Event as TerminalViewEvent, TerminalView};
#[cfg(target_family = "wasm")]
use crate::uri::browser_url_handler::parse_current_url;
#[cfg(target_family = "wasm")]
use crate::uri::viewer_location::ViewerLocation;
use crate::uri::viewer_location::{
    ChildAnchor, HydratedAnchorAction, hydrated_anchor_action, is_expected_direct_child,
};

#[cfg(any(target_family = "wasm", test))]
pub(crate) enum BrowserInitialChildAnchorRouterEvent {
    VerifiedChildFetched { task: AmbientAgentTask },
}

pub(crate) struct BrowserInitialChildAnchorRouter {
    parent_task_id: AmbientAgentTaskId,
    terminal_view: WeakViewHandle<TerminalView>,
    #[cfg(target_family = "wasm")]
    verified_child_viewer_model: Option<ModelHandle<OrchestrationViewerModel>>,
    initial_child_anchor: ChildAnchor,
    seeded_child_ids: Option<HashSet<AmbientAgentTaskId>>,
    registered_children: HashMap<AmbientAgentTaskId, AIConversationId>,
    initial_anchor_resolution_emitted: bool,
    initial_anchor_fetch_in_flight: bool,
}

impl Entity for BrowserInitialChildAnchorRouter {
    #[cfg(any(target_family = "wasm", test))]
    type Event = BrowserInitialChildAnchorRouterEvent;
    #[cfg(not(any(target_family = "wasm", test)))]
    type Event = ();
}

impl BrowserInitialChildAnchorRouter {
    #[cfg(target_family = "wasm")]
    pub(super) fn new_for_viewer(
        parent_task_id: AmbientAgentTaskId,
        terminal_view: WeakViewHandle<TerminalView>,
        orchestration_viewer_model: ModelHandle<OrchestrationViewerModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&orchestration_viewer_model, |router, _, event, ctx| {
            let OrchestrationViewerModelEvent::ChildRegistered {
                task_id,
                conversation_id,
            } = event;
            router.child_registered(*task_id, *conversation_id, ctx);
        });
        ctx.subscribe_to_model(
            &OrchestrationEventStreamer::handle(ctx),
            |router, _, event, ctx| {
                let OrchestrationEventStreamerEvent::ViewerModeSeeded {
                    parent_task_id,
                    child_run_ids,
                } = event
                else {
                    return;
                };
                router.viewer_mode_seeded(*parent_task_id, child_run_ids, ctx);
            },
        );
        let registered_children = orchestration_viewer_model.as_ref(ctx).registered_children();
        let mut router = Self::new(parent_task_id, terminal_view);
        router.verified_child_viewer_model = Some(orchestration_viewer_model);
        for (task_id, conversation_id) in registered_children {
            router.child_registered(task_id, conversation_id, ctx);
        }
        router
    }

    #[cfg(target_family = "wasm")]
    pub(crate) fn new(
        parent_task_id: AmbientAgentTaskId,
        terminal_view: WeakViewHandle<TerminalView>,
    ) -> Self {
        let initial_child_anchor = parse_current_url()
            .as_ref()
            .and_then(ViewerLocation::parse)
            .map(|location| location.child_anchor)
            .unwrap_or(ChildAnchor::Root);
        Self::new_with_anchor(parent_task_id, terminal_view, initial_child_anchor)
    }

    fn new_with_anchor(
        parent_task_id: AmbientAgentTaskId,
        terminal_view: WeakViewHandle<TerminalView>,
        initial_child_anchor: ChildAnchor,
    ) -> Self {
        Self {
            parent_task_id,
            terminal_view,
            #[cfg(target_family = "wasm")]
            verified_child_viewer_model: None,
            initial_child_anchor,
            seeded_child_ids: None,
            registered_children: HashMap::new(),
            initial_anchor_resolution_emitted: false,
            initial_anchor_fetch_in_flight: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        parent_task_id: AmbientAgentTaskId,
        terminal_view: WeakViewHandle<TerminalView>,
        initial_child_anchor: ChildAnchor,
    ) -> Self {
        Self::new_with_anchor(parent_task_id, terminal_view, initial_child_anchor)
    }
    pub(crate) fn child_registered(
        &mut self,
        task_id: AmbientAgentTaskId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.registered_children.insert(task_id, conversation_id);
        self.maybe_resolve_initial_child_anchor(ctx);
    }

    pub(crate) fn viewer_mode_seeded(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        child_run_ids: &[AmbientAgentTaskId],
        ctx: &mut ModelContext<Self>,
    ) {
        if parent_task_id == self.parent_task_id {
            self.seeded_child_ids = Some(child_run_ids.iter().copied().collect());
            self.maybe_resolve_initial_child_anchor(ctx);
        }
    }

    fn maybe_resolve_initial_child_anchor(&mut self, ctx: &mut ModelContext<Self>) {
        if self.initial_anchor_resolution_emitted {
            return;
        }
        let Some(seeded_child_ids) = self.seeded_child_ids.as_ref() else {
            return;
        };
        let registered_child_ids = self.registered_children.keys().copied().collect();
        let conversation_id = match hydrated_anchor_action(
            self.initial_child_anchor,
            seeded_child_ids,
            &registered_child_ids,
        ) {
            HydratedAnchorAction::None => {
                self.initial_anchor_resolution_emitted = true;
                return;
            }
            HydratedAnchorAction::Wait => return,
            HydratedAnchorAction::FetchAndVerify(task_id) => {
                self.fetch_initial_anchor_task(task_id, ctx);
                return;
            }
            HydratedAnchorAction::Clear => None,
            HydratedAnchorAction::Select(task_id) => Some(self.registered_children[&task_id]),
        };
        self.finish_initial_anchor_resolution(conversation_id, ctx);
    }

    fn fetch_initial_anchor_task(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.initial_anchor_fetch_in_flight {
            return;
        }
        self.initial_anchor_fetch_in_flight = true;
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let parent_task_id = self.parent_task_id;
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |router, result, ctx| {
                router.handle_initial_anchor_fetch_result(task_id, parent_task_id, result, ctx);
            },
        );
    }

    fn handle_initial_anchor_fetch_result(
        &mut self,
        task_id: AmbientAgentTaskId,
        parent_task_id: AmbientAgentTaskId,
        result: anyhow::Result<AmbientAgentTask>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.initial_anchor_fetch_in_flight = false;
        match result {
            Ok(task)
                if task.task_id == task_id
                    && is_expected_direct_child(&task, task_id, parent_task_id) =>
            {
                #[cfg(target_family = "wasm")]
                if let Some(model) = self.verified_child_viewer_model.clone() {
                    model.update(ctx, |model, ctx| {
                        model.register_child(task, ctx);
                    });
                } else {
                    ctx.emit(BrowserInitialChildAnchorRouterEvent::VerifiedChildFetched { task });
                }
                #[cfg(all(test, not(target_family = "wasm")))]
                ctx.emit(BrowserInitialChildAnchorRouterEvent::VerifiedChildFetched { task });
                #[cfg(not(any(target_family = "wasm", test)))]
                let _ = task;
            }
            Ok(_) | Err(_) => {
                self.finish_initial_anchor_resolution(None, ctx);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn complete_initial_anchor_fetch_for_test(
        &mut self,
        task: AmbientAgentTask,
        ctx: &mut ModelContext<Self>,
    ) {
        self.seeded_child_ids = Some(HashSet::new());
        self.initial_anchor_fetch_in_flight = true;
        self.handle_initial_anchor_fetch_result(task.task_id, self.parent_task_id, Ok(task), ctx);
    }

    fn finish_initial_anchor_resolution(
        &mut self,
        conversation_id: Option<AIConversationId>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.initial_anchor_resolution_emitted {
            return;
        }
        self.initial_anchor_resolution_emitted = true;
        if let Some(view) = self.terminal_view.upgrade(ctx) {
            view.update(ctx, |_view, ctx| {
                ctx.emit(TerminalViewEvent::RestoreInitialChildAnchor { conversation_id });
            });
        }
    }
}

#[cfg(test)]
#[path = "browser_initial_child_anchor_router_tests.rs"]
mod tests;
