use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

use tempfile::TempDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};

use super::{FILE_ENV, MODE_ENV, PreparationPolicy, PreparationRepository, launch_environment};
use crate::ai::agent_sdk::driver::environment_checkout_protocol::{CheckoutBatch, CheckoutRequest};

fn request(owner: &str, repo: &str) -> CheckoutRequest {
    CheckoutRequest {
        source: RepositoryIdentity {
            code_forge: RepositoryForge::GitHub,
            repo_owner: owner.into(),
            repo_name: repo.into(),
        },
        checkout_name: repo.into(),
        head: None,
        fetch_branch_only: false,
        expected_sha: None,
        remove_origin: false,
    }
}

fn policy(root: &TempDir) -> PreparationPolicy {
    PreparationPolicy {
        task_id: "task".into(),
        execution_id: "execution".into(),
        working_dir: root.path().to_owned(),
        supports_deferred_repositories: true,
        eager: Vec::new(),
        deferred: Vec::new(),
    }
}

#[test]
fn deferred_policy_authoritatively_resolves_only_original_identities() {
    let root = TempDir::new().unwrap();
    let mut policy = policy(&root);
    let selected = request("configured", "source");
    let mut frozen = request("benchmarks", "substitute");
    frozen.checkout_name = "source".into();
    frozen.head = Some(RepositoryHeadRef::Branch("benchmark-base".into()));
    frozen.fetch_branch_only = true;
    frozen.expected_sha = Some("0123456789abcdef0123456789abcdef01234567".into());
    frozen.remove_origin = true;
    policy.deferred.push(PreparationRepository {
        source: selected.source.clone(),
        request: frozen.clone(),
    });
    let mut conflicting = selected.clone();
    conflicting.checkout_name = "unused-alternative".into();
    conflicting.head = Some(RepositoryHeadRef::Branch("caller-branch".into()));
    let mut batch = CheckoutBatch {
        working_dir: root.path().to_owned(),
        repositories: vec![conflicting],
    };
    policy.resolve(&mut batch, true, false).unwrap();
    frozen.checkout_name = "unused-alternative".into();
    assert_eq!(batch.repositories, [frozen.clone()]);
    assert!(policy.resolve(&mut batch, true, false).is_err());
    assert!(policy.resolve(&mut batch, false, false).is_err());
    batch.repositories = vec![selected];
    batch.working_dir = root.path().join("different-root");
    assert!(policy.resolve(&mut batch, true, false).is_err());
    batch.working_dir = root.path().to_owned();
    policy.deferred.clear();
    assert!(policy.resolve(&mut batch, true, false).is_err());
}

#[test]
fn eager_policy_preserves_clone_then_selective_cleanup() {
    let root = TempDir::new().unwrap();
    let mut policy = policy(&root);
    let eager = request("configured", "eager");
    let mut removed = eager.clone();
    removed.remove_origin = true;
    policy.eager.push(PreparationRepository {
        source: eager.source.clone(),
        request: removed,
    });
    let mut batch = CheckoutBatch {
        working_dir: root.path().to_owned(),
        repositories: vec![eager],
    };
    policy.resolve(&mut batch, false, false).unwrap();
    policy.resolve(&mut batch, false, true).unwrap();
    policy.eager[0].request.remove_origin = false;
    policy.resolve(&mut batch, false, false).unwrap();
    assert!(policy.resolve(&mut batch, false, true).is_err());
    batch.repositories[0].head = Some(RepositoryHeadRef::Branch("changed".into()));
    assert!(policy.resolve(&mut batch, false, false).is_err());
}

#[test]
fn execution_policy_file_is_fresh_private_and_owned_until_drop_even_when_empty() {
    let root = TempDir::new().unwrap();
    for capability in [false, true] {
        let mut policy = policy(&root);
        policy.supports_deferred_repositories = capability;
        let mut env = HashMap::from([
            (
                OsString::from(FILE_ENV),
                OsString::from("/inherited/policy"),
            ),
            (OsString::from("WARP_EXECUTION_ID"), OsString::from("stale")),
        ]);
        let first = launch_environment(Some(&policy), Some("task"), root.path(), &mut env)
            .unwrap()
            .unwrap();
        let first_path = first.path();
        assert_eq!(env.get(OsStr::new(MODE_ENV)).unwrap(), "execution");
        assert_eq!(
            env.get(OsStr::new(FILE_ENV)).unwrap(),
            first_path.as_os_str()
        );
        assert!(!first_path.starts_with(root.path()));
        let bytes = std::fs::read(&first_path).unwrap();
        let saved: PreparationPolicy = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.supports_deferred_repositories, capability);
        assert!(saved.eager.is_empty() && saved.deferred.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(first_path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let second = launch_environment(Some(&policy), Some("task"), root.path(), &mut env)
            .unwrap()
            .unwrap();
        assert_ne!(first_path, second.path());
        drop(first);
        assert!(!first_path.exists());
        assert!(second.path().exists());
        assert!(
            launch_environment(Some(&policy), Some("other-task"), root.path(), &mut env).is_err()
        );
        assert!(
            launch_environment(
                Some(&policy),
                Some("task"),
                &root.path().join("other"),
                &mut env
            )
            .is_err()
        );
        launch_environment(None, None, root.path(), &mut env).unwrap();
        for name in [FILE_ENV, "WARP_EXECUTION_ID", "OZ_RUN_ID", "WARP_RUN_ID"] {
            assert!(env.get(OsStr::new(name)).unwrap().is_empty());
        }
        assert_eq!(env.get(OsStr::new(MODE_ENV)).unwrap(), "legacy");
    }
}
