use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

fn entries_json(entries: &[SkillReferenceEntry]) -> Value {
    entries
        .iter()
        .map(|entry| {
            let kind = match entry {
                SkillReferenceEntry::Name(_) => "name",
                SkillReferenceEntry::Path(_) => "path",
                SkillReferenceEntry::NameGlob(_) => "name_glob",
                SkillReferenceEntry::PathGlob(_) => "path_glob",
            };
            json!({"kind": kind, "value": entry.value()})
        })
        .collect()
}

/// The shared cases' normalized form of a parse result.
fn result_json(result: Result<SkillReferenceFile, SkillReferenceParseError>) -> Value {
    let file = match result {
        Ok(file) => file,
        Err(error) => return json!({"error": <&str>::from(error)}),
    };
    let reference = file.source.reference.as_ref().map(|reference| {
        let kind = match reference {
            SkillSourceRef::Sha(_) => "sha",
            SkillSourceRef::Named(_) => "named",
        };
        json!({"kind": kind, "value": reference.value()})
    });
    json!({
        "reference": {
            "source": {
                "code_forge": file.source.code_forge,
                "owner": file.source.owner,
                "repo": file.source.repo,
                "ref": reference,
            },
            "include": entries_json(&file.include),
            "exclude": entries_json(&file.exclude),
        }
    })
}

#[derive(Deserialize)]
struct SharedCase {
    name: String,
    input: String,
    expected: Value,
}

/// `testdata/` is a verbatim copy of warp-server's `logic/factoryfile/testdata/skill_references`,
/// where the server's parser runs the same cases. Change the cases there first.
#[test]
fn shared_cases_parse_to_their_expected_results() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/skills/references/testdata/cases.yaml");
    let cases: Vec<SharedCase> = serde_yaml::from_str(&fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|error| panic!("invalid {}: {error}", path.display()));
    assert!(!cases.is_empty(), "no cases in {}", path.display());
    let mut names = HashSet::new();
    let mut failures = Vec::new();
    for case in &cases {
        assert!(names.insert(&case.name), "duplicate case {}", case.name);
        let actual = result_json(parse_skill_reference_file(&case.input));
        if actual != case.expected {
            failures.push(format!(
                "case {}: expected {}, got {actual}",
                case.name, case.expected
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn globs_match_with_literal_braces_commas_and_backslashes() {
    let path = |pattern: &str| SkillReferenceEntry::PathGlob(pattern.to_owned());
    let name = |pattern: &str| SkillReferenceEntry::NameGlob(pattern.to_owned());
    for (entry, candidate, expected) in [
        (path("frontend/*"), "frontend/forms", true),
        (path("frontend/*"), "frontend/forms/inner", false),
        (path("frontend/**"), "frontend/forms/inner", true),
        (name("docs-{draft,final}*"), "docs-{draft,final}-v2", true),
        (name("docs-{draft,final}*"), "docs-draft", false),
        (name(r"tool-*\"), r"tool-a\", true),
        (name("[{,]*"), "{x", true),
    ] {
        let matcher = entry.glob_matcher().unwrap();
        assert_eq!(
            matcher.is_match(candidate),
            expected,
            "{entry:?} vs {candidate}"
        );
    }
}

#[test]
fn reference_file_names_need_a_yaml_extension() {
    for (file_name, expected) in [
        ("shared.yaml", true),
        ("shared.yml", true),
        ("shared.YAML", false),
        ("SKILL.md", false),
        ("shared.yaml.bak", false),
    ] {
        assert_eq!(
            is_skill_reference_file_name(file_name),
            expected,
            "{file_name}"
        );
    }
}
