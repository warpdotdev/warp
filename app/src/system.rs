mod info;
mod memory_footprint;
pub use info::SystemInfo;

use warpui::{Entity, ModelContext, SingletonEntity};

#[derive(Clone, Copy, Default, PartialEq)]
pub struct SystemStats;

impl SystemStats {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn dispatch_cpu_was_awakened(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(SystemStatsEvent::CpuWasAwakened);
    }

    pub fn dispatch_cpu_will_sleep(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(SystemStatsEvent::CpuWillSleep);
    }
}

pub enum SystemStatsEvent {
    CpuWasAwakened,
    CpuWillSleep,
}

impl Entity for SystemStats {
    type Event = SystemStatsEvent;
}

impl SingletonEntity for SystemStats {}
