# Technical spec: Generic syntax-highlight definition mechanism (GH-9955)

This spec is the implementation companion to `product.md`. It picks
the discovery mechanism, the schema, the loader architecture, and
the migration strategy for the 32 existing languages.

## Current state (recap from product.md investigation)

- `crates/languages/src/lib.rs` defines `SUPPORTED_LANGUAGES: [&str;
  32]` (line 23), `language_by_name`, `language_by_filename`,
  `to_arborium_name`, `get_arborium_highlight_query`, `load_language`.
- `crates/languages/grammars/<lang>/` provides per-language
  `config.yaml`, `identifiers.scm`, `indents.scm`. Embedded via
  `RustEmbed`.
- `arborium` (internal crate) provides parsers and bundled
  highlight queries via `arborium::lang_<X>::HIGHLIGHTS_QUERY`
  consts.
- Consumers of `Language`: `crates/syntax_tree/src/queries/`
  (highlight query, indent query), `app/src/code/editor`, the AI
  context indexers, the workflow view's `syntax_highlightable.rs`.

## Architecture overview

Add a `LanguageRegistry` discovery layer that loads from THREE
sources, in priority order:

1. **Compile-time hardcoded** (existing path) — the current
   `to_arborium_name` / `get_arborium_highlight_query` matches.
   This is the "first-class" path; it stays for the existing 32
   languages until per-language migration PRs convert them.
2. **Bundled directory** — `crates/languages/grammars/<lang>/` with
   a `language.toml` driving discovery. New languages can be added
   here without touching `lib.rs`.
3. **User-local directory** — `~/.warp/grammars/<lang>/` (or
   `$XDG_CONFIG_HOME/warp/grammars/<lang>/`). WASM-only grammars,
   loaded at startup.

A bundled grammar takes precedence over a user-local one with the
same `internal_name` (B2 invariant). A compile-time hardcoded
language takes precedence over a bundled grammar with the same
name; this gives us the staged migration path of B4.

## `language.toml` schema

```toml
schema_version = 1

[language]
display_name = "Nim"
internal_name = "nim"
comment_prefix = "#"
indent_unit = { spaces = 2 }       # or { tabs = 1 }

[file_associations]
extensions = ["nim", "nims"]
filenames = ["nim.cfg"]
shebangs = ["nim"]
aliases = ["nim-lang"]

[brackets]
pairs = [
  { start = "(", end = ")" },
  { start = "{", end = "}" },
  { start = "[", end = "]" },
]

[parser]
# Exactly one of `rust_crate` (bundled only) or `wasm` (bundled or
# user-local) must be set. The schema validate() rejects setting
# both or neither. The two examples below show the canonical
# bundled and user-local shapes.

# Optional: pin the tree-sitter ABI version this grammar was
# compiled against; loader rejects mismatches with a clear error.
ts_abi = 14
```

