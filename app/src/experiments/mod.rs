//! A framework for running A/B tests within Warp.
//!
//! Before starting, please read the usage guide on Notion. The guide explains
//! some important constraints that are required for proper use of the framework
//! that we are not able to assert through automated testing.
//! https://www.notion.so/warpdev/Experiment-Framework-Guide-88954c36a0c3469ea57b427b58249d5f?pvs=4

mod block_onboarding_layer;
mod login_layer;
mod rendering;
pub use block_onboarding_layer::{BlockOnboarding, BLOCK_ONBOARDING_LAYER};
pub use improved_palette_search_layer::{ImprovedPaletteSearch, IMPROVED_PALETTE_SEARCH_LAYER};
pub use login_layer::{AuthFlowInstructions, LOGIN_LAYER};
use warp_core::user_preferences::GetUserPreferences as _;

use crate::auth::auth_state::AuthStateProvider;
use crate::channel::{Channel, ChannelState};
use anyhow::Result;
use dashmap::DashMap;
use lazy_static::lazy_static;
use std::fmt;
use std::marker::Copy;
use std::ops::Range;
use std::str::FromStr;
use std::{collections::HashMap, hash::Hasher};

use warpui::{AppContext, SingletonEntity};

use crate::send_telemetry_sync_from_app_ctx;

/// Number of buckets we are using to partition user traffic. The largest valid
/// bucket index is NUM_BUCKETS - 1.
const NUM_BUCKETS: u16 = 1000;

const EXPERIMENT_OVERRIDES_KEY: &str = "ExperimentOverrides";

#[allow(dead_code)]
const INVALID_GROUP_ASSIGNMENT_ERR: &str =
    "Invalid group assignment, deriving group from experiment id instead";
#[allow(dead_code)]
const INVALID_USER_OVERRIDE_ERR: &str =
    "Invalid user override, deriving group from experiment id instead";
#[allow(dead_code)]
const NO_LAYER_FOUND_ERR: &str = "No layer found for the experiment";

lazy_static! {
    /// In-memory map that caches users' group assignments so we don't have to calculate
    /// it from their anonymous id each time. Also keeps track of experiment overrides.
    /// Key is the name of the experiment, and the value is the variant name.
    // TODO(daniel): Account for user logout. Currently the cached group assignments
    // and anonymous id persist even on logout, which may not be the correct behavior.
    static ref GROUP_ASSIGNMENTS: DashMap<&'static str, &'static str> = DashMap::new();

    /// In-memory map that stores the user's local overrides. This map differs from
    /// GROUP_ASSIGNMENTS as it uses owned strings to store the overrides read from
    /// user defaults. The data follows the same structure as GROUP_ASSIGNMENTS: the
    /// keys are experiment names and the values are the variant names.
    static ref USER_OVERRIDES: DashMap<String, String> = DashMap::new();

    /// All of the layers currently enabled in the application. A layer must be added
    /// to this vector in order to use any experiments in that layer. Trying to use an
    /// experiment in a layer not in this vector will panic in local builds, or result
    /// in users never being assigned to the experiment in non-local builds.
    ///
    /// EMPTY_LAYER is not included here, since we will never add experiments to it,
    /// and so users can never be assigned to experiments in EMPTY_LAYER.
    static ref LAYERS: Vec<&'static Layer> = vec![
        &*LOGIN_LAYER,
        &*BLOCK_ONBOARDING_LAYER,
        &*rendering::LAYER,
        &*IMPROVED_PALETTE_SEARCH_LAYER,
    ];

    /// Mapping of experiments to their respective layers. The mappings are built up
    /// during bootstrap. The keys are experiment names.
    static ref EXPERIMENT_LAYER_MAPPINGS: DashMap<&'static str, &'static Layer> = DashMap::new();

    /// A no-op layer. This layer is only used if an error state occurs where there is
    /// no layer mapping for a given experiment - the empty layer is returned and the
    /// user will never be assigned to the experiment.
    static ref EMPTY_LAYER: Layer = Layer {
        name: "EmptyLayer",
        hasher_seeds: (1, 1),
        traffic_allocations: HashMap::new(),
        bucket_ranges: Vec::new()
    };
}

