use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, PartialEq, Debug, Copy, Eq, PartialOrd, Ord, Hash)]
pub struct BufferVersion(usize);

impl BufferVersion {
    /// Constructs a new app-unique content version.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        BufferVersion(raw)
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }
}
