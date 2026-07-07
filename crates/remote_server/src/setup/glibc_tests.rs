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
