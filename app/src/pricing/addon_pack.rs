use thousands::Separable;
use warp_graphql::billing::AddonCreditsOption;

/// What an add-on pack buys, in the unit the catalog sells it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackAmount {
    Credits(i32),
    /// US cents of usage, for a catalog that sells packs in dollars.
    UsageCents(i32),
}

impl PackAmount {
    pub fn of(option: &AddonCreditsOption) -> Self {
        match option.usage_cents {
            Some(cents) => Self::UsageCents(cents),
            None => Self::Credits(option.credits),
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
pub fn pack_menu_label(option: &AddonCreditsOption, premium_bps: i32) -> String {
    format!(
        "{} / {}",
        format_price(option.price_usd_cents_with_premium(premium_bps)),
        PackAmount::of(option).label()
    )
}

/// Whether the catalog sells its packs in dollars of usage rather than credits.
pub fn packs_are_sold_in_dollars(options: &[AddonCreditsOption]) -> bool {
    !options.is_empty() && options.iter().all(|option| option.usage_cents.is_some())
}

#[cfg(test)]
#[path = "addon_pack_tests.rs"]
mod tests;
