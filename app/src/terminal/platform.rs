use anyhow::Result;

pub fn init() -> Result<()> {
    #[cfg(target_os = "macos")]
    mac::init()?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod mac {
    #![allow(clippy::let_unit_value)]

    use std::ffi::{CStr, CString};
    use std::{env, str};

    use libc::{setlocale, LC_ALL, LC_CTYPE};
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use warpui::platform::mac::utils::nsstring_as_str;

    use super::*;

    const FALLBACK_LOCALE: &str = "UTF-8";

    pub fn init() -> Result<()> {
        set_locale_environment();

        // Switch to home directory.
        env::set_current_dir(dirs::home_dir().unwrap()).unwrap();

        Ok(())
    }

    pub fn set_locale_environment() {
        let env_locale_c = CString::new("").expect("Should never fail to create empty CString");
        let env_locale_ptr = unsafe { setlocale(LC_ALL, env_locale_c.as_ptr()) };
        if !env_locale_ptr.is_null() {
            let env_locale = unsafe { CStr::from_ptr(env_locale_ptr).to_string_lossy() };

            // Assume `C` locale means unchanged, since it is the default anyways.
            if env_locale != "C" {
                log::debug!("Using locale ({env_locale}) already set via LC_ALL");
                return;
            }
        }

        let system_locale = system_locale();
        if is_valid_locale(&system_locale).unwrap_or(false) {
            // Use system locale.
            log::debug!("Using system locale ({system_locale}) for LANG");

            // Set the LANG variable to suggest (but not require) use of the
            // given locale.  This avoids errors when ssh-ing into a remote
            // machine which doesn't have the given locale available.
            env::set_var("LANG", system_locale);
        } else {
            // Use fallback locale.
            log::debug!("Using fallback locale ({FALLBACK_LOCALE}) for LC_CTYPE");

            // When using a fallback, only set LC_CTYPE.
            env::set_var("LC_CTYPE", FALLBACK_LOCALE);
        }
    }

    /// Checks whether a given locale is valid.
    ///
    /// This changes the current value of LC_CTYPE in order to check validity,
    /// but restores the previous value before returning.
    fn is_valid_locale(locale: &str) -> Result<bool> {
        unsafe {
            let check_locale = CString::new("")?;
            let new_locale = CString::new(locale)?;

            let old_locale = setlocale(LC_CTYPE, check_locale.as_ptr());
            let is_valid = !setlocale(LC_CTYPE, new_locale.as_ptr()).is_null();
            setlocale(LC_CTYPE, old_locale);
            Ok(is_valid)
        }
    }

    /// Determine system locale based on language and country code.
    fn system_locale() -> String {
        unsafe {
            // Read the current locale from `NSLocale`. We purposefully don't call release on
            // `currentLocale` since we don't own the object (it was not obtained by using `new`,
            // `alloc`, `retain` or `copy`. See https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/MemoryMgmt/Articles/mmRules.html#//apple_ref/doc/uid/20000994-BAJHFBGH
            // for more details about memory management in Objective-C.
            let locale_class = class!(NSLocale);
            let locale: *const Object = msg_send![locale_class, currentLocale];

            // `localeIdentifier` returns extra metadata with the locale (including currency and
            // collator) on newer versions of macOS. This is not a valid locale, so we use
            // `languageCode` and `countryCode`, if they're available (macOS 10.12+):
            //
            // https://developer.apple.com/documentation/foundation/nslocale/1416263-localeidentifier?language=objc
            // https://developer.apple.com/documentation/foundation/nslocale/1643060-countrycode?language=objc
            // https://developer.apple.com/documentation/foundation/nslocale/1643026-languagecode?language=objc
            let is_language_code_supported: bool =
                msg_send![locale, respondsToSelector: sel!(languageCode)];
            let is_country_code_supported: bool =
                msg_send![locale, respondsToSelector: sel!(countryCode)];
            let locale_id = if is_language_code_supported && is_country_code_supported {
                let language_code: *const Object = msg_send![locale, languageCode];
                let language_code_str = nsstring_as_str(language_code)
                    .expect("should always be valid UTF-8 string")
                    .to_owned();
                let _: () = msg_send![language_code, release];

                let country_code: *const Object = msg_send![locale, countryCode];
                let country_code_str = nsstring_as_str(country_code)
                    .expect("should always be valid UTF-8 string")
                    .to_owned();
                let _: () = msg_send![country_code, release];

                format!("{}_{}.UTF-8", &language_code_str, &country_code_str)
            } else {
                let identifier: *const Object = msg_send![locale, localeIdentifier];
                let identifier_str = nsstring_as_str(identifier)
                    .expect("should always be valid UTF-8 string")
                    .to_owned();
                let _: () = msg_send![identifier, release];

                identifier_str + ".UTF-8"
            };

            locale_id
        }
    }
}
