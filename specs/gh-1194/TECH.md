# TECH: i18n / Multiple Language Support

## Context

Warp has no existing i18n infrastructure. Every user-facing string in the codebase is a hardcoded English literal — `"Save"`, `"File"`, `"Close tab"`, etc. This spec proposes a custom, lightweight i18n framework built directly into the Warp Rust codebase, with zh-CN as the first non-English locale.

**Relevant files:**
- `crates/i18n/src/lib.rs` — core i18n engine plus single exported `t!()` and `t_required!()` macro definitions
- `app/src/lib.rs:610` — `init_locale()` call in application startup
- `resources/bundled/locales/en.yml` — English locale file (2,944 lines)
- `resources/bundled/locales/zh-CN.yml` — Chinese locale file (2,944 lines)
- `app/src/i18n.rs` — re-export of `warp_i18n::*` for app call sites
- `crates/onboarding/src/lib.rs` — import/re-export of `warp_i18n::{t, t_required}` for onboarding call sites
- `crates/onboarding/src/bin/main.rs:43` — `init_locale()` for onboarding

**Why not a third-party crate?** Existing Rust i18n crates (Fluent, gettext, ICU4X) add significant complexity and dependencies for a feature that currently only needs two locales with simple key-value lookups. A custom solution keeps the surface area small and the build fast.

## Proposed changes

### 1. New crate: `warp_i18n`

A new workspace crate at `crates/i18n/` with dependencies `serde_yaml` (for parsing YAML) and `sys-locale` (for OS-level locale detection).

**Core types:**
- `Translations`: `HashMap<Locale, HashMap<Key, String>>` — double-map structure where the outer key is locale (`"en"`, `"zh-CN"`) and the inner key is a dot-separated path (e.g., `"menu.file"`).
- `TranslationLookup`: enum with `Found(Cow<'static, str>)` and `Missing` variants. Missing state is explicit and is never inferred by comparing rendered text to the lookup key.
- Global state: `TRANSLATIONS` (lazy-initialized via `OnceLock`) and `CURRENT_LOCALE` (backed by `RwLock<&'static str>`).

**Public API:**
```rust
pub fn init_locale()                                    // Resolve & set current locale
pub fn set_locale(locale: &str)                         // Force a specific locale
pub fn lookup(key: &'static str) -> TranslationLookup   // Primary translation lookup
pub fn t(key: &'static str) -> Cow<'static, str>        // Ordinary UI convenience wrapper
pub fn t_required(key: &'static str, fallback: &'static str) -> Cow<'static, str>
pub fn interpolate(template: &str, args: &[(&str, String)]) -> String
#[macro_export]
macro_rules! t { ... }
#[macro_export]
macro_rules! t_required { ... }
```

**Locale resolution** (`init_locale`):
1. Select candidates from the first available source in priority order: `WARP_LANG` > platform-native system locale API > POSIX locale env fallback (`LANGUAGE` > `LC_ALL` > `LC_MESSAGES` > `LANG`) > `"en"` (default). `WARP_LANG` is Warp-specific and has highest priority. The platform-native API is the zero-configuration desktop default. POSIX environment variables are consulted only when the platform API is unavailable or returns no locale, so ordinary shell locale variables do not override desktop language on normal app launches. `LANGUAGE` is treated as the message-translation preference list on Linux-like fallback paths.
2. Normalize each candidate before classification:
   - Trim whitespace.
   - For `LANGUAGE`, split colon-separated preference lists and evaluate entries in order.
   - Strip encoding and modifier suffixes after `.` or `@` (`zh_CN.UTF-8`, `zh_CN.utf8`, and `zh_CN.UTF-8@pinyin` all normalize to `zh-CN`).
   - Convert `_` to `-`, compare language/script/region subtags case-insensitively, and normalize script/region casing for tests (`zh-hans-cn` → `zh-Hans-CN`).
