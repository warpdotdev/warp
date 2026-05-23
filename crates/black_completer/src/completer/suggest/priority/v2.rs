use super::Priority;

impl From<Priority> for crate::signatures::Priority {
    fn from(value: Priority) -> Self {
        Self::new(value.value())
    }
}

impl From<crate::signatures::Priority> for Priority {
    fn from(value: crate::signatures::Priority) -> Self {
        Self::new(value.value())
    }
}
