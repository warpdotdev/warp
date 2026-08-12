use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

/// Write a minimal skill folder named `name` under `dir` and return its path.
fn write_skill(dir: &Path, name: &str) -> PathBuf {
    let skill_dir = dir.join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: test skill\n---\nBody"),
    )
    .unwrap();
    skill_dir
}

#[test]
fn publish_skill_creates_symlink() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");

    let target = publish_skill(skill_root.path(), "github", &skill_dir, false)
        .unwrap()
        .unwrap();

    assert_eq!(target, skill_root.path().join("github"));
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&target).unwrap(), skill_dir);
    // The symlink resolves through to the real skill content.
    assert!(target.join("SKILL.md").is_file());
}

#[test]
fn publish_skill_uses_the_real_skill_name() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "linear");

    publish_skill(skill_root.path(), "linear", &skill_dir, false)
        .unwrap()
        .unwrap();

    // Published under the skill's own name, not some namespaced alias, so an
    // agent prompt or another skill can still reference it by name.
    assert!(skill_root.path().join("linear").exists());
}

#[test]
fn publish_skill_overrides_an_existing_symlink() {
    // A stale symlink at the target (most commonly our own, from an earlier
    // run) is always replaced outright, sandbox or not — it isn't a conflict
    // with anything that predates us.
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let old_skill_dir = write_skill(source_root.path(), "old-github");
    let new_skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    create_symlink(&old_skill_dir, &target).unwrap();

    let published = publish_skill(skill_root.path(), "github", &new_skill_dir, false)
        .unwrap()
        .unwrap();

    assert_eq!(published, target);
    assert_eq!(fs::read_link(&target).unwrap(), new_skill_dir);
}

#[test]
fn publish_skill_in_a_sandbox_replaces_a_conflicting_real_directory_and_backs_it_up() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("real-file.txt"), "do not delete me").unwrap();

    let published = publish_skill(skill_root.path(), "github", &skill_dir, true)
        .unwrap()
        .unwrap();

    // The published skill now owns the name...
    assert_eq!(published, target);
    assert!(
        fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&target).unwrap(), skill_dir);
    // ...but the real, pre-existing directory was preserved, not deleted.
    let backup = skill_root.path().join("github.backup");
    assert!(backup.is_dir());
    assert_eq!(
        fs::read_to_string(backup.join("real-file.txt")).unwrap(),
        "do not delete me"
    );
}

#[test]
fn publish_skill_in_a_sandbox_numbers_the_backup_when_one_already_exists() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    fs::create_dir_all(&target).unwrap();
    // A backup from an earlier override already occupies the first-choice name.
    fs::create_dir_all(skill_root.path().join("github.backup")).unwrap();

    publish_skill(skill_root.path(), "github", &skill_dir, true)
        .unwrap()
        .unwrap();

    assert!(skill_root.path().join("github.backup").is_dir());
    assert!(skill_root.path().join("github.backup-2").is_dir());
}

#[test]
fn publish_skill_outside_a_sandbox_leaves_a_conflicting_real_directory_untouched_and_uses_an_alternate_name()
 {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("real-file.txt"), "do not touch me").unwrap();

    let published = publish_skill(skill_root.path(), "github", &skill_dir, false)
        .unwrap()
        .unwrap();

    // The real, pre-existing directory at the real name is completely untouched:
    // still a real directory (not a symlink), with its original content, and no
    // backup was created anywhere.
    assert!(
        !fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(target.join("real-file.txt")).unwrap(),
        "do not touch me"
    );
    assert!(!skill_root.path().join("github.backup").exists());
    // The skill was published under the `warp-` alternate name instead.
    let alt_target = skill_root.path().join("warp-github");
    assert_eq!(published, alt_target);
    assert_eq!(fs::read_link(&alt_target).unwrap(), skill_dir);
}

#[test]
fn publish_skill_outside_a_sandbox_does_not_publish_when_the_alternate_name_also_conflicts() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    let alt_target = skill_root.path().join("warp-github");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("real-file.txt"), "do not touch me").unwrap();
    fs::create_dir_all(&alt_target).unwrap();
    fs::write(
        alt_target.join("other-real-file.txt"),
        "do not touch me either",
    )
    .unwrap();

    let published = publish_skill(skill_root.path(), "github", &skill_dir, false).unwrap();

    // Never fall back to replacing: nothing was published under either name.
    assert_eq!(published, None);
    // Both pre-existing real entries are completely untouched.
    assert!(
        !fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(target.join("real-file.txt")).unwrap(),
        "do not touch me"
    );
    assert!(
        !fs::symlink_metadata(&alt_target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(alt_target.join("other-real-file.txt")).unwrap(),
        "do not touch me either"
    );
}

