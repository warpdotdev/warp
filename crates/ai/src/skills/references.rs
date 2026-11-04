//! Parser for Factory skill reference files: YAML files that sit directly in a Factory definition's
//! `skills/` or `agents/<name>/skills/` folder and select skills from another repository.

use std::collections::HashSet;

use globset::{Glob, GlobBuilder, GlobMatcher};
use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::{Error as ValueError, StrDeserializer};
use strum_macros::IntoStaticStr;
use warp_cli::agent::RepositoryForge;
use yaml_rust::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust::scanner::Marker;
use yaml_rust::{Yaml, YamlLoader};

/// A parsed reference file: one source repository and the skills to load from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillReferenceFile {
    pub source: SkillSource,
    pub include: Vec<SkillReferenceEntry>,
    /// Entries removed from the result of `include`.
    pub exclude: Vec<SkillReferenceEntry>,
}

/// The repository a reference file loads skills from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillSource {
    /// `None` when the file doesn't name a forge, which is allowed only for a Factory with a
    /// single code forge.
    pub code_forge: Option<RepositoryForge>,
    /// May contain `/`, for GitLab subgroups or an Azure DevOps `org/project`.
    pub owner: String,
    pub repo: String,
    /// `None` selects the latest commit on the default branch.
    pub reference: Option<SkillSourceRef>,
}

/// Parses a code forge token such as `GITHUB`, spelled as in `factory.yaml` repository entries.
pub fn parse_code_forge(token: &str) -> Option<RepositoryForge> {
    let deserializer: StrDeserializer<'_, ValueError> = token.into_deserializer();
    RepositoryForge::deserialize(deserializer).ok()
}

fn code_forge_from_host(host: &str) -> Option<RepositoryForge> {
    match host {
        "github.com" => Some(RepositoryForge::GitHub),
        "gitlab.com" => Some(RepositoryForge::GitLab),
        "dev.azure.com" => Some(RepositoryForge::AzureDevOps),
        _ => None,
    }
}

/// The revision of a skill source to load.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkillSourceRef {
    /// A 40-character lowercase hexadecimal commit SHA.
    Sha(String),
    /// A branch or tag name; a branch wins when both exist.
    Named(String),
}

impl SkillSourceRef {
    pub fn value(&self) -> &str {
        match self {
            Self::Sha(value) | Self::Named(value) => value,
        }
    }
}

/// One `include` or `exclude` entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkillReferenceEntry {
    /// A skill name, matched against `SKILL.md` frontmatter in the standard skill folders.
    Name(String),
    /// A repository-relative path to a skill folder or its `SKILL.md`.
    Path(String),
    /// A glob over skill names in the standard skill folders.
    NameGlob(String),
    /// A glob over repository-relative skill folder paths.
    PathGlob(String),
}

impl SkillReferenceEntry {
    pub fn value(&self) -> &str {
        match self {
            Self::Name(value)
            | Self::Path(value)
            | Self::NameGlob(value)
            | Self::PathGlob(value) => value,
        }
    }

    /// The compiled matcher for glob entries; `None` for names and paths.
    pub fn glob_matcher(&self) -> Option<GlobMatcher> {
        match self {
            Self::NameGlob(pattern) | Self::PathGlob(pattern) => compile_glob(pattern)
                .ok()
                .map(|glob| glob.compile_matcher()),
            Self::Name(_) | Self::Path(_) => None,
        }
    }

    fn classify(value: &str) -> Result<Self, SkillReferenceParseError> {
        let value = value.to_owned();
        let is_glob = value.contains(['*', '?', '[']);
        let is_path = value.contains('/');
        if is_path {
            if value.starts_with('/') {
                return Err(SkillReferenceParseError::AbsolutePath);
            }
            if value.split('/').any(|segment| segment == "..") {
                return Err(SkillReferenceParseError::ParentPath);
            }
        }
        if is_glob {
            compile_glob(&value).map_err(|_| SkillReferenceParseError::InvalidGlob)?;
        }
        Ok(match (is_glob, is_path) {
            (true, true) => Self::PathGlob(value),
            (true, false) => Self::NameGlob(value),
            (false, true) => Self::Path(value),
            (false, false) => Self::Name(value),
        })
    }
}

/// Why a reference file is invalid. Each kind's snake_case name is a code shared with the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum SkillReferenceParseError {
    InvalidYaml,
    /// Includes an unquoted entry that starts with `*`.
    YamlAlias,
    UnknownKey,
    MissingSource,
    MalformedSource,
    MalformedRef,
    UnknownCodeForge,
    MissingInclude,
    EmptyInclude,
    NonStringEntry,
    InvalidGlob,
    AbsolutePath,
    ParentPath,
}

/// Whether `file_name` names a reference file when it sits directly in a skills folder.
pub fn is_skill_reference_file_name(file_name: &str) -> bool {
    file_name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| matches!(extension, "yaml" | "yml"))
}

