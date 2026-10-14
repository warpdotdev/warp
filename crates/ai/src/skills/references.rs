//! Parser for Factory skill reference files: YAML files that sit directly in a Factory definition's
//! `skills/` or `agents/<name>/skills/` folder and select skills from another repository.

use std::collections::HashSet;
use std::fmt;

use globset::{Glob, GlobBuilder, GlobMatcher};
use thiserror::Error;
use yaml_rust::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust::scanner::Marker;
use yaml_rust::{Yaml, YamlLoader};

/// Extensions that mark a file directly inside a skills folder as a reference file.
pub const SKILL_REFERENCE_FILE_EXTENSIONS: [&str; 2] = ["yaml", "yml"];

const SOURCE_KEY: &str = "source";
const INCLUDE_KEY: &str = "include";
const EXCLUDE_KEY: &str = "exclude";
const SOURCE_OWNER_KEY: &str = "owner";
const SOURCE_NAME_KEY: &str = "name";
const SOURCE_CODE_FORGE_KEY: &str = "codeForge";
const SOURCE_REF_KEY: &str = "ref";

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
    /// `None` when the file doesn't name a forge, which means the Factory's primary forge.
    pub code_forge: Option<SkillSourceForge>,
    /// May contain `/`, for GitLab subgroups or an Azure DevOps `org/project`.
    pub owner: String,
    pub name: String,
    /// `None` selects the latest commit on the default branch.
    pub reference: Option<SkillSourceRef>,
}

/// Code forges a skill source can live on, spelled as in `factory.yaml` repository entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillSourceForge {
    GitHub,
    GitLab,
    AzureDevOps,
}

impl SkillSourceForge {
    /// Parses a forge token such as `GITHUB`.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "GITHUB" => Some(Self::GitHub),
            "GITLAB" => Some(Self::GitLab),
            "AZURE_DEVOPS" => Some(Self::AzureDevOps),
            _ => None,
        }
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::GitHub => "GITHUB",
            Self::GitLab => "GITLAB",
            Self::AzureDevOps => "AZURE_DEVOPS",
        }
    }
}

impl fmt::Display for SkillSourceForge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// The revision of a skill source to load.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkillSourceRef {
    /// A 40- or 64-character lowercase hexadecimal commit SHA.
    Sha(String),
    /// A branch or tag name; a branch wins when both exist.
    Named(String),
}

impl SkillSourceRef {
    fn classify(value: String) -> Self {
        if is_commit_sha(&value) {
            Self::Sha(value)
        } else {
            Self::Named(value)
        }
    }

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

    /// The compiled matcher for glob entries; `None` for names and paths. `*` matches within one
    /// path segment and `**` across segments.
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

impl fmt::Display for SkillReferenceEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.value())
    }
}

/// Why a reference file is invalid. Each kind has a stable code shared with the server's parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum SkillReferenceParseError {
    #[error("the file is not valid YAML")]
    InvalidYaml,
    #[error("YAML anchors and aliases are not allowed; quote entries that start with `*`")]
    YamlAlias,
    #[error("the file has an unknown key")]
    UnknownKey,
    #[error("`source` is missing or empty")]
    MissingSource,
    #[error("`source` is malformed")]
    MalformedSource,
    #[error("`ref` is empty or malformed")]
    MalformedRef,
    #[error("`codeForge` is not a supported code forge")]
    UnknownCodeForge,
    #[error("`include` is missing")]
    MissingInclude,
    #[error("`include` is empty")]
    EmptyInclude,
    #[error("`include` and `exclude` entries must be strings")]
    NonStringEntry,
    #[error("an entry is not a valid glob")]
    InvalidGlob,
    #[error("paths must be relative")]
    AbsolutePath,
    #[error("paths must not contain `..`")]
    ParentPath,
}

