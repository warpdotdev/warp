# gh-8803: Technical Spec — User-Configurable Custom Language Servers

> Companion to [product.md](./product.md). Reference the numbered invariants there for user-visible behavior; this doc covers implementation.

## Context

Built-in behavior is unchanged: the five built-in language servers (rust-analyzer, gopls, pyright-langserver, typescript-language-server, clangd) keep their existing persistence rows, footer surfaces, and install flow. Mechanically, a small set of shared identity types changes payload (three `LspRepoStatus` variants, one persistence event — see the design decision below); those changes are compiler-guided renames with no behavioral effect on built-ins. The new system lives next to the built-in pipeline and runs first when a file is opened. Per product.md invariant 3, a custom `[[editor.language_servers]]` entry overrides built-ins two ways: **by filetype** — if a custom entry's `filetypes` matches the opened file, the custom server handles it — and **by name** — a custom entry whose `name` case-insensitively equals a built-in's display name (its `binary_name()`) suppresses that built-in entirely, for all of its filetypes. Only when no custom entry matches the file and the file's built-in is not name-suppressed does the existing built-in dispatch handle it unchanged.

**Design choice.** One shared identity enum — `ServerKey { BuiltIn(LSPServerType), Custom(String) }` — threads through the manager, the status enum, persistence write-sites, and the footer. Built-in and custom servers differ structurally only where their data genuinely differs (the config payload); everywhere identity is the question, both kinds flow through the same enum, the same map, the same event, and the same render path, with exhaustive `match`es on `ServerKey` making every divergence point explicit. Persistence rows are discriminated by a `kind` column — the on-disk serialization of the `ServerKey` discriminant.

### Current architecture, with line refs

The relevant existing code is left in place by this work. Citations are for grounding, not modification points. Line numbers are verified against master `05927696c` (2026-07-04); symbols are authoritative, line numbers drift as master moves:

LSP crate (`crates/lsp/`):

- `crates/lsp/src/supported_servers.rs:38-45` — `LSPServerType` enum, the central identity type for the five built-ins. Methods (`binary_name`, `args`, `languages`, `language_name`, `candidate`, `create_command`, `find_installed_binary_config`, `is_working_on_path`) stay as-is.
- `crates/lsp/src/config.rs:24-87` — `LanguageId` enum, `LanguageId::from_path` (extension → `LanguageId`), and `LanguageId::server_type` (1:1 `LanguageId` → `LSPServerType`). The **server-dispatch** path (`from_path` → `server_type`) stays as-is for built-ins; only the **languageId string** derivation (`lsp_language_identifier`) moves to the shared registry (see the design decision above), so built-ins and customs derive `languageId` from one source.
- `crates/languages/src/lib.rs` — the tree-sitter grammar registry: `Language` (grammar + queries + `comment_prefix`/`bracket_pairs`/`display_name`) and `language_by_filename_parts` (extension/filename → grammar *name*). Its extension→grammar-name lookup migrates to read from the shared `language_registry`; grammar/query loading (the heavy, `arborium`-backed part) stays here.
- `crates/lsp/src/config.rs:93-214` — `LspServerConfig` carries `LSPServerType`. Stays as-is; the custom-server path gets a parallel config type.
- `crates/lsp/src/manager.rs:30-203` — `LspManagerModel` keys by `PathBuf` workspace root, with `Vec<ModelHandle<LspServerModel>>` per workspace. Today the vec contains only built-ins (keyed within the vec by `LSPServerType`); the change extends this to hold custom-server models too, keyed by descriptor `name`.
- `crates/lsp/src/model.rs:56-71` — `LspState` (`Stopped`/`Starting`/`Available`/`Stopping`/`Failed`). Identity-free, reusable for custom servers.
- `crates/lsp/src/language_server_candidate.rs` — install machinery for built-ins. Stays as-is; v1 does not install custom-server binaries (product.md invariant 26).

Settings + persistence:

- `[[editor.language_servers]]` is stored in the user's existing `settings.toml` — a new top-level array-of-tables in the same file all other Warp user settings live in. No new file or directory. The path is `warp_core::paths::config_local_dir().join("settings.toml")` (`crates/warp_core/src/paths.rs`), which resolves to `~/.warp/settings.toml` on macOS, `$XDG_CONFIG_HOME/dev.warp.Warp/settings.toml` on Linux, and `%LOCALAPPDATA%\dev.warp.Warp\settings.toml` on Windows. Per-workspace `.warp/settings.toml` overrides are out of scope for v1.
- `crates/settings/src/macros.rs:716-804` — the `define_settings_group!` macro hosts every existing settings group (31 invocations across `app/src/settings/`). The macro accepts arbitrary types including `Vec<T>` of complex structs — see `app/src/settings/ai.rs::agent_mode_command_execution_allowlist` which uses `type: Vec<AgentModeCommandExecutionPredicate>`. We use the same macro for our setting.
- `crates/settings/src/manager.rs:59-92` — `SettingsManager` is the singleton that registers settings and dispatches `SettingsEvent::LocalPreferencesUpdated { storage_key, sync_to_cloud }` on changes. The macro's expansion (in particular `register_settings_events!` at `crates/settings/src/macros.rs:811-820`) emits these events automatically for every macro-registered setting.
- `app/src/settings/mod.rs:69-116` — `SettingsFileError::InvalidSettings(Vec<String>)` carries the **storage keys** (not free-text messages) of settings whose values failed to load. The UI renders one line per key. Per invariant 23, custom-server validation errors flow through this surface as a single bare key — `editor.language_servers` — when any entry is invalid; per-entry detail (which entry index, which field, why) is emitted via `log::warn!` and stays out of the banner. This matches the existing array-setting precedent at `agents.profiles.agent_mode_command_execution_allowlist`.
- `app/src/settings/init.rs:114-150` — settings load + validation entry point. Hot-reload is already wired; saving `settings.toml` re-parses and re-validates without restart.
- **Persistence shape (current).** Per-workspace LSP enablement is stored in the SQLite table `workspace_language_server`. On master the on-disk shape is `(workspace_id, language_server_name: TEXT, enabled: TEXT)`, built from two migrations: `2025-10-31-201353_add_workspace_language_server/` (initial table) and `2025-11-11-230915_change_workspace_language_server_enabled_to_text/` (typing fix). Today only built-in servers populate this table; rows use `language_server_name` set to the serialized `LSPServerType` variant name (`"RustAnalyzer"`, `"GoPls"`, etc.). The `HashMap<LSPServerType, EnablementState>` at `app/src/ai/persisted_workspace.rs:134` is an in-memory cache, not the on-disk shape. **Phase 4 of this work adds an additive `kind` column (migration `2026-05-24-180000_add_kind_to_workspace_language_server/`) to discriminate `'BuiltIn'` from `'Custom'` rows; the full description and rationale live in the Phase 4 persistence section below — implementers should treat that as the proposed change.**

JSON Schema generation:

- `app/src/bin/generate_settings_schema.rs` — build-time binary that walks the `inventory::iter::<SettingSchemaEntry>` registry and emits a single JSON Schema artifact for `settings.toml`. Schemas come from `(entry.schema_fn)(&mut SchemaGenerator)`. Each setting's schema is normally derived via `#[derive(schemars::JsonSchema)]` on the public type. Where a public type has a non-serializable field (e.g. `Regex` in `app/src/settings/privacy.rs:41-50::CustomSecretRegex`), the codebase uses `#[schemars(with = "String")]` to describe the user-typed shape. Custom LSP descriptors follow the same pattern: `LspFiletypePattern` (user-typed pattern string + private compiled `globset::GlobMatcher`) presents as a plain string via `#[schemars(with = "String")]`.

Footer:

- `app/src/code/footer.rs:1410-1855` — status rendering, Enable button dispatch, error rendering. Action enum is server-type-agnostic; the surfaces that take `&LSPServerType` for display move to the shared `ServerKey` (see Phase 4).

Glob support is already in-tree: `globset = { workspace = true }` is in `crates/lsp/Cargo.toml:23` (version `0.4.18` pinned at the root `Cargo.toml:161`) and used at `crates/lsp/src/service.rs:374-378`. product.md invariant 1 cites the `glob` crate's `Pattern` syntax — that's a strict subset of what `globset` accepts, so we don't add a dependency.

Path utilities live in `crates/warp_util/src/path.rs`; leading-`~` expansion uses the existing `shellexpand` workspace dependency (no new helper). Data/cache dirs come from `crates/warp_core/src/paths.rs` (`data_dir()` at `:100`, `cache_dir()` at `:222`).

### Design decision: one identity enum, two config payloads

Custom servers reuse the existing LSP runtime end to end (`LspService`, `ProcessTransport`, `LspState`, diagnostics tracking — all verified identity-free). Identity is carried by a single shared enum, defined in `crates/lsp` and consumed by `app/`:

```rust
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerKey {
    BuiltIn(LSPServerType),
    Custom(String),          // descriptor `name`
}

impl ServerKey {
    /// Footer display name: `binary_name()` for built-ins, the descriptor `name` for customs.
    pub fn display_name(&self) -> &str { ... }
}
```

Where it threads:

