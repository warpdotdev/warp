use std::path::Path;

use super::*;

#[test]
fn try_new_unix_absolute() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    assert_eq!(p.as_str(), "/home/user/project");
    assert!(p.is_unix());
}

#[test]
fn try_new_windows_absolute() {
    let p = StandardizedPath::try_new("C:\\Users\\user\\project").unwrap();
    assert_eq!(p.as_str(), "C:\\Users\\user\\project");
    assert!(p.is_windows());
}

#[test]
fn try_new_normalizes_dot_segments() {
    let p = StandardizedPath::try_new("/home/user/./project/../project/src").unwrap();
    assert_eq!(p.as_str(), "/home/user/project/src");
}

#[test]
fn try_new_rejects_relative() {
    assert!(StandardizedPath::try_new("relative/path").is_err());
}

#[test]
fn try_from_local_absolute() {
    // Use a platform-appropriate absolute path.
    #[cfg(unix)]
    let (input, expected) = (Path::new("/tmp/test"), "/tmp/test");
    #[cfg(windows)]
    let (input, expected) = (Path::new("C:\\Windows"), "C:\\Windows");

    let p = StandardizedPath::try_from_local(input).unwrap();
    assert_eq!(p.as_str(), expected);
}

#[test]
fn try_from_local_rejects_relative() {
    assert!(StandardizedPath::try_from_local(Path::new("relative")).is_err());
}

#[test]
fn from_local_canonicalized_existing_path() {
    // Use a path that exists on all platforms.
    let existing = std::env::temp_dir();
    let p = StandardizedPath::from_local_canonicalized(&existing).unwrap();
    assert!(!p.as_str().is_empty());
    // Encoding should match the local OS.
    #[cfg(unix)]
    assert!(p.is_unix());
    #[cfg(windows)]
    assert!(p.is_windows());
}

#[test]
fn from_local_canonicalized_nonexistent() {
    #[cfg(unix)]
    let path = Path::new("/nonexistent_path_xyz_123");
    #[cfg(windows)]
    let path = Path::new("C:\\nonexistent_path_xyz_123");

    assert!(StandardizedPath::from_local_canonicalized(path).is_err());
}

#[test]
fn from_local_absolute_unchecked_accepts_absolute() {
    // Regression: on wasm32-unknown-unknown, std's `Path::is_absolute()` returns
    // false for a Unix-rooted path, which used to trip the debug assert here and
    // panic in debug wasm builds. The guard now checks absoluteness via the
    // encoding-aware `typed_path` path, so a genuinely-absolute path must be
    // accepted (and not panic) on every target.
    #[cfg(unix)]
    let (input, expected) = (Path::new("/Users/david/src/warp"), "/Users/david/src/warp");
    #[cfg(windows)]
    let (input, expected) = (Path::new("C:\\Users\\david\\src"), "C:\\Users\\david\\src");

    let p = StandardizedPath::from_local_absolute_unchecked(input);
    assert_eq!(p.as_str(), expected);
}

#[test]
fn from_local_absolute_unchecked_normalizes() {
    #[cfg(unix)]
    let (input, expected) = (
        Path::new("/home/user/./project/../project"),
        "/home/user/project",
    );
    #[cfg(windows)]
    let (input, expected) = (
        Path::new("C:\\home\\user\\.\\project\\..\\project"),
        "C:\\home\\user\\project",
    );

    let p = StandardizedPath::from_local_absolute_unchecked(input);
    assert_eq!(p.as_str(), expected);
}

#[test]
fn file_name() {
    let p = StandardizedPath::try_new("/home/user/file.rs").unwrap();
    assert_eq!(p.file_name(), Some("file.rs"));
}

#[test]
fn extension() {
    let p = StandardizedPath::try_new("/home/user/file.rs").unwrap();
    assert_eq!(p.extension(), Some("rs"));
}

#[test]
fn parent() {
    let p = StandardizedPath::try_new("/home/user/file.rs").unwrap();
    let parent = p.parent().unwrap();
    assert_eq!(parent.as_str(), "/home/user");
}

#[test]
fn starts_with() {
    let p = StandardizedPath::try_new("/home/user/project/src").unwrap();
    let base = StandardizedPath::try_new("/home/user/project").unwrap();
    assert!(p.starts_with(&base));
    let other = StandardizedPath::try_new("/other").unwrap();
    assert!(!p.starts_with(&other));
}

#[test]
fn strip_prefix() {
    let p = StandardizedPath::try_new("/home/user/project/src/main.rs").unwrap();
    let base = StandardizedPath::try_new("/home/user/project").unwrap();
    assert_eq!(p.strip_prefix(&base), Some("src/main.rs"));
}

#[test]
fn strip_prefix_equal_path() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    let base = StandardizedPath::try_new("/home/user/project").unwrap();
    assert_eq!(p.strip_prefix(&base), Some(""));
}

#[test]
fn strip_prefix_matches_only_whole_components() {
    // `/repository` is a string-prefix sibling of `/repo`, but it is NOT a
    // path-component descendant of it. `strip_prefix` must behave like
    // `starts_with` and refuse to strip, returning `None` rather than a
    // bogus mid-component remainder like `Some("sitory/foo.rs")`.
    let base = StandardizedPath::try_new("/repo").unwrap();
    let sibling = StandardizedPath::try_new("/repository/foo.rs").unwrap();
    assert!(!sibling.starts_with(&base));
    assert_eq!(sibling.strip_prefix(&base), None);

    let nested_base = StandardizedPath::try_new("/home/user").unwrap();
    let nested_sibling = StandardizedPath::try_new("/home/username/x").unwrap();
    assert!(!nested_sibling.starts_with(&nested_base));
    assert_eq!(nested_sibling.strip_prefix(&nested_base), None);
}

#[test]
fn join() {
    let p = StandardizedPath::try_new("/home/user").unwrap();
    let joined = p.join("project/src");
    assert_eq!(joined.as_str(), "/home/user/project/src");
}

#[test]
fn to_local_path() {
    // to_local_path returns Some only when encoding matches the OS.
    let existing = std::env::temp_dir();
    let p = StandardizedPath::from_local_canonicalized(&existing).unwrap();
    let local = p.to_local_path();
    assert!(local.is_some());
}

#[test]
#[cfg(unix)]
fn to_local_path_unix_on_unix() {
    let p = StandardizedPath::try_new("/home/user").unwrap();
    assert_eq!(p.to_local_path().unwrap(), Path::new("/home/user"));
}

#[test]
fn display() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    assert_eq!(format!("{p}"), "/home/user/project");
}

#[test]
fn serde_roundtrip() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    let json = serde_json::to_string(&p).unwrap();
    assert_eq!(json, "\"/home/user/project\"");
    let deserialized: StandardizedPath = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, p);
}

#[test]
fn equality_and_hash() {
    use std::collections::HashSet;
    let a = StandardizedPath::try_new("/home/user").unwrap();
    let b = StandardizedPath::try_new("/home/user").unwrap();
    assert_eq!(a, b);
    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}
