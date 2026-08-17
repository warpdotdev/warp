use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use futures::executor::block_on;
use repo_metadata::{RepositoryUpdate, TargetFile};
use tempfile::TempDir;

use super::*;

fn create_test_file(dir: &TempDir, filename: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(filename);
    let mut file = File::create(&file_path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file_path
}

/// `TargetFile::is_ignored` is only computed against a repo's root and global gitignores (see
/// `Repository::check_gitignore_status`), so it can't see exclusions from a nested `.gitignore`.
/// This is the real watcher regression: a `subdir/.gitignore` already exists when the outline is
/// built, then a file matching it is added afterward and tagged `is_ignored: false`, exactly as
/// `Repository::check_gitignore_status` would tag it.
#[test]
fn update_excludes_files_under_nested_gitignore() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    fs::create_dir(repo_path.join("subdir")).unwrap();
    fs::write(repo_path.join("subdir/.gitignore"), "ignored.txt\n").unwrap();

    let mut outline = block_on(build_outline(repo_path, None)).unwrap();

    let ignored_path = repo_path.join("subdir/ignored.txt");
    let kept_path = repo_path.join("kept.rs");
    fs::write(&ignored_path, "secret").unwrap();
    fs::write(&kept_path, "fn kept() {}\n").unwrap();

    let mut update = RepositoryUpdate::default();
    update
        .added
        .insert(TargetFile::new(ignored_path.clone(), false));
    update
        .added
        .insert(TargetFile::new(kept_path.clone(), false));

    block_on(outline.update(update));

    let files = outline.to_symbols_by_file(None);
    assert!(
        files.contains_key(&kept_path),
        "non-ignored file should be added: {:?}",
        files.keys().collect::<Vec<_>>()
    );
    assert!(
        !files.contains_key(&ignored_path),
        "file under a nested .gitignore must stay excluded: {:?}",
        files.keys().collect::<Vec<_>>()
    );
}

/// `find_or_insert_path_to_file_tree` also guards against `.git`-internal paths directly. In
/// practice the watcher never produces a `TargetFile` for one of these (it routes them through
/// `record_git_internal_path_update` instead), so this feeds one in synthetically to exercise the
/// guard on its own.
#[test]
fn find_or_insert_path_to_file_tree_excludes_git_internal_paths() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();
    fs::create_dir(repo_path.join(".git")).unwrap();

    let mut outline = block_on(build_outline(repo_path, None)).unwrap();

    let git_internal_path = repo_path.join(".git/COMMIT_EDITMSG");
    fs::write(&git_internal_path, "message").unwrap();

    let mut update = RepositoryUpdate::default();
    update
        .added
        .insert(TargetFile::new(git_internal_path.clone(), false));

    block_on(outline.update(update));

    let files = outline.to_symbols_by_file(None);
    assert!(
        !files.contains_key(&git_internal_path),
        ".git-internal file must stay excluded: {:?}",
        files.keys().collect::<Vec<_>>()
    );
}

#[test]
fn test_parse_comments() {
    let temp_dir = TempDir::new().unwrap();
    let content = r#"
/// This is a struct for NewFunc
struct NewFunc {
a: str,
}

// Hello
// World
fn first_function() {
println!("First");
}

impl NewFunc {
fn second_function() {
    println!("Second");
}
}
"#;
    let file_path = create_test_file(&temp_dir, "multiple.rs", content);

    let outline = parse_file_outline(&file_path).unwrap();
    let symbols = outline.symbols.unwrap();
    assert_eq!(symbols[0].name, "NewFunc");
    assert_eq!(symbols[0].type_prefix, Some("struct".to_owned()));
    assert_eq!(
        symbols[0].comment,
        Some(vec!["/// This is a struct for NewFunc".to_owned()])
    );
    assert_eq!(symbols[0].line_number, 3); // struct NewFunc is on line 3
    assert_eq!(symbols[1].name, "first_function");
    assert_eq!(symbols[1].type_prefix, Some("fn".to_owned()));
    assert_eq!(symbols[1].line_number, 9); // first_function is on line 9
    assert_eq!(symbols[2].name, "second_function");
    assert_eq!(symbols[2].type_prefix, Some("fn".to_owned()));
    assert_eq!(symbols[2].line_number, 14); // second_function is on line 14
}