impl SkillReferenceParseError {
    /// The stable code for this error kind.
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidYaml => "invalid_yaml",
            Self::YamlAlias => "yaml_alias",
            Self::UnknownKey => "unknown_key",
            Self::MissingSource => "missing_source",
            Self::MalformedSource => "malformed_source",
            Self::MalformedRef => "malformed_ref",
            Self::UnknownCodeForge => "unknown_code_forge",
            Self::MissingInclude => "missing_include",
            Self::EmptyInclude => "empty_include",
            Self::NonStringEntry => "non_string_entry",
            Self::InvalidGlob => "invalid_glob",
            Self::AbsolutePath => "absolute_path",
            Self::ParentPath => "parent_path",
        }
    }
}

/// Whether `file_name` names a reference file when it sits directly in a skills folder.
pub fn is_skill_reference_file_name(file_name: &str) -> bool {
    file_name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| SKILL_REFERENCE_FILE_EXTENSIONS.contains(&extension))
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
    let mapping = match root {
        Yaml::Hash(mapping) => mapping,
        // An empty or null document is an empty mapping.
        Yaml::Null => return Err(SkillReferenceParseError::MissingSource),
        Yaml::Real(_)
        | Yaml::Integer(_)
        | Yaml::String(_)
        | Yaml::Boolean(_)
        | Yaml::Array(_)
        | Yaml::Alias(_)
        | Yaml::BadValue => return Err(SkillReferenceParseError::InvalidYaml),
    };
    for key in mapping.keys() {
        match key.as_str() {
            Some(SOURCE_KEY | INCLUDE_KEY | EXCLUDE_KEY) => {}
            _ => return Err(SkillReferenceParseError::UnknownKey),
        }
    }
    let source = parse_source(mapping.get(&Yaml::String(SOURCE_KEY.to_owned())))?;
    let include = match mapping.get(&Yaml::String(INCLUDE_KEY.to_owned())) {
        None | Some(Yaml::Null) => return Err(SkillReferenceParseError::MissingInclude),
        Some(value) => parse_entries(value)?,
    };
    if include.is_empty() {
        return Err(SkillReferenceParseError::EmptyInclude);
    }
    let exclude = match mapping.get(&Yaml::String(EXCLUDE_KEY.to_owned())) {
        None | Some(Yaml::Null) => Vec::new(),
        Some(value) => parse_entries(value)?,
    };
    Ok(SkillReferenceFile {
        source,
        include,
        exclude,
    })
}

fn parse_source(value: Option<&Yaml>) -> Result<SkillSource, SkillReferenceParseError> {
    match value {
        None | Some(Yaml::Null) => Err(SkillReferenceParseError::MissingSource),
        Some(Yaml::String(shorthand)) => parse_source_shorthand(shorthand.trim()),
        Some(Yaml::Hash(mapping)) => {
            for key in mapping.keys() {
                match key.as_str() {
                    Some(
                        SOURCE_OWNER_KEY | SOURCE_NAME_KEY | SOURCE_CODE_FORGE_KEY | SOURCE_REF_KEY,
                    ) => {}
                    _ => return Err(SkillReferenceParseError::UnknownKey),
                }
            }
            let field = |key: &str| mapping.get(&Yaml::String(key.to_owned()));
            let owner = match field(SOURCE_OWNER_KEY) {
                Some(Yaml::String(owner)) if is_valid_owner(owner.trim()) => owner.trim(),
                _ => return Err(SkillReferenceParseError::MalformedSource),
            };
            let name = match field(SOURCE_NAME_KEY) {
                Some(Yaml::String(name)) if is_valid_repository_segment(name.trim()) => name.trim(),
                _ => return Err(SkillReferenceParseError::MalformedSource),
            };
            let code_forge = match field(SOURCE_CODE_FORGE_KEY) {
                None | Some(Yaml::Null) => None,
                Some(Yaml::String(token)) => Some(
                    SkillSourceForge::from_token(token.trim())
                        .ok_or(SkillReferenceParseError::UnknownCodeForge)?,
                ),
                Some(_) => return Err(SkillReferenceParseError::UnknownCodeForge),
            };
            let reference = match field(SOURCE_REF_KEY) {
                None | Some(Yaml::Null) => None,
                Some(Yaml::String(reference)) => Some(parse_ref(reference.trim())?),
                Some(_) => return Err(SkillReferenceParseError::MalformedRef),
            };
            Ok(SkillSource {
                code_forge,
                owner: owner.to_owned(),
                name: name.to_owned(),
                reference,
            })
        }
        Some(_) => Err(SkillReferenceParseError::MalformedSource),
    }
}

