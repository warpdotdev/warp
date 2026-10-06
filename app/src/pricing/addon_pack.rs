use thousands::Separable;
use warp_graphql::billing::AddonCreditsOption;
use warpui::{AppContext, SingletonEntity};

use crate::workspaces::user_workspaces::UserWorkspaces;

/// The unit the viewer's plan sells add-on packs in, which decides whether the purchase surfaces
/// talk about credits or usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackUnit {
    Credits,
    /// Dollars of usage.
    Usage,
}

impl PackUnit {
    /// The unit for the viewer's plan: usage when it is billed in dollars, otherwise credits.
    pub fn for_viewer(app: &AppContext) -> Self {
        Self::from_billed_in_dollars(UserWorkspaces::as_ref(app).is_billed_in_dollars())
    }

    pub fn from_billed_in_dollars(billed_in_dollars: bool) -> Self {
        if billed_in_dollars {
            Self::Usage
        } else {
            Self::Credits
        }
    }
}

/// What an add-on pack buys, in the unit it is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackAmount {
    Credits(i32),
    /// US cents of usage.
    UsageCents(i32),
}

impl PackAmount {
    /// What `option` buys for a plan sold in `unit`. A plan sold in usage still shows a pack's
    /// credit count when the catalog states no usage for it.
    pub fn of(option: &AddonCreditsOption, unit: PackUnit) -> Self {
        match (unit, option.usage_cents) {
            (PackUnit::Usage, Some(cents)) => Self::UsageCents(cents),
            (PackUnit::Usage, None) | (PackUnit::Credits, _) => Self::Credits(option.credits),
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
pub fn pack_menu_label(option: &AddonCreditsOption, premium_bps: i32, unit: PackUnit) -> String {
    format!(
        "{} / {}",
        format_price(option.price_usd_cents_with_premium(premium_bps)),
        PackAmount::of(option, unit).label()
    )
}

#[cfg(test)]
#[path = "addon_pack_tests.rs"]
mod tests;
