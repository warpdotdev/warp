use std::path::PathBuf;
use windows::{
    core::BSTR,
    Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB},
    },
};

use super::Error;

#[derive(Default)]
pub struct SecureStorage {
    service_name: String,
    storage_dir: PathBuf,
}

impl SecureStorage {
    pub fn new_with_path(service_name: &str, storage_dir: PathBuf) -> Self {
        Self {
            service_name: service_name.to_string(),
            storage_dir,
        }
    }

    fn storage_file(&self, key: &str) -> PathBuf {
        let filename = format!("{}-{key}", self.service_name);
        self.storage_dir.join(filename)
    }

    fn byte_vec_to_blob(byte_vec: &mut Vec<u8>) -> CRYPT_INTEGER_BLOB {
        let byte_slice = byte_vec.as_mut_slice();
        CRYPT_INTEGER_BLOB {
            cbData: byte_slice.len() as u32,
            pbData: byte_slice.as_mut_ptr(),
        }
    }

    fn encrypt(key: &str, mut plaintext: String) -> Result<Vec<u8>, Error> {
        let mut encrypted_blob = CRYPT_INTEGER_BLOB::default();
        let encrypted_bytes = unsafe {
            let plaintext_bytes = plaintext.as_bytes_mut();
            let plaintext_blob = CRYPT_INTEGER_BLOB {
                cbData: plaintext_bytes.len() as u32,
                pbData: plaintext_bytes.as_mut_ptr(),
            };
            CryptProtectData(
                &plaintext_blob,
                &BSTR::from(key),
                None,
                None,
                None,
                0,
                &mut encrypted_blob,
            )?;
            let encrypted_bytes =
                std::slice::from_raw_parts(encrypted_blob.pbData, encrypted_blob.cbData as usize)
                    .to_vec();
            LocalFree(Some(HLOCAL(encrypted_blob.pbData.cast())));
            encrypted_bytes
        };
        Ok(encrypted_bytes)
    }

    fn decrypt(mut encrypted_bytes: Vec<u8>) -> Result<String, Error> {
        let encrypted_blob = Self::byte_vec_to_blob(&mut encrypted_bytes);
        let mut decrypted_blob = CRYPT_INTEGER_BLOB::default();
        let decrypted_bytes = unsafe {
            CryptUnprotectData(
                &encrypted_blob,
                None,
                None,
                None,
                None,
                0,
                &mut decrypted_blob,
            )?;
            let byte_vec =
                std::slice::from_raw_parts(decrypted_blob.pbData, decrypted_blob.cbData as usize)
                    .to_vec();
            LocalFree(Some(HLOCAL(decrypted_blob.pbData.cast())));
            byte_vec
        };
        Ok(String::from_utf8(decrypted_bytes)?)
    }
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let storage_file = self.storage_file(key);
        let encrypted_bytes = Self::encrypt(key, value.to_string())?;
        std::fs::write(storage_file, encrypted_bytes).map_err(Error::from)
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        let storage_file = self.storage_file(key);
        let file_bytes = std::fs::read(storage_file)?;
        Self::decrypt(file_bytes)
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        let storage_file = self.storage_file(key);
        std::fs::remove_file(storage_file).map_err(Error::from)
    }
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod test;
