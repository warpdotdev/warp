use std::collections::HashMap;
use std::str::FromStr;

use lazy_static::lazy_static;
use warpui::AppContext;

use crate::experiments::{BucketRange, Experiment, Layer};

lazy_static! {
    pub static ref IMPROVED_PALETTE_SEARCH_LAYER: Layer = Layer {
        name: "ImprovedPaletteSearchLayer",
        hasher_seeds: (20250401, 1917),
        traffic_allocations: HashMap::from([
            (ImprovedPaletteSearch::Control.get_group_id(), 0.0),
            (ImprovedPaletteSearch::Experiment.get_group_id(), 100.0),
        ]),
        bucket_ranges: vec![
            BucketRange::new(ImprovedPaletteSearch::Control, 0..0),
            BucketRange::new(ImprovedPaletteSearch::Experiment, 0..1000),
        ]
    };
}

/// An experiment to test the difference between the original search and improved full text search
#[derive(Debug)]
pub enum ImprovedPaletteSearch {
    /// Old fuzzy search
    Control,
    /// Tentivy full-text search
    Experiment,
}

const IMPROVED_PALETTE_SEARCH_CONTROL: &str = "Control";
const IMPROVED_PALETTE_SEARCH_EXPERIMENT: &str = "ImprovedPaletteSearchExperiment";

impl Experiment<ImprovedPaletteSearch> for ImprovedPaletteSearch {
    fn name() -> &'static str {
        "ImprovedPaletteSearch"
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::Control => IMPROVED_PALETTE_SEARCH_CONTROL,
            Self::Experiment => IMPROVED_PALETTE_SEARCH_EXPERIMENT,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        false
    }
}

impl FromStr for ImprovedPaletteSearch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            IMPROVED_PALETTE_SEARCH_CONTROL => Ok(Self::Control),
            IMPROVED_PALETTE_SEARCH_EXPERIMENT => Ok(Self::Experiment),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in ImprovedPaletteSearch",
                s
            )),
        }
    }
}

impl ImprovedPaletteSearch {
    pub fn improved_search_enabled(ctx: &mut AppContext) -> bool {
        matches!(Self::get_group(ctx), Some(Self::Experiment))
    }
}
