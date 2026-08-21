//! Deterministic color assignment for the pricing-transparency usage popover
//! (Surfaces 2, 4, 6): per-model stacked-bar colors, fixed per-category
//! context-window colors, and re-exports of the pill bar's per-agent color
//! logic so all three "stacked bar" treatments in the popover share one
//! palette.
//!
//! Colors are taken directly from the Figma "Pricing transparency" file's
//! chart palette (`191:367` / `408:23019`), rather than the app's ANSI
//! palette, so the bars visually read as data-visualization chart segments
//! rather than terminal-themed content.

use pathfinder_color::ColorU;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::persistence::model::ContextWindowSegmentType;

/// The six chart colors used across the popover's stacked bars, in the
/// order sampled from Figma: magenta, blue, yellow, cyan/lavender, green,
/// red. Both the per-model bar (Surface 2) and the context-window breakdown
/// (Surface 4) draw from this same set so the popover reads as one
/// consistent chart language.
///
/// A plain function (rather than a `const` array) because `ColorU::new` is
/// not a `const fn`.
fn chart_palette() -> [ColorU; 6] {
    [
        ColorU::new(0xff, 0x8f, 0xfd, 0xff), // magenta
        ColorU::new(0xa5, 0xd5, 0xfe, 0xff), // blue
        ColorU::new(0xfe, 0xfd, 0xc2, 0xff), // yellow
        ColorU::new(0xd0, 0xd1, 0xfe, 0xff), // cyan / lavender
        ColorU::new(0xb4, 0xfa, 0x72, 0xff), // green
        ColorU::new(0xff, 0x82, 0x72, 0xff), // red
    ]
}

/// Neutral color used for the context-window panel's "Other" bucket, which
/// has no dedicated color in the Figma taxonomy.
fn other_category_color() -> ColorU {
    ColorU::new(0x9b, 0x9b, 0x9b, 0xff)
}

/// Deterministic per-model color for the MODEL USAGE stacked bar and its
/// row swatches (Surface 2, resolved decision 8). Hashing the model id
/// keeps a given model's color stable across renders and popover reopens
/// without needing to persist an assignment anywhere.
pub fn color_for_model(model_id: &str) -> ColorU {
    let palette = chart_palette();
    let mut hasher = DefaultHasher::new();
    model_id.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % palette.len();
    palette[idx]
}

/// Fixed per-category color for the context-window breakdown panel
/// (Surface 4), matching the Figma taxonomy exactly. Unlike model colors,
/// these are **not** hashed: the six named categories always take the same
/// color so the legend stays consistent across conversations, and "Other"
/// always renders as a neutral gray regardless of which raw segment types
/// it aggregates.
pub fn color_for_context_window_category(bucket: ContextWindowSegmentType) -> ColorU {
    let palette = chart_palette();
    match bucket {
        ContextWindowSegmentType::ConversationHistory => palette[0],
        ContextWindowSegmentType::SystemPrompt => palette[1],
        ContextWindowSegmentType::ToolDefinitions => palette[2],
        ContextWindowSegmentType::Rules => palette[3],
        ContextWindowSegmentType::Skills => palette[4],
        ContextWindowSegmentType::Memory => palette[5],
        ContextWindowSegmentType::Unknown
        | ContextWindowSegmentType::LatestInput
        | ContextWindowSegmentType::Images
        | ContextWindowSegmentType::Other => other_category_color(),
    }
}

#[cfg(test)]
#[path = "colors_tests.rs"]
mod tests;
