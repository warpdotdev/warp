use std::path::PathBuf;

use warp_util::path::LineAndColumnArg;

use super::generate_editor_command;

#[test]
fn test_editor_missing_no_line_col() {
    let path = PathBuf::from("/path/to/file.txt");
    let result = generate_editor_command(&path, None, None);
    assert_eq!(result, "\"$EDITOR\" /path/to/file.txt");
}

#[test]
fn test_editor_missing_with_line_col() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 42,
        column_num: Some(10),
    });
    let result = generate_editor_command(&path, line_col, None);
    assert_eq!(result, "\"$EDITOR\" /path/to/file.txt");
}

#[test]
fn test_editor_present_no_line_col() {
    let path = PathBuf::from("/path/to/file.txt");
    let result = generate_editor_command(&path, None, Some("vim"));
    assert_eq!(result, "vim /path/to/file.txt");
}

#[test]
fn test_editor_present_line_missing() {
    let path = PathBuf::from("/path/to/file.txt");
    let result = generate_editor_command(&path, None, Some("emacs"));
    assert_eq!(result, "emacs /path/to/file.txt");
}

#[test]
fn test_vim_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 42,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("vim"));
    assert_eq!(result, "vim +42 /path/to/file.txt");
}

#[test]
fn test_vim_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 42,
        column_num: Some(10),
    });
    let result = generate_editor_command(&path, line_col, Some("vim"));
    assert_eq!(result, "vim +42:10 /path/to/file.txt");
}

#[test]
fn test_neovim_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 100,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("nvim"));
    assert_eq!(result, "nvim +100 /path/to/file.txt");
}

#[test]
fn test_neovim_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 100,
        column_num: Some(25),
    });
    let result = generate_editor_command(&path, line_col, Some("nvim"));
    assert_eq!(result, "nvim +100:25 /path/to/file.txt");
}

#[test]
fn test_emacs_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 15,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("emacs"));
    assert_eq!(result, "emacs +15 /path/to/file.txt");
}

#[test]
fn test_emacs_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 15,
        column_num: Some(5),
    });
    let result = generate_editor_command(&path, line_col, Some("emacs"));
    assert_eq!(result, "emacs +15:5 /path/to/file.txt");
}

#[test]
fn test_nano_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 20,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("nano"));
    assert_eq!(result, "nano +20 /path/to/file.txt");
}

#[test]
fn test_nano_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 20,
        column_num: Some(8),
    });
    let result = generate_editor_command(&path, line_col, Some("nano"));
    assert_eq!(result, "nano +20,8 /path/to/file.txt");
}

#[test]
fn test_pico_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 35,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("pico"));
    assert_eq!(result, "pico +35 /path/to/file.txt");
}

#[test]
fn test_pico_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 35,
        column_num: Some(12),
    });
    let result = generate_editor_command(&path, line_col, Some("pico"));
    assert_eq!(result, "pico +35,12 /path/to/file.txt");
}

#[test]
fn test_micro_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 50,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("micro"));
    assert_eq!(result, "micro +50 /path/to/file.txt");
}

#[test]
fn test_micro_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 50,
        column_num: Some(15),
    });
    let result = generate_editor_command(&path, line_col, Some("micro"));
    assert_eq!(result, "micro +50:15 /path/to/file.txt");
}

#[test]
fn test_helix_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 75,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("hx"));
    assert_eq!(result, "hx /path/to/file.txt:75");
}

#[test]
fn test_helix_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 75,
        column_num: Some(20),
    });
    let result = generate_editor_command(&path, line_col, Some("helix"));
    assert_eq!(result, "helix /path/to/file.txt:75:20");
}

#[test]
fn test_vscode_with_line_only() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 90,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("code"));
    assert_eq!(result, "code --goto /path/to/file.txt:90");
}

#[test]
fn test_vscode_with_line_and_column() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 90,
        column_num: Some(30),
    });
    let result = generate_editor_command(&path, line_col, Some("code"));
    assert_eq!(result, "code --goto /path/to/file.txt:90:30");
}

#[test]
fn test_unknown_editor_with_line_col() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 123,
        column_num: Some(45),
    });
    let result = generate_editor_command(&path, line_col, Some("unknown-editor"));
    assert_eq!(result, "unknown-editor /path/to/file.txt");
}

#[test]
fn test_path_with_spaces() {
    let path = PathBuf::from("/path with spaces/my file.txt");
    let result = generate_editor_command(&path, None, Some("vim"));
    assert_eq!(result, "vim '/path with spaces/my file.txt'");
}

#[test]
fn test_path_with_special_characters() {
    let path = PathBuf::from("/path/with$pecial&chars.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 1,
        column_num: Some(1),
    });
    let result = generate_editor_command(&path, line_col, Some("emacs"));
    assert_eq!(result, "emacs +1:1 '/path/with$pecial&chars.txt'");
}

#[test]
fn test_editor_with_path() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 10,
        column_num: None,
    });
    let result = generate_editor_command(&path, line_col, Some("/usr/bin/vim"));
    assert_eq!(result, "/usr/bin/vim +10 /path/to/file.txt");
}

#[test]
fn test_case_insensitive_editor_matching() {
    let path = PathBuf::from("/path/to/file.txt");
    let line_col = Some(LineAndColumnArg {
        line_num: 33,
        column_num: Some(7),
    });

    // Test uppercase
    let result = generate_editor_command(&path, line_col, Some("VIM"));
    assert_eq!(result, "VIM +33:7 /path/to/file.txt");

    // Test mixed case
    let result = generate_editor_command(&path, line_col, Some("Emacs"));
    assert_eq!(result, "Emacs +33:7 /path/to/file.txt");
}

#[test]
fn test_editor_try_from_supported_editors() {
    use super::Editor;

    // Test VSCode variants
    assert_eq!(Editor::try_from("code"), Ok(Editor::VSCode));
    assert_eq!(
        Editor::try_from("code-insiders"),
        Ok(Editor::VSCodeInsiders)
    );

    // Test Zed variants
    assert_eq!(Editor::try_from("zed"), Ok(Editor::Zed));
    assert_eq!(Editor::try_from("zed-preview"), Ok(Editor::ZedPreview));

    // Test other popular editors
    assert_eq!(Editor::try_from("cursor"), Ok(Editor::Cursor));
    assert_eq!(Editor::try_from("windsurf"), Ok(Editor::Windsurf));
    assert_eq!(Editor::try_from("clion"), Ok(Editor::CLion));

    // Test with paths
    assert_eq!(Editor::try_from("/usr/local/bin/code"), Ok(Editor::VSCode));
    assert_eq!(
        Editor::try_from("/Applications/Zed.app/Contents/MacOS/zed"),
        Ok(Editor::Zed)
    );

    // Test case insensitivity
    assert_eq!(Editor::try_from("CODE"), Ok(Editor::VSCode));
    assert_eq!(Editor::try_from("Zed"), Ok(Editor::Zed));
}

#[test]
fn test_editor_try_from_unsupported_editors() {
    use super::Editor;

    // Test unsupported terminal editors
    assert!(Editor::try_from("vim").is_err());
    assert!(Editor::try_from("emacs").is_err());
    assert!(Editor::try_from("nano").is_err());
    assert!(Editor::try_from("unknown-editor").is_err());
}
