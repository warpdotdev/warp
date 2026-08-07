use std::collections::HashSet;

use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::send_telemetry_from_ctx;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};
use warp_core::user_preferences::GetUserPreferences;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::pricing::{PricingInfoModel, PricingInfoModelEvent};

const AGENT_DISMISSED_KEY: &str = "pricing_promotion_agent_dismissed";
const TERMINAL_DISMISSED_KEY: &str = "pricing_promotion_terminal_dismissed";
const DISMISSED_VALUE: &str = "true";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PricingPromotionSurface {
    AgentMessageBar,
    TerminalMessageBar,
}

impl PricingPromotionSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::AgentMessageBar => "agent_message_bar",
            Self::TerminalMessageBar => "terminal_message_bar",
        }
    }

    fn dismissal_key(self) -> &'static str {
        match self {
            Self::AgentMessageBar => AGENT_DISMISSED_KEY,
            Self::TerminalMessageBar => TERMINAL_DISMISSED_KEY,
        }
    }
}

#[derive(Clone, Debug)]
pub enum PricingPromotionStateEvent {
    Updated,
}

pub struct PricingPromotionState {
    agent_dismissed: bool,
    terminal_dismissed: bool,
    displayed_surfaces: HashSet<PricingPromotionSurface>,
}

impl PricingPromotionState {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&PricingInfoModel::handle(ctx), |_, _, event, ctx| {
            if matches!(event, PricingInfoModelEvent::PricingInfoUpdated) {
                ctx.emit(PricingPromotionStateEvent::Updated);
                ctx.notify();
            }
        });

        Self {
            agent_dismissed: Self::read_dismissed(AGENT_DISMISSED_KEY, ctx),
            terminal_dismissed: Self::read_dismissed(TERMINAL_DISMISSED_KEY, ctx),
            displayed_surfaces: HashSet::new(),
        }
    }

    pub fn visible_message(
        &self,
        surface: PricingPromotionSurface,
        app: &AppContext,
    ) -> Option<String> {
        if self.is_dismissed(surface) {
            return None;
        }
        PricingInfoModel::as_ref(app)
            .promotion_message()
            .map(str::to_owned)
    }

    pub fn record_displayed(
        &mut self,
        surface: PricingPromotionSurface,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.displayed_surfaces.insert(surface) {
            send_telemetry_from_ctx!(PricingPromotionTelemetryEvent::Shown { surface }, ctx);
        }
    }

    pub fn record_clicked(&self, surface: PricingPromotionSurface, ctx: &mut ModelContext<Self>) {
        send_telemetry_from_ctx!(PricingPromotionTelemetryEvent::Clicked { surface }, ctx);
    }

    pub fn dismiss(&mut self, surface: PricingPromotionSurface, ctx: &mut ModelContext<Self>) {
        match surface {
            PricingPromotionSurface::AgentMessageBar => self.agent_dismissed = true,
            PricingPromotionSurface::TerminalMessageBar => self.terminal_dismissed = true,
        }
        if let Err(error) = ctx
            .private_user_preferences()
            .write_value(surface.dismissal_key(), DISMISSED_VALUE.to_string())
        {
            log::warn!("Failed to persist pricing promotion dismissal: {error:#}");
        }
        send_telemetry_from_ctx!(PricingPromotionTelemetryEvent::Dismissed { surface }, ctx);
        ctx.emit(PricingPromotionStateEvent::Updated);
        ctx.notify();
    }

    fn is_dismissed(&self, surface: PricingPromotionSurface) -> bool {
        match surface {
            PricingPromotionSurface::AgentMessageBar => self.agent_dismissed,
            PricingPromotionSurface::TerminalMessageBar => self.terminal_dismissed,
        }
    }

    fn read_dismissed(key: &str, ctx: &AppContext) -> bool {
        ctx.private_user_preferences()
            .read_value(key)
            .unwrap_or_default()
            .is_some_and(|value| value == DISMISSED_VALUE)
    }
}

impl Entity for PricingPromotionState {
    type Event = PricingPromotionStateEvent;
}

impl SingletonEntity for PricingPromotionState {}

#[derive(Clone, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
enum PricingPromotionTelemetryEvent {
    Shown { surface: PricingPromotionSurface },
    Clicked { surface: PricingPromotionSurface },
    Dismissed { surface: PricingPromotionSurface },
}

impl TelemetryEvent for PricingPromotionTelemetryEvent {
    fn name(&self) -> &'static str {
        PricingPromotionTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        let surface = match self {
            Self::Shown { surface } | Self::Clicked { surface } | Self::Dismissed { surface } => {
                surface
            }
        };
        Some(json!({
            "surface": surface.as_str(),
        }))
    }

    fn description(&self) -> &'static str {
        PricingPromotionTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for PricingPromotionTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Shown => "PricingPromotion.Shown",
            Self::Clicked => "PricingPromotion.Clicked",
            Self::Dismissed => "PricingPromotion.Dismissed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Shown => "A pricing promotion was shown",
            Self::Clicked => "A pricing promotion was clicked",
            Self::Dismissed => "A pricing promotion was dismissed",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(PricingPromotionTelemetryEvent);

#[cfg(test)]
#[path = "pricing_promotion_tests.rs"]
mod tests;