/// A range of buckets associated with an experiment group.
#[derive(Clone)]
struct BucketRange {
    /// The group to assign this range of buckets to.
    group: GroupId,
    /// The range of buckets.
    range: Range<u16>,
}

impl BucketRange {
    // Ignoring the warning that appears when there are no experiments running currently.
    #[allow(dead_code)]
    fn new<T>(exp: impl Experiment<T>, range: Range<u16>) -> Self
    where
        T: Experiment<T>,
    {
        Self {
            group: exp.get_group_id(),
            range,
        }
    }
}

/// A unique id representing an experiment group. Contains the name of the
/// experiment and the variant that it represents.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct GroupId {
    experiment: &'static str,
    variant: &'static str,
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.experiment, self.variant)
    }
}

/// A data structure used to define user traffic allocations to experiment groups.
/// Each layer divides users into buckets 0-999, each covering 0.1% of users. Ranges
/// of buckets are allocated to experiment groups to route users to experiments.
///
/// All groups of an experiment must live in a single layer. All experiments in a layer
/// will be mutually exclusive, but this is not true for experiments located in different
/// layers.
///
/// Newly created layers should be added to the `LAYERS` vector to be picked up by
/// automated tests that provide a basic guarantee of correctness.
///
/// For more info, see the tech doc: https://docs.google.com/document/d/1BEEeT1Ia7bK-ExK9w-FJKNkEZaRNwTATqZAmX55VldY/edit?usp=sharing
#[allow(dead_code)]
pub struct Layer {
    /// Name of the layer.
    name: &'static str,
    /// Seeds used to construct the hasher for this layer.
    hasher_seeds: (u64, u64),
    /// The amount of traffic allocated to each experiment group in this layer,
    /// should be written as percentages: e.g. 20.5 is 20.5%. The values here
    /// are a safeguard against user error in specifying the bucket ranges,
    /// which is the sole authority over assigning users to experiments.
    traffic_allocations: HashMap<GroupId, f64>,
    /// Ranges of buckets allocated to experiment groups. When increasing the
    /// allocations for an experiment, we want to ensure users that were previously
    /// in the experiment remain in the same group. This involves fragmenting the
    /// bucket ranges for each group. See the `Increasing experiment traffic
    /// allocations` section in the Notion guide for more details.
    bucket_ranges: Vec<BucketRange>,
}

#[allow(dead_code)]
impl Layer {
    /// Returns the name of the layer.
    fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the seeds used to construct the hasher for this layer. These
    /// will stay consistent across app runs and will be unique for each layer.
    fn hasher_seeds(&self) -> (u64, u64) {
        self.hasher_seeds
    }

    /// Searches for a bucket range that includes the given bucket. If found, returns
    /// the experiment group for that range, or None if no satisfying range was found.
    fn get_group_for_bucket(&self, bucket: u16) -> Option<GroupId> {
        if bucket >= NUM_BUCKETS {
            log::error!("User assigned a bucket greater than the max: {bucket}");
            return None;
        }
        for BucketRange { group, range } in self.bucket_ranges.iter() {
            if range.contains(&bucket) {
                return Some(*group);
            }
        }
        None
    }

    /// Determines the assigned bucket based on a hash of the anonymous id. The
    /// returned bucket will be in the range 0-999 (inclusive) and is deterministic.
    fn assigned_bucket(&self, anonymous_id: &str) -> u16 {
        let (seed_1, seed_2) = self.hasher_seeds();
        let hash = {
            let mut hasher = siphasher::sip::SipHasher::new_with_keys(seed_1, seed_2);
            hasher.write(anonymous_id.as_bytes());
            hasher.finish()
        };
        (hash % 1000) as u16
    }

