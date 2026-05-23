//! No-op [`SecureStorage`] service for use in unit and integration tests.

use super::Error;

#[derive(Default)]
pub struct SecureStorage {}

impl SecureStorage {
    pub fn new(_service_name: &str) -> Self {
        Self {}
    }
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, _key: &str, _value: &str) -> Result<(), Error> {
        Ok(())
    }

    fn read_value(&self, _key: &str) -> Result<String, Error> {
        Ok("".to_string())
    }

    fn remove_value(&self, _key: &str) -> Result<(), Error> {
        Ok(())
    }
}
