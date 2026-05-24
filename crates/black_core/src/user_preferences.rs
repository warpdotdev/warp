//! Extension trait for accessing the private user preferences backend.
//!
//! Public settings are accessed exclusively through the settings macros
//! (`define_settings_group!`) and the `settings::PublicPreferences` wrapper,
//! which restricts direct access at compile time.

use std::ops::Deref;

use settings::PrivatePreferences;
use black_ui::SingletonEntity;
use black_ui_extras::user_preferences::UserPreferences;

/// An extension trait on [`black_ui::AppContext`] for accessing private user
/// preferences.
///
/// Private settings are always stored in the platform-native store (e.g.
/// UserDefaults on macOS, registry on Windows, JSON file on Linux) and never
/// appear in the user-visible settings file.
pub trait GetUserPreferences {
    /// Returns the preferences backend for private settings.
    ///
    /// Private settings are always stored in the platform-native store and never
    /// appear in the user-visible settings file.
    fn private_user_preferences(&self) -> &dyn UserPreferences;
}

impl GetUserPreferences for black_ui::AppContext {
    fn private_user_preferences(&self) -> &dyn UserPreferences {
        <PrivatePreferences as SingletonEntity>::as_ref(self).deref()
    }
}