    /// Returns the experiment group that the anonymous id is assigned to in this
    /// layer, if it exists. Returns None otherwise.
    fn get_assigned_group<T>(&self, anonymous_id: &str) -> Option<T>
    where
        T: Experiment<T>,
        <T as FromStr>::Err: fmt::Debug,
    {
        let bucket = self.assigned_bucket(anonymous_id);
        let group_id = self.get_group_for_bucket(bucket)?;

        // We ignore errors converting group id to T as users can be assigned to a group
        // in another experiment in this layer, in which case the conversion would fail.
        T::from_group_id(group_id).ok()
    }
}

/// Trait to be implemented by all experiments created for A/B testing, with T
/// being the type of the experiment itself.
pub trait Experiment<T: Experiment<T>>: FromStr {
    /// Returns the name associated with this experiment.
    fn name() -> &'static str;

    /// Returns the layer this experiment is in. Returns the empty layer if an error
    /// occurs and no experiment-layer mapping exists for this layer.
    fn layer() -> &'static Layer {
        match EXPERIMENT_LAYER_MAPPINGS.get(Self::name()) {
            Some(layer) => *layer,
            None => {
                if cfg!(debug_assertions) {
                    panic!("{}: {}", NO_LAYER_FOUND_ERR, Self::name());
                } else {
                    log::error!("{}: {}", NO_LAYER_FOUND_ERR, Self::name());
                }
                &EMPTY_LAYER
            }
        }
    }

    /// Returns the string representation of the current experiment variant.
    fn variant(&self) -> &'static str;

    /// Whether or not we allow end users to manually override their assigned
    /// group by modifying their user preferences.
    ///
    /// This should only return true if the "experiment" isn't actually an A/B
    /// test with a hypothesis (e.g. the experiment is used for an incremental
    /// rollout). Otherwise, the randomness of the data should be preserved and
    /// we should not allow overrides.
    ///
    /// This method should generally not be used in isolation. Instead, use
    /// [`Self::can_use_user_override`], which calls this method _and_ checks
    /// the current channel to see if overrides are allowed.
    fn allow_user_overrides_in_stable() -> bool;

    /// Returns the group id from the current experiment group.
    fn get_group_id(&self) -> GroupId {
        GroupId {
            experiment: Self::name(),
            variant: self.variant(),
        }
    }

    /// Parses a group id to return the associated experiment. Will fail if the
    /// group id is ill-formatted or is not an arm of this experiment.
    fn from_group_id(group_id: GroupId) -> Result<T>
    where
        <T as FromStr>::Err: fmt::Debug,
    {
        if group_id.experiment != Self::name() {
            return Err(anyhow::anyhow!(
                "Cannot parse a GroupId of experiment {} into experiment {}",
                group_id.experiment,
                Self::name()
            ));
        }
        T::from_str(group_id.variant)
            .map_err(|e| anyhow::anyhow!("Failed to parse GroupId: {:?}", e))
    }

    /// Gets the assigned group of the experiment for the current user. Returns None
    /// if the user is not in this experiment.
    ///
    /// TODO: we should investigate if we can suffice with just a AppContext
    /// here to allow `get_group` to be used when a AppContext isn't available
    /// (e.g. when rendering a view). We currently need it because `get_group`
    /// might emit telemetry.
    fn get_group(ctx: &mut AppContext) -> Option<T>
    where
        <T as FromStr>::Err: fmt::Debug,
    {
        // Check if we have cached the group assignment in memory.
        if let Some(variant) = GROUP_ASSIGNMENTS.get(Self::name()) {
            match T::from_str(*variant) {
                Ok(group) => return Some(group),
                Err(e) => {
                    if cfg!(debug_assertions) {
                        panic!("{INVALID_GROUP_ASSIGNMENT_ERR}: {e:?}");
                    } else {
                        log::error!("{INVALID_GROUP_ASSIGNMENT_ERR}: {e:?}");
                    }
                }
            };
        }

        let mut assigned_group = None;

        // Check for user override. Only used in local and dev builds or if the
        // this experiment allows overrides.
        if Self::can_use_user_override(ChannelState::channel()) {
            if let Some(variant) = USER_OVERRIDES.get(Self::name()) {
                match T::from_str(&variant) {
                    Ok(group) => assigned_group = Some(group),
                    Err(e) => {
                        log::error!("{INVALID_USER_OVERRIDE_ERR}: {e:?}");
                    }
                };
            }
        }

        // If there was no override, derive the assignment from the user's anonymous id.
        if assigned_group.is_none() {
            let anonymous_id = AuthStateProvider::as_ref(ctx).get().anonymous_id();
            assigned_group = Self::layer().get_assigned_group(&anonymous_id);

            if let Some(group) = assigned_group.as_ref() {
                let group_assignment = group.variant();
                // Send synchronously since this we rely on this event to collect experiment data.
                send_telemetry_sync_from_app_ctx!(
                    crate::server::telemetry::TelemetryEvent::ExperimentTriggered {
                        experiment: Self::name(),
                        layer: Self::layer().name(),
                        group_assignment,
                    },
                    ctx
                );
            }
        }

        // If the user is in a group for this experiment, cache the result of
        // the work above and do any one-time accounting.
        if let Some(group) = assigned_group.as_ref() {
            GROUP_ASSIGNMENTS.insert(Self::name(), group.variant());

            #[cfg(feature = "crash_reporting")]
            {
                let tag_name = format!("warp.experiments.{}", Self::name());
                crate::crash_reporting::set_tag(&tag_name, group.variant());
            }
        }

        assigned_group
    }

    /// Overrides the user's assigned group for this experiment for the remainder
    /// of the app lifecycle.
    #[allow(dead_code)]
    fn set_override<S: Experiment<T>>(override_group: S) {
        GROUP_ASSIGNMENTS.insert(Self::name(), override_group.variant());
    }

    /// Returns whether user overrides should be allowed based on the channel and
    /// experiment setting. User overrides are always allowed in Local and Dev
    /// channels, or if the specific experiment supports overrides.
    fn can_use_user_override(channel: Channel) -> bool {
        Self::allow_user_overrides_in_stable() || channel.is_dogfood()
    }
}