fn parse_source_shorthand(shorthand: &str) -> Result<SkillSource, SkillReferenceParseError> {
    if shorthand.is_empty() {
        return Err(SkillReferenceParseError::MissingSource);
    }
    let (repository, reference) = match shorthand.rsplit_once('@') {
        Some((repository, reference)) => (repository, Some(reference)),
        None => (shorthand, None),
    };
    let Some((owner, name)) = repository.rsplit_once('/') else {
        return Err(SkillReferenceParseError::MalformedSource);
    };
    if !is_valid_owner(owner) || !is_valid_repository_segment(name) {
        return Err(SkillReferenceParseError::MalformedSource);
    }
    Ok(SkillSource {
        code_forge: None,
        owner: owner.to_owned(),
        name: name.to_owned(),
        reference: reference.map(parse_ref).transpose()?,
    })
}

fn parse_ref(value: &str) -> Result<SkillSourceRef, SkillReferenceParseError> {
    if is_valid_ref(value) {
        Ok(SkillSourceRef::classify(value.to_owned()))
    } else {
        Err(SkillReferenceParseError::MalformedRef)
    }
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
    !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(['/', '@'])
}

/// A subset of `git check-ref-format` that rejects anything git would refuse as a branch or tag.
fn is_valid_ref(value: &str) -> bool {
    !value.is_empty()
        && value != "@"
        && !value.starts_with(['-', '/'])
        && !value.ends_with(['/', '.'])
        && !value.ends_with(".lock")
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains("@{")
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_ascii_control()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
}

fn is_commit_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
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
    frames: Vec<YamlFrame>,
    error: Option<SkillReferenceParseError>,
}

enum YamlFrame {
    Sequence,
    Mapping {
        keys: HashSet<String>,
        expecting_key: bool,
    },
}

impl YamlStructureChecker {
    fn record(&mut self, error: SkillReferenceParseError) {
        self.error.get_or_insert(error);
    }

    /// Accounts for one complete node in the enclosing collection, checking mapping keys for
    /// duplicates.
    fn on_node(&mut self, scalar_key: Option<&str>) {
        let mut duplicate = false;
        if let Some(YamlFrame::Mapping {
            keys,
            expecting_key,
        }) = self.frames.last_mut()
        {
            if *expecting_key
                && let Some(key) = scalar_key
                && !keys.insert(key.to_owned())
            {
                duplicate = true;
            }
            *expecting_key = !*expecting_key;
        }
        if duplicate {
            self.record(SkillReferenceParseError::InvalidYaml);
        }
    }
}

impl MarkedEventReceiver for YamlStructureChecker {
    fn on_event(&mut self, event: Event, _mark: Marker) {
        match event {
            Event::Alias(_) => {
                self.record(SkillReferenceParseError::YamlAlias);
                self.on_node(None);
            }
            Event::Scalar(value, _, anchor, tag) => {
                if anchor != 0 {
                    self.record(SkillReferenceParseError::YamlAlias);
                }
                if tag.is_some() {
                    self.record(SkillReferenceParseError::InvalidYaml);
                }
                self.on_node(Some(&value));
            }
            Event::SequenceStart(anchor) => {
                if anchor != 0 {
                    self.record(SkillReferenceParseError::YamlAlias);
                }
                self.on_node(None);
                self.frames.push(YamlFrame::Sequence);
            }
            Event::MappingStart(anchor) => {
                if anchor != 0 {
                    self.record(SkillReferenceParseError::YamlAlias);
                }
                self.on_node(None);
                self.frames.push(YamlFrame::Mapping {
                    keys: HashSet::new(),
                    expecting_key: true,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                self.frames.pop();
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
        }
    }
}

#[cfg(test)]
#[path = "references_tests.rs"]
mod tests;
