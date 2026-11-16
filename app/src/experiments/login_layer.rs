use std::collections::HashMap;
use std::str::FromStr;

use lazy_static::lazy_static;

use super::{BucketRange, Experiment, Layer};

lazy_static! {
    pub static ref LOGIN_LAYER: Layer = Layer {
        name: "LoginLayer",
        hasher_seeds: (100, 9000),
        traffic_allocations: HashMap::from([
            (AuthFlowInstructions::Control.get_group_id(), 25.0),
            (AuthFlowInstructions::Experiment.get_group_id(), 25.0),
        ]),
        bucket_ranges: vec![
            BucketRange::new(AuthFlowInstructions::Control, 0..250),
            BucketRange::new(AuthFlowInstructions::Experiment, 250..500),
        ]
    };
}

const AUTH_FLOW_INSTRUCTIONS_CONTROL: &str = "Control";
const AUTH_FLOW_INSTRUCTIONS_EXPERIMENT: &str = "AuthFlowInstructionsExperiment";

/// An experiment to test retention when explicitly instructing users to go to
/// their browser to continue the authentication flow.
pub enum AuthFlowInstructions {
    /// The existing auth flow.
    Control,

    /// An auth flow that explicitly instructs users to go to their browser
    /// to continue authenticating.
    Experiment,
}

impl Experiment<AuthFlowInstructions> for AuthFlowInstructions {
    fn name() -> &'static str {
        "AuthFlowInstructions"
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::Control => AUTH_FLOW_INSTRUCTIONS_CONTROL,
            Self::Experiment => AUTH_FLOW_INSTRUCTIONS_EXPERIMENT,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        false
    }
}

impl FromStr for AuthFlowInstructions {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            AUTH_FLOW_INSTRUCTIONS_CONTROL => Ok(Self::Control),
            AUTH_FLOW_INSTRUCTIONS_EXPERIMENT => Ok(Self::Experiment),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in AuthFlowInstructions",
                s
            )),
        }
    }
}
