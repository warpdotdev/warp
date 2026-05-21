//! Implementation of the [`UserPreferences`] trait using macOS user defaults.

#![allow(deprecated)]

use cocoa::base::{id, nil};
use objc::rc::StrongPtr;
use objc::{class, msg_send, sel, sel_impl};

/// A user preferences store backed by macOS user defaults (`NSUserDefaults`).
pub struct UserDefaultsPreferencesStorage {
    /// A strong reference to the `NSUserDefaults` backing store.
    user_defaults: StrongPtr,
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
    fn user_defaults(suite_name: Option<String>) -> StrongPtr {
        unsafe {
            // Calling `[[NSUserDefaults alloc] initWithSuiteName]`` where the suite name is the
            // application's bundle ID (the default `data_domain` if `data_profile` is unset)
            // _should_ be equivalent to `[NSUserDefaults standardUserDefaults]`. However, in case
            // the two ever deviate, we explicitly use `standardUserDefaults` below. The Apple docs
            // also imply that `standardUserDefaults` is cached.
            if let Some(suite_name) = &suite_name {
                let defaults: id = msg_send![class!(NSUserDefaults), alloc];
                let suite_name = util::make_nsstring(suite_name);

                StrongPtr::new(msg_send![defaults, initWithSuiteName: *suite_name])
            } else {
                StrongPtr::retain(msg_send![class!(NSUserDefaults), standardUserDefaults])
            }
        }
    }
}

impl super::UserPreferences for UserDefaultsPreferencesStorage {
    fn write_value(&self, key: &str, value: String) -> Result<(), super::Error> {
        unsafe {
            let key = util::make_nsstring(key);
            let value = util::make_nsstring(&value);

            let _: () = msg_send![*self.user_defaults, setObject: *value forKey: *key];
            Ok(())
        }
    }

    fn read_value(&self, key: &str) -> Result<Option<String>, super::Error> {
        unsafe {
            let key = util::make_nsstring(key);
            let value: id = msg_send![*self.user_defaults, stringForKey: *key];
            if value != nil {
                Ok(Some(
                    warpui::platform::mac::utils::nsstring_as_str(value)?.to_owned(),
                ))
            } else {
                Ok(None)
            }
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), super::Error> {
        unsafe {
            let key = util::make_nsstring(key);
            let _: () = msg_send![*self.user_defaults, removeObjectForKey: *key];
            Ok(())
        }
    }
}

mod util {
    use cocoa::base::nil;
    use cocoa::foundation::NSString;
    use objc::rc::StrongPtr;

    /// Creates a new `NSString` from the given `&str`, wrapped in a
    /// [`StrongPtr`] so it is released when the `StrongPtr` is dropped.
    ///
    /// **Important:** when passing the result to `msg_send!`, always
    /// dereference it (e.g. `msg_send![obj, foo: *nsstring]`). Passing the
    /// `StrongPtr` itself by value causes it to be moved into the
    /// `unsafe extern fn` call that `msg_send!` transmutes to, and Rust will
    /// not run the `StrongPtr`'s `Drop` glue after that call, leaking the
    /// underlying `NSString`.
    pub fn make_nsstring(value: &str) -> StrongPtr {
        unsafe { StrongPtr::new(NSString::alloc(nil).init_str(value)) }
    }
}
