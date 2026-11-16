/// Detects if we're running in a Windows Parallels VM.
#[cfg(windows)]
pub fn is_running_in_windows_parallels_vm() -> bool {
    use winreg::RegKey;
    use winreg::enums::*;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    if let Ok(system_key) = hklm.open_subkey(r"HARDWARE\DESCRIPTION\System\BIOS")
        && let Ok(bios_version) = system_key.get_value::<String, _>("SystemManufacturer")
        && bios_version.to_lowercase().contains("parallels")
    {
        return true;
    }

    false
}

#[cfg(not(windows))]
pub fn is_running_in_windows_parallels_vm() -> bool {
    // On non-Windows platforms, we don't need this check
    false
}
