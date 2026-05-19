//! Loading animation components for AI features.

mod shimmering_warp_loading_text;
pub use shimmering_warp_loading_text::shimmering_warp_loading_text;

mod warping_verb;
pub use warping_verb::{
    normalize_warping_verb, normalize_warping_verbs, WarpingVerbSelector, MAX_CUSTOM_WARPING_VERBS,
};
// Re-exported for tests and a planned Settings UI follow-up.
#[allow(unused_imports)]
pub use warping_verb::{DEFAULT_WARPING_VERB, MAX_WARPING_VERB_CHARS};

mod warping_verb_pack;
// Re-exported for use by a planned Settings UI follow-up and by natural-language
// agent flows that look up packs by identifier.
#[allow(unused_imports)]
pub use warping_verb_pack::WarpingVerbPack;
