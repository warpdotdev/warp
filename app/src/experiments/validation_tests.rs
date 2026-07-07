use std::cmp::max;
use std::collections::HashSet;

use super::*;

#[test]
fn test_all_layers_have_unique_seeds() {
    let mut prev_seen_seeds = HashSet::new();
    for layer in LAYERS.iter() {
        let layer_seeds = layer.hasher_seeds();
        assert!(
            !prev_seen_seeds.contains(&layer_seeds),
            "There are two layers that use the seeds {layer_seeds:?} to construct a hasher"
        );
        prev_seen_seeds.insert(layer.hasher_seeds());
    }
}

#[test]
fn test_valid_bucket_ranges() {
    for layer in LAYERS.iter() {
        for bucket_range in &layer.bucket_ranges {
            assert!(
                bucket_range.range.start <= bucket_range.range.end,
                "The start range should be <= the end range for group {:?} in layer {}",
                bucket_range.group,
                layer.name()
            );
            assert!(
                bucket_range.range.start <= NUM_BUCKETS,
                "{} is not a valid range start for group {:?} in layer {}",
                bucket_range.range.start,
                bucket_range.group,
                layer.name()
            );
            assert!(
                bucket_range.range.end <= NUM_BUCKETS,
                "{} is not a valid range end for group {:?} in layer {}",
                bucket_range.range.end,
                bucket_range.group,
                layer.name()
            );
        }
    }
}

#[test]
fn test_no_overlapping_bucket_ranges() {
    for layer in LAYERS.iter() {
        // Sort by ranges by range start.
        let mut sorted_ranges = layer.bucket_ranges.to_vec();
        sorted_ranges.sort_by_key(|r| r.range.start);

        // Track the largest range end we have seen so far.
        let mut prev_end: Option<u16> = None;
        for bucket_range in sorted_ranges {
            if let Some(prev_end_range) = prev_end {
                // If the current range start < a previous end range, they must overlap
                // since the current range start >= the previous range start.
                assert!(
                    bucket_range.range.start >= prev_end_range,
                    "Overlapping bucket ranges in layer: {}",
                    layer.name()
                );
                prev_end = Some(max(bucket_range.range.end, prev_end_range));
            } else {
                prev_end = Some(bucket_range.range.end);
            }
        }
    }
}

#[test]
fn test_bucket_ranges_sum_to_traffic_allocations() {
    for layer in LAYERS.iter() {
        let mut allocations: HashMap<GroupId, f64> = HashMap::new();
        // Manually sum bucket range sizes to see if they match the layer's defined
        // traffic ranges
        for bucket_range in &layer.bucket_ranges {
            let range_size = bucket_range.range.end - bucket_range.range.start;
            let traffic_percentage = (range_size as f64) / 10.0;

            let allocation = allocations.entry(bucket_range.group).or_insert(0.0);
            *allocation += traffic_percentage;
        }
        assert!(
            allocations == layer.traffic_allocations,
            "Bucket ranges do not sum up to the traffic ranges for layer {}",
            layer.name()
        );
    }
}

#[test]
fn test_no_experiments_in_multiple_layers() {
    let mut prev_seen_experiments: HashSet<String> = HashSet::new();
    for layer in LAYERS.iter() {
        // Set of experiments in the current layer
        let mut cur_experiments: HashSet<String> = HashSet::new();
        for group_id in layer.traffic_allocations.keys() {
            // group_id will follow format "{experiment_name}::{variant_name}"
            let experiment_name = group_id.to_string().split_once(':').unwrap().0.to_owned();
            assert!(
                !prev_seen_experiments.contains(&experiment_name),
                "Experiment {experiment_name} is used in multiple layers"
            );
            cur_experiments.insert(experiment_name);
        }
        prev_seen_experiments.extend(cur_experiments);
    }
}