#[test]
fn test_parse_multiple_languages() {
    let temp_dir = TempDir::new().unwrap();
    let content = r#"
struct NewFunc {
a: str,
}

fn first_function() {
println!("First");
}

impl NewFunc {
fn second_function() {
    println!("Second");
}
}
"#;
    let file_path = create_test_file(&temp_dir, "multiple.rs", content);

    let outline = parse_file_outline(&file_path).unwrap();
    let symbols = outline.symbols.unwrap();
    assert_eq!(symbols.len(), 3);
    assert_eq!(symbols[0].name, "NewFunc");
    assert_eq!(symbols[0].type_prefix, Some("struct".to_owned()));
    assert_eq!(symbols[1].name, "first_function");
    assert_eq!(symbols[1].type_prefix, Some("fn".to_owned()));
    assert_eq!(symbols[2].name, "second_function");
    assert_eq!(symbols[2].type_prefix, Some("fn".to_owned()));

    // Test parsing Python code with multiple symbol definitions
    // This verifies parsing of:
    // - Regular function definitions (def keyword)
    // - Class definitions (class keyword)
    // - Method definitions within a class (def keyword)
    let python_content = r#"
def first_function():
print("First")

class TestClass:
def __init__(self):
    pass

def class_method(self):
    print("Method")

def second_function():
print("Second")
"#;
    let file_path = create_test_file(&temp_dir, "multiple.py", python_content);
    let outline = parse_file_outline(&file_path).unwrap();
    let symbols = outline.symbols.unwrap();
    assert_eq!(symbols.len(), 5);
    assert_eq!(symbols[0].name, "first_function");
    assert_eq!(symbols[0].type_prefix, Some("def".to_owned()));
    assert_eq!(symbols[1].name, "TestClass");
    assert_eq!(symbols[1].type_prefix, Some("class".to_owned()));
    assert_eq!(symbols[2].name, "__init__");
    assert_eq!(symbols[2].type_prefix, Some("def".to_owned()));
    assert_eq!(symbols[3].name, "class_method");
    assert_eq!(symbols[3].type_prefix, Some("def".to_owned()));
    assert_eq!(symbols[4].name, "second_function");
    assert_eq!(symbols[4].type_prefix, Some("def".to_owned()));

    // Test parsing JavaScript code with multiple symbol definitions
    // This verifies parsing of:
    // - Function declarations
    // - Class declarations
    // - Method definitions
    // - Arrow functions assigned to variables
    let js_content = r#"
function regularFunction() {
console.log('Regular function');
}

class TestClass {
constructor() {
    this.value = 42;
}

classMethod() {
    return this.value;
}
}
"#;
    let file_path = create_test_file(&temp_dir, "multiple.js", js_content);
    let outline = parse_file_outline(&file_path).unwrap();
    let symbols = outline.symbols.unwrap();
    assert_eq!(symbols.len(), 4);
    assert_eq!(symbols[0].name, "regularFunction");
    assert_eq!(symbols[0].type_prefix, Some("function".to_owned()));
    assert_eq!(symbols[1].name, "TestClass");
    assert_eq!(symbols[1].type_prefix, Some("class".to_owned()));
    assert_eq!(symbols[2].name, "constructor");
    assert_eq!(symbols[2].type_prefix, None);
    assert_eq!(symbols[3].name, "classMethod");
    assert_eq!(symbols[3].type_prefix, None);

    // Test parsing Go code with multiple symbol definitions
    // This verifies parsing of:
    // - Function definitions (func keyword)
    // - Type definitions (struct, interface)
    // - Method definitions (func with receiver)
    let go_content = r#"
package main

func mainFunction() {
fmt.Println("Main function")
}

type TestStruct struct {
field string
}

func (t *TestStruct) structMethod() string {
return t.field
}

type TestInterface interface {
InterfaceMethod() string
}

func helperFunction() {
fmt.Println("Helper function")
}
"#;
    let file_path = create_test_file(&temp_dir, "multiple.go", go_content);
    let outline = parse_file_outline(&file_path).unwrap();
    let symbols = outline.symbols.unwrap();
    assert_eq!(symbols.len(), 5);
    assert_eq!(symbols[0].name, "mainFunction");
    assert_eq!(symbols[0].type_prefix, Some("func".to_owned()));
    assert_eq!(symbols[1].name, "TestStruct");
    assert_eq!(symbols[1].type_prefix, Some("type".to_owned()));
    assert_eq!(symbols[2].name, "structMethod");
    assert_eq!(symbols[2].type_prefix, Some("func".to_owned()));
    assert_eq!(symbols[3].name, "TestInterface");
    assert_eq!(symbols[3].type_prefix, Some("type".to_owned()));
    assert_eq!(symbols[4].name, "helperFunction");
    assert_eq!(symbols[4].type_prefix, Some("func".to_owned()));
}
