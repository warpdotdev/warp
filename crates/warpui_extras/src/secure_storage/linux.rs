//! Implementation of the [`SecureStorage`] service for the Linux platform.

use std::{cell::OnceCell, collections::HashMap, path::PathBuf};

use anyhow::{anyhow, Context};
use rand::RngCore;
use ring::aead;
use secret_service::{
    blocking::{Item, SecretService},
    EncryptionType,
};

use super::Error;

/// Implementation of the SecureStorage service using the Secret Service API.
pub struct SecureStorage {
    /// The value to set for the "service" attribute, used to define a
    /// namespace for keys for the application.
    service_name: String,

    /// A lazily-initialized reference to the default secret collection as
    /// provided by the installed Secret Service API provider.
    collection: OnceCell<Option<Collection>>,

    /// The fallback path to a directory in case a secret collection is
    /// not available.
    fallback_dir: Option<PathBuf>,

    /// The encryption fallback key.
    encryption_key: OnceCell<Option<aead::LessSafeKey>>,
}

impl SecureStorage {
    /// Creates a new [`SecureStorage`] instance.
    ///
    /// This does not eagerly open a connection to dbus or the underlying
    /// Secret Service provider.
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
            collection: OnceCell::new(),
            fallback_dir: None,
            encryption_key: OnceCell::new(),
        }
    }

    /// Creates a new [`SecureStorage`] instance with disk fallback
    ///
    /// Does the same work as [`SecureStorage::new`], as well as storing
    /// a path to a fallback directory.
    pub fn new_with_fallback(service_name: &str, fallback_dir: PathBuf) -> Self {
        Self {
            service_name: service_name.to_owned(),
            collection: OnceCell::new(),
            fallback_dir: Some(fallback_dir),
            encryption_key: OnceCell::new(),
        }
    }

    /// Returns a reference to the default secret collection, lazily
    /// instantiating the underlying service and collection reference,
    /// returning an error if the connection cannot be established or the
    /// collection cannot be opened.
    ///
    /// TODO(vorporeal): Decide whether we want to "cache" a failed connection
    /// or instead only store the collection on a successful connection and
    /// return the error on an initialization failure.
    fn collection(&self) -> Result<&secret_service::blocking::Collection<'_>, Error> {
        self.collection
            .get_or_init(|| match Collection::open_default_collection() {
                Ok(collection) => Some(collection),
                Err(err) => {
                    log::error!("Failed to acquire default Secret Service collection: {err:#}");
                    None
                }
            })
            .as_ref()
            .ok_or_else(|| {
                Error::Unknown(anyhow!("Failed to initialize Secret Service connection"))
            })
            .and_then(|collection| {
                let collection = collection.borrow_collection();
                // Ensure that the collection is unlocked, otherwise we will be unable
                // add or modify items.
                collection.unlock()?;
                Ok(collection)
            })
    }

    /// Returns an encryption key that can be used to encrypt
    /// values if the default secret collection is not available
    /// or otherwise not working. The key is lazy initialized since
    /// it does not need to be created unless the main method of
    /// storing secrets has failed
    fn encryption_key(&self) -> Result<&aead::LessSafeKey, Error> {
        self.encryption_key
            .get_or_init(|| {
                // We can use whatever super duper foolproof secure key we want here.
                // Here we are specifically choosing a value that will look inconspicuous
                // in case someone chooses to scan our binary for strings.
                let mut key_bytes = Vec::from("https://releases.warp.dev/channel_versions.json");
                key_bytes.resize(aead::AES_256_GCM.key_len(), 0);
                match aead::UnboundKey::new(&aead::AES_256_GCM, key_bytes.as_slice()) {
                    Ok(key) => Some(aead::LessSafeKey::new(key)),
                    Err(_) => {
                        log::error!("Failed to initialize fallback encryption key");
                        None
                    }
                }
            })
            .as_ref()
            .ok_or_else(|| Error::Unknown(anyhow!("Invalid encryption key")))
    }

    /// Returns the set of attributes which should be used when interacting
    /// with a secret item that is identified by the given key.
    fn attributes_for_key<'a>(&'a self, key: &'a str) -> HashMap<&'static str, &'a str> {
        HashMap::from([
            // Ensure our keys don't conflict with ones stored by another
            // application.
            ("service", self.service_name.as_str()),
            // Specify the key for the secret.
            ("key", key),
        ])
    }

    /// Provides the given function access to a secret item with the given key
    /// in order to read or manipulate the item.
    fn with_item<T>(
        &self,
        key: &str,
        func: impl FnOnce(&Item) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let collection = self.collection()?;
        let items = collection.search_items(self.attributes_for_key(key))?;
        let Some(item) = items.first() else {
            return Err(Error::NotFound);
        };
        func(item)
    }

    fn write_secret_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let collection = self.collection()?;
        // Construct a slightly more human-readable label for the secret than
        // using the key alone.
        let label = format!("{}: {key}", self.service_name);
        collection.create_item(
            &label,
            self.attributes_for_key(key),
            value.as_bytes(),
            // replace the existing key, if any
            true,
            "text/plain",
        )?;
        Ok(())
    }

    fn fallback_encrypt(&self, value: &str) -> Result<Vec<u8>, Error> {
        let encryption_key = self.encryption_key()?;

        // Generates nonce by randomly generating numbers
        // This is not the official best way to do this, but it should
        // be fine for our purposes.
        let mut rng = rand::thread_rng();
        let mut nonce_bytes = [0u8; aead::NONCE_LEN];
        rng.fill_bytes(&mut nonce_bytes);
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);

        let mut data = value.as_bytes().to_vec();
        encryption_key
            .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut data)
            .map_err(Into::<Error>::into)
            .context("Fallback encryption failed")?;

        // We serialize this to disk as the 12 byte nonce followed by the message.
        let mut output = Vec::<u8>::with_capacity(aead::NONCE_LEN + data.len());
        output.extend_from_slice(&nonce_bytes);
        output.append(&mut data);

        Ok(output)
    }

    fn fallback_decrypt(&self, value: &[u8]) -> Result<String, Error> {
        if value.len() < aead::NONCE_LEN + 1 {
            return Err(Error::Unknown(anyhow!(
                "Attempting to decrypt too small value for fallback decryption"
            )));
        }

        let encryption_key = self.encryption_key()?;

        // The first 12 bytes of the message are the nonce.
        let nonce_bytes = &value[0..aead::NONCE_LEN];
        let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(Into::<Error>::into)
            .context("Failed to parse nonce for fallback decryption")?;

        // The remaining bytes in the message are the data.
        // We convert this to owned b/c the decryption happens in place.
        let mut data_bytes = value[aead::NONCE_LEN..].to_owned();
        let decrypted_length = encryption_key
            .open_in_place(nonce, aead::Aad::empty(), &mut data_bytes)
            .map_err(Into::<Error>::into)
            .context("Fallback decryption failed")?
            .len();

        // The decryption happens in place, but does not resize the vec.
        // Meanwhile, a slice referring to the decrypted data is returned.
        // We use the length of that slice to resize the currently owned Vec,
        // so it can be consumed by String::from_utf8 later on.
        data_bytes.resize(decrypted_length, 0);

        String::from_utf8(data_bytes).map_err(|err| Error::DecodeError(err.utf8_error()))
    }

    fn fallback_file(&self, key: &str) -> Result<PathBuf, Error> {
        let Some(dir) = &self.fallback_dir else {
            return Err(Error::NotFound);
        };
        let filename = format!("{}-{key}", self.service_name);
        let mut path = dir.clone();
        path.push(filename);
        Ok(path)
    }

    fn write_fallback_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let fallback_file = self.fallback_file(key)?;

        let encrypted = self.fallback_encrypt(value)?;

        std::fs::write(fallback_file, encrypted).map_err(|err| Error::Unknown(err.into()))
    }

    fn read_fallback_value(&self, key: &str) -> Result<String, Error> {
        let fallback_file = self.fallback_file(key)?;

        let data = std::fs::read(fallback_file).map_err(|_| Error::NotFound)?;
        self.fallback_decrypt(&data)
    }

    fn delete_fallback_value(&self, key: &str) -> Result<(), Error> {
        let fallback_file = self.fallback_file(key)?;
        std::fs::remove_file(fallback_file).map_err(|err| match err {
            ref io_error if io_error.kind() == std::io::ErrorKind::NotFound => Error::NotFound,
            io_error => Error::Unknown(io_error.into()),
        })
    }
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let secret_result = self.write_secret_value(key, value);

        match secret_result {
            Ok(_) => {
                // If we are able to write the secret value, we attempt to delete any fallback values
                let _ = self.delete_fallback_value(key);
                Ok(())
            }
            Err(_) => self.write_fallback_value(key, value),
        }
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        let secret_result = self.with_item(key, |item| {
            let bytes = item.get_secret()?;
            String::from_utf8(bytes).map_err(|err| Error::DecodeError(err.utf8_error()))
        });

        match secret_result {
            Ok(value) => {
                // If we are able to read the secret value, we attempt to delete any fallback values
                let _ = self.delete_fallback_value(key);
                Ok(value)
            }
            // TODO(daprahamian): We might want to filter on specific error values, rather than all errors
            Err(_) => self.read_fallback_value(key),
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        let secret_result = self.with_item(key, |item| item.delete().map_err(Into::into));
        let fs_result = self.delete_fallback_value(key);

        // We delete both the value in the secret store and the fallback values.
        // As long as one succeeds, we consider the delete a success.
        match (secret_result, fs_result) {
            (Err(secret_err), Err(_)) => Err(secret_err),
            _ => Ok(()),
        }
    }
}

