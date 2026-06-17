use crate::host_id::HostId;
use crate::local_or_remote_path::LocalOrRemotePath;
use crate::remote_path::RemotePath;
use crate::standardized_path::StandardizedPath;

fn test_host_id() -> HostId {
    HostId::new("test-host".to_string())
}

fn local_repo_path() -> std::path::PathBuf {
    #[cfg(unix)]
    let path = "/repo";
    #[cfg(windows)]
    let path = r"C:\repo";

    path.into()
}

fn local_file_path() -> std::path::PathBuf {
    local_repo_path().join("file.txt")
}

fn local_absolute_file_path() -> std::path::PathBuf {
    #[cfg(unix)]
    let path = "/server/repo/src/foo.rs";
    #[cfg(windows)]
    let path = r"C:\server\repo\src\foo.rs";

    path.into()
}

#[test]
fn local_or_remote_path_helpers_return_local_path_components() {
    let local_file = local_file_path();
    let path = LocalOrRemotePath::Local(local_file.clone());

    assert_eq!(path.display_name(), "file.txt");
    assert_eq!(
        path.path_component(),
        StandardizedPath::try_from_local(&local_file).unwrap()
    );
    assert_eq!(
        path.display_path(),
        local_file.to_string_lossy().into_owned()
    );
    assert_eq!(path.to_local_path(), Some(local_file.as_path()));
}

#[test]
fn local_or_remote_path_helpers_return_remote_path_components() {
    let path = LocalOrRemotePath::Remote(RemotePath::new(
        test_host_id(),
        StandardizedPath::try_new("/tmp/repo/file.txt").unwrap(),
    ));

    assert_eq!(path.display_name(), "file.txt");
    assert_eq!(
        path.path_component(),
        StandardizedPath::try_new("/tmp/repo/file.txt").unwrap()
    );
    assert_eq!(path.display_path(), "/tmp/repo/file.txt");
    assert_eq!(path.to_local_path(), None);
}

#[test]
fn local_or_remote_path_is_local_and_is_remote_classify_variants() {
    let local = LocalOrRemotePath::Local(local_repo_path());
    let remote = LocalOrRemotePath::Remote(RemotePath::new(
        test_host_id(),
        StandardizedPath::try_new("/tmp/repo").unwrap(),
    ));

    assert!(local.is_local());
    assert!(!local.is_remote());
    assert!(!remote.is_local());
    assert!(remote.is_remote());
}

#[test]
fn local_or_remote_path_join_preserves_host_for_remote() {
    let local_repo = local_repo_path();
    let local = LocalOrRemotePath::Local(local_repo.clone());
    let remote = LocalOrRemotePath::Remote(RemotePath::new(
        test_host_id(),
        StandardizedPath::try_new("/repo").unwrap(),
    ));

    let local_joined = local.join("src/foo.rs");
    let expected_local_joined = local_repo.join("src/foo.rs");
    assert_eq!(
        local_joined.path_component(),
        StandardizedPath::try_from_local(&expected_local_joined).unwrap()
    );
    assert!(local_joined.is_local());

    let remote_joined = remote.join("src/foo.rs");
    assert_eq!(
        remote_joined.path_component(),
        StandardizedPath::try_new("/repo/src/foo.rs").unwrap()
    );
    assert!(remote_joined.is_remote());
    if let LocalOrRemotePath::Remote(remote) = remote_joined {
        assert_eq!(remote.host_id, test_host_id());
    } else {
        panic!("expected remote variant");
    }
}

#[test]
fn local_or_remote_path_join_with_absolute_replaces_prefix() {
    let local = LocalOrRemotePath::Local(local_repo_path());
    let remote = LocalOrRemotePath::Remote(RemotePath::new(
        test_host_id(),
        StandardizedPath::try_new("/some/repo").unwrap(),
    ));
    let local_abs = local_absolute_file_path();
    let local_abs_str = local_abs.to_string_lossy().into_owned();
    let remote_abs = "/server/repo/src/foo.rs";

    // Path::join replacement semantics on absolute argument.
    let local_joined = local.join(&local_abs_str);
    assert_eq!(
        local_joined.path_component(),
        StandardizedPath::try_from_local(&local_abs).unwrap()
    );

    let remote_joined = remote.join(remote_abs);
    assert_eq!(
        remote_joined.path_component(),
        StandardizedPath::try_new("/server/repo/src/foo.rs").unwrap()
    );
}

#[test]
fn local_or_remote_path_strip_repo_prefix_local_local() {
    let repo = LocalOrRemotePath::Local(local_repo_path());
    let inside = LocalOrRemotePath::Local(local_repo_path().join("src").join("foo.rs"));
    let outside = LocalOrRemotePath::Local(local_absolute_file_path());
    let expected_relative = std::path::Path::new("src")
        .join("foo.rs")
        .to_string_lossy()
        .into_owned();

    assert_eq!(repo.strip_repo_prefix(&inside), Some(expected_relative));
    assert_eq!(repo.strip_repo_prefix(&outside), None);
}

#[test]
fn local_or_remote_path_strip_repo_prefix_requires_same_host() {
    let host_a = HostId::new("host-a".to_string());
    let host_b = HostId::new("host-b".to_string());
    let repo_a = LocalOrRemotePath::Remote(RemotePath::new(
        host_a.clone(),
        StandardizedPath::try_new("/repo").unwrap(),
    ));
    let file_a = LocalOrRemotePath::Remote(RemotePath::new(
        host_a,
        StandardizedPath::try_new("/repo/src/foo.rs").unwrap(),
    ));
    let file_b = LocalOrRemotePath::Remote(RemotePath::new(
        host_b,
        StandardizedPath::try_new("/repo/src/foo.rs").unwrap(),
    ));
    let file_local = LocalOrRemotePath::Local(local_repo_path().join("src/foo.rs"));

    assert_eq!(
        repo_a.strip_repo_prefix(&file_a),
        Some("src/foo.rs".to_string()),
    );
    assert_eq!(
        repo_a.strip_repo_prefix(&file_b),
        None,
        "cross-host strip should be rejected"
    );
    assert_eq!(
        repo_a.strip_repo_prefix(&file_local),
        None,
        "local-vs-remote strip should be rejected"
    );
}

#[test]
fn local_or_remote_path_strip_repo_prefix_rejects_sibling_dirs() {
    // A remote file under a *sibling* directory whose name merely shares a
    // string prefix with the repo (`/repository` vs `/repo`) is not inside
    // the repo and must not be stripped. This must match the `Local` arm,
    // which already rejects it via component-aware `Path::strip_prefix`.
    let host = HostId::new("host".to_string());
    let repo = LocalOrRemotePath::Remote(RemotePath::new(
        host.clone(),
        StandardizedPath::try_new("/repo").unwrap(),
    ));
    let sibling = LocalOrRemotePath::Remote(RemotePath::new(
        host,
        StandardizedPath::try_new("/repository/src/foo.rs").unwrap(),
    ));

    assert_eq!(
        repo.strip_repo_prefix(&sibling),
        None,
        "sibling dir sharing a string prefix is not inside the repo"
    );
}
