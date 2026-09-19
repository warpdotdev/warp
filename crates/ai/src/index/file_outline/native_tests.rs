use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use repo_metadata::TargetFile;
use tempfile::TempDir;

use super::*;

fn create_test_file(dir: &TempDir, filename: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(filename);
    let mut file = File::create(&file_path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file_path
}

#[test]
fn parse_comments_respects_line_and_utf8_byte_limits() {
    let temp_dir = TempDir::new().unwrap();
    let long_comment = format!("/// {}", "界".repeat(200));
    let content = format!(
        "{long_comment}\nfn byte_limited() {{}}\n\n// one\n// two\n// three\n// four\n// five\n// six\n// seven\n// eight\n// nine\nfn line_limited() {{}}\n"
    );
    let file_path = create_test_file(&temp_dir, "comments.rs", &content);

    let outline = parse_file_outline(&file_path).unwrap();
    let symbols = outline.symbols.unwrap();

    assert_eq!(symbols[0].comment.as_ref().unwrap()[0].len(), 511);
    assert!(symbols[0].comment.as_ref().unwrap()[0].ends_with('界'));
    assert_eq!(symbols[1].comment.as_ref().unwrap().len(), 8);
    assert_eq!(
        symbols[1].comment.as_ref().unwrap().last().unwrap(),
        "// eight"
    );
}

#[test]
fn initial_outline_stops_retaining_files_at_the_byte_budget() {
    let temp_dir = TempDir::new().unwrap();
    let first_path = create_test_file(&temp_dir, "a.rs", "fn retained_symbol() {}\n");
    let second_path = create_test_file(&temp_dir, "b.rs", "fn omitted_symbol() {}\n");
    let first_outline = parse_file_outline(&first_path).unwrap();
    let second_outline = parse_file_outline(&second_path).unwrap();
    let byte_budget = retained_file_outline_bytes(&first_outline);
    let first_file_id = FileMetadata::new(first_path, false).file_id;
    let second_file_id = FileMetadata::new(second_path, false).file_id;
    let (retained, retained_bytes) = retain_file_outlines(
        vec![
            (first_file_id, first_outline),
            (second_file_id, second_outline),
        ],
        byte_budget,
    );

    assert_eq!(retained_bytes, byte_budget);
    assert_eq!(retained.len(), 1);
    assert!(retained.contains_key(&first_file_id));
    assert!(!retained.contains_key(&second_file_id));
}

#[tokio::test]
async fn incremental_update_does_not_insert_files_after_the_byte_budget() {
    let temp_dir = TempDir::new().unwrap();
    let mut outline = build_outline(temp_dir.path(), None).await.unwrap();
    outline.retained_outline_bytes = MAX_OUTLINE_TOTAL_BYTES;
    let added_path = create_test_file(&temp_dir, "added.rs", "fn omitted_symbol() {}\n");
    let update = RepositoryUpdate {
        added: [TargetFile::new(added_path.clone(), false)].into(),
        ..Default::default()
    };
    outline.update(update).await;

    assert_eq!(outline.retained_outline_bytes, MAX_OUTLINE_TOTAL_BYTES);
    assert!(outline.to_file_symbols(None).is_empty());
    assert!(!outline.to_symbols_by_file(None).contains_key(&added_path));
}
#[tokio::test]
async fn rejected_new_file_does_not_evict_a_later_modified_file() {
    let temp_dir = TempDir::new().unwrap();
    let modified_path = create_test_file(&temp_dir, "z.rs", "fn old_symbol() {}\n");
    let mut outline = build_outline(temp_dir.path(), None).await.unwrap();
    outline.retained_outline_bytes = MAX_OUTLINE_TOTAL_BYTES;
    let added_path = create_test_file(&temp_dir, "a.rs", "fn added_symbol() {}\n");
    std::fs::write(&modified_path, "fn new_symbol() {}\n").unwrap();
    let update = RepositoryUpdate {
        added: [TargetFile::new(added_path.clone(), false)].into(),
        modified: [TargetFile::new(modified_path.clone(), false)].into(),
        ..Default::default()
    };

    outline.update(update).await;
    let symbols_by_file = outline.to_symbols_by_file(None);

    assert!(
        !outline
            .to_file_symbols(None)
            .iter()
            .any(|file| file.path == "a.rs")
    );
    assert!(!symbols_by_file.contains_key(&added_path));
    assert_eq!(
        symbols_by_file[&modified_path].symbols().unwrap()[0].name,
        "new_symbol"
    );
}
#[test]
fn multiline_block_comments_count_each_physical_line() {
    let temp_dir = TempDir::new().unwrap();
    let content = "/** one\n * two\n * three\n * four\n * five\n * six\n * seven\n * eight\n * nine\n */\nfn documented() {}\n";
    let file_path = create_test_file(&temp_dir, "block_comment.rs", content);

    let outline = parse_file_outline(&file_path).unwrap();
    let comments = outline.symbols.unwrap()[0].comment.clone().unwrap();

    assert_eq!(comments.len(), MAX_SYMBOL_COMMENT_LINES);
    assert!(comments.last().unwrap().contains("eight"));
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
