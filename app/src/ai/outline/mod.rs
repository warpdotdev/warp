use ai::index::Outline;

mod native;
pub use native::*;

#[derive(Debug)]
pub enum OutlineStatus {
    /// The outline is being computed.
    Pending,
    /// The successfully computed outline.
    Complete(Outline),
    /// Outline creation failed.
    Failed,
}