3. Resolve the first non-empty source authoritatively:
   - `WARP_LANG`: if the normalized value is Simplified Chinese (`zh`, `zh-CN`, `zh_CN`, `zh-Hans*`, or `zh_Hans*`), return `"zh-CN"`; otherwise return `"en"` without consulting lower-priority sources.
   - Platform-native system locale API: return `"zh-CN"` for Simplified Chinese values; otherwise return `"en"` without consulting POSIX locale environment variables.
   - Fallback `LANGUAGE`: evaluate colon-separated entries in order. Return `"zh-CN"` for the first Simplified Chinese entry. Skip unsupported entries within the list (`fr`, `zh-TW`, `zh-HK`, `zh-Hant*`, etc.); if no entry is Simplified Chinese, return `"en"` without consulting lower-priority fallback sources.
   - Fallback `LC_ALL`, `LC_MESSAGES`, and `LANG`: return `"zh-CN"` for Simplified Chinese values; otherwise return `"en"` without consulting lower-priority fallback sources.
4. No raw candidate is ever used as a locale key; runtime locale is always one of the two supported values.

**Translation lookup** (`lookup` / `t`):
1. Look up key in `TRANSLATIONS[current_locale]`.
2. If missing, fall back to `TRANSLATIONS["en"]`.
3. If a value is found, return `TranslationLookup::Found(value)`.
4. If still missing, return `TranslationLookup::Missing`. The ordinary `t(key)` wrapper converts `Missing` to `Cow::Borrowed(key)` for non-sensitive UI. Security-sensitive UI must use `t_required` / `t_required!()` instead of the ordinary wrapper.

**Required translation lookup** (`t_required`):
1. Look up key in `TRANSLATIONS[current_locale]`.
2. If missing, fall back to `TRANSLATIONS["en"]`.
3. If still missing, return the embedded English fallback passed by the call site.
4. `t_required` never returns the raw key. Auth, billing, privacy, sharing/permission, and agent-consent surfaces must use this API, either directly or through the `t_required!()` macro.
5. The fallback literal is not an independent source of English copy. CI verifies that each `t_required!("key", "fallback")` literal is byte-identical to the canonical `en.yml` value for that key, after normalizing line endings. Any exception must be listed in a checked-in allowlist with a reason and owner approval.

**Interpolation** (`interpolate`):
- Simple `str::replace` of `{name}` placeholders with provided values.
- Not a template engine — no escaping, no positional arguments, no format specifiers.
- Values are pre-formatted to `String` by the `t!()` macro before reaching `interpolate`.
- `interpolate()` returns an owned `String`; the macros wrap it in `Cow::Owned(...)`. This avoids returning a borrowed value tied to a temporary `Cow` from `t()` or `t_required()`.
- Interpolated values are treated as plain text. Call sites that render into rich, Markdown, link-capable, or otherwise parsed UI must pass interpolated values through the appropriate escaping layer or build the rich UI from structured fragments rather than by concatenating formatted strings. File paths, branch names, agent-provided labels, and server-provided metadata must never be allowed to inject markup, links, or formatting through translation interpolation.

**Rich/parsed rendering contract:**

Interpolated translations are allowed to enter only these renderer categories:

| Renderer category | Approved strategy |
|-------------------|-------------------|
| Plain UI text, labels, buttons, menus, tooltips, toasts without links | Use `t!()` / `t_required!()` directly; interpolation values are plain text. |
| Formatted text fragments | Build structured fragments and pass dynamic values only through plain-text fragment constructors such as `FormattedTextFragment::plain_text(...)`; do not concatenate dynamic values into markup-bearing fragments. |
| Markdown body text | Escape dynamic values with `warp_i18n::escape_markdown_text(...)` before interpolation, or avoid interpolation and insert the dynamic value as a plain-text fragment after Markdown parsing. |
| Markdown link text | Escape dynamic values with `warp_i18n::escape_markdown_link_text(...)`; URLs must not be interpolated into Markdown source. |
| Link destinations / hrefs | Do not translate or interpolate raw hrefs. Build links with typed link APIs such as `ToastLink::with_href(...)`, `ctx.open_url(...)`, or the relevant URL type after validating the URL with existing URL parsing/allowlist logic. |
| HTML, webview, or other markup-capable renderers | Do not pass interpolated translated strings directly. Use renderer-specific escaping or structured nodes, with a local allowlist entry naming the API used. |

