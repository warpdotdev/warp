use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use crate::platform::SystemTheme;

const SYSTEM_THEME_SUBKEY_PATH: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const LIGHT_MODE_SUBKEY_NAME: &str = "AppsUseLightTheme";

/// Retrieves the system theme from the Windows Registry.
/// https://github.com/wez/wezterm/blob/b8f94c474ce48ac195b51c1aeacf41ae049b774e/window/src/os/windows/connection.rs#L42
pub fn get_system_theme() -> Result<SystemTheme, std::io::Error> {
    let theme_subkey = RegKey::predef(HKEY_CURRENT_USER).open_subkey(SYSTEM_THEME_SUBKEY_PATH)?;
    let theme_value = theme_subkey.get_value::<u32, _>(LIGHT_MODE_SUBKEY_NAME)?;
    match theme_value {
        1 => Ok(SystemTheme::Light),
        0 => Ok(SystemTheme::Dark),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("System theme value {theme_value:?} was invalid"),
        )),
    }
}
