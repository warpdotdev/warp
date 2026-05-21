use super::*;

#[test]
fn test_create_version() {
    ContentVersion::new();
}

#[test]
fn test_versions_equal() {
    let version1 = ContentVersion::new();
    let version2 = version1;

    assert_eq!(version1, version2);
}

#[test]
fn test_versions_not_equal() {
    let version1 = ContentVersion::new();
    let version2 = ContentVersion::new();

    assert_ne!(version1, version2);
}

#[test]
fn test_from_raw_roundtrip() {
    let version = ContentVersion::from_raw(42);
    assert_eq!(version.as_u64(), 42);
}

#[test]
fn test_from_raw_preserves_equality() {
    let a = ContentVersion::from_raw(7);
    let b = ContentVersion::from_raw(7);
    assert_eq!(a, b);
}

#[test]
fn test_as_u64_matches_as_i32() {
    let version = ContentVersion::from_raw(100);
    assert_eq!(version.as_u64(), 100);
    assert_eq!(version.as_i32(), 100);
}