The approved helper functions live in `warp_i18n` so the lint can recognize them. New renderer APIs that parse markup must be added to the lint configuration in the same PR that introduces their first translated call site.

**YAML loading:**
```rust
fn load_translations() -> Translations {
    for dir in locale_dirs() {          // Try multiple filesystem paths
        if let Some(t) = load_complete_dir(dir) { // First complete directory wins
            return t;
        }
    }
    // On WASM, use include_str!() instead of filesystem access
}
```

`load_complete_dir` reads locale files from one directory, parses them with `serde_yaml`, identifies each locale from the top-level key, and recursively flattens nested YAML into dot-separated keys via `flatten_value`.

A directory is accepted only if it is complete:
- It contains parseable `en.yml`.
- It contains parseable `zh-CN.yml`.
- Both files have the expected top-level locale key (`en:` / `zh-CN:`).
- Both flattened maps have the same key set after applying the checked-in intentional-exception allowlists.
- Placeholder parity holds for every shared key.
- Required security-sensitive keys are present in `en.yml`.

If any completeness check fails, the whole directory is skipped and the next discovery path is tried. A partial `$WARP_LOCALES_DIR`, damaged source-tree checkout, or damaged bundle must not shadow a later complete directory.

**File discovery order** (non-WASM):

Release builds (`#[cfg(not(debug_assertions))]`) only search the platform bundled resources path. Dev builds add the following higher-priority paths for rapid iteration:

| Priority | Path | Gate |
|----------|------|------|
| 1 | `$WARP_LOCALES_DIR` env var | `#[cfg(debug_assertions)]` |
| 2 | Platform bundled resources: `<exe_dir>/resources/bundled/locales` | Always enabled |
| 3 | `$CARGO_MANIFEST_DIR/../resources/bundled/locales` (and up to 4 ancestor levels) | `#[cfg(debug_assertions)]` |
| 4 | `$PWD/resources/bundled/locales` | `#[cfg(debug_assertions)]` |

The release resource path is platform-specific and must be shared by runtime discovery and bundle validation:

| Platform/artifact | Runtime locale directory |
|-------------------|--------------------------|
| macOS `.app` | `<Warp*.app>/Contents/Resources/bundled/locales` |
| Windows installer output | `<install-dir>\resources\bundled\locales` (Inno `{app}\resources\bundled\locales`) |
| Linux app packages/AppImage | `/opt/warpdotdev/<package>/resources/bundled/locales` inside the package/AppDir |
| Linux CLI artifact | `<bundle-output>/resources/bundled/locales` adjacent to the CLI binary bundle |

**Security:** Paths 1, 3, and 4 are compiled out of release binaries. This prevents shipped builds from loading arbitrary YAML from environment variables or the current working directory into the startup parsing and UI rendering pipeline. Every runtime-loaded locale file, including release-bundled files, is subject to parser bounds before `serde_yaml` parsing: maximum 8 MB per file, maximum YAML nesting depth 16, maximum 10,000 flattened keys per locale, and scalar string values only after flattening. If a locale file exceeds the bounds, has unsupported YAML shape, is malformed, or fails directory completeness checks, the directory is skipped and the next discovery path is tried. The application does not crash.

**Parser dependency posture:** `serde_yaml` is treated as a security-sensitive startup parser dependency. The implementation must pin it through `Cargo.lock`, include it in the repository's dependency review / audit workflow, and avoid enabling optional features beyond YAML parsing. Locale files use a deliberately restricted YAML subset: one top-level locale mapping, nested mappings, and string scalar leaves only. Anchors, aliases, merge keys, custom tags, non-string leaves, and multi-document YAML are rejected by shape validation before the flattened map is accepted. Dependency updates to `serde_yaml` or its parser stack must run the malformed/oversized locale tests before merge.

