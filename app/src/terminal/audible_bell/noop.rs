//! Module containing a noop implementation of an audible bell.
pub(super) struct AudibleBell;

impl AudibleBell {
    pub fn new() -> Self {
        Self
    }

    pub fn ring(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
