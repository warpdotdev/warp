use std::collections::HashMap;
use std::str::FromStr;

use lazy_static::lazy_static;

use super::Experiment;
use crate::experiments::{BucketRange, Layer};

lazy_static! {
    pub static ref BLOCK_ONBOARDING_LAYER: Layer = Layer {
        name: "BlockOnboardingLayer",
        hasher_seeds: (2222, 9999),
        traffic_allocations: HashMap::from([
            (BlockOnboarding::VariantOne.get_group_id(), 30.0),
            (BlockOnboarding::VariantTwo.get_group_id(), 70.0)
        ]),
        bucket_ranges: vec![
            BucketRange::new(BlockOnboarding::VariantTwo, 0..333),
            BucketRange::new(BlockOnboarding::VariantOne, 333..633),
            BucketRange::new(BlockOnboarding::VariantTwo, 633..1000),
        ]
    };
}

/// An experiment to test block onboarding's impact on user activation.
#[derive(Debug)]
pub enum BlockOnboarding {
    /// No onboarding survey, just theme + prompt. No welcome block (ascii)
    VariantOne,
    /// Onboarding survey and theme + prompt. No welcome block (ascii)
    VariantTwo,
}

const BLOCK_ONBOARDING_VARIANT_ONE: &str = "VariantOne";
const BLOCK_ONBOARDING_VARIANT_TWO: &str = "VariantTwo";

impl Experiment<BlockOnboarding> for BlockOnboarding {
    fn name() -> &'static str {
        "BlockOnboarding"
    }

    fn variant(&self) -> &'static str {
        match self {
            BlockOnboarding::VariantOne => BLOCK_ONBOARDING_VARIANT_ONE,
            BlockOnboarding::VariantTwo => BLOCK_ONBOARDING_VARIANT_TWO,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        false
    }
}

impl FromStr for BlockOnboarding {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            BLOCK_ONBOARDING_VARIANT_ONE => Ok(BlockOnboarding::VariantOne),
            BLOCK_ONBOARDING_VARIANT_TWO => Ok(BlockOnboarding::VariantTwo),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in BlockOnboarding",
                s
            )),
        }
    }
}