**No-locale fallback:** If no locale file can be loaded from any discovery path (e.g., corrupt installation, missing resource directory), `init_locale()` still completes successfully. For ordinary non-sensitive UI, the translation map remains empty and `t!()` returns the raw key string as the rendered text. Security-sensitive UI is different: auth, billing, privacy, sharing/permission, and agent-consent surfaces must not render raw dot-path keys. Those call sites must use `t_required!()` with an embedded English fallback, or fail closed by disabling the affected action and showing a readable English error that also uses `t_required!()`.

### 2. `t!()` and `t_required!()` macros (defined once in `crates/i18n`)

`t!()` is a `#[macro_export] macro_rules!` macro owned by `crates/i18n`. The app and onboarding binaries import the same macro from `warp_i18n`; they must not maintain local duplicate macro definitions. This keeps fallback and security-sensitive behavior single-sourced.

The macro has three match arms. The actual expansion uses `match` (not combinator chains) to preserve `Cow<'static, str>` type flow:

```rust
// Arm 1: Simple lookup
t!("menu.file")
// Expands to:
match $crate::lookup("menu.file") {
    $crate::TranslationLookup::Found(value) => value,
    $crate::TranslationLookup::Missing => Cow::Borrowed("menu.file"),
}

// Arm 2: Explicit interpolation
t!("terminal.hand_off", environment = name)
// Expands to:
match $crate::lookup("terminal.hand_off") {
    $crate::TranslationLookup::Found(value) => {
        Cow::Owned($crate::interpolate(value.as_ref(), &[("environment", format!("{}", name))]))
    }
    $crate::TranslationLookup::Missing => Cow::Borrowed("terminal.hand_off"),
}

// Arm 3: Implicit interpolation (variable name = key name)
t!("some.key", count)
// Expands to:
match $crate::lookup("some.key") {
    $crate::TranslationLookup::Found(value) => {
        Cow::Owned($crate::interpolate(value.as_ref(), &[("count", format!("{}", count))]))
    }
    $crate::TranslationLookup::Missing => Cow::Borrowed("some.key"),
}
```

The macros branch on `TranslationLookup::Found` versus `TranslationLookup::Missing`. They must not infer missing state by comparing the rendered string to the key, because a valid translation may intentionally equal its key text. When lookup is missing, interpolation is skipped because there is no template to interpolate into.

`t_required!()` is the security-sensitive variant. It mirrors the same simple and interpolated call shapes, but requires an embedded English fallback literal:

```rust
t_required!("auth.require_login_share", "In order to share, please create an account.")
t_required!(
    "billing.shared_objects_limit_reached",
    "Shared {object_type}s limit reached",
    object_type = object_type_name,
)
```

The required macro expands through `$crate::t_required(key, fallback)`, then applies interpolation to the translated value or embedded fallback and wraps interpolated output with `Cow::Owned(...)`. It must never branch back to the raw key. The fallback argument must be a string literal so static analysis can verify that every security-sensitive call site has readable English text even when locale files are missing.

Both `app` and `crates/onboarding` import these exported macros from `warp_i18n`. A CI lint rejects any local `macro_rules! t` or `macro_rules! t_required` definitions outside `crates/i18n`, preventing macro drift across binaries.

### 3. Locale files

Two YAML files at `resources/bundled/locales/`:

| File | Locale | Keys | Lines |
|------|--------|------|-------|
| `en.yml` | English (fallback) | ~2,732 | 2,944 |
| `zh-CN.yml` | Simplified Chinese | ~2,732 | 2,944 |

Both files share identical structure — 94 top-level YAML sections corresponding to UI areas: `menu`, `tab`, `workspace`, `auth`, `billing`, `settings`, `ai_settings_page`, `terminal`, `shared_session`, `common`, etc.

