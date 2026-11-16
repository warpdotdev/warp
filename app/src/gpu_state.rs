use warpui::{Entity, ModelContext, SingletonEntity};

/// Singleton model that tracks GPU state.
#[derive(Debug, Default, Clone)]
pub struct GPUState {
    has_low_power_gpu: bool,
}

impl GPUState {
    /// Creates a new GPUState with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the low power GPU as available and stable
    pub(super) fn set_has_lower_power_gpu(
        &mut self,
        has_low_power_gpu: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.has_low_power_gpu = has_low_power_gpu;

        if has_low_power_gpu {
            ctx.emit(GPUStateEvent::LowPowerGPUAvailable);
        }
    }

    /// Returns whether the low power GPU is available for use
    pub fn is_low_power_gpu_available(&self) -> bool {
        self.has_low_power_gpu
    }
}

pub enum GPUStateEvent {
    LowPowerGPUAvailable,
}

impl SingletonEntity for GPUState {}

impl Entity for GPUState {
    type Event = GPUStateEvent;
}