impl From<secret_service::Error> for Error {
    fn from(value: secret_service::Error) -> Self {
        // TODO(vorporeal): Check to see if we can return any more specific
        // values.
        Error::Unknown(anyhow!(value))
    }
}

impl From<ring::error::Unspecified> for Error {
    fn from(value: ring::error::Unspecified) -> Self {
        Error::Unknown(anyhow!(value))
    }
}

/// A helper structure that maintains access to the default collection.
///
/// [`secret_service::SecretService`] is a self-referential struct that leaks
/// its internal reference lifetime, which is why we use [`ouroboros`] here to
/// provide a safe API for interacting with the service and collection.
#[ouroboros::self_referencing]
struct Collection {
    /// An encrypted dbus connection to the Secret Service API provider.
    #[borrows()]
    #[covariant]
    service: SecretService<'this>,

    /// A reference to the default secret collection, which can be used to
    /// add, remove and read secrets.
    #[borrows(service)]
    #[covariant]
    collection: secret_service::blocking::Collection<'this>,
}

impl Collection {
    /// Tries to open the default secret collection via the Secret Service
    /// API.
    fn open_default_collection() -> Result<Self, Error> {
        SecretService::connect(EncryptionType::Plain)
            .and_then(|service| {
                CollectionTryBuilder {
                    service,
                    collection_builder: |service| service.get_default_collection(),
                }
                .try_build()
            })
            .map_err(Into::into)
    }
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