The dot-separated YAML path (e.g., `menu.file`) is the lookup key used in `t!()` calls. The English string value at that path is the runtime fallback rendered when the active locale has no entry. The zh-CN string value is the Chinese rendering shown when zh-CN is active. Both values are independently authored — the key is the path, not the English text.

### 4. Usage patterns in the codebase

All user-facing string literals in the required source tree are replaced with `t!()` or `t_required!()` calls. The initial app migration is expected to touch ~4,700+ call sites, plus onboarding, shared UI crates, and platform resource copy discovered by the extractor. Key patterns:

| Pattern | Example | When |
|---------|---------|------|
| Simple string | `t!("common.save")` | Static labels |
| With interpolation | `t!("ai_output.in_path", path = display_path)` | Dynamic content |
| Security-sensitive fallback | `t_required!("auth.require_login_share", "In order to share, please create an account.")` | Auth, billing, privacy, sharing, permission, and agent-consent copy |
| `.to_string()` | `t!("common.save").to_string()` | APIs requiring `String` |
| In UI components | `ActionButton::new(t!("key"), theme)` | Buttons, menus |
| In formatted text | `FormattedTextFragment::plain_text(t!("key"))` | Rich text blocks |

### 5. Migration scope

The migration scope must match the product requirement for the whole Warp UI surface. The implementation PR is not complete until all user-visible string literals in these areas are either localized or explicitly documented as out of scope:

| Area | Required coverage |
|------|-------------------|
| `app/src/**` | workspace, panes, tabs, toolbars, settings, modals, notifications, toasts, tooltips, empty states, command palette labels, terminal-adjacent UI chrome |
| `crates/onboarding/src/**` | first-run onboarding screens, onboarding prompts, onboarding buttons, onboarding error text |
| UI support crates used by the app (`crates/ui_components/**`, `crates/warpui*/**`, `crates/editor/**` where user-visible labels are authored) | reusable controls, formatted text components, editor labels, shared prompts, and component-owned empty/error states |
| Platform/app resources (`app/channels/**`, macOS menu/plist resources, Windows installer-visible app metadata when rendered in Warp UI, Linux desktop metadata shown by the app) | app/menu chrome, platform-visible labels owned by Warp, resource strings that surface inside the running application |
| Account and monetization | auth, onboarding login, teams, billing, plan/credit warnings, payment restriction copy |
| Sharing and permissions | Warp Drive sharing, shared sessions, access labels, permission prompts |
| AI and agent surfaces | agent input/output chrome, Oz/cloud handoff, autonomy permissions, MCP/tool permissions, code review UI, agent management |
| Bundled locale resources | `resources/bundled/locales/en.yml` and `resources/bundled/locales/zh-CN.yml` included in platform release artifacts |

Explicit non-goals remain untranslated: PTY output, shell command output, file contents opened by the user, keyboard shortcut glyphs, protocol/log/debug identifiers, telemetry event names, test fixtures, and developer-only tracing.

Coverage is verified by a static extractor that scans Rust call sites, macro invocations, platform resource files, and known UI-construction APIs in the required areas. The extractor must include common button/menu/tooltip/modal/toast/settings/action/empty-state constructors and renderer APIs, not only direct `t!()` call sites. Any intentional exception must be listed in a checked-in allowlist with a short reason and an owning surface.

### 6. Application integration

`init_locale()` is called at the top of `app::run()` (`app/src/lib.rs:610`), before feature flags are initialized and before any UI is created. This ensures translations are available for the entire application lifecycle.

The `app/src/i18n.rs` file re-exports `warp_i18n::*` for direct function access from app modules. The `t!()` and `t_required!()` macros are imported from `warp_i18n` itself so both the app and onboarding binaries share one macro implementation.

### 7. Windows compiler support

`windows-rs` crate macros generate code referencing `i18n::t()` on the `app` crate. Windows resource compilation works correctly because no i18n call appears in a const-evaluation context.

