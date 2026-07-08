//! Loading animation components for AI features.

mod shimmering_warp_loading_text;
pub use shimmering_warp_loading_text::shimmering_warp_loading_text;

mod warping_verb;
pub use warping_verb::{normalize_warping_verb, WarpingVerbSelector, MAX_CUSTOM_WARPING_VERBS};

mod warping_verb_pack;
pub use warping_verb_pack::WarpingVerbPack;
