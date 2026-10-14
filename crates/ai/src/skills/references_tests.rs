use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::*;

fn parse(contents: &str) -> Result<SkillReferenceFile, SkillReferenceParseError> {
    parse_skill_reference_file(contents)
}

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

/// The shared fixture format for a parse result.
fn result_json(result: Result<SkillReferenceFile, SkillReferenceParseError>) -> Value {
    let file = match result {
        Ok(file) => file,
        Err(error) => return json!({"error": error.code()}),
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
                "code_forge": file.source.code_forge.map(SkillSourceForge::token),
                "owner": file.source.owner,
                "name": file.source.name,
                "ref": reference,
            },
            "include": entries_json(&file.include),
            "exclude": entries_json(&file.exclude),
        }
    })
}

#[test]
fn shared_fixtures_parse_to_their_expected_results() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/skills/references/testdata");
    let mut cases = 0;
    let mut failures = Vec::new();
    for entry in fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let Some(case) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".yaml"))
        else {
            continue;
        };
        let expected: Value = serde_json::from_str(
            &fs::read_to_string(directory.join(format!("{case}.expected.json"))).unwrap(),
        )
        .unwrap();
        let actual = result_json(parse(&fs::read_to_string(&path).unwrap()));
        if actual != expected {
            failures.push(format!("{case}: expected {expected}, got {actual}"));
        }
        cases += 1;
    }
    assert!(cases > 0, "no cases in {}", directory.display());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn shorthand_splits_at_last_at_and_last_slash() {
    let file =
        parse("source: acme/platform/shared-skills@release/v1\ninclude:\n  - safe-secrets\n")
            .unwrap();
    assert_eq!(
        file.source,
        SkillSource {
            code_forge: None,
            owner: "acme/platform".to_owned(),
            name: "shared-skills".to_owned(),
            reference: Some(SkillSourceRef::Named("release/v1".to_owned())),
        }
    );
    assert!(file.exclude.is_empty());
}

#[test]
fn refs_classify_full_lowercase_shas_only() {
    let sha1 = "a".repeat(40);
    let sha256 = "0123456789abcdef".repeat(4);
    for (reference, expected) in [
        (sha1.clone(), SkillSourceRef::Sha(sha1)),
        (sha256.clone(), SkillSourceRef::Sha(sha256)),
        (
            "abc1234".to_owned(),
            SkillSourceRef::Named("abc1234".to_owned()),
        ),
        ("A".repeat(40), SkillSourceRef::Named("A".repeat(40))),
    ] {
        let file = parse(&format!("source: acme/skills@{reference}\ninclude: [x]\n")).unwrap();
        assert_eq!(file.source.reference, Some(expected));
    }
}

#[test]
fn mapping_source_accepts_code_forge_and_ref() {
    let file = parse(
        "source:\n  owner: acme\n  name: shared-skills\n  codeForge: GITLAB\n  ref: v2\ninclude:\n  - \"*\"\n",
    )
    .unwrap();
    assert_eq!(file.source.code_forge, Some(SkillSourceForge::GitLab));
    assert_eq!(
        file.source.reference,
        Some(SkillSourceRef::Named("v2".to_owned()))
    );
    assert_eq!(
        file.include,
        vec![SkillReferenceEntry::NameGlob("*".to_owned())]
    );
}

#[test]
fn entries_are_classified_by_slash_and_glob_characters() {
    let file = parse(
        "source: acme/skills\ninclude:\n  - safe-secrets\n  - skills/release-notes\n  - \"frontend/*\"\n  - \"legacy-?\"\n  - \"**/SKILL.md\"\nexclude:\n  - \"legacy-[ab]\"\n",
    )
    .unwrap();
    assert_eq!(
        file.include,
        vec![
            SkillReferenceEntry::Name("safe-secrets".to_owned()),
            SkillReferenceEntry::Path("skills/release-notes".to_owned()),
            SkillReferenceEntry::PathGlob("frontend/*".to_owned()),
            SkillReferenceEntry::NameGlob("legacy-?".to_owned()),
            SkillReferenceEntry::PathGlob("**/SKILL.md".to_owned()),
        ]
    );
    assert_eq!(
        file.exclude,
        vec![SkillReferenceEntry::NameGlob("legacy-[ab]".to_owned())]
    );
}

