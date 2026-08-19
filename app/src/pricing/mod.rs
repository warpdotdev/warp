use onboarding::CreditPackOption;
use warp_graphql::billing::{
    AddonCreditsOption, OveragesPricing, PlanPricing, PricingInfo, StripeSubscriptionPlan,
};
use warpui::{Entity, ModelContext, SingletonEntity};

/// Converts the server's add-on credit packs into the display options shown on
/// the onboarding offer slide.
///
/// `premium_bps` is the viewer's `PurchaseAddOnCreditsPolicy` surcharge (see
/// [`crate::workspaces::workspace::PurchaseAddOnCreditsPolicy::effective_premium_bps`]),
/// applied with the same integer math the server charges with, so the price we
/// show is the price billed. Savings are computed against the smallest pack's
/// per-credit list rate — the premium scales every pack equally, so it doesn't
/// change the relative volume discount.
pub fn onboarding_credit_pack_options(
    options: &[AddonCreditsOption],
    premium_bps: i32,
) -> Vec<CreditPackOption> {
    let base_rate = options.first().map_or(0., |option| option.rate());
    options
        .iter()
        .map(|option| {
            let savings_percent = if base_rate > 0. {
                (((base_rate - option.rate()) / base_rate) * 100.)
                    .round()
                    .max(0.) as u32
            } else {
                0
            };
            CreditPackOption {
                credits: option.credits,
                price_usd_cents: option.price_usd_cents_with_premium(premium_bps),
                savings_percent,
            }
        })
        .collect()
}

/// A global model for maintaining pricing information from the server.
#[derive(Debug)]
pub struct PricingInfoModel {
    /// The latest-known pricing information from the server.
    pricing_info: Option<PricingInfo>,
}

impl PricingInfoModel {
    pub fn new() -> Self {
        Self { pricing_info: None }
    }

    /// Updates the model with the latest pricing information from the server.
    pub fn update_pricing_info(&mut self, pricing_info: PricingInfo, ctx: &mut ModelContext<Self>) {
        self.pricing_info = Some(pricing_info);
        ctx.emit(PricingInfoModelEvent::PricingInfoUpdated);
    }

    /// Returns the current overage pricing information.
    #[allow(dead_code)]
    fn overage_pricing(&self) -> Option<&OveragesPricing> {
        self.pricing_info.as_ref().map(|info| &info.overages)
    }

    /// Returns the pricing for a specific plan.
    #[allow(dead_code)]
    pub fn plan_pricing(&self, plan: &StripeSubscriptionPlan) -> Option<&PlanPricing> {
        self.pricing_info
            .as_ref()?
            .plans
            .iter()
            .find(|p| &p.plan == plan)
    }

    /// Returns the pricing data for all known plans, or an empty slice if
    /// pricing information has not yet been fetched from the server.
    pub fn plans(&self) -> &[PlanPricing] {
        self.pricing_info
            .as_ref()
            .map(|info| info.plans.as_slice())
            .unwrap_or(&[])
    }

    /// Returns the overage cost in dollars (converted from cents).
    #[allow(dead_code)]
    pub fn overage_cost_dollars(&self) -> Option<f64> {
        self.overage_pricing()
            .map(|overages| overages.price_per_request_usd_cents as f64 / 100.0)
    }

    /// Returns the monthly cost for a plan in dollars (converted from cents).
    #[allow(dead_code)]
    pub fn monthly_plan_cost_dollars(&self, plan: &StripeSubscriptionPlan) -> Option<f64> {
        self.plan_pricing(plan)
            .map(|pricing| pricing.monthly_plan_price_per_month_usd_cents as f64 / 100.0)
    }

    pub fn addon_credits_options(&self) -> Option<&[AddonCreditsOption]> {
        self.pricing_info
            .as_ref()
            .map(|info| info.addon_credits_options.as_slice())
    }

    pub fn promotion_message(&self) -> Option<&str> {
        self.pricing_info.as_ref()?.promotion_message.as_deref()
    }
}

impl Default for PricingInfoModel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum PricingInfoModelEvent {
    PricingInfoUpdated,
}

impl Entity for PricingInfoModel {
    type Event = PricingInfoModelEvent;
}

impl SingletonEntity for PricingInfoModel {}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
