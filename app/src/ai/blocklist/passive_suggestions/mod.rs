mod legacy;
mod maa;
mod static_prompt_suggestions;

/// Size limits used to avoid materializing very large files for passive code
/// diff suggestions. They bound both the per-file size and the aggregate size
/// across all files in a single suggestion, and are shared between the legacy
/// path (which applies them to the files read before requesting) and the MAA
/// path (which applies them to the diffs returned before displaying them).
pub(super) const PASSIVE_CODE_DIFF_LONG_FILE_LINE_LIMIT: usize = 2000;
pub(super) const PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT: usize = 100_000;
pub(super) const PASSIVE_CODE_DIFF_TOTAL_LINE_LIMIT: usize = 2500;
pub(super) const PASSIVE_CODE_DIFF_TOTAL_BYTE_LIMIT: usize = 150_000;

pub use legacy::{
    PassiveSuggestionsEvent as LegacyPassiveSuggestionsEvent,
    PassiveSuggestionsModel as LegacyPassiveSuggestionsModel,
};
pub use maa::{
    PassiveSuggestionsEvent as MaaPassiveSuggestionsEvent,
    PassiveSuggestionsModel as MaaPassiveSuggestionsModel,
};
use warpui::ModelHandle;

#[derive(Clone)]
pub struct PassiveSuggestionsModels {
    pub legacy: ModelHandle<LegacyPassiveSuggestionsModel>,
    pub maa: ModelHandle<MaaPassiveSuggestionsModel>,
}
