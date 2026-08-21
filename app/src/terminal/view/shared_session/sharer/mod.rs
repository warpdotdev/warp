pub mod inactivity_modal;
use async_channel::Sender;
use inactivity_modal::InactivityModal;
use warpui::r#async::SpawnedFutureHandle;
use warpui::elements::MouseStateHandle;
use warpui::{SingletonEntity, ViewContext, ViewHandle};

use crate::terminal::TerminalView;
use crate::terminal::shared_session::settings::{
    SharedSessionSettings, SharedSessionSettingsChangedEvent,
};

/// Where the sharer currently sits in the inactivity ladder, independent of which specific
/// timer (if any) is armed for the next phase -- needed to safely re-evaluate the ladder
/// when the duration settings change mid-session (see
/// `TerminalView::handle_shared_session_inactivity_settings_changed`), since re-arming from
/// scratch must not re-revoke roles that have already been revoked in this idle period.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InactivityLadderPosition {
    /// No phase has fired yet since the last activity reset (or ever, for a fresh
    /// session). The next phase to arm is whichever `next_inactivity_phase` returns.
    AwaitingFirstPhase,
    /// Roles have already been revoked in this idle period. The next phase to arm is
    /// whichever `next_phase_after_revoke` returns.
    RolesRevoked,
}

pub struct Sharer {
    pub(super) activity_tx: Sender<()>,
    pub(super) revoke_all_mouse_state_handle: MouseStateHandle,
    pub(super) inactivity_timer_abort_handle: Option<SpawnedFutureHandle>,
    pub(super) is_inactivity_warning_modal_open: bool,
    pub(super) inactivity_modal: ViewHandle<InactivityModal>,
    pub(super) ladder_position: InactivityLadderPosition,
}

impl Sharer {
    pub(super) fn new(activity_tx: Sender<()>, ctx: &mut ViewContext<TerminalView>) -> Self {
        let inactivity_modal = ctx.add_view(InactivityModal::new);
        ctx.subscribe_to_view(&inactivity_modal, |me, _, event, ctx| {
            me.handle_inactivity_modal_event(event, ctx)
        });

        // A live sharer's ladder must react to the duration settings changing mid-session,
        // not just at the moment a timer was originally armed -- otherwise a phase that was
        // just disabled (e.g. `end` while its zero-duration warning countdown is armed) can
        // still fire.
        ctx.subscribe_to_model(&SharedSessionSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                SharedSessionSettingsChangedEvent::InactivityPeriodBeforeRevokingRoles { .. }
                    | SharedSessionSettingsChangedEvent::InactivityPeriodBeforeWarning { .. }
                    | SharedSessionSettingsChangedEvent::InactivityPeriodBeforeEndingSession { .. }
            ) {
                me.handle_shared_session_inactivity_settings_changed(ctx);
            }
        });

        Self {
            activity_tx,
            revoke_all_mouse_state_handle: Default::default(),
            inactivity_timer_abort_handle: None,
            is_inactivity_warning_modal_open: false,
            inactivity_modal,
            ladder_position: InactivityLadderPosition::AwaitingFirstPhase,
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

    /// Closes the inactivity warning modal, including stopping its internal countdown --
    /// not just the flag that gates whether it renders. Without this, a countdown already
    /// in flight keeps ticking (and can still fire `TimedOut`) even once the modal is no
    /// longer shown. Safe to call even when the modal's own body already stopped its
    /// countdown itself (e.g. via a button click); stopping an already-stopped countdown
    /// is a no-op.
    pub fn close_inactivity_warning_modal(&mut self, ctx: &mut ViewContext<TerminalView>) {
        self.is_inactivity_warning_modal_open = false;
        self.inactivity_modal.update(ctx, |modal, ctx| {
            modal.stop_countdown(ctx);
        });
    }

    pub fn inactivity_modal(&self) -> &ViewHandle<InactivityModal> {
        &self.inactivity_modal
    }

    pub fn revoke_all_mouse_state_handle(&self) -> &MouseStateHandle {
        &self.revoke_all_mouse_state_handle
    }
}
