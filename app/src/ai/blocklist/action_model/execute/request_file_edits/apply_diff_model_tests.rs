use super::*;

#[test]
fn revision_paths_include_v4a_rename_destination() {
    let source = "/tmp/source.rs".to_string();
    let target = "/tmp/target.rs".to_string();
    let edits = [FileEdit::Edit(ParsedDiff::V4AEdit {
        file: Some(source.clone()),
        move_to: Some(target.clone()),
        hunks: vec![],
    })];

    let paths = absolute_edit_paths(&edits, &SessionContext::new_for_test());

    assert!(paths.contains(&source));
    assert!(paths.contains(&target));
}

#[test]
fn persisted_revision_uses_exact_candidate_content_not_a_disk_reread() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("post-save-race.rs");
    std::fs::write(&path, "other writer\n").unwrap();
    let path = path.to_string_lossy().to_string();
    let files = vec![FileSnapshot {
        updated: Some(crate::ai::blocklist::diff_storage::UpdatedFileState {
            path: path.clone(),
            changed_lines: std::iter::once(1..2).collect(),
            final_content: "agent content\n".to_string(),
            was_edited: false,
        }),
        deleted_paths: Vec::new(),
        diff_base: String::new(),
        diff_new: "agent content\n".to_string(),
        diff_name: path.clone(),
    }];

    let revisions = revisions_from_persisted_files(&files);

    assert_eq!(
        revisions,
        vec![(
            path,
            super::super::super::file_revisions::FileRevision::present("agent content\n", None,),
        )]
    );
}
