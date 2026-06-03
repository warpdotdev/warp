# i18n — Warp UI internationalization

Lightweight, dependency-free (within the workspace) internationalization for the
Warp client. Provides a runtime-switchable locale and a `t("key")` lookup that
drops into every Warp text constructor. Ships with **English** and **Simplified
Chinese**, defaulting to Chinese.

## How it works

- Translation catalogs live in [`locales/`](./locales) — one flat JSON
  `key -> string` file per locale (`en.json`, `zh-CN.json`). They are embedded
  into the binary at build time with `rust-embed` (same approach as the
  `languages` crate).
- A single active locale is held in a global `RwLock`. `i18n::set_locale(tag)`
  rebuilds the active catalog; `i18n::t(key)` reads it.
- Lookups degrade gracefully: `zh-CN` resolves through `en → zh → zh-CN`
  (more specific overrides less specific, English is the base). A key missing
  from every catalog returns the key itself, so the UI never blanks or panics.

## API

```rust
i18n::set_locale("zh-CN");               // set active locale (call at startup + on change)
let s: String = i18n::t("settings.x");   // translate; returns owned String
let s: String = i18n::t!("settings.x");  // macro form (identical)
let tag: String = i18n::current_locale();
```

`t()` returns `String`, which satisfies `impl Into<Cow<'static, str>>` — the
type accepted by `Text::new(..)`, `Span::new`, `Paragraph::new`, dialog/tooltip
params, etc. For button labels (which take `String`) it drops in directly.

## Wiring in the app (already done)

- **Setting**: `app/src/settings/language.rs` defines `Language { ZhCn (default),
  En }`, persisted to `settings.toml` as `appearance.language = "zh-CN"`.
- **Startup**: `app/src/settings/init.rs` reads the saved language and calls
  `i18n::set_locale(..)` before the first frame.
- **Live switching**: `init.rs` subscribes to the language setting; on change it
  swaps the catalog and calls `ctx.invalidate_all_views()` to repaint every view
  — no restart.
- **Switcher UI**: a dropdown in the **Appearance** settings page
  (`app/src/settings_view/appearance_page.rs`, the `LanguageWidget`).

## Adding translations

1. Add the English string to `locales/en.json` under a namespaced key, e.g.
   `"settings.account.log_out": "Log out"`.
2. Add the Chinese string to `locales/zh-CN.json` under the same key. (Omit it to
   fall back to English.)
3. At the **call site**, replace the literal with `i18n::t("settings.account.log_out")`.

### Which call sites are safe to translate

Prefer sites evaluated **on every render** so they update live when the language
changes:

| Pattern | Safe? | Notes |
|---|---|---|
| `Text::new("X", ..)` inside a `render`/`view` method | ✅ | `Text::new(i18n::t("key"), ..)` — type-safe, updates live |
| `.with_text_label("X".into())` / button labels | ✅ | `.with_text_label(i18n::t("key"))` |
| dropdown item / inline labels built per render | ✅ | |
| `const FOO: &str = "X";` | ⚠️ | `t()` can't run in `const` context — translate at the **use site** instead, or convert the const to a `fn() -> String` |
| `Category::new("X", ..)`, `SettingsUmbrella::new("X", ..)`, struct fields typed `&'static str` | ⚠️ | These take `&'static str` and are set once at construction, so they neither accept a `String` nor update live. Add a dedicated render-time accessor (e.g. a `nav_label(&self) -> String`) and translate where it's rendered, **not** where it's stored |
| `impl Display for SettingsSection` | ⛔ | Do **not** localize `Display` — it feeds crash-reporting tags and `FromStr`. Add a separate `nav_label()` method for the UI |
| Native app menu bar (`app/src/app_menus.rs`) | ⚠️ | Menus are built once and registered with the OS; they render in the startup language correctly but won't switch live without a menu rebuild/restart |

## Tests

```sh
cargo test -p i18n
```
