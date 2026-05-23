//! Implementations of the [`SecureStorage`] service for the macOS platform.

use anyhow::anyhow;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::keychain_item::SecKeychainItem;
use security_framework::os::macos::passwords::SecKeychainItemPassword;

use super::Error;

/// Implementation of the SecureStorage service using macOS Security
/// framework keychains.
pub struct SecureStorage {
    /// The name of the service under which to store the values.
    service_name: String,
}

impl SecureStorage {
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
        }
    }
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let keychain = SecKeychain::default()?;
        keychain
            .set_generic_password(self.service_name.as_str(), key, value.as_bytes())
            .map_err(Into::into)
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        let (password, _) = self.get_password_item(key)?;
        String::from_utf8(password.as_ref().to_vec())
            .map_err(|err| Error::DecodeError(err.utf8_error()))
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        let (_, item) = self.get_password_item(key)?;
        item.delete();
        Ok(())
    }
}

impl SecureStorage {
    fn get_password_item(
        &self,
        key: &str,
    ) -> Result<(SecKeychainItemPassword, SecKeychainItem), Error> {
        let keychain = SecKeychain::default()?;
        keychain
            .find_generic_password(&self.service_name, key)
            .map_err(|_| Error::NotFound)
    }
}

impl From<security_framework::base::Error> for Error {
    fn from(value: security_framework::base::Error) -> Self {
        Error::Unknown(anyhow!(value))
    }
}