/// Creates the experiment-layer mappings given a list of layers. This method assumes
/// that each experiment is included in a single layer, which should be asserted by
/// the validation tests.
fn create_experiment_layer_mappings(layers: &[&'static Layer]) {
    for layer in layers.iter() {
        for group_id in layer.traffic_allocations.keys() {
            EXPERIMENT_LAYER_MAPPINGS.insert(group_id.experiment, layer);
        }
    }
}

/// Reads in the user overrides. Overrides should be a comma delimited list of
/// group ids under the EXPERIMENT_OVERRIDES_KEY. For example:
/// "ExperimentOverrides": "Experiment1::GroupA,Experiment2::GroupB"
///
/// Note that an override will only be applied if the current channel or the
/// specific experiment supports overrides.
fn set_user_overrides(ctx: &mut AppContext) {
    if let Some(overrides) = ctx
        .private_user_preferences()
        .read_value(EXPERIMENT_OVERRIDES_KEY)
        .unwrap_or_default()
    {
        for group in overrides.split(',') {
            if let Some((experiment_name, variant)) = group.split_once("::") {
                USER_OVERRIDES.insert(experiment_name.into(), variant.into());
            }
        }
    }
}

/// Initializes the experiment framework.
pub fn init(ctx: &mut AppContext) {
    create_experiment_layer_mappings(&LAYERS);
    set_user_overrides(ctx);
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

mod improved_palette_search_layer;
#[cfg(test)]
mod validation_tests;
