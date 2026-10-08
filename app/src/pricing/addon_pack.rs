use thousands::Separable;
use warp_graphql::billing::AddonCreditsOption;

use crate::settings::UsageDisplayUnit;

const ADDON_CREDITS_DESCRIPTION_LEAD: &str = "Add-on credits are purchased in prepaid packages that roll over each billing cycle and expire after one year.";
const ADDON_CREDITS_VOLUME_DISCOUNT_NOTE: &str =
    "The more you purchase, the better the per-credit rate.";
const ADDON_CREDITS_DESCRIPTION_TAIL: &str =
    "Once your base plan credits are used, add-on credits will be consumed.";

/// What an add-on pack buys, in the unit it is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackAmount {
    Credits(i32),
    /// US cents of usage.
    UsageCents(i32),
}

impl PackAmount {
    /// What `option` buys, shown in `unit`. A pack displayed in dollars still shows its credit
    /// count when the catalog states no usage for it.
    pub fn of(option: &AddonCreditsOption, unit: UsageDisplayUnit) -> Self {
        match (unit, option.usage_cents) {
            (UsageDisplayUnit::Dollars, Some(cents)) => Self::UsageCents(cents),
            (UsageDisplayUnit::Dollars, None) | (UsageDisplayUnit::Credits, _) => {
                Self::Credits(option.credits)
            }
        }
    }

    pub fn is_usage(self) -> bool {
        matches!(self, Self::UsageCents(_))
    }

    /// The bare figure, as shown on a pack button: `1,000` or `$10`.
    pub fn short_label(self) -> String {
        match self {
            Self::Credits(credits) => credits.separate_with_commas(),
            Self::UsageCents(cents) => format_price(cents),
        }
    }

    /// The figure with its unit, for prose: `1,000 credits` or `$10 of usage`.
    pub fn label(self) -> String {
        match self {
            Self::Credits(1) => "1 credit".to_string(),
            Self::Credits(credits) => format!("{} credits", credits.separate_with_commas()),
            Self::UsageCents(cents) => format!("{} of usage", format_price(cents)),
        }
    }
}

/// Formats a price in US cents as `$10` when it is a whole number of dollars and as `$10.50`
/// otherwise.
pub fn format_price(cents: i32) -> String {
    if cents % 100 == 0 {
        format!("${}", cents / 100)
    } else {
        format!("${:.2}", f64::from(cents) / 100.)
    }
}

/// A pack's menu label: its price after any plan premium, then what it buys, e.g.
/// `$10 / 1,000 credits` or `$11 / $10 of usage`.
pub fn pack_menu_label(
    option: &AddonCreditsOption,
    premium_bps: i32,
    unit: UsageDisplayUnit,
) -> String {
    format!(
        "{} / {}",
        format_price(option.price_usd_cents_with_premium(premium_bps)),
        PackAmount::of(option, unit).label()
    )
}

/// Whether some pack is cheaper per credit than the catalog's first (smallest) one, by the
/// whole-percent measure the pack menus badge.
pub fn larger_packs_cost_less_per_credit(options: &[AddonCreditsOption]) -> bool {
    let Some(base_rate) = options.first().map(AddonCreditsOption::rate) else {
        return false;
    };
    options
        .iter()
        .any(|option| option.discount_percent(base_rate) > 0)
}

/// The add-on credits panel's description. It promises a better per-credit rate on larger packs
/// only when `options` actually offers one, so it stays true for flat-rate catalogs such as
/// usage packs shown in credits.
pub fn addon_credits_description(options: &[AddonCreditsOption]) -> String {
    if larger_packs_cost_less_per_credit(options) {
        format!(
            "{ADDON_CREDITS_DESCRIPTION_LEAD} {ADDON_CREDITS_VOLUME_DISCOUNT_NOTE} \
             {ADDON_CREDITS_DESCRIPTION_TAIL}"
        )
    } else {
        format!("{ADDON_CREDITS_DESCRIPTION_LEAD} {ADDON_CREDITS_DESCRIPTION_TAIL}")
    }
}

#[cfg(test)]
#[path = "addon_pack_tests.rs"]
mod tests;
