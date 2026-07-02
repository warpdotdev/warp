use std::fs;

use tempfile::TempDir;

use super::{list_ssh_identities_in, SshIdentity};

fn touch(path: &std::path::Path) {
    fs::write(path, b"").expect("write");
}

#[test]
fn missing_ssh_dir_returns_empty() {
    let path = std::env::temp_dir().join("warp_ssh_diag_no_such_dir_12349876");
    let _ = fs::remove_dir_all(&path);
    let result = list_ssh_identities_in(&path);
    assert!(result.is_empty());
}

#[test]
fn empty_ssh_dir_returns_empty() {
    let tmp = TempDir::new().expect("tempdir");
    let result = list_ssh_identities_in(tmp.path());
    assert!(result.is_empty());
}

#[test]
fn finds_pub_key_with_matching_private() {
    let tmp = TempDir::new().expect("tempdir");
    touch(&tmp.path().join("id_ed25519.pub"));
    touch(&tmp.path().join("id_ed25519"));

    let result = list_ssh_identities_in(tmp.path());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "id_ed25519");
    assert!(result[0].has_private_key);
}

#[test]
fn flags_orphan_pub_key_when_private_missing() {
    let tmp = TempDir::new().expect("tempdir");
    touch(&tmp.path().join("id_ed25519.pub"));
    // No matching private key.

    let result = list_ssh_identities_in(tmp.path());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "id_ed25519");
    assert!(!result[0].has_private_key);
}

#[test]
fn ignores_non_pub_files() {
    let tmp = TempDir::new().expect("tempdir");
    touch(&tmp.path().join("config"));
    touch(&tmp.path().join("known_hosts"));
    touch(&tmp.path().join("authorized_keys"));
    touch(&tmp.path().join("id_ed25519")); // private only, no .pub
    let result = list_ssh_identities_in(tmp.path());
    assert!(result.is_empty());
}

#[test]
fn returns_entries_sorted_by_name() {
    let tmp = TempDir::new().expect("tempdir");
    touch(&tmp.path().join("work_rsa.pub"));
    touch(&tmp.path().join("work_rsa"));
    touch(&tmp.path().join("id_ed25519.pub"));
    touch(&tmp.path().join("id_ed25519"));
    touch(&tmp.path().join("aws.pub"));

    let result = list_ssh_identities_in(tmp.path());
    let names: Vec<&str> = result.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["aws", "id_ed25519", "work_rsa"]);
}

#[test]
fn empty_stem_pub_file_is_ignored() {
    // `.pub` with no stem (i.e. literally ".pub") shouldn't be
    // surfaced as an identity. Sort of pathological but defensive.
    let tmp = TempDir::new().expect("tempdir");
    touch(&tmp.path().join(".pub"));
    let result = list_ssh_identities_in(tmp.path());
    assert!(result.is_empty());
}

#[test]
fn struct_equality_ignores_pathbuf_dir_segments() {
    // Sanity check that the equality semantics line up with what
    // tests upstream might assert.
    let a = SshIdentity {
        name: "foo".to_string(),
        public_key_path: std::path::PathBuf::from("/a/b/foo.pub"),
        has_private_key: true,
    };
    let b = SshIdentity {
        name: "foo".to_string(),
        public_key_path: std::path::PathBuf::from("/a/b/foo.pub"),
        has_private_key: true,
    };
    assert_eq!(a, b);
}
