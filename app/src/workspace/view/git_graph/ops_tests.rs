//! Unit tests for the pure helpers in [`super`] (the write-operation layer):
//! [`GitWriteOp::args`], [`GitWriteOp::confirm_message`], and the small path /
//! ref helpers. IO (`run_write_op`) is exercised by the integration test, not
//! here.

use super::*;

#[test]
fn add_tag_lightweight_omits_annotation_flags() {
    let op = GitWriteOp::AddTag {
        hash: "abc123".into(),
        name: "v1.0".into(),
        message: None,
    };
    assert_eq!(op.args(), vec!["tag", "v1.0", "abc123"]);
}

#[test]
fn add_tag_annotated_uses_dash_a_and_message() {
    let op = GitWriteOp::AddTag {
        hash: "abc123".into(),
        name: "v1.0".into(),
        message: Some("first release".into()),
    };
    assert_eq!(
        op.args(),
        vec!["tag", "-a", "v1.0", "-m", "first release", "abc123"]
    );
}

#[test]
fn create_branch_passes_name_then_hash() {
    let op = GitWriteOp::CreateBranch {
        hash: "deadbeef".into(),
        name: "feature/x".into(),
    };
    assert_eq!(op.args(), vec!["branch", "feature/x", "deadbeef"]);
}

#[test]
fn revert_is_no_edit() {
    let op = GitWriteOp::Revert { hash: "h".into() };
    assert_eq!(op.args(), vec!["revert", "--no-edit", "h"]);
}

#[test]
fn drop_commit_rebases_onto_parent() {
    let op = GitWriteOp::DropCommit { hash: "h".into() };
    assert_eq!(op.args(), vec!["rebase", "--onto", "h^", "h"]);
}

#[test]
fn reset_modes_map_to_flags() {
    let cases = [
        (ResetMode::Soft, "--soft"),
        (ResetMode::Mixed, "--mixed"),
        (ResetMode::Hard, "--hard"),
    ];
    for (mode, flag) in cases {
        let op = GitWriteOp::Reset {
            hash: "h".into(),
            mode,
        };
        assert_eq!(op.args(), vec!["reset", flag, "h"]);
    }
}

#[test]
fn delete_remote_branch_uses_push_delete() {
    let op = GitWriteOp::DeleteRemoteBranch {
        remote: "origin".into(),
        branch: "feature".into(),
    };
    assert_eq!(op.args(), vec!["push", "origin", "--delete", "feature"]);
}

#[test]
fn pull_passes_remote_and_branch_separately() {
    let op = GitWriteOp::Pull {
        remote: "origin".into(),
        branch: "main".into(),
    };
    assert_eq!(op.args(), vec!["pull", "origin", "main"]);
}

#[test]
fn rename_branch_uses_branch_dash_m() {
    let op = GitWriteOp::RenameBranch {
        old: "old".into(),
        new: "new".into(),
    };
    assert_eq!(op.args(), vec!["branch", "-m", "old", "new"]);
}

#[test]
fn delete_local_branch_uses_branch_dash_d() {
    let op = GitWriteOp::DeleteLocalBranch {
        name: "feature".into(),
    };
    assert_eq!(op.args(), vec!["branch", "-d", "feature"]);
}

#[test]
fn rebase_onto_branch_passes_branch_name() {
    let op = GitWriteOp::RebaseOntoBranch {
        branch: "main".into(),
    };
    assert_eq!(op.args(), vec!["rebase", "main"]);
}

#[test]
fn delete_tag_uses_dash_d() {
    let op = GitWriteOp::DeleteTag { name: "v1".into() };
    assert_eq!(op.args(), vec!["tag", "-d", "v1"]);
}

#[test]
fn archive_includes_format_and_output() {
    let op = GitWriteOp::Archive {
        rev: "main".into(),
        output: PathBuf::from("/tmp/out.zip"),
        format: ArchiveFormat::Zip,
    };
    assert_eq!(
        op.args(),
        vec!["archive", "--format", "zip", "-o", "/tmp/out.zip", "main"]
    );
}

#[test]
fn archive_format_inferred_from_extension() {
    assert_eq!(
        archive_format_from_path(std::path::Path::new("/tmp/a.zip")),
        ArchiveFormat::Zip
    );
    assert_eq!(
        archive_format_from_path(std::path::Path::new("/tmp/a.tar.gz")),
        ArchiveFormat::TarGz
    );
    assert_eq!(
        archive_format_from_path(std::path::Path::new("/tmp/a.tgz")),
        ArchiveFormat::TarGz
    );
    // Unknown extension falls back to tar.gz.
    assert_eq!(
        archive_format_from_path(std::path::Path::new("/tmp/archive")),
        ArchiveFormat::TarGz
    );
}

#[test]
fn split_remote_ref_splits_on_first_slash() {
    assert_eq!(
        split_remote_ref("origin/feature"),
        ("origin".into(), "feature".into())
    );
    assert_eq!(
        split_remote_ref("origin/feat/x"),
        ("origin".into(), "feat/x".into())
    );
    assert_eq!(split_remote_ref("local"), (String::new(), "local".into()));
}

#[test]
fn confirm_message_present_for_destructive_absent_for_input_ops() {
    // Text-input ops gate themselves through their dialog.
    assert!(GitWriteOp::CreateBranch {
        hash: "h".into(),
        name: "b".into()
    }
    .confirm_message()
    .is_none());
    // Hard reset warns about losing uncommitted work.
    let msg = GitWriteOp::Reset {
        hash: "abcdef0".into(),
        mode: ResetMode::Hard,
    }
    .confirm_message()
    .expect("hard reset must confirm");
    assert!(msg.contains("lost"));
    // Remote deletion warns it cannot be undone.
    assert!(GitWriteOp::DeleteRemoteBranch {
        remote: "origin".into(),
        branch: "x".into()
    }
    .confirm_message()
    .unwrap()
    .contains("cannot be undone"));
}
