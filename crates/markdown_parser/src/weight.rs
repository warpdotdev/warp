use enum_iterator::Sequence;

/// All [`Weight`]s that are not [`Weight::Normal`] are considered custom weights.
/// Avoid importing `CustomWeight`, and prefer using [`Weight`] throughout the codebase,
/// except in cases where you want to specifically track explicit weight overrides.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Sequence)]
pub enum CustomWeight {
    Thin,
    ExtraLight,
    Light,
    Medium,
    Semibold,
    Bold,
    ExtraBold,
    Black,
}

impl CustomWeight {
    /// Maps a numeric CSS `font-weight` value to the closest named weight.
    ///
    /// CSS numeric weights run 1–1000, with the common named steps landing on the hundreds
    /// (100 Thin … 900 Black). Out-of-range input (including values far outside 1..=1000, e.g.
    /// from malformed pasted HTML) is clamped to that range before rounding. We then round to
    /// the nearest hundred and map that bucket to a variant. `400` (Normal) has no `CustomWeight`
    /// and returns `None`, as do values that round to it.
    pub fn from_css_numeric(value: i32) -> Option<CustomWeight> {
        // Clamp into the valid CSS range first so the rounding arithmetic below can never overflow.
        let value = value.clamp(1, 1000);
        // Round to the nearest hundred, then clamp into the 100..=900 named range.
        let bucket = (((value + 50) / 100) * 100).clamp(100, 900);
        match bucket {
            100 => Some(CustomWeight::Thin),
            200 => Some(CustomWeight::ExtraLight),
            300 => Some(CustomWeight::Light),
            400 => None,
            500 => Some(CustomWeight::Medium),
            600 => Some(CustomWeight::Semibold),
            700 => Some(CustomWeight::Bold),
            800 => Some(CustomWeight::ExtraBold),
            900 => Some(CustomWeight::Black),
            _ => None,
        }
    }

    /// Returns true if the weight is bold or heavier.
    pub fn is_at_least_bold(&self) -> bool {
        matches!(
            self,
            CustomWeight::Bold | CustomWeight::ExtraBold | CustomWeight::Black
        )
    }

    /// We do not support nested weights at this time! The outer weight will
    /// be the only respected weight.
    pub fn merge_weights(
        first: Option<CustomWeight>,
        second: Option<CustomWeight>,
    ) -> Option<CustomWeight> {
        // We don't currently support text containing text of varying weights.
        // We will just respect the outer weight if you specify a non-Normal weight.
        first.or(second)
    }
}

#[cfg(test)]
mod tests {
    use super::CustomWeight;

    #[test]
    fn from_css_numeric_maps_named_steps() {
        assert_eq!(
            CustomWeight::from_css_numeric(100),
            Some(CustomWeight::Thin)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(200),
            Some(CustomWeight::ExtraLight)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(300),
            Some(CustomWeight::Light)
        );
        assert_eq!(CustomWeight::from_css_numeric(400), None);
        assert_eq!(
            CustomWeight::from_css_numeric(500),
            Some(CustomWeight::Medium)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(600),
            Some(CustomWeight::Semibold)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(700),
            Some(CustomWeight::Bold)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(800),
            Some(CustomWeight::ExtraBold)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(900),
            Some(CustomWeight::Black)
        );
    }

    #[test]
    fn from_css_numeric_rounds_to_nearest_hundred() {
        // Off-scale values round to the nearest named step.
        assert_eq!(
            CustomWeight::from_css_numeric(340),
            Some(CustomWeight::Light)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(660),
            Some(CustomWeight::Bold)
        );
        // Values rounding to 400 have no custom weight.
        assert_eq!(CustomWeight::from_css_numeric(380), None);
        assert_eq!(CustomWeight::from_css_numeric(449), None);
    }

    #[test]
    fn from_css_numeric_clamps_out_of_range() {
        assert_eq!(CustomWeight::from_css_numeric(1), Some(CustomWeight::Thin));
        assert_eq!(CustomWeight::from_css_numeric(50), Some(CustomWeight::Thin));
        assert_eq!(
            CustomWeight::from_css_numeric(1000),
            Some(CustomWeight::Black)
        );
    }

    #[test]
    fn from_css_numeric_does_not_overflow_on_extreme_input() {
        // Values far outside the CSS 1..=1000 range must not panic (debug) or
        // wrap around (release) when added to before clamping.
        assert_eq!(
            CustomWeight::from_css_numeric(i32::MAX),
            Some(CustomWeight::Black)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(i32::MIN),
            Some(CustomWeight::Thin)
        );
        assert_eq!(CustomWeight::from_css_numeric(0), Some(CustomWeight::Thin));
        assert_eq!(CustomWeight::from_css_numeric(-5), Some(CustomWeight::Thin));
        assert_eq!(
            CustomWeight::from_css_numeric(1000),
            Some(CustomWeight::Black)
        );
        assert_eq!(
            CustomWeight::from_css_numeric(1_000_000),
            Some(CustomWeight::Black)
        );
    }
}
