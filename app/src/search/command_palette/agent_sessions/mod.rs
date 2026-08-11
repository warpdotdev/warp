//! The session-search popup's corpus: every CLI-agent session Warp can name.
//!
//! Three sources feed it — the sessions running in open tabs, the durable
//! handle store, and the on-disk scan of the agent's own transcripts — merged
//! into one deduped candidate list when the popup opens.
//!
//! The word "conversation" is deliberately absent from every identifier here.
//! Warp's own AI history (`ConversationNavigationData`, the `conversations:`
//! filter) is a completely disjoint corpus, and a shared vocabulary between the
//! two would be a standing invitation to wire one to the other.

pub mod candidate;
pub mod content_data_source;
pub mod content_search_item;
pub mod data_source;
pub mod search;
pub mod search_item;
pub mod tiers;

pub use candidate::{AgentSessionCandidate, CandidateOrigin};
pub use content_data_source::ContentDataSource;
pub use data_source::DataSource;
