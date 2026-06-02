//! Implementation of the [`UserPreferences`] trait using macOS user defaults.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_foundation::{NSString, NSUserDefaults};

/// A user preferences store backed by macOS user defaults (`NSUserDefaults`).
pub struct UserDefaultsPreferencesStorage {
    /// A strong reference to the `NSUserDefaults` backing store.
    user_defaults: Retained<NSUserDefaults>,
}

impl UserDefaultsPreferencesStorage {
    /// Constructs a new preferences store.
    ///
    /// If `suite_name` is provided, it is used as the domain within
    /// the user defaults system.  Otherwise, the standard user defaults for
    /// the current application are used.
    pub fn new(suite_name: Option<String>) -> Self {
        Self {
            user_defaults: Self::user_defaults(suite_name),
        }
    }

    /// Returns a strong reference to the `NSUserDefaults` backing store that
    /// should be used for the given suite name.
    ///
    /// If [`None`] is provided as the suite name, the standard user defaults
    /// will be used (namespaced based on the current application).
    fn user_defaults(suite_name: Option<String>) -> Retained<NSUserDefaults> {
        // Calling `[[NSUserDefaults alloc] initWithSuiteName]`` where the suite name is the
        // application's bundle ID (the default `data_domain` if `data_profile` is unset)
        // _should_ be equivalent to `[NSUserDefaults standardUserDefaults]`. However, in case
        // the two ever deviate, we explicitly use `standardUserDefaults` below. The Apple docs
        // also imply that `standardUserDefaults` is cached.
        if let Some(suite_name) = &suite_name {
            let suite_name = NSString::from_str(suite_name);

            // `initWithSuiteName:` only returns nil when the suite name is a reserved domain
            // (NSGlobalDomain/NSArgumentDomain/NSRegistrationDomain) or the app's own bundle
            // identifier; our `{app_id}-{profile}` suite name is never either of those, so this
            // is unreachable in practice.
            NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&suite_name)).expect(
                "initWithSuiteName: only returns nil for a reserved domain or the app's own bundle id",
            )
        } else {
            NSUserDefaults::standardUserDefaults()
        }
    }
}

impl super::UserPreferences for UserDefaultsPreferencesStorage {
    fn write_value(&self, key: &str, value: String) -> Result<(), super::Error> {
        let key = NSString::from_str(key);
        let value = NSString::from_str(&value);
        let value: &AnyObject = &value;

        // `setObject:forKey:` stores an arbitrary object; the value and key are
        // both `NSString`s, which are valid property-list types.
        unsafe {
            self.user_defaults.setObject_forKey(Some(value), &key);
        }
        Ok(())
    }

    fn read_value(&self, key: &str) -> Result<Option<String>, super::Error> {
        let key = NSString::from_str(key);
        match self.user_defaults.stringForKey(&key) {
            Some(value) => Ok(Some(value.to_string())),
            None => Ok(None),
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), super::Error> {
        let key = NSString::from_str(key);
        self.user_defaults.removeObjectForKey(&key);
        Ok(())
    }
}
