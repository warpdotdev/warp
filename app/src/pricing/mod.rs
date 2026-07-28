use warp_graphql::billing::{
    AddonCreditsOption, OveragesPricing, PlanPricing, PricingInfo, StripeSubscriptionPlan,
};
use warpui::{Entity, ModelContext, SingletonEntity};

/// Format an integer cent amount as a US-dollar string (e.g. 1099 → "$10.99").
///
/// The client must never compute markup or round amounts; only use this with
/// server-authored integer-cent fields.
pub fn format_usd_cents(cents: i32) -> String {
    format!("${:.2}", cents as f64 / 100.0)
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

    /// Find the pack option that matches the given credit denomination.
    #[allow(dead_code)]
    pub fn find_addon_option_by_credits(&self, credits: i32) -> Option<&AddonCreditsOption> {
        self.addon_credits_options()?
            .iter()
            .find(|o| o.credits == credits)
    }
}

/// A snapshot of price fields for a single add-on credit pack, ready for display.
///
/// All amounts are in integer US cents as provided by the server. Use
/// `format_usd_cents` to render them; never compute markup locally.
/// Fields and methods are infrastructure for the markup UI (pending design mocks).
#[allow(dead_code)]
pub struct AddonPackPriceInfo {
    pub credits: i32,
    /// Base price before any plan markup.
    pub base_price_usd_cents: i32,
    /// Server-applied markup (zero for paid plans).
    pub markup_usd_cents: i32,
    /// Total charged to the user (`base + markup`, as rounded by the server).
    pub total_price_usd_cents: i32,
}

#[allow(dead_code)]
impl AddonPackPriceInfo {
    pub fn from_option(option: &AddonCreditsOption) -> Self {
        Self {
            credits: option.credits,
            base_price_usd_cents: option.effective_base_price_cents(),
            markup_usd_cents: option.markup_usd_cents.unwrap_or(0),
            total_price_usd_cents: option.total_price_cents(),
        }
    }

    /// Whether this option has a Free-plan markup applied.
    pub fn has_markup(&self) -> bool {
        self.markup_usd_cents > 0
    }

    /// Returns false when the server has returned internally inconsistent or
    /// unsafe price data. Purchase must be disabled when false.
    pub fn is_valid(&self) -> bool {
        self.total_price_usd_cents > 0
            && self.base_price_usd_cents > 0
            && self.total_price_usd_cents >= self.base_price_usd_cents
    }

    /// Formatted base price string (e.g. "$10.00").
    pub fn formatted_base_price(&self) -> String {
        format_usd_cents(self.base_price_usd_cents)
    }

    /// Formatted markup string (e.g. "$1.00"). Empty string when markup is zero.
    pub fn formatted_markup(&self) -> String {
        if self.markup_usd_cents == 0 {
            String::new()
        } else {
            format_usd_cents(self.markup_usd_cents)
        }
    }

    /// Formatted total price string (e.g. "$11.00").
    pub fn formatted_total_price(&self) -> String {
        format_usd_cents(self.total_price_usd_cents)
    }
}

/// Formats the dropdown label for an add-on credits option in the banner
/// (`"{price} / {credits} credits"`).
///
/// Shows `"Pricing unavailable / {credits} credits"` for invalid server totals
/// so the dropdown never fabricates a bad amount (Behavior 18).
pub fn addon_credits_dropdown_label(opt: &AddonCreditsOption) -> String {
    if !opt.is_price_valid() {
        return format!("Pricing unavailable / {} credits", opt.credits);
    }
    format!(
        "{} / {} credits",
        format_usd_cents(opt.total_price_cents()),
        opt.credits
    )
}

/// Computes the volume-discount percentage for an add-on credits option relative
/// to `base_rate` (the cheapest pack's price-per-credit rate).
///
/// A non-zero result means a discount badge should be displayed (Behavior 7).
pub fn addon_credits_discount_percent(base_rate: f32, opt: &AddonCreditsOption) -> u32 {
    if base_rate <= 0.0 {
        return 0;
    }
    let actual_rate = opt.rate();
    ((base_rate - actual_rate) / base_rate * 100.0).round() as u32
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
