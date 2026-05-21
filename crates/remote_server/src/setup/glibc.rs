//! Parsing helpers for the libc family + version reported by the
//! `preinstall_check.sh` script.
//!
//! This is split into its own submodule so the libc-specific logic
//! (version parsing, family classification) can evolve independently
//! from the rest of [`crate::setup`].

use std::fmt;

/// A glibc `(major, minor)` version pair, e.g. `2.31`.
///
/// Wraps a labelled struct rather than a raw `(u32, u32)` so the meaning
/// of each field is obvious at call sites and in event payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlibcVersion {
    pub major: u32,
    pub minor: u32,
}

impl GlibcVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Parses a `<major>.<minor>` (or `<major>.<minor>.<patch>`) string.
    /// Only the first two segments are consumed; trailing components
    /// (e.g. patch versions, distro suffixes) are ignored.
    ///
    /// Returns `None` if either segment is missing or non-numeric.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let (major, rest) = value.split_once('.')?;
        // Allow `2.31`, `2.31.0`, `2.31-foo`, etc.
        let minor = rest.split(|c: char| !c.is_ascii_digit()).next()?;
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }
}

impl fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Detected libc on the remote host, derived from the `libc_family` /
/// `libc_version` keys emitted by `preinstall_check.sh`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteLibc {
    Glibc(GlibcVersion),
    NonGlibc { name: String },
    Unknown,
}

/// Builds a [`RemoteLibc`] from the raw `libc_family` / `libc_version`
/// values pulled out of the script's `key=value` stdout.
pub(super) fn parse_libc(family: Option<&str>, version: Option<&str>) -> RemoteLibc {
    match family {
        Some("glibc") => match version.and_then(GlibcVersion::parse) {
            Some(v) => RemoteLibc::Glibc(v),
            None => RemoteLibc::Unknown,
        },
        Some(name) if !name.is_empty() && name != "unknown" => RemoteLibc::NonGlibc {
            name: name.to_string(),
        },
        _ => RemoteLibc::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_major_minor() {
        assert_eq!(GlibcVersion::parse("2.31"), Some(GlibcVersion::new(2, 31)));
    }

    #[test]
    fn parses_major_minor_patch() {
        assert_eq!(
            GlibcVersion::parse("2.35.0"),
            Some(GlibcVersion::new(2, 35))
        );
    }

    #[test]
    fn parses_distro_suffix() {
        // e.g. "2.35-0ubuntu3.4"
        assert_eq!(
            GlibcVersion::parse("2.35-0ubuntu3.4"),
            Some(GlibcVersion::new(2, 35))
        );
    }

    #[test]
    fn returns_none_on_garbage() {
        assert_eq!(GlibcVersion::parse("garbage"), None);
        assert_eq!(GlibcVersion::parse(""), None);
        assert_eq!(GlibcVersion::parse("2.x"), None);
    }

    #[test]
    fn parse_libc_glibc() {
        assert_eq!(
            parse_libc(Some("glibc"), Some("2.31")),
            RemoteLibc::Glibc(GlibcVersion::new(2, 31))
        );
    }

    #[test]
    fn parse_libc_glibc_unparseable_version() {
        assert_eq!(
            parse_libc(Some("glibc"), Some("garbage")),
            RemoteLibc::Unknown
        );
        assert_eq!(parse_libc(Some("glibc"), None), RemoteLibc::Unknown);
    }

    #[test]
    fn parse_libc_non_glibc() {
        assert_eq!(
            parse_libc(Some("musl"), None),
            RemoteLibc::NonGlibc {
                name: "musl".to_string()
            }
        );
    }

    #[test]
    fn parse_libc_unknown_family_treated_as_unknown() {
        assert_eq!(parse_libc(Some("unknown"), None), RemoteLibc::Unknown);
        assert_eq!(parse_libc(None, None), RemoteLibc::Unknown);
        assert_eq!(parse_libc(Some(""), None), RemoteLibc::Unknown);
    }

    #[test]
    fn glibc_version_displays_as_dotted() {
        assert_eq!(format!("{}", GlibcVersion::new(2, 31)), "2.31");
    }
}
