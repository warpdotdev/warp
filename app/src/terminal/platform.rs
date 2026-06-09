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
    use objc2::runtime::NSObjectProtocol;
    use objc2::sel;
    use objc2_foundation::NSLocale;

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
    //
    // `NSLocale::countryCode` is deprecated in the SDK, but we keep using it to
    // assemble a POSIX locale string; there is no non-deprecated accessor that
    // returns the bare country code.
    #[allow(deprecated)]
    fn system_locale() -> String {
        // Read the current locale from `NSLocale`. objc2 returns each getter
        // result as a `Retained`, which claims the autoreleased value and
        // releases it on drop.
        let locale = NSLocale::currentLocale();

        // `localeIdentifier` returns extra metadata with the locale (including currency and
        // collator) on newer versions of macOS. This is not a valid locale, so we use
        // `languageCode` and `countryCode`, if they're available (macOS 10.12+):
        //
        // https://developer.apple.com/documentation/foundation/nslocale/1416263-localeidentifier?language=objc
        // https://developer.apple.com/documentation/foundation/nslocale/1643060-countrycode?language=objc
        // https://developer.apple.com/documentation/foundation/nslocale/1643026-languagecode?language=objc
        let is_language_code_supported = locale.respondsToSelector(sel!(languageCode));
        let is_country_code_supported = locale.respondsToSelector(sel!(countryCode));
        if is_language_code_supported && is_country_code_supported {
            let language_code = locale.languageCode().to_string();
            // `countryCode` is nil for a region-less locale, so degrade to an
            // empty country.
            let country_code = locale
                .countryCode()
                .map(|c| c.to_string())
                .unwrap_or_default();

            format!("{language_code}_{country_code}.UTF-8")
        } else {
            let identifier = locale.localeIdentifier().to_string();

            identifier + ".UTF-8"
        }
    }
}