## Testing and validation

### Security-sensitive translation review (manual)
- Strings in security-relevant UI surfaces must be manually reviewed by a maintainer before merge. These surfaces include: auth dialogs (sign-in, sign-up, permission prompts), billing pages (pricing, plan descriptions, credit consumption warnings), sharing/permission controls (access grant text, visibility labels), and agent-mode consent prompts (data-handling disclosures, handoff confirmations).
- The reviewer verifies that Chinese translations preserve the legal and behavioral intent of the original English text — warnings are not weakened, consent semantics are not altered, and permission descriptions remain accurate.
- A checklist of security-sensitive keys is maintained alongside the locale files for targeted review. It must include exact YAML sections and prefixes for auth, billing, sharing, agent consent, and data-handling surfaces: `auth.*`, `billing.*`, `billing_ext.*`, `teams.*`, `privacy.*`, `shared_session.*`, `agent_management.*`, `hoa_onboarding.*`, `ai_ext.grant_access_files`, `ai_ext.grant_access_repository`, `ai_ext.missing_github_auth`, `ai_ext.authenticate_github`, `ai_ext.hand_off_to_cloud`, `ai_ext.handoff_to_cloud`, `ai_ext.hand_off_to_environment`, `ai_output.manage_ai_autonomy_permissions`, `ai_output.read_mcp_resource_permission`, `ai_output.upload_artifact_permission`, `ai_output.manage_agent_permissions`, and `ai_output.use_computer_permission`. When new permission, handoff, data-retention, or account/billing warning keys are added, the checklist must be updated in the same PR.
- Every key in that checklist must be reached through `t_required!()` or an explicit fail-closed path. A CI lint rejects security-sensitive keys rendered through plain `t!()` and rejects `t_required!()` calls whose fallback is not a string literal.
- Every `t_required!()` fallback literal must match the canonical English value in `en.yml` for the same key, after normalizing line endings. A fallback that intentionally differs must be listed in a checked-in allowlist with the key, call site, reason, and product/legal owner approval. This prevents security-sensitive embedded fallback copy from drifting away from reviewed English locale text.