> **Correction (review #10129):** earlier drafts showed both
> `rust_crate` and `wasm` set in the same example block while the
> comments said they were mutually exclusive. The two canonical
> shapes are split below.

**Bundled-grammar shape (Rust crate parser):**
```toml
[parser]
rust_crate = "tree-sitter-nim"
ts_abi = 14
```

**Bundled or user-local shape (WASM parser):**
```toml
[parser]
wasm = "grammar.wasm"   # path relative to the grammar dir
ts_abi = 14
```

The schema lives in a new module `crates/languages/src/schema.rs`
with `serde::Deserialize` derives and a `validate()` method that:
- Rejects setting both `rust_crate` and `wasm`.
- Rejects setting neither.
- Rejects `rust_crate` in user-local grammars (only WASM is allowed
  there per B5).
- Rejects unknown bracket characters.
- Rejects unknown top-level keys to surface typos to contributors.

## Loader architecture

### `crates/languages/src/loader.rs` (new)

> **Correction (review #10129):** earlier drafts had `LoadedLanguage`
> with a mandatory `language: Arc<Language>` plus an optional
> `failure`. That can't represent a grammar that fails before a
> `Language` is constructed (e.g. malformed `language.toml`). The
> shape below is a tagged sum so failed grammars are first-class.

```rust
pub enum LanguageSource {
    Hardcoded,
    Bundled { dir: PathBuf },
    UserLocal { dir: PathBuf },
}

/// One result of attempting to load a grammar from a directory or
/// from the hardcoded path. Either we got a `Language`, or we got
/// a `FailedGrammar` describing what went wrong.
pub enum LoadResult {
    Loaded(LoadedLanguage),
    Failed(FailedGrammar),
}

pub struct LoadedLanguage {
    pub language: Arc<Language>,
    pub source: LanguageSource,
    /// Non-fatal warnings (e.g. missing optional `highlights.scm`).
    /// The grammar is in the registry; these are surfaced in the
    /// Settings UI but do not prevent the language from loading.
    pub warnings: Vec<LoadWarning>,
}

pub struct FailedGrammar {
    pub source: LanguageSource,
    /// Best-effort name extracted from `language.toml` if it parsed
    /// far enough; `None` if even the TOML parse failed.
    pub internal_name: Option<String>,
    pub reason: LoadFailureReason,
    pub schema_version: Option<u32>,
}

pub enum LoadFailureReason {
    SchemaParse(String),
    SchemaVersionMismatch { found: u32 },
    NativeLibAttempted,
    ParserCrateNotFound { crate_name: String },
    WasmInstantiate(String),
    WasmAbiMismatch { host: u32, grammar: u32 },
    HighlightQueryInvalid(String),  // syntactically wrong vs grammar
    IndentQueryInvalid(String),
    SymbolsQueryInvalid(String),
    /// V1 only: user-local grammars are discovered and validated
    /// but not loaded as parsers — WASM integration is gated
    /// behind the arborium / tree-sitter version question (G1).
    /// The follow-up PR removes this variant and enables the
    /// WASM path.
    UserLocalWasmNotYetSupported,
}

pub enum LoadWarning {
    HighlightsScmMissing,   // optional file absent — no coloring,
                            // grammar still loads
    IndentsScmMissing,
    IdentifiersScmMissing,
}

pub fn discover_grammars() -> Vec<LoadResult> { ... }
```

`discover_grammars()` is called once at startup. It walks the three
sources in priority order, deduplicates by `internal_name` across
loaded results (failed grammars are kept regardless of dedup so
their failure surfaces in Settings), and returns one `LoadResult`
per attempted directory.

### Loading a single grammar

> **Correction (review #10129):** earlier drafts treated highlight-
> query load failures as `LoadFailure` and returned, contradicting
> product B6 which allowed missing `highlights.scm` to load without
> coloring. The two cases are now distinct.

For each grammar directory:
1. Parse `language.toml` (`schema.rs::parse`). On failure: return
   `LoadResult::Failed { reason: SchemaParse }`.
2. Validate schema constraints (`schema.rs::validate`). On failure:
   return `LoadResult::Failed`.
3. Resolve the parser:
   - `rust_crate`: look up via the compile-time `bundled_parsers.rs`
     map. On miss: return `LoadResult::Failed { reason:
     ParserCrateNotFound }`.
   - `wasm` (bundled source): `tree_sitter::WasmStore::load_language(&wasm_bytes)`
     once G1 is enabled. **In V1, bundled WASM is also gated**
     until G1 lands; a bundled `wasm` parser returns
     `LoadResult::Failed { reason: UserLocalWasmNotYetSupported }`
     in V1 (the variant name is shared across both bundled-WASM
     and user-local-WASM since both depend on the same gate).
     V1 ships only the `rust_crate` parser path.
   - `wasm` (user-local source): **always** returns
     `LoadResult::Failed { reason: UserLocalWasmNotYetSupported }`
     in V1, regardless of whether `WasmStore::load_language` would
     succeed. The loader does NOT call `WasmStore::load_language`
     for user-local sources in V1 — the gate prevents WASM
     instantiation entirely until G1 enables it.

   When G1 is enabled (follow-up PR): the gate is removed, both
   bundled-WASM and user-local-WASM call
   `tree_sitter::WasmStore::load_language(&wasm_bytes)`. On
   instantiate failure: `LoadResult::Failed { reason:
   WasmInstantiate }`. On ABI mismatch with the host's
   `tree_sitter::TREE_SITTER_LANGUAGE_VERSION`:
   `LoadResult::Failed { reason: WasmAbiMismatch }`.
4. **`highlights.scm` (optional file):**

   > **Correction (re-review #10129):** the previous draft said
   > missing-`highlights.scm` "loads without coloring," but
   > [`crates/languages/src/lib.rs`](crates/languages/src/lib.rs)
   > defines `pub struct Language { ..., pub highlight_query: Query,
   > ... }` — `highlight_query` is **not** `Option<Query>`, so a
   > `Language` cannot be constructed without one. The corrected
   > design uses an empty query as the missing-file substitute
   > rather than changing the `Language` API.

   - **File missing:** synthesize an empty highlight query via
     `Query::new(grammar, "")`. Tree-sitter accepts empty source
     (zero patterns). The language loads with the same `Language`
     struct shape; matches at runtime return zero captures so no
     coloring is applied. Record `LoadWarning::HighlightsScmMissing`
     so the diagnostic surface in Settings still flags the missing
     file. **The `Language` API stays unchanged**; preserving B8.
   - **File present but `Query::new` fails:** the contributor
     intended to provide a query and got it wrong; return
     `LoadResult::Failed { reason: HighlightQueryInvalid }`. This
     is treated as a hard failure because shipping a grammar with
     a broken query is worse than no query at all.

   The same empty-query synthesis applies to the optional indent
   and identifiers queries: their `Language` fields ARE
   `Option<Query>` already, so missing-file = `None`, and
   invalid-file = `LoadResult::Failed` per (5) below.
5. **`indents.scm` and `identifiers.scm` (optional files):** same
   missing-vs-invalid split. Missing → `LoadWarning`. Invalid →
   `LoadResult::Failed`.
6. Construct the `Language` struct with all available fields.
7. Return `LoadResult::Loaded(LoadedLanguage { ..., warnings })`.

### Native dynamic-library rejection

The loader explicitly checks the `parser` table for any field other
than `rust_crate` or `wasm`. If a `native_lib = "grammar.so"` field
is present (or any unknown field starting with `dl` / `native` /
`so` / `dylib` / `dll`), the loader rejects the grammar with the
B5 error message and never attempts to open the file. No
`libloading::Library::new` call exists anywhere on the loader path.

### File-association registration

After all grammars load, the loader populates two maps:

```rust
struct AssociationIndex {
    by_extension: HashMap<String, Arc<Language>>,
    by_filename:  HashMap<String, Arc<Language>>,
    by_shebang:   HashMap<String, Arc<Language>>,
    by_alias:     HashMap<String, Arc<Language>>,
    by_internal_name: HashMap<String, Arc<Language>>,
}
```

The hardcoded path (the current `language_by_filename` match) is
queried first; if it returns `None`, fall through to the
`AssociationIndex`. This keeps existing behavior identical for the
32 languages until they migrate.

## Public API changes

`crates/languages/src/lib.rs`:

- `language_by_name(name: &str)` — unchanged signature; internally
  consults hardcoded match first, then `AssociationIndex.by_internal_name`
  / `by_alias`.
- `language_by_filename(path: &Path)` — unchanged signature;
  consults hardcoded path first, then `AssociationIndex` extension
  / filename / shebang lookups.
- New: `loaded_languages() -> &[LoadResult]` — for the new
  Settings → Editor → Languages page. Returns one entry per
  attempted directory (loaded, with-warnings, or failed).

No change for any current consumer; they continue to call the same
two functions.

## Settings page integration

`Settings → Editor → Languages` (new sub-page):

- Lists each loaded language: display name, internal name, source
  (Hardcoded / Bundled / User-local), file extensions claimed,
  parser revision (from `language.toml` `ts_abi`).
- Lists each failed grammar: directory, reason.
- A "Reveal in Finder/Files" button next to user-local grammars.
- A "Refresh after restart" pill at the top reminding users that
  changes require restart (V1 has no hot-reload).

## Migration strategy for the 32 existing languages

> **Correction (review #10129):** earlier drafts called this "a
> single mechanical PR." It is actually **one PR per language**,
> each independently revertable. The product spec is now consistent
> with this. Each PR follows this template:

1. Create `crates/languages/grammars/<lang>/language.toml` with the
   file associations and parser reference matching the current
   hardcoded behavior.
2. Move the `arborium::lang_X::HIGHLIGHTS_QUERY` const into a
   `highlights.scm` file in the same directory.
3. Remove the language's arms from `to_arborium_name`,
   `get_arborium_highlight_query`, `language_by_filename`, and the
   `SUPPORTED_LANGUAGES` array.
4. Verify `crates/languages/src/lib_tests.rs` still passes.
5. Verify any language-specific indent / highlight tests in
   `crates/syntax_tree/` still pass.

The `bundled_parsers.rs` map gets one new entry per migration. The
priority-order rule (hardcoded > bundled) means a partial migration
is safe: an unmigrated language uses the hardcoded path; a migrated
one uses the bundled path. There is no flag day.

V1 of THIS PR migrates **zero** existing languages — only adds the
discovery mechanism beside them. Each follow-up PR migrates one
language and is independently revertable.

## Telemetry and logging privacy

> **Correction (review #10129):** product B6 said logs include the
> grammar directory path; this section said paths are PII for
> telemetry. The two were inconsistent. Resolved: paths are PII
> across both surfaces.

> **Correction (re-review #10129, security):** the previous draft
> sent `internal_name` for all grammar sources. Hardcoded and
> bundled names are well-known (they ship in the Warp binary), but
> user-local `internal_name` is **user-controlled** — a customer's
> private project might define a grammar named `acme-internal-dsl`
> and disclose that name to analytics on every startup. Resolved
> below by stripping `internal_name` for `UserLocal`-sourced
> events.

**Telemetry events** (sent to Warp's analytics):
- `grammar_loaded` (one-time at startup):
  - For **Hardcoded** and **Bundled** sources: payload includes
    `internal_name`, `source` tag, `parser_kind` (rust_crate /
    wasm), `ts_abi`. Names are public (they ship in Warp).
  - For **UserLocal** sources: payload is `{ source:
    "user_local", parser_kind, ts_abi }` — **no `internal_name`,
    no path, no `name_hash`.** The team gets aggregate counts of
    user-local-grammar adoption without learning *which* grammars
    individual users installed.
- `grammar_load_failed` (one-time):
  - For **Hardcoded** and **Bundled**: `internal_name` (if the
    TOML parsed far enough to extract it), `reason_kind` (one of
    `schema_parse`, `schema_version`, `native_lib`,
    `parser_crate_not_found`, `wasm_instantiate`, `wasm_abi`,
    `highlight_query`, `indent_query`, `symbols_query`),
    `source_kind`. No paths.
  - For **UserLocal**: `{ source_kind: "user_local",
    reason_kind }` — no `internal_name`, no path. The
    reason_kind alone is enough to identify systemic user-local
    failure modes (e.g. `wasm_abi` mismatches after a Warp
    upgrade) without disclosing user-controlled strings.
- Both events respect Warp's existing global telemetry opt-out.

**Local logs** (`log::error!`, `log::warn!`, `log::info!`):
- Use the **basename** of the grammar directory only (e.g.,
  `nim`, `zig`). The full path is never logged.
- The exception is the in-app Settings UI, which DOES show the
  full path because it is local to the user and useful for
  debugging. The Settings UI is not log output.

**No payload contains:** raw `language.toml` contents, the
contents of `.scm` files, the WASM binary, or absolute paths.

## Test plan

### Unit tests (`crates/languages/src/schema_test.rs` — new)

- T1: Parse a minimal valid `language.toml` (display_name +
  internal_name + extensions + parser).
- T2: Parse a fully-populated `language.toml` and verify all fields
  round-trip.
- T3: Parser table with both `rust_crate` and `wasm` fields fails
  validation.
- T4: Parser table with `native_lib = "..."` is rejected with the
  B5 error message.
- T5: `schema_version` mismatch (e.g., 999) is rejected with a
  clear "unsupported schema version" error.

### Unit tests (`crates/languages/src/loader_test.rs` — new)

- T6: A bundled grammar with a stub WASM that fails to instantiate
  surfaces as `LoadResult::Failed { reason: WasmInstantiate, .. }`
  and does NOT panic.
- T7: A user-local grammar whose `internal_name` collides with a
  hardcoded language is dropped from the merged list and a warn
  fires (basename only, no full path in the log message).
- T8: A user-local grammar whose `internal_name` collides with a
  bundled grammar (after a hypothetical migration) is dropped;
  bundled wins.
- T9: ABI mismatch (host ABI 14, grammar declares ABI 13) surfaces
  as `LoadResult::Failed { reason: WasmAbiMismatch { host: 14,
  grammar: 13 } }`.
- T10 (new): Missing `highlights.scm` produces
  `LoadResult::Loaded { warnings: [HighlightsScmMissing], .. }`.
  The grammar IS in the registry; coloring is absent.
- T11 (new): Present-but-invalid `highlights.scm` (parses against
  a different grammar) produces `LoadResult::Failed { reason:
  HighlightQueryInvalid }`. The grammar is NOT in the registry.
- T12 (new): Same missing-vs-invalid distinction for `indents.scm`
  and `identifiers.scm`.
- T13 (new): A WASM grammar whose import list includes a non-
  tree-sitter symbol (e.g. fs read) is rejected at load.
- T14 (new): A grammar that triggers `parser.set_timeout_micros`
  (parse exceeds 100ms on a fixture buffer) returns partial parse
  results and emits one warn-level log.

### Integration test (`crates/languages/src/integration_test.rs` — new)

**V1 integration tests (bundled-only):**

- IT1: Create a temp dir with a bundled-fixture grammar that uses
  `rust_crate` (not WASM), point the discovery path at it, call
  `discover_grammars()`. Assert the language returns as
  `LoadResult::Loaded(...)` and
  `language_by_filename(Path::new("test.example"))` returns it.
- IT2: Same as IT1 but with malformed TOML; assert the rest of the
  registry loads normally and the failure is reported as
  `LoadResult::Failed { reason: SchemaParse(_) }`.
- IT3: Call `loaded_languages()` after discovery and assert the 32
  hardcoded languages are present alongside the test fixture.
- IT4 (V1): Drop a fixture grammar at
  `WARP_USER_GRAMMAR_DIR/<lang>/` with `language.toml` and
  `grammar.wasm`. Assert the result is
  `LoadResult::Failed { reason: UserLocalWasmNotYetSupported }`,
  the `internal_name` is extracted from the TOML, the entry
  surfaces in `loaded_languages()`, and **`WasmStore::load_language`
  is not called** (verified via a test-only counter on the WASM
  loading path).

**V1.5 / G1 integration tests (deferred to follow-up PR):**

- IT4.future: Drop a fixture WASM grammar with the same
  `internal_name` as a hardcoded language; verify user-local is
  dropped, a warn fires with basename only, and the hardcoded
  language continues to handle that name.
- IT5.future: Feed a 16MiB buffer through a user-local grammar;
  verify the input-size cap kicks in and the buffer falls back to
  plain rendering with a one-time info log.

### Regression (existing test files unchanged)

- Existing `crates/languages/src/lib_tests.rs` (which iterates
  `SUPPORTED_LANGUAGES` and verifies each loads) must pass with no
  modifications. The 32 hardcoded languages remain in
  `SUPPORTED_LANGUAGES` until their migration PRs.
- Existing `crates/syntax_tree/src/queries/*_tests.rs` calling
  `language_by_filename` must pass with no modifications.

## Files touched

V1 (this PR — discovery mechanism only, zero migrations):

- `crates/languages/src/lib.rs` — fall-through call to the new
  `AssociationIndex` after the existing hardcoded match.
- `crates/languages/src/schema.rs` (new) — `language.toml` parser.
- `crates/languages/src/loader.rs` (new) — discovery + load.
- `crates/languages/src/bundled_parsers.rs` (new) — empty map
  initially; entries added per-migration.
- `crates/languages/src/association_index.rs` (new) — lookup maps.
- `crates/languages/Cargo.toml` — add `tree-sitter` (for
  WasmStore) and `toml` deps if not already present.
- `crates/languages/src/schema_test.rs` (new) — T1–T5.
- `crates/languages/src/loader_test.rs` (new) — T6–T9.
- `crates/languages/src/integration_test.rs` (new) — IT1–IT3.
- `app/src/settings_view/editor_languages_page.rs` (new) — the
  Languages sub-page.

V1 explicitly does NOT touch:
- The 32 hardcoded language arms.
- Any consumer of `Language`.
- `arborium` crate.

## Out-of-scope follow-ups (each independently revertable)

- Per-language migration PRs (one per language) moving from
  hardcoded to `crates/languages/grammars/<lang>/`.
- Sub-language injection mechanism (Vue, TSX, Markdown code
  blocks).
- Hot-reload of user-local grammars.
- Package-manager / cloud distribution of community grammars.
- LSP integration via a `[lsp]` section in `language.toml`.

## Open questions for maintainer review

1. WASM grammars require a tree-sitter version that supports
   `WasmStore`. Confirm the version Warp uses today (verify
   `Cargo.lock`) supports it.
2. The user-local grammar directory: `~/.warp/grammars/` vs.
   `$XDG_CONFIG_HOME/warp/grammars/`. Recommendation: XDG when set,
   fall back to `~/.warp/`.
3. Should the Settings → Editor → Languages page allow disabling
   individual loaded languages? Recommendation: yes, but a
   follow-up PR; not V1.
4. The `bundled_parsers.rs` compile-time map is the only hand-
   edited file for adding bundled grammars. Can we use an
   `inventory`-style auto-registration pattern instead? (Would
   eliminate the only remaining hand-edit but adds a build-time
   crate dependency.)
