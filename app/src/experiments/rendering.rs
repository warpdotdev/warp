use std::collections::HashMap;

use lazy_static::lazy_static;

use super::Layer;

lazy_static! {
    pub(super) static ref LAYER: Layer = Layer {
        name: "RenderingLayer",
        hasher_seeds: (20241104, 132105),
        traffic_allocations: HashMap::from([]),
        bucket_ranges: vec![]
    };
}
