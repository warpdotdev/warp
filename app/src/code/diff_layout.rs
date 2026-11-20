use enum_iterator::Sequence;
use serde::{Deserialize, Serialize};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting as _;
use warp_core::telemetry::TelemetryEvent as _;
use warp_errors::report_if_error;
use warpui::{AppContext, SingletonEntity as _};

use crate::code_review::telemetry_event::{CodeReviewTelemetryEvent, DiffLayoutChangeSource};
use crate::send_telemetry_from_app_ctx;
use crate::settings::CodeSettings;

/// How diffs are laid out in the code review pane.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "Layout used to render code review diffs.",
    rename_all = "snake_case"
)]
pub enum DiffLayout {
    /// The unified view: removed lines are shown inline above their replacements.
    #[default]
    Inline,
    /// The split view: the base file on the left and the working file on the right.
    SideBySide,
}

impl DiffLayout {
    pub fn is_side_by_side(self) -> bool {
        matches!(self, Self::SideBySide)
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Inline => Self::SideBySide,
            Self::SideBySide => Self::Inline,
        }
    }

    /// The layout actually rendered for a stored setting value. The split layout is only
    /// available while its feature flag is enabled; the stored value is left untouched so it
    /// applies again once the flag is enabled.
    pub fn effective(stored: Self) -> Self {
        if FeatureFlag::SideBySideDiffLayout.is_enabled() {
            stored
        } else {
            Self::Inline
        }
    }
}

/// Stores a new diff layout and records the switch. Does nothing when the layout is unchanged,
/// so each user-visible switch produces exactly one telemetry event.
pub fn switch_diff_layout(to: DiffLayout, source: DiffLayoutChangeSource, ctx: &mut AppContext) {
    let from = *CodeSettings::as_ref(ctx).diff_layout.value();
    if from == to {
        return;
    }
    CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
        report_if_error!(settings.diff_layout.set_value(to, ctx));
    });
    send_telemetry_from_app_ctx!(
        CodeReviewTelemetryEvent::DiffLayoutChanged { from, to, source },
        ctx
    );
}

#[cfg(test)]
#[path = "diff_layout_tests.rs"]
mod tests;
