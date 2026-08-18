cfg_if::cfg_if! {
    if #[cfg(not(target_family = "wasm"))] {
        mod info;
        mod memory_footprint;
        pub use info::SystemInfo;
    }
}

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

#[cfg(not(target_family = "wasm"))]
pub fn long_os_version(ctx: &warpui::AppContext) -> Option<String> {
    crate::system::SystemInfo::as_ref(ctx)
        .long_os_version()
        .map(ToOwned::to_owned)
}

#[cfg(target_family = "wasm")]
pub fn long_os_version(_ctx: &warpui::AppContext) -> Option<String> {
    None
}
