//! Module containing operating system information such as the name, category, and version.

use serde::Serialize;
use serde_with::SerializeDisplay;
use std::fmt::{Display, Formatter};
use std::sync::OnceLock;

#[cfg(target_family = "wasm")]
use warpui::platform::wasm;
#[cfg(target_family = "wasm")]
use warpui::platform::OperatingSystem;

static OS_INFO: OnceLock<Result<OperatingSystemInfo, OperatingSystemInfoError>> = OnceLock::new();

/// Information of the operating system of the client.
#[derive(Serialize)]
pub struct OperatingSystemInfo {
    /// The name of the operating system. On Linux this is the name of the distribution.
    name: String,
    /// The version of the operating system. On Linux this is the version of the distribution, not
    /// the Linux kernel version. `None` if the version could not be computed for any reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    /// The category of the operating system (e.g. "Linux", "macOS", "Windows", or "Web").
    category: OperatingSystemCategory,
    /// The version of the linux kernel, if running on Linux. If not on Linux, this is always
    /// `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    linux_kernel_version: Option<String>,
    /// The name of the browser parsed from the user agent, if running on Web. If not on Web,
    /// this is always `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_name: Option<String>,
    /// The version of the browser parsed from the user agent, if running on Web. If not on
    /// Web, this is always `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_version: Option<String>,
}

impl OperatingSystemInfo {
    #[cfg(not(target_family = "wasm"))]
    fn new() -> Result<Self, OperatingSystemInfoError> {
        let os_category =
            OperatingSystemCategory::new().ok_or(OperatingSystemInfoError::Unknown)?;

        let (os_name, version, linux_kernel_version) =
            if os_category == OperatingSystemCategory::Linux {
                (
                    // If we can't compute the distro name, fallback to "Linux" as
                    // the os release name.
                    sysinfo::System::name().unwrap_or_else(|| "Linux".to_string()),
                    sysinfo::System::os_version(),
                    sysinfo::System::kernel_version(),
                )
            } else {
                (os_category.to_string(), sysinfo::System::os_version(), None)
            };

        Ok(Self {
            name: os_name,
            version,
            category: os_category,
            linux_kernel_version,
            browser_name: None,
            browser_version: None,
        })
    }

    #[cfg(target_family = "wasm")]
    fn new() -> Result<Self, OperatingSystemInfoError> {
        // To make sure the operating system names are consistent between native
        // and web platforms, we try to use the display names encoded by the
        // `OperatingSystemCategory` enum.
        let os = match OperatingSystem::get() {
            OperatingSystem::Linux => OperatingSystemCategory::Linux.to_string(),
            OperatingSystem::Mac => OperatingSystemCategory::Mac.to_string(),
            OperatingSystem::Windows => OperatingSystemCategory::Windows.to_string(),
            OperatingSystem::Other(Some(os)) => os.to_string(),
            _ => "Unknown".to_string(),
        };

        Ok(Self {
            name: os,
            version: wasm::current_os_version().map(str::to_string),
            category: OperatingSystemCategory::Web,
            browser_name: wasm::current_browser().map(str::to_string),
            browser_version: wasm::current_browser_version().map(str::to_string),
            linux_kernel_version: None,
        })
    }

    /// Returns the current [`OperatingSystemInfo`]. If the system information was unable to be
    /// computed, an `Err` is returned.
    pub fn get() -> Result<&'static Self, OperatingSystemInfoError> {
        let inner = OS_INFO.get_or_init(Self::new);
        inner.as_ref().map_err(|error| *error)
    }

    /// Returns the name of the operating system. On Linux this is the name of the distribution.
    /// On all other platforms it should be equivalent to `category`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the version of the operating system. On Linux this is the version of the
    /// distribution, not the Linux kernel version. Returns `None` if the version could not be
    /// computed for any reason.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns the category of the operating system (e.g. "Linux", "macOS", or "Windows").
    pub fn category(&self) -> &OperatingSystemCategory {
        &self.category
    }

    pub fn linux_kernel_version(&self) -> Option<&str> {
        self.linux_kernel_version.as_deref()
    }
}

#[derive(SerializeDisplay, PartialEq)]
pub enum OperatingSystemCategory {
    Linux,
    Mac,
    #[allow(dead_code)]
    Windows,
    Web,
}

impl OperatingSystemCategory {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn new() -> Option<Self> {
        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            Some(OperatingSystemCategory::Linux)
        } else if cfg!(target_os = "macos") {
            Some(OperatingSystemCategory::Mac)
        } else if cfg!(target_os = "windows") {
            Some(OperatingSystemCategory::Windows)
        } else if cfg!(target_family = "wasm") {
            Some(OperatingSystemCategory::Web)
        } else {
            None
        }
    }
}

impl Display for OperatingSystemCategory {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            OperatingSystemCategory::Linux => "Linux",
            OperatingSystemCategory::Mac => "macOS",
            OperatingSystemCategory::Windows => "Windows",
            OperatingSystemCategory::Web => "Web",
        };
        write!(f, "{str}")
    }
}

/// Error type returned when trying to compute the [`OperatingSystemInfo`].
#[derive(thiserror::Error, Debug, Clone, Copy)]
pub enum OperatingSystemInfoError {
    #[error("computing the operating system information is unsupported on this platform")]
    #[allow(dead_code)]
    UnsupportedPlatform,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    #[error("unable to compute the operating system information")]
    Unknown,
}
