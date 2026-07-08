use std::any::Any;
use std::sync::Arc;

use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use warpui::{AppContext, ModelHandle, SingletonEntity, ViewHandle, WindowId};

use super::event_listener::ChannelEventListener;
use super::model::session::Sessions;
use super::model_events::ModelEventDispatcher;
use super::terminal_manager::BlockSpacing;
use super::{ShellLaunchState, TerminalManager, TerminalModel, TerminalView};
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::context_chips::prompt_type::PromptType;
use crate::pane_group::TerminalViewResources;
use crate::terminal::view::ConversationRestorationInNewPaneType;

pub struct MockTerminalManager {
    model: Arc<FairMutex<TerminalModel>>,
    view: ViewHandle<TerminalView>,
}
pub struct MockTerminalManagerInit {
    pub(crate) manager: ModelHandle<Box<dyn TerminalManager>>,
    pub(crate) view: ViewHandle<TerminalView>,
}

impl MockTerminalManager {
    pub fn create_model(
        shell_state: ShellLaunchState,
        resources: TerminalViewResources,
        restored_blocks: Option<&Vec<SerializedBlockListItem>>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        initial_size: Vector2F,
        window_id: WindowId,
        ctx: &mut AppContext,
    ) -> MockTerminalManagerInit {
        // Create all the necessary channels we need for communication.
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (pty_reads_tx, _pty_reads_rx) = async_broadcast::broadcast(1);
        let (executor_command_tx, _executor_command_rx) = async_channel::unbounded();

        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

        let model = super::terminal_manager::create_terminal_model(
            None,
            restored_blocks,
            initial_size,
            channel_event_proxy,
            shell_state,
            BlockSpacing::for_gui(ctx),
            ctx,
        );
        let colors = model.colors();
        let model = Arc::new(FairMutex::new(model));

        let sessions: ModelHandle<Sessions> =
            ctx.add_model(|ctx| Sessions::new(executor_command_tx, ctx));
        let model_events_dispatcher =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));

        let cloned_model = model.clone();
        let prompt_type =
            ctx.add_model(|ctx| PromptType::new_dynamic_from_sessions(sessions.clone(), ctx));
        let view = ctx.add_typed_action_view(window_id, |ctx| {
            let size_info = cloned_model.lock().block_list().size().to_owned();
            TerminalView::new(
                resources,
                wakeups_rx,
                model_events_dispatcher.clone(),
                cloned_model,
                sessions.clone(),
                size_info,
                colors,
                None,
                prompt_type,
                None,
                // We use conversation restoration to load a view-only cloud conversation
                // into the web view.
                conversation_restoration,
                None, // inactive_pty_reads_rx
                false,
                ctx,
            )
        });

        // Ensure we retain the shell events model for as long as the
        // terminal view lives by giving ownership to a closure which
        // is held onto for the duration of a task that never completes
        // (and will only be discarded when the TerminalView's refcount
        // drops to 0).
        view.update(ctx, |_view, ctx| {
            ctx.spawn(futures::future::pending::<()>(), move |_, _, _| {
                std::mem::drop(model_events_dispatcher);
            });
        });

        let terminal_view = view.clone();
        let terminal_manager = Self { model, view };
        let manager_model = ctx.add_model(|_ctx| {
            let manager: Box<dyn TerminalManager> = Box::new(terminal_manager);
            manager
        });
        MockTerminalManagerInit {
            manager: manager_model,
            view: terminal_view,
        }
    }
}

impl TerminalManager for MockTerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn on_view_detached(
        &self,
        _detach_type: crate::pane_group::pane::DetachType,
        app: &mut AppContext,
    ) {
        // If this is a conversation transcript viewer, unregister the ambient session.
        if self.model.lock().is_conversation_transcript_viewer() {
            let terminal_view_id = self.view.id();
            ActiveAgentViewsModel::handle(app).update(app, |model, ctx| {
                model.unregister_ambient_session(terminal_view_id, ctx);
            });
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod testing {
    use warpui::platform::WindowStyle;
    use warpui::{App, Element, SingletonEntity};

    use super::*;
    use crate::server::server_api::ServerApiProvider;
    use crate::terminal::shell::{ShellName, ShellType};
    use crate::terminal::ShellLaunchState;

    struct TerminalRootView {
        terminal_view: ViewHandle<TerminalView>,
    }

    impl warpui::Entity for TerminalRootView {
        type Event = ();
    }

    impl warpui::View for TerminalRootView {
        fn ui_name() -> &'static str {
            "TerminalRootView"
        }

        fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
            warpui::elements::ChildView::new(&self.terminal_view).finish()
        }
    }

    impl warpui::TypedActionView for TerminalRootView {
        type Action = ();
    }

    impl MockTerminalManager {
        pub fn create_new_terminal_view_window_for_test(
            app: &mut App,
            restored_blocks: Option<&[SerializedBlockListItem]>,
        ) -> ViewHandle<TerminalView> {
            let server_api = app.read(|ctx| ServerApiProvider::as_ref(ctx).get());
            let tips_model = app.add_model(|_| Default::default());

            let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
                let resources = TerminalViewResources {
                    tips_completed: tips_model,
                    server_api,
                    model_event_sender: None,
                };
                let terminal_init = MockTerminalManager::create_model(
                    ShellLaunchState::ShellSpawned {
                        available_shell: None,
                        display_name: ShellName::blank(),
                        shell_type: ShellType::Zsh,
                    },
                    resources,
                    restored_blocks.map(|blocks| blocks.to_vec()).as_ref(),
                    None,
                    Vector2F::new(7., 10.5),
                    ctx.window_id(),
                    ctx,
                );
                let terminal_view = terminal_init.view;

                TerminalRootView { terminal_view }
            });

            app.views_of_type::<TerminalView>(window_id)
                .expect("just created window")
                .first()
                .expect("window should have a TerminalView")
                .clone()
        }
    }
}
