use uuid::Uuid;
pub const DEFAULT_PLANNING_DOCUMENT_TITLE: &str = "Planning document";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AIDocumentId(Uuid);

impl AIDocumentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for AIDocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for AIDocumentId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::try_parse(&value)?))
    }
}

impl TryFrom<&str> for AIDocumentId {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::try_parse(value)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AIDocumentVersion(pub usize);

impl AIDocumentVersion {
    #[cfg(feature = "test-util")]
    pub fn new_for_test(version: usize) -> Self {
        Self(version)
    }

    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl std::fmt::Display for AIDocumentVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl Default for AIDocumentVersion {
    fn default() -> Self {
        Self(1)
    }
}
