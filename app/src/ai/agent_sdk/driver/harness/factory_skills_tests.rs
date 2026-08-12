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
fn publish_factory_skill_creates_symlink() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");

    let target = publish_factory_skill(skill_root.path(), "github", &skill_dir).unwrap();

    assert_eq!(target, skill_root.path().join("github"));
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&target).unwrap(), skill_dir);
    // The symlink resolves through to the real skill content.
    assert!(target.join("SKILL.md").is_file());
}

#[test]
fn publish_factory_skill_publishes_under_the_real_skill_name() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "linear");

    publish_factory_skill(skill_root.path(), "linear", &skill_dir).unwrap();

    // Published under the skill's own name, not some namespaced alias, so an
    // agent prompt or another skill can still reference it by name.
    assert!(skill_root.path().join("linear").exists());
    assert!(!skill_root.path().join("factory-linear").exists());
}

#[test]
fn publish_factory_skill_overrides_an_existing_symlink() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let old_skill_dir = write_skill(source_root.path(), "old-github");
    let new_skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    create_symlink(&old_skill_dir, &target).unwrap();

    let published = publish_factory_skill(skill_root.path(), "github", &new_skill_dir).unwrap();

    assert_eq!(published, target);
    assert_eq!(fs::read_link(&target).unwrap(), new_skill_dir);
}

#[test]
fn publish_factory_skill_moves_a_conflicting_real_directory_aside_instead_of_deleting_it() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("real-file.txt"), "do not delete me").unwrap();

    let published = publish_factory_skill(skill_root.path(), "github", &skill_dir).unwrap();

    // The factory skill now owns the name...
    assert_eq!(published, target);
    assert!(
        fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&target).unwrap(), skill_dir);
    // ...but the real, pre-existing directory was preserved, not deleted.
    let backup = skill_root.path().join("github.pre-factory-backup");
    assert!(backup.is_dir());
    assert_eq!(
        fs::read_to_string(backup.join("real-file.txt")).unwrap(),
        "do not delete me"
    );
}

#[test]
fn publish_factory_skill_numbers_the_backup_when_one_already_exists() {
    let source_root = TempDir::new().unwrap();
    let skill_root = TempDir::new().unwrap();
    let skill_dir = write_skill(source_root.path(), "github");
    let target = skill_root.path().join("github");
    fs::create_dir_all(&target).unwrap();
    // A backup from an earlier override already occupies the first-choice name.
    fs::create_dir_all(skill_root.path().join("github.pre-factory-backup")).unwrap();

    publish_factory_skill(skill_root.path(), "github", &skill_dir).unwrap();

    assert!(skill_root.path().join("github.pre-factory-backup").is_dir());
    assert!(
        skill_root
            .path()
            .join("github.pre-factory-backup-2")
            .is_dir()
    );
}

#[test]
fn publish_factory_skill_errors_on_missing_source() {
    let skill_root = TempDir::new().unwrap();
    let missing_source = skill_root.path().join("does-not-exist");

    let result = publish_factory_skill(skill_root.path(), "github", &missing_source);

    assert!(result.is_err());
    assert!(!skill_root.path().join("github").exists());
}

#[test]
fn publish_factory_skills_prefers_most_specific_directory_on_name_collision() {
    let root = TempDir::new().unwrap();
    let specific_dir = root.path().join("agents/triage/skills");
    let general_dir = root.path().join("skills");
    fs::create_dir_all(&specific_dir).unwrap();
    fs::create_dir_all(&general_dir).unwrap();
    let specific_github = write_skill(&specific_dir, "github");
    write_skill(&general_dir, "github");
    let general_linear = write_skill(&general_dir, "linear");
    let skill_root = TempDir::new().unwrap();

    let published = publish_factory_skills(skill_root.path(), &[specific_dir, general_dir]);

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
fn publish_factory_skills_overrides_an_existing_environment_skill_and_preserves_it() {
    let root = TempDir::new().unwrap();
    let source_dir = root.path().join("skills");
    fs::create_dir_all(&source_dir).unwrap();
    let factory_github = write_skill(&source_dir, "github");
    let skill_root = TempDir::new().unwrap();
    // Simulate a real, pre-existing "github" skill already installed in the
    // harness's environment before the factory publish runs.
    let existing_target = skill_root.path().join("github");
    fs::create_dir_all(&existing_target).unwrap();
    fs::write(existing_target.join("SKILL.md"), "pre-existing skill").unwrap();

    let published = publish_factory_skills(skill_root.path(), &[source_dir]);

    assert_eq!(published, 1);
    // The factory skill wins under the real name...
    assert_eq!(
        fs::read_link(skill_root.path().join("github")).unwrap(),
        factory_github
    );
    // ...and the pre-existing skill was moved aside, not deleted.
    let backup = skill_root.path().join("github.pre-factory-backup");
    assert_eq!(
        fs::read_to_string(backup.join("SKILL.md")).unwrap(),
        "pre-existing skill"
    );
}

#[test]
fn publish_factory_skills_skips_entries_without_skill_md() {
    let root = TempDir::new().unwrap();
    let source_dir = root.path().join("skills");
    fs::create_dir_all(source_dir.join("not-a-skill")).unwrap();
    write_skill(&source_dir, "github");
    let skill_root = TempDir::new().unwrap();

    let published = publish_factory_skills(skill_root.path(), &[source_dir]);

    assert_eq!(published, 1);
    assert!(skill_root.path().join("github").exists());
    assert!(!skill_root.path().join("not-a-skill").exists());
}

#[test]
fn publish_factory_skills_is_a_noop_for_empty_source_dirs() {
    let outer = TempDir::new().unwrap();
    let skill_root = outer.path().join("skills");

    assert_eq!(publish_factory_skills(&skill_root, &[]), 0);
    // Doesn't even create the skill root when there's nothing to publish.
    assert!(!skill_root.exists());
}

#[test]
fn publish_factory_skills_recovers_from_missing_source_directory() {
    let root = TempDir::new().unwrap();
    let missing_dir = root.path().join("does-not-exist/skills");
    let present_dir = root.path().join("skills");
    fs::create_dir_all(&present_dir).unwrap();
    write_skill(&present_dir, "github");
    let skill_root = TempDir::new().unwrap();

    let published = publish_factory_skills(skill_root.path(), &[missing_dir, present_dir]);

    assert_eq!(published, 1);
    assert!(skill_root.path().join("github").exists());
}