#[test]
fn path_globs_do_not_cross_segments_with_a_single_star() {
    let matcher = SkillReferenceEntry::PathGlob("frontend/*".to_owned())
        .glob_matcher()
        .unwrap();
    assert!(matcher.is_match("frontend/forms"));
    assert!(!matcher.is_match("frontend/forms/inner"));
    let recursive = SkillReferenceEntry::PathGlob("frontend/**".to_owned())
        .glob_matcher()
        .unwrap();
    assert!(recursive.is_match("frontend/forms/inner"));
}

#[test]
fn braces_commas_and_backslashes_match_literally() {
    let braces = SkillReferenceEntry::NameGlob("docs-{draft,final}*".to_owned())
        .glob_matcher()
        .unwrap();
    assert!(braces.is_match("docs-{draft,final}-v2"));
    assert!(!braces.is_match("docs-draft"));
    let backslash = SkillReferenceEntry::NameGlob(r"tool-*\".to_owned())
        .glob_matcher()
        .unwrap();
    assert!(backslash.is_match(r"tool-a\"));
    let class = SkillReferenceEntry::NameGlob("[{,]*".to_owned())
        .glob_matcher()
        .unwrap();
    assert!(class.is_match(",x"));
    assert!(class.is_match("{x"));
}

#[test]
fn invalid_files_report_their_error_kind() {
    let cases = [
        ("source: [", SkillReferenceParseError::InvalidYaml),
        (
            "base: &base acme/skills\nsource: *base\ninclude: [x]\n",
            SkillReferenceParseError::YamlAlias,
        ),
        (
            "source: acme/skills\ninclude:\n  - *\n",
            SkillReferenceParseError::YamlAlias,
        ),
        (
            "source: acme/skills\ninclude: [x]\nextra: true\n",
            SkillReferenceParseError::UnknownKey,
        ),
        (
            "source:\n  owner: acme\n  name: skills\n  branch: main\ninclude: [x]\n",
            SkillReferenceParseError::UnknownKey,
        ),
        ("include: [x]\n", SkillReferenceParseError::MissingSource),
        (
            "source: \"\"\ninclude: [x]\n",
            SkillReferenceParseError::MissingSource,
        ),
        (
            "source: skills\ninclude: [x]\n",
            SkillReferenceParseError::MalformedSource,
        ),
        (
            "source: acme/\ninclude: [x]\n",
            SkillReferenceParseError::MalformedSource,
        ),
        (
            "source: acme/skills@\ninclude: [x]\n",
            SkillReferenceParseError::MalformedRef,
        ),
        (
            "source: acme/skills@bad..ref\ninclude: [x]\n",
            SkillReferenceParseError::MalformedRef,
        ),
        (
            "source:\n  owner: acme\n  name: skills\n  codeForge: BITBUCKET\ninclude: [x]\n",
            SkillReferenceParseError::UnknownCodeForge,
        ),
        (
            "source: acme/skills\n",
            SkillReferenceParseError::MissingInclude,
        ),
        (
            "source: acme/skills\ninclude: []\n",
            SkillReferenceParseError::EmptyInclude,
        ),
        (
            "source: acme/skills\ninclude:\n  - 3\n",
            SkillReferenceParseError::NonStringEntry,
        ),
        (
            "source: acme/skills\ninclude: [x]\nexclude:\n  - {name: y}\n",
            SkillReferenceParseError::NonStringEntry,
        ),
        (
            "source: acme/skills\ninclude:\n  - \"skills/[unclosed\"\n",
            SkillReferenceParseError::InvalidGlob,
        ),
        (
            "source: acme/skills\ninclude:\n  - /skills/x\n",
            SkillReferenceParseError::AbsolutePath,
        ),
        (
            "source: acme/skills\ninclude:\n  - skills/../secrets\n",
            SkillReferenceParseError::ParentPath,
        ),
    ];
    for (contents, expected) in cases {
        assert_eq!(parse(contents), Err(expected), "{contents}");
    }
}

#[test]
fn reference_file_names_need_a_yaml_extension() {
    assert!(is_skill_reference_file_name("shared.yaml"));
    assert!(is_skill_reference_file_name("shared.yml"));
    assert!(!is_skill_reference_file_name("shared.YAML"));
    assert!(!is_skill_reference_file_name("SKILL.md"));
    assert!(!is_skill_reference_file_name("shared.yaml.bak"));
}
