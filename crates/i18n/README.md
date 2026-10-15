# Interface localization

`warp_i18n` provides embedded English and Spanish catalogs for an initial set of GUI
surfaces. This is an experimental localization layer; some interface text remains in English.

## Selecting a language

Set `WARP_LANG` before starting the app:

```bash
WARP_LANG=es cargo run -p warp --bin warp-oss --features gui
WARP_LANG=en cargo run -p warp --bin warp-oss --features gui
```

In PowerShell:

```powershell
$env:WARP_LANG = 'es'
cargo run -p warp --bin warp-oss --features gui
```

Spanish regional variants such as `es-CL`, `es_ES.UTF-8`, and `es-419` resolve to `es`.
When `WARP_LANG` is unset, the engine checks `LANG`, `LANGUAGE`, `LC_ALL`, and `LC_MESSAGES`,
then the system locale. Unsupported languages use English. Restart the app after changing
the language; this implementation does not provide a settings picker or live UI updates.

## Catalogs and labels

The catalogs live in `resources/bundled/locales/en.yml` and `es.yml`. Both are embedded
in the executable. Optional catalogs from `WARP_LOCALES_DIR`, or the application's
bundled resources directory, override embedded values. Missing translations fall back
to English, then to the key itself.

```rust
let label = warp_i18n::t!("workspace.settings");
let query = "example";
let description = warp_i18n::t!("ai_output.query_suffix", query);
```

Labels without arguments return `&'static str`. Named arguments are inserted once,
without interpreting placeholder-like text inside their values. Add new keys to both
catalogs, preserve placeholders and technical identifiers, and translate only display
text. Actions, settings keys, session slugs, URLs, and command identifiers remain stable.

The Spanish catalog was initially generated with OPUS-MT. Settings, dropdown options,
onboarding tips, and built-in suggestions have additional manual translations.
Generated agent responses and external command descriptions retain their original language.
The remaining catalog and GUI layout still require language review and manual verification.

## Validation

```bash
cargo test -p warp_i18n
cargo test -p warp --lib settings_view:: --features gui
cargo test -p warpui_core --lib localized_window_menu_retains_system_role
cargo clippy -p warp_i18n --all-targets --tests -- -D warnings
cargo check -p warp --bin warp-oss --features gui
```

Manual review should cover startup, settings navigation, tab menus, authentication,
dialogs, and long labels in both languages. Verify that existing session restoration
and shortcuts still work. On macOS, also verify the system Window menu behavior.

## Prior work

The YAML catalog structure and initial UI string inventory are adapted from
[PR #10630](https://github.com/warpdotdev/warp/pull/10630). The related
[PR #11382](https://github.com/warpdotdev/warp/pull/11382) was reviewed when comparing
localization approaches.
