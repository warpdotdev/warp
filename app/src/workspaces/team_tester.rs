use warpui::{Entity, ModelContext, SingletonEntity};

#[derive(Clone)]
pub struct TeamTesterStatus {}

impl TeamTesterStatus {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {}
    }

    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        Self::new(ctx)
    }

    /// Emit an event to start or force-refresh the cloud object and workspace metadata pollers.
    /// Polling is started when a user logs in; this method is also called with
    /// `force_refresh: true` when data is known to be invalidated (e.g. joining a team via an
    /// intent link).
    pub fn initiate_data_pollers(&mut self, force_refresh: bool, ctx: &mut ModelContext<Self>) {
        ctx.emit(TeamTesterStatusEvent::InitiateDataPollers { force_refresh })
    }
}

pub enum TeamTesterStatusEvent {
    InitiateDataPollers {
        /// If true, the subscriber should attempt to refresh any state
        /// immediately rather than just wait for the next poll.
        /// Specifically used when a user joins a team via an intent link.
        force_refresh: bool,
    },
}

impl Entity for TeamTesterStatus {
    type Event = TeamTesterStatusEvent;
}

impl SingletonEntity for TeamTesterStatus {}