### Locale file integrity (automated)
- Every key present in `en.yml` must have a corresponding key in `zh-CN.yml`. A script or build-time check verifies this invariant — missing keys in `zh-CN.yml` cause a CI failure.
- Every `zh-CN.yml` value must be non-empty after trimming whitespace, unless the key is listed in a checked-in empty-value allowlist with a reason.
- **Interpolation placeholder parity:** for every key whose English value contains `{name}` placeholders, the zh-CN value must contain the exact same set of placeholder names (same count, same names). Mismatched placeholder names (e.g., en has `{count}` but zh-CN has `{number}`) produce runtime rendering bugs and must be rejected at CI time.
- Both YAML files must parse successfully as valid YAML and produce the expected top-level locale key (`en:` / `zh-CN:`).
- No orphaned keys: every key referenced by a `t!()` or `t_required!()` call in the codebase must exist in `en.yml`. This check must use a Rust-aware parser (`syn` over Rust source files or tree-sitter-rust) to extract macro invocations, not regex-only matching. It must handle multiline macro calls, interpolation arguments, raw string literals, module-qualified macro imports, and both `t!` and `t_required!` forms. Regex may be used only as a fast prefilter before AST parsing.
- The number of keys in `en.yml` and `zh-CN.yml` must be equal (after accounting for any intentionally untranslatable keys).
- Copied-English detection: a CI check rejects `zh-CN.yml` values that are byte-identical to their English value, except keys in a checked-in allowlist for brand names, product names, protocol identifiers, keyboard shortcuts, commands, file extensions, URLs, and intentionally untranslated technical terms.
- zh-CN script coverage: for non-allowlisted values longer than two visible characters, the translation must contain at least one CJK Unified Ideograph or full-width CJK punctuation character. This catches accidental English-only translations without blocking short symbols or brand terms.
- Security-sensitive translation quality: keys in the security-sensitive checklist must pass the copied-English and CJK checks with no broad section-level allowlist. Any exception requires an inline reason naming the legal/product owner who approved preserving the English text.
- Required fallback drift check: AST extraction of `t_required!()` calls compares each fallback literal to the corresponding `en.yml` value and fails CI on mismatch unless the exact call site is allowlisted with owner approval.
- No stale locale keys: every key in `en.yml` is either referenced by `t!()` / `t_required!()` or listed in a checked-in allowlist for platform/resource-only strings.
- No unlocalized required-scope strings: the static extractor scans the migration-scope directories for UI literals passed to common label/button/menu/tooltip/modal APIs without `t!()` or `t_required!()`. Remaining literals must be intentionally allowed with a reason.
- Rich/parsed text interpolation audit: a lint scans call sites that pass translated strings into Markdown, rich text, URL/link-capable text, or formatted-fragment APIs. Interpolated values from file paths, branch names, agent labels, server metadata, repository names, or URLs must either be escaped for that renderer or supplied as structured plain-text fragments. The lint rejects direct string concatenation or direct `t!()`/`t_required!()` interpolation into parsed markup.
- Release bundle check: every packaged app artifact must contain both `en.yml` and `zh-CN.yml` at the exact runtime path for that artifact: macOS `<Warp*.app>/Contents/Resources/bundled/locales`, Windows `{app}\resources\bundled\locales`, Linux app packages/AppImage `/opt/warpdotdev/<package>/resources/bundled/locales`, and Linux CLI `<bundle-output>/resources/bundled/locales`. CI fails if either file is missing from any packaging output or if the validation path differs from runtime discovery.
- Dev-only locale loading check: release binaries must not reference `$WARP_LOCALES_DIR`, `$PWD/resources/bundled/locales`, or source-tree fallback paths. A binary/string or build-time assertion verifies those discovery paths are compiled out outside `debug_assertions`.
- Runtime locale bounds check: unit tests cover oversized files, excessive YAML nesting, non-string scalar values after flattening, too many flattened keys, missing `en.yml`, missing `zh-CN.yml`, mismatched key sets, and placeholder mismatch. Every one of these cases must skip the directory rather than partially loading it.
- YAML parser supply-chain check: dependency review and audit tooling must cover `serde_yaml` and its parser dependencies, and parser updates must run malformed, oversized, multi-document, alias/tag, and unsupported-shape locale tests.

### Unit tests

- `warp_i18n` must include tests for:
  - `t()` with a key present in both locales returns the current locale's value
  - `t()` with a key present only in English falls back to English
  - `t()` with a missing key returns the key string itself
  - `t_required()` with a missing key returns the embedded English fallback, never the raw key
  - `t_required!()` fallback literals match `en.yml` canonical English values or fail the drift lint
  - `interpolate()` correctly substitutes one and multiple placeholders
  - exported `t!()` and `t_required!()` macros compile from both `app` and `crates/onboarding` without local macro copies
  - `set_locale("zh-CN")` correctly switches the active locale
  - `set_locale("fr")` falls back to `en`
  - locale normalization handles `zh_CN.UTF-8`, `zh_CN.utf8`, `zh_CN.UTF-8@pinyin`, mixed-case `zh-hans-cn`, and colon-separated `LANGUAGE` lists
  - `WARP_LANG=fr` forces `en` even when lower-priority locale sources are Simplified Chinese
  - platform-native Simplified Chinese locale resolves to `zh-CN` even when `LANG=en_US.UTF-8`
  - platform-native English locale resolves to `en` even when `LANG=zh_CN.UTF-8`
  - `LANGUAGE=fr:zh_TW:en` resolves to `en` without falling through to `LANG` when the platform-native locale API is unavailable
  - `LC_ALL=fr_FR.UTF-8` with `LANG=zh_CN.UTF-8` resolves to `en` when the platform-native locale API is unavailable because `LC_ALL` is the first non-empty fallback source
  - `zh-TW`, `zh-HK`, and `zh-Hant*` resolve to `en`
  - `load_complete_dir()` correctly parses YAML and produces flattened keys
  - partial locale directories are skipped and do not shadow later complete directories
  - malformed or over-size dev locale files are skipped without panicking
  - malformed or over-size release-bundled locale files are skipped without panicking
  - YAML shape bounds reject excessive nesting, too many flattened keys, and non-string flattened values
  - YAML shape validation rejects anchors, aliases, merge keys, custom tags, and multi-document YAML
  - AST-based key extraction finds single-line and multiline `t!()` / `t_required!()` calls, including security-sensitive required fallback calls
  - Markdown/rich-text lint rejects unescaped interpolation into parsed renderers and accepts approved escaping or structured-fragment APIs
  - locale quality checks reject empty zh-CN values, copied-English values, and non-allowlisted English-only zh-CN values