/// Parses the contents of one reference file.
pub fn parse_skill_reference_file(
    contents: &str,
) -> Result<SkillReferenceFile, SkillReferenceParseError> {
    check_yaml_structure(contents)?;
    let documents =
        YamlLoader::load_from_str(contents).map_err(|_| SkillReferenceParseError::InvalidYaml)?;
    let root = match documents.as_slice() {
        [] => return Err(SkillReferenceParseError::MissingSource),
        [root] => root,
        [_, _, ..] => return Err(SkillReferenceParseError::InvalidYaml),
    };
    let mapping = match root.as_hash() {
        Some(mapping) => mapping,
        // An empty or null document is an empty mapping.
        None if root.is_null() => return Err(SkillReferenceParseError::MissingSource),
        None => return Err(SkillReferenceParseError::InvalidYaml),
    };
    if !keys_are_known(mapping, &["source", "include", "exclude"]) {
        return Err(SkillReferenceParseError::UnknownKey);
    }
    let field = |key: &str| mapping.get(&Yaml::String(key.to_owned()));
    let source = parse_source(field("source"))?;
    let include = match field("include") {
        None | Some(Yaml::Null) => return Err(SkillReferenceParseError::MissingInclude),
        Some(value) => parse_entries(value)?,
    };
    if include.is_empty() {
        return Err(SkillReferenceParseError::EmptyInclude);
    }
    let exclude = match field("exclude") {
        None | Some(Yaml::Null) => Vec::new(),
        Some(value) => parse_entries(value)?,
    };
    Ok(SkillReferenceFile {
        source,
        include,
        exclude,
    })
}

fn keys_are_known(mapping: &yaml_rust::yaml::Hash, known: &[&str]) -> bool {
    mapping
        .keys()
        .all(|key| key.as_str().is_some_and(|key| known.contains(&key)))
}

fn parse_source(value: Option<&Yaml>) -> Result<SkillSource, SkillReferenceParseError> {
    match value {
        None | Some(Yaml::Null) => Err(SkillReferenceParseError::MissingSource),
        Some(Yaml::String(shorthand)) => parse_source_shorthand(shorthand.trim()),
        Some(Yaml::Hash(mapping)) => {
            if !keys_are_known(mapping, &["owner", "repo", "codeForge", "ref"]) {
                return Err(SkillReferenceParseError::UnknownKey);
            }
            let field = |key: &str| mapping.get(&Yaml::String(key.to_owned()));
            let owner = match field("owner") {
                Some(Yaml::String(owner)) if is_valid_owner(owner.trim()) => owner.trim(),
                _ => return Err(SkillReferenceParseError::MalformedSource),
            };
            let repo = match field("repo") {
                Some(Yaml::String(repo)) if is_valid_repository_segment(repo.trim()) => repo.trim(),
                _ => return Err(SkillReferenceParseError::MalformedSource),
            };
            let code_forge = match field("codeForge") {
                None | Some(Yaml::Null) => None,
                Some(Yaml::String(token)) => Some(
                    parse_code_forge(token.trim())
                        .ok_or(SkillReferenceParseError::UnknownCodeForge)?,
                ),
                Some(_) => return Err(SkillReferenceParseError::UnknownCodeForge),
            };
            let reference = match field("ref") {
                None | Some(Yaml::Null) => None,
                Some(Yaml::String(reference)) => Some(parse_ref(reference.trim())?),
                Some(_) => return Err(SkillReferenceParseError::MalformedRef),
            };
            Ok(SkillSource {
                code_forge,
                owner: owner.to_owned(),
                repo: repo.to_owned(),
                reference,
            })
        }
        Some(_) => Err(SkillReferenceParseError::MalformedSource),
    }
}

/// Parses `[<host>:]<owner>/<repo>[@<ref>]`. Text before the first `:` is a host only when it
/// has no `/` or `@`; otherwise the `:` belongs to the owner, repo or ref, which reject it.
fn parse_source_shorthand(shorthand: &str) -> Result<SkillSource, SkillReferenceParseError> {
    if shorthand.is_empty() {
        return Err(SkillReferenceParseError::MissingSource);
    }
    let (host, rest) = match shorthand.split_once(':') {
        Some((host, rest)) if !host.contains(['/', '@']) => (Some(host), rest),
        _ => (None, shorthand),
    };
    let (repository, reference) = match rest.rsplit_once('@') {
        Some((repository, reference)) => (repository, Some(reference)),
        None => (rest, None),
    };
    let Some((owner, repo)) = repository.rsplit_once('/') else {
        return Err(SkillReferenceParseError::MalformedSource);
    };
    if !is_valid_owner(owner) || !is_valid_repository_segment(repo) {
        return Err(SkillReferenceParseError::MalformedSource);
    }
    let code_forge = host
        .map(|host| code_forge_from_host(host).ok_or(SkillReferenceParseError::UnknownCodeForge))
        .transpose()?;
    Ok(SkillSource {
        code_forge,
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        reference: reference.map(parse_ref).transpose()?,
    })
}

