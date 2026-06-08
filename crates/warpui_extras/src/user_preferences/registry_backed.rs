use std::io;

use windows_registry::{Key, CURRENT_USER};
use windows_result::HRESULT;

/// Store user preferences in the Windows Registry.
/// Modeled after https://github.com/neovide/neovide/blob/main/src/windows_utils.rs .
use super::UserPreferences;

pub struct RegistryBackedPreferences {
    app_key_path: String,
}

static WARP_REGISTRY_BASE_PATH: &str = "Software\\Warp.dev\\";
pub const KEY_NOT_FOUND_ERR: HRESULT = HRESULT::from_win32(0x80070002);

impl RegistryBackedPreferences {
    /// Construct a separate registry path for each channel (stable, dev, local, etc.)
    pub fn new(app_name: &str) -> Self {
        Self {
            app_key_path: WARP_REGISTRY_BASE_PATH.to_owned() + app_name,
        }
    }

    /// Gets Warp's registry key, creating it if it does not already exist.
    fn get_warp_registry(&self) -> Result<Key, super::Error> {
        CURRENT_USER.create(self.app_key_path.clone()).map_err(|e| {
            log::error!("unable to access Warp app key in Windows Registry: {e:#}");
            super::Error::IoError(io::Error::from(e))
        })
    }
}

impl UserPreferences for RegistryBackedPreferences {
    fn read_value(&self, name: &str) -> Result<Option<String>, super::Error> {
        Ok(self.get_warp_registry()?.get_string(name).ok())
    }

    fn write_value(&self, key: &str, value: String) -> Result<(), super::Error> {
        self.get_warp_registry()?
            .set_string(key, value.as_str())
            .map_err(|e| super::Error::from(io::Error::from(e)))
    }

    fn remove_value(&self, key: &str) -> Result<(), super::Error> {
        match self.get_warp_registry()?.remove_value(key) {
            Ok(_) => Ok(()),
            // If the key doesn't exist, then treat removal of that nonexistent key as a success.
            Err(e) if e.code() == KEY_NOT_FOUND_ERR => Ok(()),
            Err(e) => Err(super::Error::from(io::Error::from(e))),
        }
    }
}