- **Matcher** — checks customs first via `LanguageServersSettings::match_for_path`; when no custom matches, falls back to the existing `LanguageId::from_path` → `LSPServerType` dispatch — unless that built-in is suppressed by a same-named custom entry (`suppresses_builtin`, product.md invariant 3 by name), in which case no server handles the file. Resolution (Phase 4's `resolve_server_for_path`) produces the identity everything downstream consumes.
- **Status** — `LspRepoStatus` serves both kinds. Its three identity-bearing variants (`DisabledAndInstalled`, `DisabledAndNotInstalled`, `Installing` — `app/src/ai/persisted_workspace.rs:92-105`) change payload from `server_type: LSPServerType` to `server: ServerKey`; the compiler surfaces all ~24 existing match sites for a mechanical update. A `Custom` key can carry `DisabledAndInstalled` (v1 assumes a configured custom is installed — that variant drives its Enable CTA, see Phase 4), but the three install-*flow* variants — `DisabledAndNotInstalled`, `Installing`, `CheckingForInstallation` — are never *produced* for `Custom` keys in v1 (no custom install flow, product.md invariant 26), which a `debug_assert!` in `detect_lsp_workspace_status` documents. The footer's exhaustive matches render both kinds through one path.
- **Manager** — `LspManagerModel.servers: HashMap<PathBuf, Vec<ModelHandle<LspServerModel>>>` already supports multiple servers per workspace. Registration, duplicate detection, and `LspManagerModelEvent::ServerRemoved` key on `ServerKey`.
- **Persistence** — the SQLite `workspace_language_server` table gets a `kind` column (migration `2026-05-24-180000`): the on-disk serialization of the `ServerKey` discriminant. One event variant, one upsert function, and one in-memory map (`HashMap<ServerKey, EnablementState>`) serve both kinds. The `kind` column exists because a custom may share a built-in's name (the by-name override, product.md invariant 3) while both kinds persist enablement in the same `language_server_name` column — built-ins as serialized `LSPServerType` variant names (`"RustAnalyzer"`, `"GoPls"`, …), customs as `descriptor.name`. `kind` keeps the two populations disjoint without renaming existing built-in rows.
- **Footer** — one render path, keyed by `ServerKey`. Display name and Enable dispatch come from the key; the same UI affordances (status indicator, Enable button, inline error) serve both kinds.

The only structural two-ness that remains is the config payload — `LspServerConfigKind { BuiltIn(LspServerConfig), Custom(CustomLspServerConfig) }` — because the two configs genuinely carry different data (built-ins: binary discovery and install candidates; customs: descriptor and placeholder expansion). `LspServerConfigKind::key() -> ServerKey` derives identity from the shared enum; no other code re-derives the built-in/custom split.

The cost of threading `ServerKey` through is payload changes to three `LspRepoStatus` variants and one persistence event, touching every existing pattern-match site (~24 for `LspRepoStatus`) — all compiler-guided, landed as mechanical commits at the start of their phases. The benefit is a single behavioral pipeline: no sibling enums, maps, or events to keep in sync, exhaustive `match`es surface every divergence point explicitly, and follow-ups that apply to both kinds (custom install flow, management surface) extend one pipeline instead of converging two.

### Design decision: a shared language registry (product.md invariant 33)

The `languageId` a custom server receives comes from a **shared filetype → language registry**, not a table private to the LSP crate. This mirrors how VS Code and neovim are built: a single language *identity* (VS Code's language `id`, neovim's `filetype`) is the key every consumer looks up by — several detection kinds map a file onto it, and consumers (LSP, syntax grammar, icons, snippets) read from it. Warp already has this central mapping — the `languages` crate's `Language` struct (`grammar`, `highlight_query`, `indents_query`, `comment_prefix`, `bracket_pairs`, `display_name`) — but it is unreachable from `lsp` and has no `languageId` field. This work extracts the **lightweight identity + detection layer** into a shared home and adds `languageId`.

**Prior art.** The two editors bracket the design space.

*VS Code* — one **language declaration** bundles every detection kind plus presentation and a pointer to editing config; every consumer looks up by the `id`, and grammars / servers / snippets reference it. The `contributes.languages` schema (`ILanguageExtensionPoint`):

```ts
interface ILanguageExtensionPoint {
  id: string;                             // the id every consumer looks up by
  extensions?: string[];                  // detection: ".rb", ".rake"
  filenames?: string[];                   // detection: "Gemfile"
  filenamePatterns?: string[];            // detection: "*.rb.erb"
  firstLine?: string;                     // detection: shebang/content regex
  aliases?: string[];                     // presentation: display names
  mimetypes?: string[];
  icon?: { light: string; dark: string }; // presentation
  configuration?: string;                 // → language-configuration.json
}                                         //   (comments, brackets, autoClosingPairs,
                                          //    indentationRules, folding, wordPattern)
```

*neovim* — no bundling struct: detection is three keyed tables mapping to a bare **filetype string** (or a function), and all behavior (LSP via `lspconfig` `filetypes`, syntax, `ftplugin`) is attached elsewhere, keyed on that string. `vim.filetype.add`:

```lua
vim.filetype.add({
  extension = { foo = "fooscript" },                  -- ext      → filetype (or fn)
  filename  = { ["Gemfile"] = "ruby", [".foorc"] = "foo" }, -- basename → filetype
  pattern   = { [".*/etc/foo/.*"] = "foo" },          -- path glob → filetype
})
-- value may also be a function: fun(path, bufnr, ...captures): (ft, on_detect?)
```

Warp's `LanguageEntry` (below) follows VS Code's **bundled-declaration** shape — one entry per language, keyed by `id`, carrying detection inputs and per-consumer fields — but as static Rust data: no config-file pointer and no detection functions in v1, and consumer-specific data (`language_id`, `grammar`) as `Option` fields rather than a `configuration` file reference. neovim's contribution to the design is the reminder that the identity is ultimately just a **string** every consumer keys off — which is exactly what `language_id` (LSP) and `grammar` (tree-sitter) are in our entry.

**Placement — a new leaf crate `crates/language_registry`.** It holds only plain data (identity strings, extension/filename lists, and per-consumer string fields) and depends on nothing heavy — no `arborium`, no `warp_editor`. Both `languages` and `lsp` depend **down** onto it; neither depends on the other. This is required, not cosmetic: `lsp` is compiled for `wasm32` (see the `wasm32` cfgs throughout `crates/lsp/src/`), while `languages` pulls `arborium` (native tree-sitter parsers built via `cc`) and `warp_editor` — neither wasm-compatible. A leaf crate of strings compiles fast, builds on every target `lsp` targets, and keeps the two sibling subsystems compiling in parallel. (Alternative considered: split `languages` into `languages-core` + the heavy grammar crate — same effect, deferred as the larger refactor.)

**Entry shape.** Two extension axes — detection *inputs* (how a file is matched) and per-consumer *fields* (what each consumer reads) — both growable without reshaping callers:

```rust
pub struct LanguageEntry {
    pub id: &'static str,                       // the id every consumer looks up by
    pub extensions: &'static [&'static str],    // detection input (v1)
    pub filenames:  &'static [&'static str],    // detection input (v1) — literal basenames
    // first_line: reserved detection input (empty in v1; see Extensibility)
    pub language_id: Option<&'static str>,       // LSP reads this (VS Code languageId)
    pub grammar:     Option<&'static str>,       // syntax highlighting reads this (grammar name, not the parser)
    // icon, display_name, comment_prefix, brackets: reserved fields (later migration)
}

/// Precedence: literal filename → extension → first-line regex (VS Code order,
/// minus filename globs — not needed at v1's extension/filename parity scope).
/// v1 populates only the filename and (lowercased) extension arms; first-line is inert.
pub fn resolve(path: &Path, first_line: Option<&str>) -> Option<&'static LanguageEntry>;
```

Both fields are independently `Option`: Warp bundles 33 grammars but only 5 built-in LSPs, and a custom LSP can target a language with a grammar but no built-in server, or vice versa. The registry holds the grammar **name** (`"golang"`), a string — the heavy `ParserGrammar`/`Query` stay in `languages`, which turns the name into a parser at load time. So neither the registry nor `lsp` ever links `arborium`.

**Migration scope (this feature).** Two consumers move onto the registry now:

- **LSP `languageId`** — both custom *and* built-in servers resolve `languageId` via `resolve(path, None).and_then(|e| e.language_id)`. Built-in `did_open` switches from `LanguageId::lsp_language_identifier()` to the registry, so there is **one source of truth** (this supersedes the earlier two-sources-kept-in-agreement-by-test design). Built-in *server dispatch* — `LanguageId::from_path → LSPServerType` — is unchanged; only the languageId string moves.
- **Tree-sitter grammar selection** — the editor's extension→grammar lookup (`languages::language_by_filename_parts`) resolves the grammar *name* from the registry, then loads the parser as it does today. No user-visible change; the same files highlight the same way.

Deferred to follow-ups (named in `## Follow-ups`), not v1: file icons (`app/src/code/icon.rs`), the agent `ProgrammingLanguage` map (`TODO(INT-605)`), notebook code-block languages, and `warp_util` text/binary classification.

**Extensibility.** The v1 API is forward-only — `resolve(file) → entry` — but is deliberately shaped so the other Warp language mappings can migrate onto it **additively**, without reshaping v1 callers. Three growth axes, none precluded:

- *New detection inputs* — first-line/shebang lookup: `resolve` already takes `first_line: Option<&str>`, the precedence order is fixed, and the arm is inert in v1 (consistent with invariants 20–21). Filename-glob patterns land the same way.
- *New fields* — icons, display names, comment/bracket config are added as `Option` fields on the entry; each consumer reads its own.
- *New access directions* — consumers that key by **alias** or need the **inverse** mapping (the agent's `ProgrammingLanguage` / `TODO(INT-605)`: id → extension and `c#`/`golang` alias → id; notebook gating by display name) are served by an `aliases` field plus a reverse index (`alias → id`, `id → representative extension`) built from the same data, offered as helpers alongside `resolve`. v1 does not build these, but the forward-only shape must not block them — hence they are called out here.

Not every current mapping belongs in the registry: `warp_util`'s text/binary/markdown classification (`file_type.rs`) stays separate — it classifies *all* files including binaries (a content/byte concern), which the language registry does not model.

The registry is **not user-extensible in v1** — it is compiled-in data with no settings surface; user-defined mappings are a separate cross-feature effort (product.md invariant 33).

## Proposed changes

### Phase 1 — Foundation (no behavior change yet)

Goal: land data types, parsing, matching, substitution as pure modules with full unit-test coverage. Nothing in the LSP runtime path consumes them yet.

New module `crates/lsp/src/descriptor.rs`:

```rust
pub struct LspServerDescriptor {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub filetypes: Vec<LspFiletypePattern>,
    pub env: BTreeMap<String, String>,
    pub initialization_options: Option<serde_json::Value>,
}

pub struct LspFiletypePattern {
    pub pattern: String,                  // raw user-typed pattern (e.g. "*.rb")
    matcher: globset::GlobMatcher,        // private; compile-once, dispatched via is_match()
}
```

Sibling modules:

- `crates/lsp/src/descriptor/parse.rs` — serde-driven TOML parsing. Accepts the `[[editor.language_servers]]` array. Each `filetypes` entry is a string pattern (see invariant 1). Classification between `Glob` and `LiteralBasename` is by metacharacter presence (`*`, `?`, `[`). **Unknown-field logging (invariant 24):** before `serde_json::from_value::<RawDescriptor>(...)`, the parser walks the entry's top-level keys and emits one `log::warn!` per ignored key — formatted as `"editor.language_servers entry <ref>: unknown field '<key>' ignored"`, where `<ref>` is the entry's `name` **only when that name already satisfies invariant 1's character set** (`[A-Za-z0-9._-]{1,64}`) and otherwise the entry's source index (e.g. `#3`) — for any key not in the known set `{name, command, args, filetypes, env, initialization_options}`. This walk runs before `validate.rs`, so the name has not yet been validated; gating on the charset inline (a cheap check, not the full validation) keeps a malformed name from echoing user-typed bytes into the log, per the logging policy below. The known-field list is single-sourced from the field names of `RawDescriptor` (extracted at compile time via a small declarative macro or a unit test that asserts the constant set against `RawDescriptor`'s reflection) so adding a recognized field automatically extends the allowlist. Serde's default behavior is to silently drop unknown fields; the explicit walk is what makes invariant 24's "logged with a warning" promise real — `deny_unknown_fields` is intentionally NOT used because invariant 24 commits to keeping unknown fields forward-compatible.
- `crates/lsp/src/descriptor/matcher.rs` — `pub fn match_descriptor<'a>(descriptors: &'a [LspServerDescriptor], file_path: &Path) -> Option<LspMatchedDescriptor<'a>>`. Returns the **last**-in-source-order match per invariant 4 (a later entry overrides an earlier one). Computes the LSP `languageId` per invariants 1 and 33 via the shared registry: `language_registry::resolve(path, None).and_then(|e| e.language_id)`, else the file's lowercase extension, else its literal basename. Registry lookup lowercases the extension (its keys are lowercase; matching is case-insensitive per invariant 20, so `.H` hits the same entry as `.h`). Descriptor-pattern matching (which entry claims the file) applies to the file's **basename only** (`path.file_name()`), per invariants 1 and 20 — the compiled matchers are never handed the full path.
- `crates/language_registry` (**new leaf crate**) — the shared filetype → language registry (product.md invariant 33; full rationale in the design decision above). Holds the static `LanguageEntry` table and `pub fn resolve(path: &Path, first_line: Option<&str>) -> Option<&'static LanguageEntry>`. Plain-data only — no `arborium`, no `warp_editor` — so `lsp` (incl. its wasm target) and `languages` both depend down onto it. The matcher above reads the `language_id` field; `languages` reads the `grammar` field; `TextDocumentService::did_open` receives the resolved id from the caller (see Phase 3).

  **Seeding the registry is part of Phase 1; the feature does not work for new languages without it.** Descriptors carry no language-id field (invariant 33), so the registry is the only source of a correct `languageId` for a custom server. A language absent from the registry degrades to the raw lowercase extension (`ex` instead of `elixir`) — tolerated by single-language servers that ignore `languageId`, mis-handled by strict ones. Per the coverage commitment (invariant 33), the `language_id` field ships seeded at parity with the **union** of (1) VS Code's core language identifiers and their default extension/filename associations, and (2) neovim's `vim.filetype` `extension` and `filename` tables (its first-line and path-pattern detection excluded, per invariants 20–21); for a neovim-only language the id is the one its dominant VS Code extension registers (`zig`, `elixir`, `gleam`, …). A **parity test** enumerates both sources and asserts the registry is a superset of each, so coverage cannot silently regress; a rustdoc comment states the sourcing rule. The `grammar` field is seeded by migrating `languages`' existing extension→grammar associations verbatim (bounded by the 33 bundled grammars — not expanded to parity), with a test asserting every current `language_by_filename_parts` mapping is preserved. There is no built-in-enum "agreement" test any more: built-ins read the registry directly, so there is one source, not two to reconcile.
- `crates/lsp/src/descriptor/placeholder.rs` — substitution engine for invariants 5, 6:
  - `pub fn expand(input: &str, ctx: &LspPlaceholderContext) -> String`
  - `LspPlaceholderContext` holds `workspace_root: &Path` and `workspace_cache_dir: &Path` (the latter already resolved to the per-(server, workspace) directory — see `lsp_server_cache_dir` below), plus a logger handle for "unknown placeholder" warnings — de-duplicated so each unknown name is logged **once per launch** (invariant 6), not once per occurrence across `args`/`env`/`initialization_options`.
  - Delegates to the in-tree `crates/handlebars` engine — the same one tab configs and MCP rendering use — for `{{name}}` substitution. We use `handlebars::get_arguments` to discover referenced names, build a `HashMap<String, String>` populated with the resolved values for known names (and env vars discovered via the `env_` prefix), and call `handlebars::render_template`. Two distinct miss cases (invariants 5 vs 6): a name with the `env_` prefix is always **recognized** — if the variable is unset, the map gets an **empty string** for it and the miss is logged (invariant 5's "undefined variable expands to the empty string"). Only names that match no recognized placeholder at all are left absent from the map, so the engine leaves them in place verbatim (invariant 6).
  - The Handlebars parser only allows alphanumeric / `-` / `_` in argument names. Env-var lookups therefore use the `env_` prefix instead of a colon: `{{env_HOME}}`. The descriptor module strips the prefix and looks up the env var. Names with whitespace inside the braces are invalid per the engine; the spec adopts that constraint.
  - `~` / `~/` at position 0 expand to the user's home directory via `shellexpand::tilde`. Embedded `~` is untouched.
  - `pub fn expand_json(input: &serde_json::Value, ctx: &LspPlaceholderContext) -> serde_json::Value` walks `initialization_options` and calls `expand` on string leaves only.
  - **Redaction at log boundaries (invariant 32).** `crates/lsp` cannot directly depend on `app/src/settings/privacy.rs::CustomSecretRegex` — `app/` is downstream of `lsp/` in the workspace dependency graph. So the redactor is **injected from app at the LSP boundary** rather than imported. Concretely: `crates/lsp` defines a trait `pub trait LogRedactor: Send + Sync { fn redact_for_log<'a>(&self, value: &'a str) -> Cow<'a, str>; }`, and `LspPlaceholderContext` (or equivalent app-boundary type — confirm during Phase 4 implementation) carries an `Arc<dyn LogRedactor>` populated at construction by `app/`'s wiring, where `CustomSecretRegex` lives. Callers in `crates/lsp/src/manager.rs` and `crates/lsp/src/service.rs` call the trait method before reaching `log::info!` / `log::warn!` / `log::error!`. What goes where (per invariant 32): ✅ logged verbatim, no redaction needed — descriptor `name`, `env` *keys*; ✅ logged verbatim but PII-bearing — resolved `workspace_root` and per-server, per-workspace `workspace_cache_dir` absolute paths (these can contain usernames or repo names but are not secret-redaction targets, matching how built-in LSP spawn already logs paths today); ⚠️ must pass through the injected redactor — substituted `command`, substituted `args` strings, and substituted `env` *values*; ❌ never log verbatim — substituted `initialization_options` JSON: serialize it and run the serialized form through the injected redactor before logging, the same treatment as command/args/env values (product.md invariant 32's string leaves are logged after redaction; a keys-only structural summary is not sufficient). Verification during Phase 4: (1) the injection point exists or is added at the natural boundary, (2) `CustomSecretRegex` is a log-time filter and not a UI-display-only filter.
- `crates/lsp/src/descriptor/validate.rs` — runs after parse and produces `Vec<LspDescriptorError>`. Per invariant 23, **any** validation failure rejects the entire `editor.language_servers` setting (all-or-nothing); `SettingsFileError::InvalidSettings` receives a single bare `editor.language_servers` entry (see the bullet on `app/src/settings/mod.rs:69-116` above), and per-entry detail (which entry index, which field, why it failed) is emitted via `log::warn!` so users can find the offending entry without that detail surfacing in the banner. Validates the `name` character set and length per invariant 1 (1–64 chars from `[A-Za-z0-9._-]`, not `.`/`..`, no leading `.` or `-`). Name **uniqueness checks are case-insensitive** (ASCII fold). A `name` may match a built-in's display name — that is the by-name override (product.md invariant 3), not a validation error. Validates `command`'s trust boundary per invariant 1, on the **literal** form only: after `~`/`~/` home expansion via `shellexpand::tilde`, the value must be either **absolute** or a **bare name** (no `/` or `\` characters). "Absolute" is platform-specific: on Unix, starts with `/`; on Windows, starts with a drive letter followed by `\` or `/` (e.g. `C:\path`, `C:/path`), OR is a UNC path (`\\server\share\path` or `//server/share/path`). Windows root-relative paths (`\path`, `/path` without a drive) are **rejected** — they depend on the current drive at exec time, which is exactly the cwd-dependent resolution invariant 1 is preventing. Anything else (`./server`, `bin/server`, `..\\server`, Windows `\path`) is rejected with `LspDescriptorErrorKind::UnsafeCommandPath { command, reason }`. Per invariant 16, the **post-substitution** `command` is not revalidated against this rule — `{{env_VAR}}` placeholders inside `command` may expand at launch time to any string, including paths that the literal-form check would reject. That is intentional: the user's environment is user-controlled, the same as the contents of `settings.toml`, and Warp does not second-guess user-controlled inputs elsewhere (MCP, terminal, built-in LSP PATH). The validator's job here is to catch obvious settings-file errors, not to defend against the user's own environment. Also checks empty `filetypes`, entries that are not **non-empty strings** (an empty array, a non-string element like `123`, or an empty-string element `""` are all invariant-23 errors), and uses `globset::GlobBuilder::case_insensitive(true).build()` to validate glob patterns. Patterns classified as **literal basenames** (no `*`, `?`, `[` — invariant 1) are passed through `globset::escape` before compiling, so characters that are glob-significant to globset but not to invariant 1's classifier (`{`, `}`, `\`) match literally instead of being brace-parsed or rejected. Rejects `**` and brace alternation explicitly — in glob-classified patterns only — to match product.md invariant 1.

  **Parse/validate-time logging policy (invariant 32).** All `log::warn!` calls emitted from `parse.rs` and `validate.rs` run at settings-load time, in `crates/lsp/`, before any `LogRedactor` has been injected from `app/` (the redactor is constructed at LSP-spawn time per the Phase 3 redaction note). To avoid leaking secrets that may legitimately appear in user-typed fields (e.g. `command = "/usr/local/bin/server-with-AKIA..."`, `args = ["--token=..."]`), parse/validate log lines include only **two safe categories**: (1) the offending entry's `name` **only when it already satisfies invariant 1's character set** (`[A-Za-z0-9._-]{1,64}` — checked inline before logging, since parse-time logs run before `validate.rs` has enforced it), which then can carry no secrets — otherwise the entry's source index (e.g. `#3`), never the raw unvalidated name; (2) the error variant name (`DuplicateName`, `EmptyFiletypes`, `MissingCommand`, `MalformedEntry`, `InvalidGlob`, `UnsupportedGlobFeature`, `UnsafeCommandPath`, etc.) and any **structural** detail that does not echo user-typed values back (e.g. the *index* of the offending entry, the *field name* that failed, but not the field's raw value). Raw user-typed strings (the offending pattern, the underlying serde error message, the unsafe command path itself) stay inside the `LspDescriptorError` struct's data fields but are **not** stringified into the `log::warn!` output. Implementation: `LspDescriptorError`'s `Display` impl is split — one stable, redaction-safe summary used by `log::warn!`, and the full-detail formatter reserved for UI surfaces that have their own redaction (or for tests, gated by `#[cfg(test)]`).

Helpers in adjacent crates:

- `crates/warp_util/src/path.rs` — `pub fn workspace_hash(path: &Path) -> String` (first 8 bytes of SHA-256 over the path's `to_string_lossy` bytes, encoded as 16 lowercase hex chars). The hash powers the per-workspace path segment of `{{workspace_cache_dir}}` (see `lsp_server_cache_dir` below). `app/src/code/lsp_logs.rs` has its own private `hash_workspace_path` (`:27`) for LSP log filenames; unifying the two onto `workspace_hash` is optional cleanup, not required by this work. Leading `~` / `~/` expansion uses `shellexpand::tilde` (existing workspace dep), not a custom helper.
- `crates/warp_core/src/paths.rs` — `pub fn lsp_server_cache_dir(server_name: &str, workspace_root: &Path) -> PathBuf` returning `cache_dir().join("lsp").join(<hashed-name>).join(<hashed-workspace-root>)`, where each hashed segment uses the `workspace_hash` shape (first 8 bytes of SHA-256, 16 lowercase hex chars) — one over `server_name`, one over the workspace root. The workspace-root segment makes `{{workspace_cache_dir}}` unique per (server, workspace) (product.md invariant 5), so JDTLS-style `-data` directories never collide across workspaces. Hashing both segments decouples the cache-dir filesystem path from `name`'s allowed character set and the workspace path's characters — eliminating platform-specific edge cases like Windows-reserved device names (`CON`, `NUL`, `COM1`, etc.), trailing-dot stripping, and extreme-length issues. The descriptor's `name` remains the user-visible identifier in the footer and in Warp's own logs. What the LSP server itself receives via `{{workspace_cache_dir}}` substitution is a path ending in the two hashed segments (e.g. `~/Library/Caches/dev.warp.Warp-Stable/lsp/8a3f...c0e1/2b7d...9f04/`), not the literal `name`; the LSP never sees `name` directly. Both hashes are deterministic, so the same `name` in the same workspace always resolves to the same cache directory across launches.

Tests: unit tests in `crates/lsp/src/descriptor/`. Covers invariants 1, 2, 4, 5, 6, 20, 21, 22, 23, 24, 32 (the parse/validate-time logging policy), and 33 (the shared language registry: `resolve` + parity/grammar-migration tests).

**Pause for review after Phase 1.**

### Phase 2 — Settings schema + descriptor loading + hot-reload

Goal: surface `[[editor.language_servers]]` in the settings pipeline, validate, populate the parsed descriptors in app state, react to file changes. Still no LSP runtime hookup.

Phase 2 uses `define_settings_group!` the same way every other settings group does (precedent: `app/src/settings/ai.rs::AISettings` uses the macro with `type: Vec<AgentModeCommandExecutionPredicate>` — exact same shape as our `Vec<LspServerDescriptor>`). The macro generates the settings group struct, the change-event enum, the `register_setting` call with all five callbacks, the `SettingSchemaEntry` submission, and the `LocalPreferencesUpdated` event emission.

Phase 2 lands as **three small, independently reviewable sub-phases**. Each compiles on its own and ends in a pause-for-review checkpoint.

#### Phase 2a — `JsonSchema` derives on descriptor types

Adds `JsonSchema` derives to the Phase-1 types so the schema generator can read them. Follows the codebase convention from `privacy.rs::CustomSecretRegex` — derive on the runtime type, with `LspFiletypePattern` presenting as a plain string (`#[serde(try_from = "String", into = "String")]`, `#[schemars(with = "String")]` — see Phase 5). An initial version is implemented on the code PR (#10700), pending this review.

#### Phase 2b — `define_settings_group!` invocation + `LspServerDescriptors` newtype

New module `app/src/settings/language_servers.rs` introduces a thin newtype around `Vec<LspServerDescriptor>` and registers it through the macro. The newtype exists because the default blanket `Vec<T>` `SettingsValue` impl deserializes elements independently, which cannot express invariant 23's all-or-nothing contract or invariant 24's unknown-field warning walk. The newtype overrides `from_file_value` to route through `descriptor::parse::parse_entries`, which compiles glob matchers from the user's bare-string patterns, collects per-entry errors for the all-or-nothing decision, and emits the unknown-field warnings. This follows the hand-rolled-`SettingsValue` precedent of `AgentModeCommandExecutionPredicate` (struct at `crates/cloud_object_models/src/ai_execution_profile.rs:258`, `SettingsValue` impl at `:288`; registered as a `Vec<…>` setting in `app/src/settings/ai.rs:982`) and `ToolbarCommandMap` (`app/src/settings/ai.rs:705`, impl at `:754`).

`LspServerDescriptors` is a transparent newtype over `Vec<LspServerDescriptor>` with the standard derives (`Serialize`, `Deserialize`, `JsonSchema`, `Default`). Its `SettingsValue::from_file_value` implements the all-or-nothing contract from invariant 23:

- If the underlying value is not an array → return `None` (wrong-type is a settings error per invariant 23).
- Otherwise, call `descriptor::parse::parse_entries` over the array, which yields a `{ descriptors, errors }` pair.
- If any per-entry validation errors exist → emit one `log::warn!` per entry containing the entry index and reason (per-entry detail stays out of the banner per the bullet on `app/src/settings/mod.rs:69-116` above), then return `None`.
- Otherwise return `Some(LspServerDescriptors(descriptors))`.

Returning `None` causes the macro's generated `load_fn` to surface `editor.language_servers` as a single bare key in `SettingsFileError::InvalidSettings`, which is what the banner reads.

The macro invocation registers our setting alongside the existing pattern:

```rust
define_settings_group!(LanguageServersSettings, settings: [
    language_servers: LanguageServers {
        type: LspServerDescriptors,
        default: LspServerDescriptors::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "editor.language_servers",
        description: "User-configured language servers for the editor.",
    },
]);
```

`surface: SettingSurfaces::GUI` because the editor pane and LSP footer exist only in the desktop GUI; the `warp_tui` front-end has no code editor. Widening to `ALL` is a one-line change if that changes.

The macro generates: the `LanguageServersSettings` singleton struct holding the value, an `LanguageServersSettingsChangedEvent` enum, the `register()` method that wires up the `register_setting` callbacks, the `SettingSchemaEntry` for build-time schema generation, and the `LocalPreferencesUpdated` event emission on changes. `LanguageServersSettings::register(ctx)` gets called during app init alongside the other settings groups.

Domain methods are added to the generated group via a normal `impl` block in the same file — same pattern as `AppEditorSettings::toggle_cursor_blink` at `app/src/settings/editor.rs:288`:

```rust
impl LanguageServersSettings {
    /// Returns the last user-configured descriptor (in source order) whose
    /// filetypes match the given path, or `None` if no custom descriptor
    /// claims the file.
    pub fn match_for_path<'a>(&'a self, path: &Path) -> Option<LspMatchedDescriptor<'a>> {
        descriptor::matcher::match_descriptor(&self.language_servers.value().0, path)
    }

    /// True when a custom entry's `name` case-insensitively equals the given
    /// built-in's display name (`binary_name()`) — that built-in is replaced
    /// entirely and must not be dispatched (product.md invariant 3, by name).
    pub fn suppresses_builtin(&self, server_type: LSPServerType) -> bool {
        self.language_servers.value().0.iter()
            .any(|d| d.name.eq_ignore_ascii_case(server_type.binary_name()))
    }
}
```

Verification: `cargo run --bin generate_settings_schema` now emits an `editor.language_servers` section in the schema artifact; the settings group registers cleanly during app init; `match_for_path` returns `None` until the user writes an entry, then returns the matching descriptor.

**Pause for review after Phase 2b.**

#### Phase 2c — Validation error surfacing + integration tests

Plug parse errors into the existing settings-error banner by following the codebase's standard `Vec<T>: SettingsValue` pattern: `LspServerDescriptors::from_file_value` returns `None` if `parse_entries` produced any errors, which causes the macro's generated `load_fn` to surface `editor.language_servers` in `SettingsFileError::InvalidSettings`. Per-entry reasons (which entry, which field) go to `log::warn!` so they are findable when the banner alone isn't enough. This matches `AgentModeCommandExecutionPredicate` (`crates/cloud_object_models/src/ai_execution_profile.rs:258`, a `Vec<...>` setting element whose per-element `from_file_value` returns `None` on bad regex, flowing through the blanket `impl<T: SettingsValue> SettingsValue for Vec<T>` at `crates/settings_value/src/lib.rs:138-146`).

Trade-off: per-entry detail does not appear in the in-app banner. A typo in one entry takes down the whole `[[editor.language_servers]]` array until the user fixes it. This is the behavior product.md invariant 23 specifies; partial success / per-entry banner detail can be added as a follow-up if user feedback warrants the extra infrastructure.

Integration tests in `crates/integration/src/test/` (mirror `settings_file_hot_reload.rs`'s pattern) covering parse → settings group → events:

- empty array (no descriptors, no errors, no events on save)
- one valid entry (settings group populates, `LanguageServersSettingsChangedEvent::LanguageServers` fires once on load)
- one invalid entry (whole setting fails to load; `editor.language_servers` surfaces in `SettingsFileError::InvalidSettings`; per-entry reason appears in the log)
- edit-then-save (change event fires; settings group state updates)
- remove-then-save (change event fires; settings group state shrinks)

Covers product.md invariants 2, 23, 24, plus the settings-layer half of 8 and 19 (state updates on save with no process interaction); their runtime behavior — next-open pickup, running servers unaffected — is verified by Phase 4's integration tests.

**Pause for review after Phase 2c.**

---

After all three sub-phases land, the settings group mirrors the user's `[[editor.language_servers]]` in real time, parse errors surface in the existing settings UI, and the schema artifact for external editors is published. **No LSP runtime hookup yet** — that comes in Phase 3, which reads descriptors on demand at file-open time (no settings-group subscription; see Phase 3 "No settings-group subscription" and product.md invariant 19).

### Phase 3 — Custom-server runtime path

Goal: enable a user-configured descriptor to spawn a server, exchange JSON-RPC, and tear down. Built-in path is untouched.

**The LSP runtime below the spawn line is identity-free** (verified by exploration). `LspService` (`crates/lsp/src/service.rs`) and `transport::ProcessTransport` have zero `LSPServerType` references; `repo_watcher` only uses `config.initial_workspace()`. Customs reuse all of this directly. Identity matters at four places: spawn-time config construction, manager-internal duplicate detection, display, and `TextDocumentService::did_open`'s `language_id` derivation (currently `LanguageId::from_path(path)`, which silently fails for custom filetypes — Phase 3 changes it to take `language_id: String` from the caller, sourced the same way for both kinds — `language_registry::resolve` per invariant 33, else lowercase extension, else basename (built-ins no longer use `LanguageId::lsp_language_identifier()`; the registry is the one source)).

**`CustomLspServerConfig` parallel to `LspServerConfig`** in `crates/lsp/src/config.rs`:

```rust
pub struct CustomLspServerConfig {
    pub descriptor: LspServerDescriptor,
    pub initial_workspace: PathBuf,
    pub path_env_var: Option<String>,
    pub client_name: String,
    pub client: Arc<http_client::Client>,
    pub log_relative_path: Option<PathBuf>,
}

impl CustomLspServerConfig {
    pub(crate) async fn command_and_params(self) -> Result<ResolvedLspCommand> { ... }
}
```

`command_and_params()` builds the `Command` from `descriptor.command + descriptor.args` after running both through `placeholder::expand` (and via `shellexpand::tilde` inside that, for the leading-`~` rule). Before substitution it creates the resolved `workspace_cache_dir` via `create_dir_all` (idempotent; unconditional — no sniffing whether the entry references the placeholder), satisfying product.md invariant 5's "Warp creates the directory before launch"; a creation failure fails the launch through the same named error path as a spawn failure (invariant 18), so `{{workspace_cache_dir}}` is never substituted as a path that does not exist. The spawn environment base is Warp's process environment with `PATH` set from the interactive shell via the existing `path_env_var` threading (`CommandBuilder`), identical to built-in servers; `descriptor.env` is merged on top after expanding each value (product.md invariant 16). Sets `current_dir` to `initial_workspace`. Builds `InitializeParams` with `initialization_options` substituted via `placeholder::expand_json`; `default_init_params` is reused.

No PATH-fallback logic. Per product.md invariant 26 there is no install flow for custom servers; if `descriptor.command` does not resolve, the launch fails through the existing `LspState::Failed` path. The failure message must include the descriptor's `name` so the user can tell which custom server failed.

**`LspServerModel` config kind enum** at `crates/lsp/src/model.rs:97-107`. Today the model carries `config: LspServerConfig` directly; we swap that for an enum:

```rust
pub enum LspServerConfigKind {
    BuiltIn(LspServerConfig),
    Custom(CustomLspServerConfig),
}

pub struct LspServerModel {
    id: LanguageServerId,
    server_state: LspState,
    config: LspServerConfigKind,    // ← changed from LspServerConfig
    in_progress_tasks: HashMap<String, BackgroundTaskInfo>,
    diagnostics_by_path: HashMap<PathBuf, DocumentDiagnostics>,
    pub(crate) repo_watcher: LspRepoWatcher,
}
```

`LspServerModel::start()` (`model.rs:255-327`) branches on the kind at the single line that calls `spawn_lsp_service` (`lib.rs:71`):

```rust
let resolved = match config {
    LspServerConfigKind::BuiltIn(c) => c.command_and_params().await?,
    LspServerConfigKind::Custom(c) => c.command_and_params().await?,
};
// everything below: unchanged, operates on `resolved.command` / `resolved.params`
```

`spawn_lsp_service`'s signature changes from `(config: LspServerConfig, ...)` to `(config: LspServerConfigKind, ...)`. Other config accessors used by the model (`server_name`, `initial_workspace`, `log_relative_path`) become enum-dispatching helpers — call into the appropriate inner variant.

**Accessors on `LspServerModel`** for the manager and footer:

- `pub fn key(&self) -> ServerKey` — delegates to `LspServerConfigKind::key()`; the identity every consumer (manager, footer, persistence write-sites) uses.
- `pub fn display_name(&self) -> &str` — delegates to `ServerKey::display_name()`.
- `pub fn supports_path(&self, path: &Path) -> bool` — built-ins check `LanguageId::from_path` against `LSPServerType::languages()`; customs check the descriptor's compiled matchers against `path.file_name()` (basename only, invariants 1 and 20).

**`ServerKey` is the shared identity enum** (defined in `crates/lsp` — see the design decision section for the type). Phase 3 lands it and its first consumers:

- `LspManagerModel::server_registered(path, key, ctx)` for duplicate detection within a workspace's server vec
- `LspManagerModelEvent::ServerRemoved { workspace_root, key, server_id }` — payload type changes from `server_type: LSPServerType` to `key: ServerKey`

External subscribers to `ServerRemoved` (in `app/`) need updates when this payload changes. The compiler will surface every site. Phase 4 extends the same enum into `LspRepoStatus`, persistence, and the footer.

**`LspManagerModel::register()`** (`crates/lsp/src/manager.rs:170-203`) accepts the new kind enum:

```rust
pub fn register(
    &mut self,
    path: PathBuf,
    config: LspServerConfigKind,
    ctx: &mut ModelContext<Self>,
) -> bool
```

Built-in callers wrap their existing `LspServerConfig` in `LspServerConfigKind::BuiltIn(...)` at the call site — mechanical, compiler-guided.

**No settings-group subscription.** The manager does not subscribe to `LanguageServersSettingsChangedEvent`. Per product.md invariant 19 — and matching the existing built-in behavior where the only path to `LspServerModel::restart()` is the user-initiated footer action at `app/src/code/local_code_editor.rs:1323` — config edits do not trigger any automatic action. Descriptors live in `LanguageServersSettings::as_ref(ctx)` and are read on demand at file-open time (Phase 4's dispatch path). New entries take effect on the next matching file open; edited entries take effect on the next launch the user triggers (close-and-reopen, or an explicit restart action that already exists for built-ins).

**A custom (re)launch always rebuilds config from current settings — never reuses a running model's copy.** This is what makes invariants 8/19 mechanically hold given `ServerKey::Custom(name)` duplicate detection:

- While a custom model is **live**, its config is frozen (invariant 19), and duplicate detection by `ServerKey::Custom(name)` correctly suppresses registering a second model for that name — so edits do not disturb it.
- When the last editor pane for a filetype closes, the custom model is **stopped and removed from the workspace vec** (not left registered). Because the slot is gone, the ServerKey check no longer matches, so the next matching file open **re-registers fresh**, constructing `CustomLspServerConfig` from the *current* descriptor in `LanguageServersSettings` (looked up by name). This is the "close-and-reopen picks up new values" path.
- The **restart action**, for a custom, does not reuse the model's stored config: it re-looks-up the descriptor by name in `LanguageServersSettings` and rebuilds `CustomLspServerConfig` before respawn (built-ins rebuild from their static `LSPServerType`, unchanged). If the name is no longer present in settings, restart tears the model down without respawning.

So a live server keeps its launch-time config, and every path that starts a new process — reopen after teardown, or restart — rehydrates from settings; there is no path that respawns a stale custom config.

**Tests.** Manager unit tests covering custom-server register/start/stop/remove (mirror existing built-in tests). Mock-LSP integration test: a small Rust binary in `crates/integration/test_fixtures/` that echoes JSON-RPC `initialize`/`initialized`/`shutdown` exchanges. Run it via a custom descriptor and verify the lifecycle, including a placeholder in `args` asserted substituted before spawn (invariant 7). Covers invariants 7, 16, 17, 18 for the custom path.

**Pause for review after Phase 3.**

### Phase 4 — File-open dispatch + footer + persistence

Goal: wire the editor's file-open path to consult the custom descriptors first; extend the footer and persistence layer to handle custom servers alongside built-ins. Workspace-root resolution reuses the existing chain (`PersistedWorkspace::root_for_workspace` → `DetectedRepositories::get_root_for_path` → `path.parent()` fallback) — see `app/src/code/local_code_editor.rs:1402-1414`. Custom descriptors do not declare their own root markers.

**File-open dispatch sites.** `LanguageId::from_path` is called from 9 sites in `app/src/` (the "what LSP handles this file?" decision points). Each gets a custom-descriptor-first lookup added:

- `app/src/code/local_code_editor.rs:1391, 1435, 1479` — primary file-open routing (3 sites)
- `app/src/code/footer.rs:364, 410, 693` — status detection in the footer (3 sites)
- `app/src/code_review/code_review_view.rs:882, 927` — fallback server lookup in code review (2 sites)
- `app/src/ai/persisted_workspace.rs:368` — enablement check (1 site)
- Plus 4 callers of `LspManagerModel::server_for_path` (`code_review/code_review_view.rs:852`, `local_code_editor.rs:905`, `local_code_editor.rs:1493`, `global_buffer_model.rs:1327`) get the same pattern

All of these sites route through a single resolution helper — a **day-one requirement**, not a later refactor. No call site re-implements the sequence, and `match_for_path` / `suppresses_builtin` are consumed only by the helper, so the resolution policy has exactly one home:

```rust
// app/src/code/lsp_dispatch.rs (new module)

pub enum ResolvedLspServer<'a> {
    BuiltIn(LSPServerType),
    Custom(LspMatchedDescriptor<'a>),
}

pub fn resolve_server_for_path<'a>(
    settings: &'a LanguageServersSettings,
    path: &Path,
) -> Option<ResolvedLspServer<'a>> {
    if let Some(matched) = settings.match_for_path(path) {   // invariant 4: last match wins
        return Some(ResolvedLspServer::Custom(matched));
    }
    let server_type = LanguageId::from_path(path)?.server_type();
    if settings.suppresses_builtin(server_type) {             // invariant 3, by name
        return None;                                          // built-in replaced by a same-named custom
    }
    Some(ResolvedLspServer::BuiltIn(server_type))
}
```

Each of the 13 sites calls `resolve_server_for_path` and matches on the result: `Custom` routes to the custom server, `BuiltIn` continues into the existing built-in path, `None` means no LSP for this file. The helper lives in `app/` (not `crates/lsp`) because resolution needs the `LanguageServersSettings` group, which is unreachable from the `lsp` crate. Resolution-policy unit tests (custom beats built-in, last-in-source-order, by-name suppression, built-in fallback, no match → `None`) live on this helper — once, instead of implied at 13 call sites.

**Status — one enum, `ServerKey` payloads.** `LspRepoStatus` (`app/src/ai/persisted_workspace.rs:92-105`) serves both kinds. Its three identity-bearing variants change payload from `server_type: LSPServerType` to `server: ServerKey`:

```rust
pub enum LspRepoStatus {
    Ready,
    Enabled,
    CheckingForInstallation,
    DisabledAndInstalled { server: ServerKey },      // was server_type: LSPServerType
    DisabledAndNotInstalled { server: ServerKey },   // ditto
    Installing { server: ServerKey },                // ditto
}
```

The ~24 existing match sites update mechanically (compiler-guided; landed as its own commit at the start of Phase 4). `detect_lsp_workspace_status` (`persisted_workspace.rs:1202`) extends to compute status for `Custom` keys through the same code path.

**How `EnablementState` maps to `LspRepoStatus` for customs (product.md invariant 14).** Customs reuse `EnablementState` unchanged, including the memory-only `Suggested` variant: when a custom descriptor first matches a file in a workspace, the shared enablement map gets a `Suggested` entry under its `ServerKey::Custom` — the same candidate-detection insert built-ins use (`persisted_workspace.rs:597`), never persisted to SQLite. Status derivation is the same as for built-ins, minus the install probe: `Yes` → `Enabled` (→ `Ready` once the footer subscribes to a live server); `Suggested` and `No` → `DisabledAndInstalled { server: ServerKey::Custom(name) }`, which drives the footer's Enable affordance. v1 assumes a configured custom is installed — there is no install probe or install flow (invariant 26), so `DisabledAndNotInstalled`, `Installing`, and `CheckingForInstallation` are never produced for `Custom` keys (a `debug_assert!` in `detect_lsp_workspace_status` documents this). The `Suggested`/`No` distinction stays in the enablement map, not the status enum: `Suggested` vanishes on restart (re-detected on the next matching file open), while `No` — written only by the explicit disable actions in the footer dropdown and settings code page — persists across restarts per invariant 14.

**Footer dispatch.** `app/src/code/footer.rs` (1988 lines total) renders both kinds through one path, keyed by `ServerKey`:

- `compute_status_message()` (lines 1532-1653) — takes the resolved `ServerKey`; message text comes from `key.display_name()`
- `render_lsp_icon()` (1410-1448), `render_status_text()` (1450-1473) — drive off the resolved `LspRepoStatus`; no change beyond the payload rename
- Enable button handler `CodeFooterViewAction::EnableLSP` (lines 1812-1855) — matches on the `ServerKey`: `BuiltIn` calls the existing `enable_lsp_server_for_path`, `Custom` calls the new `enable_custom_lsp_server_for_path(path, name)`. Button label is `format!("Enable {}", key.display_name())` for both kinds.

  **Trust boundary.** The Enable CTA is intentionally unchanged from the built-in flow and is **not** a per-launch authorization prompt for the resolved `command`/`args`. The user's `settings.toml` is the trust boundary: by writing an `[[editor.language_servers]]` entry, the user has already authorized what `command` runs (and what env values it inherits per invariants 5 and 16). The Enable CTA confirms only that the user wants this server attached *for this workspace* — exactly the same semantics it has for built-in servers today. Users who want to inspect what a custom server will run can read their own `settings.toml` (it is plain text, in a known location per the spec) and trace any `{{env_VAR}}` placeholders against their environment. Adding an in-app pre-launch verification surface (e.g., resolved-command preview before Enable) is out of scope for v1 — see the inspection/management surface in product.md invariant 31.
- Button-label dispatch `button_label_for_cta_statuses` (lines 659-680) — its `LspRepoStatus` matches update for the `ServerKey` payload; no custom-specific branch needed

Per product.md invariant 10, no new footer affordances or copy. Visual parity for both kinds.

**Persistence — schema discriminates on `kind`.** Three migrations shape the SQLite table `workspace_language_server`: `2025-10-31-201353_add_workspace_language_server/` (initial), `2025-11-11-230915_change_workspace_language_server_enabled_to_text/` (typing fix), and `2026-05-24-180000_add_kind_to_workspace_language_server/` (adds the kind discriminator):

```sql
CREATE TABLE workspace_language_server (
    id INTEGER PRIMARY KEY,
    workspace_id INTEGER NOT NULL REFERENCES workspace_metadata(id),
    language_server_name TEXT NOT NULL,
    enabled TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'BuiltIn'
);
-- The uniqueness/upsert key becomes the full triple. The pre-existing unique
-- index on (workspace_id, language_server_name) is dropped and recreated to
-- include kind, so a custom row and a built-in row that share a name coexist:
CREATE UNIQUE INDEX workspace_language_server_key
    ON workspace_language_server (workspace_id, kind, language_server_name);
```

Built-in rows: `kind = 'BuiltIn'`, `language_server_name` = serialized `LSPServerType` variant name (`"RustAnalyzer"`, `"GoPls"`, etc.). Custom rows: `kind = 'Custom'`, `language_server_name = descriptor.name` (e.g. `"ruby-lsp"`). The `(workspace_id, kind, language_server_name)` triple is the effective key — built-in and custom rows occupy disjoint subspaces, so a custom can carry the same string as a built-in variant name without an on-disk conflict. A custom may share a built-in's name (the by-name override, product.md invariant 3); the `kind` triple keeps its enable/disable rows disjoint from the built-in's even when the names coincide. **This requires `kind` in every predicate, not just conceptually:** the migration widens the unique index to the triple (above), the upsert's `ON CONFLICT` target is `(workspace_id, kind, language_server_name)`, and every read `WHERE` filters on `kind` alongside workspace and name. The pre-existing path keyed only on `(workspace_id, language_server_name)` is updated at each site the compiler surfaces. A regression test covers a custom named exactly a serialized built-in variant (e.g. `RustAnalyzer`): enabling/disabling that custom must not read or overwrite the built-in `RustAnalyzer` row for the same workspace, and vice versa.

**Persistence event shape.** Application code writes enablement state by emitting the existing `ModelEvent::UpsertWorkspaceLanguageServer` (`app/src/persistence/mod.rs:378`), whose payload changes from `lsp_type: LSPServerType` to the shared key:

```text
ModelEvent::UpsertWorkspaceLanguageServer {
    workspace_path: PathBuf,
    server: ServerKey,           // BuiltIn → kind = 'BuiltIn', variant name; Custom → kind = 'Custom', descriptor name
    enabled: EnablementState,
}
```

The dispatch arm at `app/src/persistence/sqlite.rs:750-755` and the existing `upsert_workspace_language_server` function (`sqlite.rs:1499`) serialize the key: `ServerKey::BuiltIn` writes `kind = 'BuiltIn'` with the serialized `LSPServerType` variant name, `ServerKey::Custom` writes `kind = 'Custom'` with the descriptor `name`. One event, one arm, one upsert — no sibling variants or functions. The write site (footer Enable-button handler, via `app/src/ai/persisted_workspace.rs`) obtains the key from the `ResolvedLspServer` returned by `resolve_server_for_path` (defined in the file-open dispatch section above).

The in-memory cache on `Workspace` (`persisted_workspace.rs:132-134`) — currently `language_servers: HashMap<LSPServerType, EnablementState>` — is re-keyed to `HashMap<ServerKey, EnablementState>`. Existing built-in read/write sites wrap their `LSPServerType` in `ServerKey::BuiltIn` (compiler-guided); customs use `ServerKey::Custom(name)` through the same accessors.

**Startup / read path.** Loader `app/src/persistence/sqlite.rs::get_all_workspace_language_servers_by_workspace` reads the `kind` column and reconstructs each row's `ServerKey` (`'BuiltIn'` rows parse `language_server_name` as an `LSPServerType` variant; `'Custom'` rows carry it as the descriptor name), populating the single map through `PersistedData` at app startup. A `'Custom'` row whose name no longer matches any configured descriptor stays in the map harmlessly — it only gates enablement if a matching descriptor reappears.

`has_enabled_lsp_server_for_file_path` (`persisted_workspace.rs:367`) extends by resolving the path through `resolve_server_for_path` and consulting the shared enablement map under the resolved `ServerKey`; `None` (no match, or a built-in suppressed by a same-named custom entry per invariant 3) reports no server.

**Telemetry.** `app/src/code/lsp_telemetry.rs` already uses `server_type: String` as the event field type, sourced today from `LSPServerType::binary_name()`. Both kinds source from `ServerKey::display_name()`. No event-schema changes.

**Tests.** Integration tests in `crates/integration/src/test/` covering:

- Open `.rb` file with a Ruby `[[editor.language_servers]]` entry → custom server spawns at the workspace root (resolved via the existing `PersistedWorkspace`/`DetectedRepositories` chain), footer shows Enable button labeled `"Enable ruby-lsp"`, accepting persists per-workspace
- Open `.rs` file with a custom entry whose `filetypes = ["*.rs"]` → custom runs, built-in rust-analyzer does not spawn (invariant 3, by filetype)
- Custom entry named `rust-analyzer` with `filetypes = ["*.toml"]` → opening a `.rs` file spawns nothing and the footer shows no built-in Enable affordance (built-in suppressed by name); opening a `.toml` file routes to the custom (invariant 3, by name)
- Remove the custom `.rs` entry from settings → running custom keeps running (invariant 19); next `.rs` file open in a *new* workspace routes back to built-in rust-analyzer
- Edit a running custom's `args` in settings.toml → no restart
- Open a `.zig` file with no matching custom entry and no built-in → footer shows the existing "Language support is unavailable" state (invariant 9)
- Two windows on the same workspace root with an enabled custom → one server process shared; enabling/disabling in one window applies to the other (invariant 15)
- Ignore the Enable affordance for a matched custom → nothing persisted (`Suggested` stays memory-only), affordance reappears after restart; explicitly disable an enabled custom via the footer dropdown → `No` persists across restarts and the settings code page can re-enable (invariant 14)

Covers invariants 3, 4, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19.

**Pause for review after Phase 4.**

### Phase 5 — JSON schema for external editor tooling

Goal: emit a JSON Schema entry for `[[editor.language_servers]]` so external TOML editors (e.g. editing `settings.toml` via a TOML language server) can autocomplete and validate against it. The in-app TOML text view is not schema-aware in v1; this work is purely for external consumers and the docs page.

**Derive `JsonSchema` on the runtime types** following the precedent of `app/src/settings/privacy.rs:41-50::CustomSecretRegex`, which solves the same shape of problem (a public struct with a non-serializable field).

In `crates/lsp/src/descriptor.rs`:

`LspServerDescriptor` derives `Serialize`, `Deserialize`, and `JsonSchema`. Its fields are the user-facing field set: `name: String`, `command: String`, `args: Vec<String>` (with `#[serde(default)]`), `filetypes: Vec<LspFiletypePattern>` (non-empty; serialized as an array of strings per invariant 1), `env: BTreeMap<String, String>` (with `#[serde(default)]`), and `initialization_options: Option<Value>`. Each field carries a `///` doc comment that schemars derives into the JSON Schema `description` (per invariant 25); the doc comments are the only place placeholder semantics are documented for external editor tooling.

`LspFiletypePattern` presents as a **plain string** on the wire and in the schema, because product.md invariant 1 defines `filetypes` as an array of strings. The struct keeps two fields — the user-typed `pattern: String` and the private compiled matcher (`globset::GlobMatcher`), computed from `pattern` at construction. Serde-side it uses `#[serde(try_from = "String", into = "String")]`; the `TryFrom<String>` impl is the same compile-the-glob path `parse::compile_pattern` uses, so a pattern can never exist with an uncompiled matcher. Schemars-side, `#[schemars(with = "String")]` makes the generated JSON Schema describe each `filetypes` element as a string. This is the codebase convention — see `app/src/settings/privacy.rs::CustomSecretRegex::pattern: Regex`, which uses `#[schemars(with = "String")]` for the same "private compiled artifact, user-typed source string" shape applied to a regex.

`LspServerDescriptors: SettingsValue` is the hand-rolled impl from Phase 2b at `app/src/settings/language_servers.rs`. The newtype is necessary because the default `Vec<LspServerDescriptor>` blanket impl at `crates/settings_value/src/lib.rs:138` would skip `descriptor::parse::parse_entries` — bypassing invariant 23's all-or-nothing error collection and invariant 24's unknown-field warnings. With the newtype in place, `define_settings_group!` generates the `SettingSchemaEntry` and submits it via `inventory`, so the build-time schema generator picks it up automatically with no hand-written registration. The schema describes `LspServerDescriptor` (the inner type) since `LspServerDescriptors` is `#[serde(transparent)]`.

**Field documentation strings.** Each field gets a `///` doc comment that becomes the JSON Schema `description` (schemars derives this automatically). Per product.md invariant 25, the schema descriptions for `command`, `args`, `env`, and `initialization_options` explicitly enumerate the recognized `{{...}}` placeholders (`{{workspace_root}}`, `{{workspace_cache_dir}}`, `{{env_VAR}}`) and the leading-`~`/`~/` home-directory expansion. These descriptions surface as hover text in external editors.

**Build-time artifact.** No changes to `app/src/bin/generate_settings_schema.rs`. It already walks the `inventory::iter::<SettingSchemaEntry>` registry and emits the JSON Schema document; the new entry from Phase 2 is picked up automatically. The schema is generated on demand (CI/build), not committed.

**External docs page.** A public docs page accompanies this feature as the entry point for the plugin path: how to add a custom server, example configs for popular languages (Ruby, Bash, JDTLS, etc.), the full set of placeholders, and install instructions per invariant 26. The page consumes the JSON Schema this work generates (per invariant 25) for the field-by-field reference. Per invariant 9, the in-app footer's existing "Language support is unavailable" affordance does **not** link to this page or introduce any new affordance text — the docs page is discoverable through Warp's broader docs surface (marketing site, docs index), not through the editor footer. If the docs page doesn't exist yet, this work coordinates with the docs/marketing surface — not on engineering blockers.

**Tests.** Schema-lint test in `crates/integration/src/test/` that validates the product.md worked-example TOMLs (JDTLS + the five built-in-as-plugin examples) parse and validate clean against the generated schema. Covers invariant 25.

### Out of scope for this implementation

Per product.md invariants 26–31: install-on-behalf, per-workspace settings overrides, multi-server-per-file, non-stdio transports, merging custom-into-built-in, and a dedicated management surface are deferred.

## End-to-end flow

```mermaid
flowchart TD
    A[Open file] --> B{Custom descriptor matches?}
    B -->|yes| C[Resolve custom workspace root]
    B -->|no| D{LanguageId::from_path?}
    D -->|unknown filetype| X
    D -->|known| E[LanguageId::server_type]
    E --> S{Built-in suppressed by same-named custom?}
    S -->|yes| X[No server for this file]
    S -->|no| F[Built-in LspServerConfig]
    C --> H[CustomLspServerConfig - raw descriptor]
    F --> I[LspManagerModel::register]
    H --> I
    I --> G[LspServerModel::start → command_and_params: substitute placeholders, create cache dir]
    G --> J[spawn_lsp_service]
```

Settings reload re-parses the `[[editor.language_servers]]` descriptors at Phase 2 so `LanguageServersSettings::as_ref(ctx)` reflects them (this is the settings/descriptor set, not `language_registry`, which is compiled-in and never rebuilt). The LSP manager does **not** subscribe to those changes — descriptors are read on demand at file-open time, and already-running servers keep running per invariant 19. New entries take effect on the next matching file open; edited entries take effect on the next user-initiated launch (close-and-reopen, or the existing restart action).

## Testing and validation

| PRODUCT invariant | Phase | Test home |
|---|---|---|
| 1 (field shape, filetype forms, languageId derivation) | 1, 2 | `descriptor/parse.rs` + `descriptor/matcher.rs` unit tests |
| 2 (unique name) | 1, 2 | `descriptor/validate.rs` unit; settings integration |
| 3 (custom overrides built-in, by filetype and by name) | 4 | `lsp_dispatch::resolve_server_for_path` unit tests; integration: custom `*.rs` entry preempts rust-analyzer; custom named `rust-analyzer` suppresses the built-in entirely |
| 4 (last-in-source-order, built-in fallback) | 1, 4 | matcher + `lsp_dispatch` unit tests + file-open integration |
| 5, 6 (placeholders, `~` expansion) | 1 | `descriptor/placeholder.rs` unit tests |
| 7 (substitution before spawn) | 3, 4 | Phase 3 mock-LSP test asserts substituted args at spawn; Phase 4 manual gate exercises the JDTLS `{{workspace_cache_dir}}` entry |
| 8, 19 (edits take effect on next launch) | 2, 4 | Hot-reload integration test asserting no restart |
| 9 (file-open routing) | 4 | `lsp_dispatch` unit tests + integration: `.rb` custom, `.rs` built-in, `.zig` unsupported |
| 10 (footer unchanged) | 4 | Existing footer regression tests must still pass for built-ins; new tests for custom display name parity |
| 11–15 (Enable flow) | 4 | Footer integration + persisted-workspace round-trip for custom servers |
| 16 (launch invariants) | 3, 4 | Manager test: post-substitution args, `cwd`, env merge |
| 17 (initialize + initializationOptions) | 3, 4 | LSP wire-protocol mock test |
| 18 (failure path) | 3, 4 | Manager test: nonexistent custom command → `Failed` state → footer renders error |
| 20–22 (filetype matching, no content sniff) | 1 | matcher unit tests |
| 23 (settings errors) | 1, 2 | settings integration: any invalid entry produces exactly one banner line for the bare `editor.language_servers` key; per-error-class detail is asserted on `log::warn!` output, not on the banner |
| 24 (unknown fields ignored) | 1, 2 | parse unit test with extra field, asserts log + clean parse |
| 25 (JSON schema) | 5 | Schema is generated by `generate_settings_schema.rs` at build time; CI lint validates the product.md worked-example TOMLs against the generated artifact |
| 32 (log redaction) | 1, 3, 4 | `parse.rs`/`validate.rs` logging-policy unit tests (redaction-safe `Display` summary, no raw user-typed values in `log::warn!` output); Phase 3 spawn-path test asserts substituted `command`/`args`/`env` values pass through the injected `LogRedactor`; Phase 4 manual gate asserts no `env` values verbatim and `CustomSecretRegex` matches redacted in launch logs |
| 33 (shared language registry) | 1 | `language_registry` unit tests: parity test (superset of VS Code core ids + neovim ext/filename tables), grammar-migration test (every current `language_by_filename_parts` mapping preserved), `resolve` precedence/case tests |

Manual validation gate at the end of Phase 4: load settings.toml with the JDTLS entry from product.md, open a Java file in a workspace with `pom.xml`, confirm initialize-time logs show the substituted command path and a redaction-safe summary of args (no `env` values logged verbatim; any args matching `CustomSecretRegex` patterns appear redacted per invariant 32), confirm hover/diagnostics work, confirm the per-workspace `{{workspace_cache_dir}}` is stable across restarts and distinct across two different Java workspaces. Also confirm a custom entry named `rust-analyzer` fully replaces the built-in (no built-in spawn, no built-in Enable affordance). Separately, confirm rust-analyzer / gopls / pyright-langserver / typescript-language-server / clangd behave identically to today on a workspace without any custom entries.

## Risks and mitigations

- **Schema correctness depends on two unenforced conventions.** `define_settings_group!` generates the `SettingSchemaEntry` for `[[editor.language_servers]]`, so there's no hand-written registration to drift. The remaining risks are (1) the string-presentation attributes on `LspFiletypePattern` (`#[serde(try_from = "String", into = "String")]`, `#[schemars(with = "String")]`) — if dropped, the schema would emit a bogus object shape for what invariant 1 defines as a plain string; (2) the `///` doc comments on `command`/`args`/`env`/`initialization_options` that enumerate the `{{...}}` placeholders per invariant 25 — these are the *only* place placeholder semantics are documented for external editor tooling, and they can silently rot if the placeholder set changes without the doc comments being updated. Mitigation: the schema integration test (Phase 5) validates the product.md worked-example TOMLs against the generated schema, which would catch (1); (2) requires the placeholder-substitution code and these doc comments to live next to each other or be cross-referenced.
- **Shared-type payload changes touch existing built-in code.** Threading `ServerKey` changes the payloads of `LspManagerModelEvent::ServerRemoved` (Phase 3), three `LspRepoStatus` variants (~24 match sites), `ModelEvent::UpsertWorkspaceLanguageServer`, and the `Workspace` enablement map key (all Phase 4). No behavior changes for built-ins — every site wraps its existing `LSPServerType` in `ServerKey::BuiltIn`. Mitigation: each payload change is compiler-guided — every affected site fails to build until updated — and lands as its own mechanical commit at the start of its phase so review can separate the rename from the feature diff.
- **A registry gap breaks strict servers.** A language absent from `language_registry` gets its raw lowercase extension as the LSP `languageId`, and descriptors offer no way to override it (product.md invariant 33). Single-language servers that ignore `languageId` still work; strict servers mis-handle or reject the document — and the user has no config-side workaround in v1. Mitigation: the Phase 1 coverage commitment (parity with the VS Code + neovim union, asserted by the parity test), plus treating post-ship gaps as data-only one-line additions that ride the normal release train. The docs page should list the id sent for each recognized extension so users can check it against their server's expectations before filing a gap.
- **Glob crate edge cases.** product.md cites `glob` crate syntax; implementation uses `globset` (already a dependency, a superset). The validator at `crates/lsp/src/descriptor/validate.rs` rejects `**` and `{a,b}` explicitly to keep behavior matching the documented syntax. Tests in `validate_tests.rs` cover both.
- **`workspace_hash` collision.** 64-bit prefix of SHA-256 has ~2^-32 collision probability across all workspaces a single user ever opens — well below the threshold where it matters. The same bound applies to the server-name segment of `lsp_server_cache_dir`. Documented in rustdoc on `warp_util::path::workspace_hash`.
- **No install path for custom servers.** A user who writes `command = "ruby-lsp"` without having `ruby-lsp` on `PATH` sees an `LspState::Failed` and the existing footer error. The error message must include the descriptor's `name` and the underlying OS error (e.g. `"ruby-lsp: command not found"`), not just `"command not found"`, so multiple-custom-server users can tell which entry failed.
- **Hot-reload diff cost.** Comparing descriptor sets on every settings save involves re-parsing the descriptor set and diffing O(n) descriptors. For realistic n (single-digit to low double-digit entries), this is trivial; flagged here so reviewers can challenge.
- **PATH-based command resolution (consistency with built-in LSPs).** Per product.md invariant 16, custom-LSP `command` resolution uses the OS process loader against the user's `$PATH` — the same path Warp's five built-in LSPs (rust-analyzer, gopls, pyright-langserver, typescript-language-server, clangd) take through `crates/lsp/src/command_builder.rs::CommandBuilder::command` and `crates/lsp/src/config.rs::command_and_params`, which call `Command::new(server_type.binary_name())` followed by `current_dir(workspace_root)`. Both built-in and custom LSPs spawn with the same trust model: the user's `$PATH` is trusted, and a `$PATH` containing relative entries (`.`, `..`, `bin`, empty entries) admits workspace-controlled binaries via bare-name lookup at exec time (Unix `execvp`, Windows `cmd.exe /c`). This spec intentionally **does not** add a custom-LSP-specific sanitization layer because (1) doing so would leave the same risk on built-in LSPs while declaring it fixed for customs — false sense of security; (2) no other Warp process-spawn site (MCP servers, terminal, agent shell-outs) sanitizes `$PATH`, so a one-off here would be inconsistent. The right place for sanitization, if Warp adopts it, is `CommandBuilder` itself — tracked as a follow-up below. The validator-side rejection of `command` values that already contain separators (invariant 23) still applies and is the cheap, consistent partial mitigation.

## Follow-ups

These are product.md non-goals for v1, but the architecture supports each as additive work.

- **Custom-server install registry.** Extend the existing `LanguageServerCandidate` install machinery to accept user-descriptor-defined install sources (GitHub release URL + asset selector). Likely the next feature after v1.
- **Remaining consumer migrations onto `language_registry`.** v1 migrates two consumers (LSP `languageId`, tree-sitter grammar selection). Others can follow, all additive (see the design decision's Extensibility note), but they are not uniform:
  - **File icons** (`app/src/code/icon.rs`) — a clean forward fit: extension → asset, added as an `icon` field.
  - **Agent `ProgrammingLanguage`** (`TODO(INT-605)`) and **notebook** code-block gating — need capability v1 doesn't build: an `aliases` field and a reverse index (`id → extension`, `alias → id`). Additive to the entry plus reverse-lookup helpers; the v1 forward-only API is shaped not to preclude them.
  - **`warp_util` text/binary classification** (`file_type.rs`) is **not** a registry consumer — it classifies all files including binaries (a content/byte concern), outside the language registry's model.
- **First-line / shebang detection.** `resolve` already accepts `first_line: Option<&str>` with the arm inert in v1 (invariants 20–21). Populating it — plus filename-glob patterns — extends detection without changing the custom-server surface.
- **User-extensible filetype configuration.** The registry is compiled-in in v1, so covering a new language requires a Warp release (see the coverage risk above). The future direction is a separate, user-extensible mechanism (a VS Code `files.associations` analog) that maps patterns to language ids — layered above the built-in registry, serving all its consumers. Product.md invariant 33 marks this out of scope here.
- **`CommandBuilder` PATH sanitization (security hardening, all LSPs).** Today `crates/lsp/src/command_builder.rs::CommandBuilder::command` passes the user's `$PATH` through unchanged. Adding a sanitization pass that drops relative and empty `$PATH` entries before spawn would close the bare-name-via-relative-PATH attack surface for **all** LSP spawns (built-in and custom) consistently — see the Risks entry on PATH-based command resolution. This is the principled fix for the residual risk noted in product.md invariant 16 but is intentionally out of scope here: it changes built-in LSP behavior, needs its own design discussion, and should ship as a standalone change.
- **Per-workspace `.warp/settings.toml` overrides** (PRODUCT invariant 27). Add a per-workspace settings layer that merges descriptors above the user-level `LanguageServersSettings` set. The matcher accessor (`match_for_path`) on the settings group is the natural extension point.
- **Multi-server-per-file** (PRODUCT invariant 28). `match_for_path` already returns `Option<LspMatchedDescriptor>`; broaden to `Vec<LspMatchedDescriptor>` and update the file-open chain to fan out.
- **Inspection/management surface** (PRODUCT invariant 31). With users configuring more LSPs over time, a "list of configured servers, with status indicators and per-workspace enable controls" page becomes valuable. The `LanguageServersSettings` descriptor set is the source of truth; a new settings page consumes it directly.
- **In-app schema-aware autocomplete** (PRODUCT invariant 25 future work). Today's in-app `settings.toml` text view opens an external editor. A future version could integrate a TOML LSP into the in-app editor and consume the same JSON Schema this work generates.
- **`{{env_VAR}}` env-var capture source.** The placeholder currently resolves via `std::env::var(...)` — i.e., Warp's process environment. When Warp is launched from a GUI (Finder, Spotlight, dock, systemd user session), that env does **not** include the user's interactive shell exports from `~/.zshrc` / `~/.bashrc` / etc. So `{{env_HOME}}` works (set by launchd / systemd) but `{{env_AWS_TOKEN}}` would resolve to empty if the user only exports it in their shell rc — defeating the secret-injection use case.

  Warp already captures interactive shell env elsewhere (the terminal needs `$PATH`, `$EDITOR`, etc.; `crates/lsp/src/config.rs` already threads a `path_env_var: Option<String>` through LSP spawn). Investigation needed: identify the canonical "interactive shell env" source in this codebase and switch the placeholder resolver to read from it instead of (or in addition to) `std::env`. Worth deferring until we see whether users actually hit this — but worth documenting as a known v1 limitation in the docs page so users with secret-in-shell-rc patterns aren't surprised.

- **`descriptor.env` values are effectively hard-coded.** A direct consequence of the `{{env_VAR}}` capture limitation above: users who need dynamic env vars on the LSP process (API tokens, project-specific overrides, anything sourced from a shell rc) currently have only one reliable option — write the literal value into `settings.toml`. That's a poor fit for secrets and a fragile fit for any value that changes per environment. The fix is the same investigation as the `{{env_VAR}}` note (canonical interactive-shell env source), but worth tracking separately because the user-facing pain shows up at *every* descriptor that wants non-literal env, not just at descriptors that explicitly use the `{{env_VAR}}` placeholder. Likely candidates to investigate: a dynamic-source syntax (e.g. `env = { AWS_TOKEN = { from_shell = "AWS_TOKEN" } }`), or just fixing capture so `{{env_VAR}}` "just works".
