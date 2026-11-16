pub mod inactivity_modal;
use async_channel::Sender;
use inactivity_modal::InactivityModal;
use warpui::r#async::SpawnedFutureHandle;
use warpui::elements::MouseStateHandle;
use warpui::{SingletonEntity, ViewContext, ViewHandle};

use crate::terminal::TerminalView;
use crate::terminal::shared_session::settings::SharedSessionSettings;

pub struct Sharer {
    pub(super) activity_tx: Sender<()>,
    pub(super) revoke_all_mouse_state_handle: MouseStateHandle,
    pub(super) inactivity_timer_abort_handle: Option<SpawnedFutureHandle>,
    pub(super) is_inactivity_warning_modal_open: bool,
    pub(super) inactivity_modal: ViewHandle<InactivityModal>,
}

impl Sharer {
    pub(super) fn new(activity_tx: Sender<()>, ctx: &mut ViewContext<TerminalView>) -> Self {
        let inactivity_modal = ctx.add_view(InactivityModal::new);
        ctx.subscribe_to_view(&inactivity_modal, |me, _, event, ctx| {
            me.handle_inactivity_modal_event(event, ctx)
        });

        Self {
            activity_tx,
            revoke_all_mouse_state_handle: Default::default(),
            inactivity_timer_abort_handle: None,
            is_inactivity_warning_modal_open: false,
            inactivity_modal,
        }
    }

    pub fn activity_tx(&self) -> &Sender<()> {
        &self.activity_tx
    }

    pub fn is_inactivity_warning_modal_open(&self) -> bool {
        self.is_inactivity_warning_modal_open
    }

    /// Opens inactivity warning modal and resets the timer.
    pub fn open_inactivity_warning_modal(&mut self, ctx: &mut ViewContext<TerminalView>) {
        let duration = SharedSessionSettings::as_ref(ctx)
            .inactivity_period_between_warning_and_ending_session();
        self.is_inactivity_warning_modal_open = true;
        self.inactivity_modal.update(ctx, |modal, ctx| {
            modal.reset_timer(duration, ctx);
        });
        ctx.focus(&self.inactivity_modal);
    }

    pub fn close_inactivity_warning_modal(&mut self) {
        self.is_inactivity_warning_modal_open = false
    }

    pub fn inactivity_modal(&self) -> &ViewHandle<InactivityModal> {
        &self.inactivity_modal
    }

    pub fn revoke_all_mouse_state_handle(&self) -> &MouseStateHandle {
        &self.revoke_all_mouse_state_handle
    }
}
