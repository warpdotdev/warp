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
