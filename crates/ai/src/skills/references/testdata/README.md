# Skill reference parser cases

Shared cases for the skill reference file parser (`skills/<file>.yaml` and
`agents/<name>/skills/<file>.yaml`). The Go parser in `logic/factoryfile` and
the Rust parser in the `warp` client both run every case here, so the two stay
in step. Edit this directory first and copy it to the client.

Each case is a pair of files:

- `<case>.yaml`: the reference file contents.
- `<case>.expected.json`: on success,
  `{"reference": {"source": {...}, "include": [...], "exclude": [...]}}`; on
  failure, `{"error": "<kind>"}`.

On success, `source` is
`{"code_forge": null | "<forge>", "owner": "...", "name": "...", "ref": null | {"kind": "sha" | "named", "value": "..."}}`,
and each entry is `{"kind": "name" | "path" | "name_glob" | "path_glob", "value": "..."}`.
`code_forge` is null when the file doesn't name a forge; `exclude` is `[]`
when absent.

Cross-file checks (an agent-level file under an undeclared agent, the same
literal name twice at one level) aren't covered here.

## Rules

Document:
- A YAML syntax error, more than one document, a root that isn't a mapping,
  a duplicate key, or an explicit tag is `invalid_yaml`.
- Any anchor or alias is `yaml_alias`. That includes the scanner error for
  `*` or `&` with no name (an unquoted `- *`) and an alias to an unknown
  anchor (an unquoted `- *-review`).
- An empty or null document is an empty mapping.
- Keys other than `source`, `include` and `exclude` are `unknown_key`.

Strings: every string value is trimmed of surrounding whitespace before it's
checked or returned. Non-string scalars are never coerced to strings.

`source`:
- Absent, null, or an empty string is `missing_source`.
- Anything other than a string or a mapping is `malformed_source`.
- Shorthand: split at the last `@`; the rest splits at the last `/` into
  owner and name. `code_forge` is null.
- Mapping: keys `owner`, `name`, `codeForge` and `ref` only (`unknown_key`
  otherwise). `owner` and `name` are required strings. A null `codeForge` or
  `ref` is absent.
- `malformed_source` when there's no `/` before the ref, the owner or name is
  empty, the owner has an empty `/` segment, the name contains `/`, the owner
  or name contains `@`, or any owner segment or the name is `.` or `..`.
  A ref containing `@` therefore needs the mapping form.
- `codeForge` must be exactly `GITHUB`, `GITLAB` or `AZURE_DEVOPS`; anything
  else, including an empty or non-string value, is `unknown_code_forge`.

`ref` (`malformed_ref` otherwise):
- A non-empty string. In the shorthand, an `@` with nothing after it is empty.
- Must not start with `-` or `/`, end with `/`, `.` or `.lock`, or be exactly
  `@`.
- Must not contain whitespace, an ASCII control character, any of
  `~ ^ : ? * [ \`, or `..`, `//` or `@{`.
- Exactly 40 or 64 lowercase hexadecimal characters is `sha`; anything else is
  `named`.

`include` and `exclude`:
- `include` absent or null is `missing_include`; `[]` is `empty_include`.
- `exclude` absent, null or `[]` is `[]`.
- A value that isn't a list, an entry that isn't a string, or an entry that's
  empty after trimming is `non_string_entry`.
- An entry containing `*`, `?` or `[` is a glob: `path_glob` if it contains
  `/`, otherwise `name_glob`. Other entries containing `/` are `path`; the
  rest are `name`. Values are returned verbatim after trimming.
- A `path` or `path_glob` starting with `/` is `absolute_path`; one with a
  `..` segment is `parent_path`.
- Glob syntax is globset 0.4.18's `*`, `**`, `?` and `[...]`. `**` may appear
  anywhere. `{`, `}`, `,` and `\` are literal, so the Rust side must escape
  them before compiling with globset. `invalid_glob` is limited to:
  - a `[` with no closing `]` (a `]` right after `[`, `[!` or `[^` is a
    member, so `[]*` is unclosed);
  - a range whose start is greater than its end, such as `[z-a]`.

## Precedence

A file with several problems reports the first in this order: document
problems, top-level `unknown_key`, then `source` (unknown keys, owner and
name, `codeForge`, then `ref`), then `include`, then `exclude`. Within a
list, entries are checked in order, and each entry is checked for
`non_string_entry`, `absolute_path`, `parent_path`, then `invalid_glob`.