fn parse_ref(value: &str) -> Result<SkillSourceRef, SkillReferenceParseError> {
    if !is_valid_ref(value) || is_lowercase_hex(value, 64) {
        return Err(SkillReferenceParseError::MalformedRef);
    }
    Ok(if is_lowercase_hex(value, 40) {
        SkillSourceRef::Sha(value.to_owned())
    } else {
        SkillSourceRef::Named(value.to_owned())
    })
}

fn parse_entries(value: &Yaml) -> Result<Vec<SkillReferenceEntry>, SkillReferenceParseError> {
    let Yaml::Array(items) = value else {
        return Err(SkillReferenceParseError::NonStringEntry);
    };
    items
        .iter()
        .map(|item| match item {
            Yaml::String(value) if !value.trim().is_empty() => {
                SkillReferenceEntry::classify(value.trim())
            }
            _ => Err(SkillReferenceParseError::NonStringEntry),
        })
        .collect()
}

fn is_valid_owner(owner: &str) -> bool {
    owner.split('/').all(is_valid_repository_segment)
}

fn is_valid_repository_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(['/', '@', ':'])
}

/// Rejects refs that could be read as a git option (`-`) or as range or reflog syntax. Other names
/// git refuses fail when checked out, which a run tolerates.
fn is_valid_ref(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.contains("..")
        && !value.contains("@{")
        && !value.chars().any(|character| {
            character.is_whitespace() || character.is_control() || character == ':'
        })
}

/// Whether `value` is exactly `len` lowercase hexadecimal characters. 64-character SHA-256 commit
/// IDs are rejected because checkouts accept only 40-character SHAs.
fn is_lowercase_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Compiles a glob with only `*`, `**`, `?` and `[...]` as syntax; every other character,
/// including globset's alternation and escape characters, is literal.
fn compile_glob(pattern: &str) -> Result<Glob, globset::Error> {
    GlobBuilder::new(&escape_literal_glob_characters(pattern))
        .literal_separator(true)
        .backslash_escape(false)
        .build()
}

/// Wraps `{`, `}`, `,` and `\` outside character classes in a one-character class, which
/// globset reads literally. Classes are copied unchanged, using globset's rule that a `]` right
/// after `[`, `[!` or `[^` is a member.
fn escape_literal_glob_characters(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len());
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '[' => {
                escaped.push('[');
                if let Some(negation @ ('!' | '^')) = characters.peek().copied() {
                    escaped.push(negation);
                    characters.next();
                }
                if characters.peek() == Some(&']') {
                    escaped.push(']');
                    characters.next();
                }
                for member in characters.by_ref() {
                    escaped.push(member);
                    if member == ']' {
                        break;
                    }
                }
            }
            '{' | '}' | ',' | '\\' => {
                escaped.push('[');
                escaped.push(character);
                escaped.push(']');
            }
            character => escaped.push(character),
        }
    }
    escaped
}

/// Rejects anchors, aliases and duplicate mapping keys, which the value loader would otherwise
/// accept silently.
fn check_yaml_structure(contents: &str) -> Result<(), SkillReferenceParseError> {
    let mut checker = YamlStructureChecker::default();
    let result = Parser::new(contents.chars()).load(&mut checker, true);
    if let Err(error) = result {
        let message = error.to_string();
        return Err(if message.contains("anchor") || message.contains("alias") {
            SkillReferenceParseError::YamlAlias
        } else {
            SkillReferenceParseError::InvalidYaml
        });
    }
    checker.error.map_or(Ok(()), Err)
}

#[derive(Default)]
struct YamlStructureChecker {
    /// One entry per open collection: a mapping's keys so far and whether a key comes next, or
    /// `None` for a sequence.
    frames: Vec<Option<(HashSet<String>, bool)>>,
    error: Option<SkillReferenceParseError>,
}

impl MarkedEventReceiver for YamlStructureChecker {
    fn on_event(&mut self, event: Event, _mark: Marker) {
        // Whether the node is anchored or an alias, its scalar value, whether it has a tag, and the
        // frame it opens.
        let (anchored, key, tagged, opens) = match &event {
            Event::Alias(_) => (true, None, false, None),
            Event::Scalar(value, _, anchor, tag) => {
                (*anchor != 0, Some(value), tag.is_some(), None)
            }
            Event::SequenceStart(anchor) => (*anchor != 0, None, false, Some(None)),
            Event::MappingStart(anchor) => (
                *anchor != 0,
                None,
                false,
                Some(Some((HashSet::new(), true))),
            ),
            Event::SequenceEnd | Event::MappingEnd => {
                self.frames.pop();
                return;
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => return,
        };
        let mut duplicate = false;
        if let Some(Some((keys, expecting_key))) = self.frames.last_mut() {
            duplicate = *expecting_key && key.is_some_and(|key| !keys.insert(key.clone()));
            *expecting_key = !*expecting_key;
        }
        let error = if anchored {
            Some(SkillReferenceParseError::YamlAlias)
        } else if tagged || duplicate {
            Some(SkillReferenceParseError::InvalidYaml)
        } else {
            None
        };
        self.error = self.error.or(error);
        self.frames.extend(opens);
    }
}

#[cfg(test)]
#[path = "references_tests.rs"]
mod tests;
