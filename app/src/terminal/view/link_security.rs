//! Centralized scheme allow-list for opening URIs sourced from terminal output.
//!
//! Terminal output is untrusted (a remote shell, a TUI app, anyone) and
//! `ctx.open_url(...)` is a thin wrapper that hands the URI to the platform —
//! it is **not** itself a validator. Every code path that opens a URI coming
//! from terminal output must call [`check_open_scheme`] first and respect the
//! result. See `specs/GH6393/tech.md` §5a.
//!
//! The validator is parameterized by [`LinkSource`] so OSC 8 (the URI is
//! attacker-chosen and decoupled from the visible text) can be conservative
//! without regressing the existing auto-detected URL behavior, where the
//! URI is always something the user could already see and copy by hand.
//!
//! Layer 5a is **not** gated by `FeatureFlag::OscHyperlinks` — it's a
//! hardening change that benefits the existing auto-detected URL flow as
//! well, and disabling OSC 8 must not regress security.

use url::Url;

/// Where the URI came from. Determines which allow-list applies. See
/// product invariant 16.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkSource {
    /// URI was emitted by the program via OSC 8. The URI is decoupled from
    /// visible text and entirely attacker-controlled — strict allow-list.
    OscHyperlink,
    /// URI was extracted from the visible cell text by the existing
    /// `urlocator`-based scanner. The user could already see and copy it
    /// by hand. Allow-list mirrors what `urlocator` emits, so the new
    /// gate is a no-op on the happy path.
    AutoDetected,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SchemeCheck {
    Allowed,
    Rejected { reason: SchemeRejectReason },
}

#[derive(Debug, Eq, PartialEq)]
pub enum SchemeRejectReason {
    /// URI failed to parse as a URL (spaces, no scheme, garbage, etc.).
    Unparseable,
    /// URI parsed but the scheme isn't on the allow-list for the source.
    DisallowedScheme { scheme: String },
}

/// Strict allow-list for OSC 8 URIs. See `product.md` invariant 16.
const OSC8_ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto", "ftp"];

/// Allow-list for URIs produced by the existing `urlocator`-based
/// auto-detector. Mirrors what `urlocator` 0.1.4 emits today (see
/// `urlocator-0.1.4/src/scheme.rs` — HTTP, HTTPS, FTP, FILE, MAILTO,
/// NEWS, GEMINI, GIT, GOPHER, SSH). Locking this down as a `const`
/// rather than asking `urlocator` at runtime catches drift if the
/// upstream scanner expands what it emits — the `LinkRejectedScheme`
/// telemetry will fire on the new schemes and signal that this list
/// needs updating.
const AUTO_DETECTED_ALLOWED_SCHEMES: &[&str] = &[
    "http", "https", "ftp", "file", "mailto", "news", "gemini", "git", "gopher", "ssh",
];

/// Returns [`SchemeCheck::Allowed`] iff `uri` parses as a URL whose scheme
/// is in the allow-list applicable to `source`. Called by every code path
/// that opens a URI coming from terminal output (OSC 8 hyperlinks and
/// auto-detected URLs).
pub fn check_open_scheme(uri: &str, source: LinkSource) -> SchemeCheck {
    let parsed = match Url::parse(uri) {
        Ok(u) => u,
        Err(_) => {
            return SchemeCheck::Rejected {
                reason: SchemeRejectReason::Unparseable,
            };
        }
    };
    let scheme = parsed.scheme().to_ascii_lowercase();
    let allow_list: &[&str] = match source {
        LinkSource::OscHyperlink => OSC8_ALLOWED_SCHEMES,
        LinkSource::AutoDetected => AUTO_DETECTED_ALLOWED_SCHEMES,
    };
    if allow_list.contains(&scheme.as_str()) {
        SchemeCheck::Allowed
    } else {
        SchemeCheck::Rejected {
            reason: SchemeRejectReason::DisallowedScheme { scheme },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc8_allows_http_https_mailto_ftp() {
        for uri in [
            "http://x.example",
            "https://x.example",
            "mailto:a@b.com",
            "ftp://x.example",
        ] {
            assert_eq!(
                check_open_scheme(uri, LinkSource::OscHyperlink),
                SchemeCheck::Allowed,
                "expected {uri} to be allowed for OscHyperlink"
            );
        }
    }

    #[test]
    fn osc8_rejects_javascript_data_file_vbscript_about() {
        for (uri, scheme) in [
            ("javascript:alert(1)", "javascript"),
            ("data:text/html,<h1>hi</h1>", "data"),
            ("file:///etc/passwd", "file"),
            ("vbscript:msgbox(1)", "vbscript"),
            ("about:blank", "about"),
        ] {
            assert_eq!(
                check_open_scheme(uri, LinkSource::OscHyperlink),
                SchemeCheck::Rejected {
                    reason: SchemeRejectReason::DisallowedScheme {
                        scheme: scheme.to_owned()
                    }
                },
                "expected {uri} to be rejected for OscHyperlink"
            );
        }
    }

    #[test]
    fn auto_detected_allows_what_urlocator_emits() {
        // Anti-regression for product invariant 18: the new validation gate
        // on the auto-detected URL flow must not reject any scheme the
        // existing detector emits.
        for uri in [
            "http://x.example",
            "https://x.example",
            "ftp://x.example",
            "file:///tmp/foo",
            "mailto:a@b.com",
            "news:foo.bar",
            "gemini://x.example",
            "git://github.com/x/y",
            "gopher://x.example",
            "ssh://user@host",
        ] {
            assert_eq!(
                check_open_scheme(uri, LinkSource::AutoDetected),
                SchemeCheck::Allowed,
                "expected {uri} to be allowed for AutoDetected"
            );
        }
    }

    #[test]
    fn osc8_strict_rejects_schemes_only_allowed_for_auto_detected() {
        // file:, ssh:, git: etc. are auto-detected today (invariant 18) but
        // OSC 8 stays strict — the program could ship a `file://` link that
        // pretends to be benign.
        assert_eq!(
            check_open_scheme("file:///etc/passwd", LinkSource::OscHyperlink),
            SchemeCheck::Rejected {
                reason: SchemeRejectReason::DisallowedScheme {
                    scheme: "file".to_owned()
                }
            }
        );
        assert_eq!(
            check_open_scheme("ssh://user@host", LinkSource::OscHyperlink),
            SchemeCheck::Rejected {
                reason: SchemeRejectReason::DisallowedScheme {
                    scheme: "ssh".to_owned()
                }
            }
        );
    }

    #[test]
    fn case_insensitive_scheme_match() {
        assert_eq!(
            check_open_scheme("HTTPS://X.EXAMPLE", LinkSource::OscHyperlink),
            SchemeCheck::Allowed,
        );
        assert_eq!(
            check_open_scheme("JavaScript:alert(1)", LinkSource::OscHyperlink),
            SchemeCheck::Rejected {
                reason: SchemeRejectReason::DisallowedScheme {
                    scheme: "javascript".to_owned()
                }
            }
        );
    }

    #[test]
    fn unparseable_uris_are_rejected() {
        for uri in ["", "hello world", "://", "no-scheme-just-text"] {
            assert_eq!(
                check_open_scheme(uri, LinkSource::OscHyperlink),
                SchemeCheck::Rejected {
                    reason: SchemeRejectReason::Unparseable
                },
                "expected {uri:?} to be Rejected/Unparseable"
            );
        }
    }
}
