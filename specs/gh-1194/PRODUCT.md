# PRODUCT: i18n / Multiple Language Support

## Summary

Add internationalization (i18n) support to Warp, enabling the application UI to render in languages other than English. Simplified Chinese (zh-CN) is shipped as the first non-English locale. English remains the default and fallback language. Locale selection is automatic based on system settings, with an environment variable override for power users.

## Problem

Warp's UI is entirely hardcoded in English. Non-English-speaking developers must navigate menus, settings, agent UI, and onboarding in a language they may not be fluent in, creating friction and limiting adoption. Internationalization is consistently one of the most-requested features from the Warp community.

## Goals / Non-goals

**Goals:**
- Provide a complete zh-CN translation of the Warp UI
- Establish an i18n framework that supports adding more locales in the future
- Automatic locale detection that requires zero configuration
- Graceful fallback to English for any untranslated strings

**Non-goals:**
- A language picker in Settings UI (this is env-var-only for the initial release)
- Pluralization or locale-aware number/date formatting
- Right-to-left (RTL) language support
- Runtime locale switching without application restart

## Behavior

### 1. Locale detection

1.1. The active locale is always exactly `"zh-CN"` or `"en"`. These are the only two supported locales. The determination is:

    Candidate locale strings are selected from:
    1. `WARP_LANG` environment variable
    2. Platform-native system locale API
    3. `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG` environment variables, only when the platform-native locale API is unavailable or returns no locale
    4. Default: `"en"`

    Each candidate is normalized before classification:
    - Trim whitespace.
    - For `LANGUAGE`, split colon-separated preference lists and evaluate each entry in order.
    - Strip encoding and modifier suffixes after `.` or `@` (for example, `.UTF-8`, `.utf8`, `@pinyin`).
    - Treat `_` and `-` as equivalent separators, and compare language/script/region subtags case-insensitively.

    The first non-empty source is authoritative:
    - `WARP_LANG` is an explicit override. If it is set, `zh`, `zh-CN`, `zh_CN`, `zh-Hans*`, or `zh_Hans*` resolve to `"zh-CN"`; every other value resolves to `"en"` and no lower-priority source is consulted.
    - The platform-native system locale API is the zero-configuration desktop default. If it returns a locale, Simplified Chinese values resolve to `"zh-CN"`; every other value resolves to `"en"` and POSIX locale environment variables are not consulted.
    - POSIX locale environment variables are a fallback for headless, test, or platform API unavailable cases. For `LANGUAGE`, entries are evaluated in order as a preference list. The first Simplified Chinese entry resolves to `"zh-CN"`. Unsupported entries, including `fr`, `zh-TW`, `zh-HK`, and `zh-Hant*`, are skipped within that list; if no entry resolves to zh-CN, the result is `"en"` and no lower-priority source is consulted.
    - For fallback `LC_ALL`, `LC_MESSAGES`, and `LANG`, Simplified Chinese values resolve to `"zh-CN"`; every other value resolves to `"en"` and no lower-priority source is consulted.

    Examples:
    - `WARP_LANG=zh-CN` → "zh-CN" candidate → zh-CN
    - `WARP_LANG=zh_CN.UTF-8` → "zh-CN" candidate → zh-CN
    - `WARP_LANG=zh-Hans` → "zh-Hans" candidate → zh-CN
    - `WARP_LANG=zh-TW` → "zh-TW" candidate → en (Traditional Chinese is not shipped yet)
    - `WARP_LANG=fr` → "fr" candidate → en (non-zh candidate, no fallthrough to system)
    - No `WARP_LANG`, Simplified Chinese OS, `LANG=en_US.UTF-8` → system "zh-CN" → zh-CN
    - No `WARP_LANG`, English OS, `LANG=zh_CN.UTF-8` → system "en-US" → en
    - `LANGUAGE=fr:zh_CN:en` with `WARP_LANG` unset and no system locale → first supported fallback candidate is "zh-CN" → zh-CN
    - `LANGUAGE=fr:zh_TW:en` with `WARP_LANG` unset and no system locale → no Simplified Chinese fallback candidate → en, no fallthrough to `LANG`
    - `LC_ALL=fr_FR.UTF-8` and `LANG=zh_CN.UTF-8` with `WARP_LANG` unset and no system locale → en because `LC_ALL` is the first non-empty fallback source
    - `LANG=zh_CN.UTF-8@pinyin` with `WARP_LANG` unset and no system locale → "zh-CN" candidate → zh-CN
    - No env vars, Simplified Chinese OS → system "zh-CN" → zh-CN
    - No env vars, Traditional Chinese OS → system "zh-TW" → en (Traditional Chinese is not shipped yet)
    - No env vars, English OS → system "en_US" → en
    - No env vars, no system locale → default "en" → en