#[test]
fn publish_skill_outside_a_sandbox_replaces_a_stale_alternate_name_symlink() {
    // A `warp-<name>` symlink from an earlier, non-sandboxed run is treated
    // the same as a symlink at the real name: it's ours, so it's safe to
    // replace outright, keeping repeated runs idempotent.
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let old_skill_dir = write_skill(source_root.path(), "old-github");
    let new_skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    let alt_target = skill_root.path().join("warp-github");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("real-file.txt"), "do not touch me").unwrap();
    create_symlink(&old_skill_dir, &alt_target).unwrap();

    let published = publish_skill(skill_root.path(), "github", &new_skill_dir, false)
        .unwrap()
        .unwrap();

    assert_eq!(published, alt_target);
    assert_eq!(fs::read_link(&alt_target).unwrap(), new_skill_dir);
    // The real conflicting directory at the real name is still untouched.
    assert!(
        !fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn publish_skill_errors_on_missing_source() {
    let skill_root = TempDir::new().unwrap();
    let missing_source = skill_root.path().join("does-not-exist");

    let result = publish_skill(skill_root.path(), "github", &missing_source, false);

    assert!(result.is_err());
    assert!(!skill_root.path().join("github").exists());
}

#[test]
fn publish_skill_dirs_prefers_most_specific_directory_on_name_collision() {
    let root = TempDir::new().unwrap();
    let specific_dir = root.path().join("agents/triage/skills");
    let general_dir = root.path().join("skills");
    fs::create_dir_all(&specific_dir).unwrap();
    fs::create_dir_all(&general_dir).unwrap();
    let specific_github = write_skill(&specific_dir, "github");
    write_skill(&general_dir, "github");
    let general_linear = write_skill(&general_dir, "linear");
    let skill_root = TempDir::new().unwrap();

    let published = publish_skill_dirs(skill_root.path(), &[specific_dir, general_dir], false);

    assert_eq!(published, 2);
    assert_eq!(
        fs::read_link(skill_root.path().join("github")).unwrap(),
        specific_github
    );
    assert_eq!(
        fs::read_link(skill_root.path().join("linear")).unwrap(),
        general_linear
    );
}

#[test]
fn publish_skill_dirs_in_a_sandbox_overrides_an_existing_environment_skill_and_preserves_it() {
    let root = TempDir::new().unwrap();
    let source_dir = root.path().join("skills");
    fs::create_dir_all(&source_dir).unwrap();
    let published_github = write_skill(&source_dir, "github");
    let skill_root = TempDir::new().unwrap();
    // Simulate a real, pre-existing "github" skill already installed in the
    // harness's environment before this publish runs.
    let existing_target = skill_root.path().join("github");
    fs::create_dir_all(&existing_target).unwrap();
    fs::write(existing_target.join("SKILL.md"), "pre-existing skill").unwrap();

    let published = publish_skill_dirs(skill_root.path(), &[source_dir], true);

    assert_eq!(published, 1);
    // The published skill wins under the real name...
    assert_eq!(
        fs::read_link(skill_root.path().join("github")).unwrap(),
        published_github
    );
    // ...and the pre-existing skill was moved aside, not deleted.
    let backup = skill_root.path().join("github.backup");
    assert_eq!(
        fs::read_to_string(backup.join("SKILL.md")).unwrap(),
        "pre-existing skill"
    );
}

#[test]
fn publish_skill_dirs_skips_entries_without_skill_md() {
    let root = TempDir::new().unwrap();
    let source_dir = root.path().join("skills");
    fs::create_dir_all(source_dir.join("not-a-skill")).unwrap();
    write_skill(&source_dir, "github");
    let skill_root = TempDir::new().unwrap();

    let published = publish_skill_dirs(skill_root.path(), &[source_dir], false);

    assert_eq!(published, 1);
    assert!(skill_root.path().join("github").exists());
    assert!(!skill_root.path().join("not-a-skill").exists());
}

#[test]
fn publish_skill_dirs_is_a_noop_for_empty_source_dirs() {
    let outer = TempDir::new().unwrap();
    let skill_root = outer.path().join("skills");

    assert_eq!(publish_skill_dirs(&skill_root, &[], false), 0);
    // Doesn't even create the skill root when there's nothing to publish.
    assert!(!skill_root.exists());
}

#[test]
fn publish_skill_dirs_recovers_from_missing_source_directory() {
    let root = TempDir::new().unwrap();
    let missing_dir = root.path().join("does-not-exist/skills");
    let present_dir = root.path().join("skills");
    fs::create_dir_all(&present_dir).unwrap();
    write_skill(&present_dir, "github");
    let skill_root = TempDir::new().unwrap();

    let published = publish_skill_dirs(skill_root.path(), &[missing_dir, present_dir], false);

    assert_eq!(published, 1);
    assert!(skill_root.path().join("github").exists());
}

#[test]
fn is_sandbox_reflects_the_is_sandbox_env_var() {
    let mut env_vars = HashMap::new();
    assert!(!is_sandbox(&env_vars));

    env_vars.insert(OsString::from("IS_SANDBOX"), OsString::from("1"));
    assert!(is_sandbox(&env_vars));
}
