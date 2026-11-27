use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use instant::Instant;
use tempfile::TempDir;

use super::*;

fn create_test_file(dir: &TempDir, filename: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(filename);
    let mut file = File::create(&file_path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file_path
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

/// Dense, deeply nested, syntactically-invalid SQL: unmatched parens force tree-sitter's
/// error-recovery machinery to repeatedly fork and merge stack versions. Tree-sitter's progress
/// callback is polled periodically from its top-level parse loop (though not from within a
/// single error-recovery pass); this input is large enough to guarantee at least one such poll,
/// which is all the already-elapsed deadline below needs to trip.
fn build_pathological_sql(repeat: usize) -> String {
    let mut source = String::from("SELECT * FROM t WHERE ");
    for _ in 0..repeat {
        source.push_str("(a = b AND (c OR (d = (");
    }
    source
}

#[test]
fn test_parse_file_outline_gives_up_once_parse_budget_is_exceeded() {
    let temp_dir = TempDir::new().unwrap();
    let content = build_pathological_sql(2_000);
    let file_path = create_test_file(&temp_dir, "pathological.sql", &content);

    // An already-elapsed deadline deterministically trips on the first progress-callback check,
    // regardless of machine speed, instead of racing a real multi-second parse against a fixed
    // wall-clock bound.
    let result = parse_file_outline_with_deadline(&file_path, Instant::now());

    match result {
        Err(err) => assert!(
            err.to_string().contains("parse budget"),
            "expected the budget-specific error, got: {err}"
        ),
        Ok(_) => panic!("a parse past its deadline should give up instead of completing"),
    }
}