### Integration / manual verification

Manual verification must cover platform resource loading and locale detection, not just a local dev run:

| Platform | Scenario | Expected result |
|----------|----------|-----------------|
| macOS release artifact | `WARP_LANG=zh-CN` | menu bar, settings, agent UI, tooltips, notifications, onboarding entry points render zh-CN |
| macOS release artifact | `WARP_LANG=zh_CN.UTF-8` | same zh-CN result |
| macOS release artifact | `WARP_LANG=zh-TW` | English UI |
| Windows MSVC release artifact | Simplified Chinese system locale with no `WARP_LANG` | zh-CN UI after bundled resource load |
| Windows MSVC release artifact | `WARP_LANG=fr` on Simplified Chinese system locale | English UI, proving explicit override behavior |
| Linux desktop release artifact | Simplified Chinese system locale, `LANG=en_US.UTF-8`, no `WARP_LANG` | zh-CN UI, proving native desktop locale beats ordinary POSIX env |
| Linux desktop release artifact | English system locale, `LANG=zh_CN.UTF-8`, no `WARP_LANG` | English UI, proving ordinary POSIX env does not override native desktop locale |
| Linux fallback/headless test build | no platform-native locale, `LANG=zh_CN.UTF-8` | zh-CN UI |
| Linux fallback/headless test build | no platform-native locale, `LANGUAGE=fr:zh_CN:en` with `WARP_LANG` unset | zh-CN UI |
| Linux fallback/headless test build | no platform-native locale, `LANGUAGE=fr:zh_TW:en` and `LANG=zh_CN.UTF-8` with `WARP_LANG` unset | English UI, proving unsupported `LANGUAGE` preferences do not fall through to `LANG` |
| Linux fallback/headless test build | no platform-native locale, `LC_ALL=fr_FR.UTF-8` and `LANG=zh_CN.UTF-8` with `WARP_LANG`/`LANGUAGE` unset | English UI, proving first-source-wins fallback semantics |
| All platforms | bundled locale directory missing or unreadable | app launches; ordinary UI may show raw keys; security-sensitive surfaces show embedded English fallback or disabled fail-closed action |
| All platforms | terminal PTY output under zh-CN UI | shell output is unchanged |
| All platforms | deliberate ordinary missing key in a test-only build | raw key renders without panic |

### Regression prevention

- The `cargo check` / `cargo build` pipeline for the `warp-oss` binary must pass on macOS, Windows MSVC, and Linux
- All existing tests must pass after the migration — no test assertions may be broken by i18n
- Behavior invariants from `PRODUCT.md` map to verification steps above
- CI must run the locale integrity script, translation quality checks, required-fallback drift lint, security-sensitive key lint, rich/parsed interpolation lint, macro drift lint, release bundle check, parser dependency audit, and locale normalization tests before the implementation PR can merge

## Parallelization

Not applicable. The i18n work is a single cohesive change across the codebase — string replacements, locale file authoring, and framework implementation are all tightly coupled and should be done in a single branch by a single author.