1.2. The application ships with exactly two locale files: `en.yml` and `zh-CN.yml`.

### 2. Translation rendering

2.1. Every user-facing text string in the Warp UI is translated via the `t!("dot.path")` macro or the security-sensitive `t_required!("dot.path", "English fallback")` macro. The key argument is a dot-separated path into a YAML locale file. The English value at that path in `en.yml` serves as the fallback text when no translation exists for the active locale. The Chinese value at the same path in `zh-CN.yml` is shown when zh-CN is active.

2.2. When a translation is requested for a given key:
    1. The active locale file is checked first.
    2. If the key is missing from the active locale, the English (`en`) locale file is checked.
    3. For ordinary, non-sensitive UI, if the key is missing from both, the raw key string is displayed (e.g., `"menu.file"`). Security-sensitive exceptions are defined in section 6.

2.3. The entire UI surface is covered: macOS menu bar, tab context menus, workspace toolbar, settings panels (AI, appearance, keybindings, billing, features), agent UI, onboarding, auth dialogs, resource center, code review, terminal prompts, tooltips, notifications, and all modal dialogs. In total, approximately 2,700 distinct UI strings are translated.

### 3. String interpolation

3.1. Some translated strings contain dynamic values (e.g., `"Hand off to {environment}"`). These placeholders use `{name}` syntax within the YAML locale strings.

3.2. At render time, the translation macro accepts named arguments: `t!("key", name = value)`. The value is converted to a string and substituted for `{name}` in the translated template.

3.3. If a key with interpolation arguments is missing from both locale files, ordinary non-sensitive UI displays the raw key (interpolation is not applied to the fallback key). Security-sensitive exceptions are defined in section 6.

### 4. Locale file format

4.1. Locale files use YAML and are stored at `resources/bundled/locales/<locale>.yml`.

4.2. The top-level key in each file is the locale name (e.g., `en:`, `zh-CN:`). Nested YAML structure maps to dot-separated translation keys. For example:
    ```yaml
    en:
      menu:
        file: "File"
        edit: "Edit"
    ```
    produces keys `menu.file` and `menu.edit`.

4.3. English (`en.yml`) is the canonical source of truth for key names. Every key present in `en.yml` must have a corresponding entry in `zh-CN.yml`.

### 5. Locale switching

5.1. There is no in-app language switcher in the initial release.

5.2. To change locale, the user sets `WARP_LANG=zh-CN` (or `WARP_LANG=zh`) before launching Warp, and the change takes effect on next launch.

5.3. Setting `WARP_LANG` to a non-Simplified-Chinese value explicitly forces English, even on a Chinese-system machine. Only unsetting `WARP_LANG` restores system-locale detection.

### 6. Fallback and missing translations

6.1. If a translation key is present in `en.yml` but missing from `zh-CN.yml`, the English string is shown. No warning or error is surfaced to the user.

6.2. For ordinary, non-sensitive UI, if a translation key is missing from both `en.yml` and `zh-CN.yml` (e.g., a newly-added UI string that has not been localized yet), the raw key string is shown. This ensures the application never panics or renders empty text due to a missing translation.

6.3. Security-sensitive UI must not degrade to raw dot-path keys. Auth, billing, sharing/permission, and agent-consent surfaces must either render meaningful embedded English fallback text or fail closed by disabling the affected action and showing a readable English error.

6.4. Security-sensitive call sites must use the required-translation API defined in the technical spec. That API accepts an embedded English fallback string and records the key as security-sensitive for automated checks. Raw-key fallback is allowed only for ordinary UI that has no auth, billing, permission, privacy, sharing, or agent-consent meaning.

6.5. The English locale file (`en.yml`) is always loaded regardless of the active locale, to serve as the fallback.

### 7. Platform consistency

7.1. macOS menu bar items are fully localized. zh-CN users see localized menu labels (e.g., "文件" for "File", "编辑" for "Edit").

7.2. Onboarding slides shown to new users on first launch are localized.

7.3. Keyboard shortcut labels and modifier key names are NOT translated (e.g., `⌘S` remains `⌘S` regardless of locale).

7.4. Terminal output (PTY content) is NOT affected by i18n. Only the Warp UI chrome is translated.

## Figma

Figma: none provided

## Open questions

- **Language picker in Settings:** Should a future iteration add a UI language selector in the Appearance or General settings? This would require runtime locale switching without restart.
- **Contributor workflow:** How should translators contribute locale files for additional languages? A CONTRIBUTING guide for new locale YAML files may be needed.
- **String extraction:** Should locale keys be auto-generated from source code, or manually authored in YAML? The current approach is manual YAML authorship.
