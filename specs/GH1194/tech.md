# Technical specification: initial Spanish GUI localization

Related issue: [#1194](https://github.com/warpdotdev/warp/issues/1194).
Status: proposed design, awaiting maintainer review.

## Context

The GUI contains English literals in Rust views and shared display-label helpers.
The contribution builds on the initial catalog structure/string inventory from
[#10630](https://github.com/warpdotdev/warp/pull/10630); the approach in
[#11382](https://github.com/warpdotdev/warp/pull/11382) was reviewed as related work.
Historical branches were not merged wholesale.

Relevant implementation locations in this proposal:

- [Locale initialization](../../crates/i18n/src/lib.rs#L19) and
  [application startup](../../app/src/lib.rs#L759).
- [Translation lookup](../../crates/i18n/src/lib.rs#L67),
  [display-only option labels](../../crates/i18n/src/lib.rs#L72), and
  [font-weight labels](../../crates/i18n/src/lib.rs#L111).
- [Named interpolation](../../crates/i18n/src/lib.rs#L137) and
  [embedded catalog loading](../../crates/i18n/src/lib.rs#L188).
- [Settings display labels](../../app/src/settings_view/mod.rs#L352),
  [stable session slugs](../../app/src/settings_view/mod.rs#L392), and
  [slug parsing](../../app/src/settings_view/mod.rs#L428).
- [Catalog/documentation guide](../../crates/i18n/README.md).

## Proposed changes

Add the workspace crate `warp_i18n`, referenced by the application and onboarding.
Embed matching English/Spanish YAML catalogs under `resources/bundled/locales`.
The catalogs currently contain 3,275 keys each; catalog size is not a claim of
complete GUI coverage.

At GUI startup, choose `WARP_LANG`, then LANG/LANGUAGE/LC_ALL/LC_MESSAGES, then the
system locale. Normalize Spanish regional variants to es; all other locales use
en. Cache translations in a OnceLock and retain the selected locale in a RwLock.

Use `t!(key)` for stable borrowed labels and named arguments for interpolated
messages. Lookup falls back from Spanish to English and then to the key. Insert
replacement values in one pass so inserted text is not interpreted recursively.
Optional catalogs in WARP_LOCALES_DIR or bundled resources override embedded values.

Replace GUI literals at display boundaries. Shared enum serialization and stored
values remain stable. Font weights need distinct labels from theme brightness:
the font weight Light is Ligera while the light theme is Claro. Dropdown item
construction and selected-label lookup use the same translated labels.

For messages with multiple evaluated arguments, preserve argument evaluation order
and reuse repeated arguments. Keep numeric formatting at the interpolation boundary.
Construct formerly constant translated labels at runtime, and update label-based
selectors to compare the labels generated for the active locale.

Separate settings Display text from persisted slugs. The settings-page crash tag
uses the stable slug instead of the translated display label; this intentionally
changes that diagnostic value. Preserve the native macOS Window menu role with an
explicit flag independent of its translated title.

This design keeps English as the compatibility fallback and limits dependency
changes to YAML parsing, system-locale detection, and the new localization crate.
It deliberately avoids live language switching because existing widgets cache
labels; introducing a live refresh mechanism is a separate design decision.

## Testing and validation

Validation was performed on Windows against base commit 7c314881924323616defd7210cedd27e1bd6b64d:

| Product invariants | Validation | Result |
| --- | --- | --- |
| 1, 2, 6, 9 | `cargo test -p warp_i18n` | 8 tests passed: regional variants, stable labels, fallback, invalid catalogs, catalog parity, and interpolation |
| 3, 7 | Contributor-provided screenshots in product.md | Agent settings and Appearance rendered; signed-out differences documented |
| 3, 4 | Settings unit tests with the two discovery tests below excluded | 281 passed |
| 4, 8 | Existing find-component test | 1 passed; internal Find context identifiers preserved |
| 5 | Interpolation and catalog-placeholder tests in warp_i18n | Passed |
| 8 | `localized_window_menu_retains_system_role` in warpui_core | Passed; native macOS end-to-end behavior still pending |

Additional completed checks:

- `cargo clippy -p warp_i18n --all-targets --tests -- -D warnings`.
- `cargo check -p warp --bin warp-oss --features gui`.
- `cargo build -p warp --bin warp-oss --features gui`.
- Executable CLI `--help` smoke test: exit code 0.
- Static checks of translation-key references, unchanged settings slug functions,
  serde/schemars identifiers, and fixed-binding IDs against the base.
- Applicable Rust formatting was applied after validation.

Two existing discovery tests waited indefinitely for events in the initial run:
`workspace_discovery_page_routes_each_option_through_the_expected_client_boundary`
and `discovery_options_are_cleared_on_joining_and_refetched_on_losing_a_team`.
The complete settings result excludes both with `--skip`. Their test bodies and
discovery control functions are unchanged from the base. Their runtime validation
is unresolved; they are not reported as passing.

Further verification should exercise the remaining GUI interactions in both
languages, including dropdown selection, shortcuts, menus, dialog layouts, offline
and error states, and session restoration. Linux/macOS builds and a real-display
integration run have not been completed. No full-workspace presubmit was run;
targeted validation followed the agent workflow in AGENTS.md.

## Risks and follow-up

The initial Spanish catalog includes machine-generated text and manual corrections.
It still needs language review and further literal extraction. Screenshots are
limited to two pages and differing account states. Maintainer scoping, readiness
labels, spec approval, any CLA requirements, and CI review remain independent of
the local checks. Keep the implementation PR as a draft until those prerequisites
and validation concerns are addressed.
