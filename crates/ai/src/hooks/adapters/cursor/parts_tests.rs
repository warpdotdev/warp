use std::ops::Range;

use super::*;
use crate::agent::action::ReadFilesRequest;
use crate::diff_validation::ParsedDiff;
use crate::hooks::adapters::cursor::test_support::shell_action;

fn read(names: &[&str]) -> AIAgentActionType {
    AIAgentActionType::ReadFiles(ReadFilesRequest {
        locations: names
            .iter()
            .map(|name| FileLocations {
                name: (*name).to_owned(),
                lines: vec![Range { start: 1, end: 10 }],
            })
            .collect(),
    })
}

fn replace(file: &str) -> FileEdit {
    FileEdit::Edit(ParsedDiff::StrReplaceEdit {
        file: Some(file.to_owned()),
        search: Some("a".to_owned()),
        replace: Some("b".to_owned()),
    })
}

fn delete(file: &str) -> FileEdit {
    FileEdit::Delete {
        file: Some(file.to_owned()),
    }
}

fn edits(file_edits: Vec<FileEdit>) -> AIAgentActionType {
    AIAgentActionType::RequestFileEdits {
        file_edits,
        title: Some("tidy".to_owned()),
    }
}

#[test]
fn a_read_is_one_part_per_file_in_the_order_requested() {
    let action = read(&["src/a.rs", "src/b.rs", "README.md"]);
    let parts = tool_parts(&action);
    assert_eq!(parts.len(), 3);
    assert_eq!(
        parts.iter().map(ToolPart::file).collect::<Vec<_>>(),
        vec![Some("src/a.rs"), Some("src/b.rs"), Some("README.md")]
    );
    assert!(parts.iter().all(|part| part.tool_name() == "Read"));

    let single = read(&["src/a.rs"]);
    let parts = tool_parts(&single);
    assert_eq!(parts.len(), 1);
    assert!(
        matches!(parts[0], ToolPart::ReadFile(_)),
        "a single-file read has the per-file shape too"
    );

    let empty = read(&[]);
    assert!(matches!(tool_parts(&empty)[0], ToolPart::Whole(_)));
}

#[test]
fn an_edit_batch_over_several_files_is_one_part_per_file_with_its_own_edits() {
    let action = edits(vec![
        replace("src/a.rs"),
        delete("old.rs"),
        replace("src/a.rs"),
        FileEdit::Create {
            file: None,
            content: None,
            allow_overwrite: false,
        },
    ]);
    let parts = tool_parts(&action);
    assert_eq!(parts.len(), 3);
    let ToolPart::EditFile { file, title, edits } = &parts[0] else {
        panic!("a per-file part");
    };
    assert_eq!(*file, Some("src/a.rs"));
    assert_eq!(*title, Some("tidy"));
    assert_eq!(edits.len(), 2, "both edits to a.rs are in its part");
    assert_eq!(parts[0].tool_name(), "Write");
    assert_eq!(parts[1].file(), Some("old.rs"));
    assert_eq!(
        parts[1].tool_name(),
        "Delete",
        "a part made only of deletes is a Delete"
    );
    assert_eq!(
        parts[2].file(),
        None,
        "edits with no file form their own part"
    );
    assert_eq!(parts[2].tool_name(), "Write");
}

#[test]
fn an_edit_batch_over_one_file_stays_whole() {
    let action = edits(vec![replace("src/a.rs"), replace("src/a.rs")]);
    let parts = tool_parts(&action);
    assert_eq!(parts.len(), 1);
    assert!(matches!(parts[0], ToolPart::Whole(_)));
    assert_eq!(parts[0].tool_name(), "Write");
    assert_eq!(parts[0].file(), None);
}

#[test]
fn every_other_action_is_one_whole_part() {
    let action = shell_action("ls");
    let parts = tool_parts(&action);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].tool_name(), "Shell");
}

#[test]
fn a_result_path_names_the_requested_file_when_it_ends_with_it() {
    assert!(names_requested_file("/work/repo/src/lib.rs", "src/lib.rs"));
    assert!(names_requested_file(
        "/work/repo/src/lib.rs",
        "./src/lib.rs"
    ));
    assert!(names_requested_file(
        "/work/repo/src/lib.rs",
        "/work/repo/src/lib.rs"
    ));
    assert!(names_requested_file("src/lib.rs", "src/lib.rs"));
    assert!(
        !names_requested_file("/work/repo/src/lib.rs", "lib.rs.bak"),
        "a suffix that is not a whole component does not match"
    );
    assert!(
        !names_requested_file("/work/repo/src/lib.rs", "other/lib.rs"),
        "the whole requested path must match"
    );
}
