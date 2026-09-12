# Incremental Internationalization and Simplified Chinese — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/10928

Related issue: https://github.com/warpdotdev/warp/issues/1194

Related proposal: https://github.com/warpdotdev/warp/issues/14397

Related prior spec: https://github.com/warpdotdev/warp/pull/10990

Figma: none provided

## Summary

Add the first production-ready internationalization foundation to Warp and
validate it with a deliberately small Simplified Chinese (`zh-CN`) release.
Users choose the UI language manually in Settings. English remains the default
and complete fallback language.

The first release translates only the language picker, the command palette, and
low-risk help/navigation entries in the user menu. Later product surfaces are
migrated in separate changes after the foundation is approved. This keeps the
initial review and visual validation bounded while establishing the behavior
required for additional languages.

## Problem

Warp currently renders its application UI in English. Chinese-speaking users
cannot select a Simplified Chinese interface, and contributors do not have a
standard way to add or maintain translations.

Earlier community implementations attempted to migrate most of the application
at once and proposed a custom message format. That created unresolved questions
about pluralization, fallback behavior, developer maintenance, enforcement,
dynamic strings, and the review risk of changing thousands of call sites in one
pull request.

## Goals

- Let users explicitly select English or Simplified Chinese from Settings.
- Keep English as a complete, reliable fallback.
- Preserve product names and technical terms when English communicates the
  concept more accurately.
- Support variables, plural-sensitive messages, and reusable terminology from
  the start.
- Make the first implementation small enough to review, test, and visually
  verify.
- Establish a repeatable contribution path for later language and product
  surface migrations.

## Non-goals

- Automatically following the operating-system language in the first release.
- Changing the language without restarting Warp.
- Translating the entire application in one pull request.
- Translating terminal output, commands, file paths, keyboard shortcuts,
  user-authored text, model output, server-provided content, or third-party
  content.
- Translating authentication, billing and purchase confirmations, sharing and
  permission prompts, or Agent consent/action descriptions in the first
  release.
- Localizing the TUI or web/wasm surfaces in the first release.
- Synchronizing the selected language through Warp Drive.
- Localizing documentation outside the Warp application.

## Terminology

The Simplified Chinese catalog keeps product names and technical terms in
English when translation would make them less precise. The initial protected
terms are:

- Warp
- Warp Drive
- Agent
- Token
- Workflow
- MCP
- API
- CLI
- Shell
- Block
- Model names

Commands, file paths, keyboard shortcuts, identifiers, and user data also remain
unchanged. Ordinary interface copy is translated, for example `Settings` to
`设置`, `Search` to `搜索`, and `Feedback` to `反馈`.

## Behavior

1. When no language has been selected, Warp renders the application UI in
   English (`en-US`). Existing users see no language change after upgrading.

2. Settings → Appearance contains a **Language** section with one selector.
   The selector initially offers exactly:
   - `English`
   - `简体中文`

3. Language options are always displayed using their self-names above, rather
   than translating both option names into the currently active language.

4. Selecting a language saves the preference on the current device. It is not
   uploaded or synchronized to other devices.

5. Changing the selection does not partially relocalize the current process.
   The Language section displays a localized `Restart Warp to apply this
   language` notice after the saved selection differs from the active language.

6. After the user quits and relaunches Warp, the saved supported language is
   active before the first application window is rendered.

7. If the stored language value is missing, invalid, or no longer supported,
   Warp uses English and keeps Settings usable so the user can make another
   selection.

8. The first Simplified Chinese release translates these complete, bounded
   surfaces:
   - the Settings window title, the Appearance navigation label, and the
     Language section;
   - the command palette search placeholder and empty-result message;
   - the low-risk user-menu entries for What's new, Settings, Keyboard
     shortcuts, Documentation, Feedback, View Warp logs, and joining the Warp
     Slack community.

9. User-menu actions involving updates, signup, billing, upgrades, referrals,
   and logout remain English in the first release. Authentication, payments,
   permissions, sharing, and Agent consent/action surfaces remain English until
   each surface receives separate translation and security review.

10. If a message is unavailable or cannot be formatted in the selected
    language, Warp renders the canonical English message. A missing translation
    never produces an empty label, raw message identifier, or unusable control.

11. Dynamic values are inserted into localized messages without being
    translated or interpreted as localization syntax. This includes usernames,
    versions, model names, paths, repository names, commands, and values
    received from a server.

12. A message that depends on a numeric quantity selects the correct grammatical
    variant for the active language. English singular/plural behavior and
    Simplified Chinese behavior are tested even if the first migrated UI slice
    does not yet expose every plural form.

13. Protected product and technical terms render exactly as defined in the
    terminology section in both English and Simplified Chinese.

14. Localized resources provide plain text only. They cannot introduce links,
    actions, Markdown, or other executable/rich UI structure. Interactive
    elements remain defined by trusted application code.

15. Localization changes visible text only. Existing action identifiers,
    settings paths, deep links, analytics values, keyboard behavior, focus
    order, and accessibility roles remain stable and language-independent.

16. At supported UI scale factors, translated controls remain readable without
    unintended clipping, overlap, replacement glyphs, or loss of their
    accessible labels.

17. English labels and behavior on the migrated surfaces remain identical to
    the current English UI, except for the addition of the Language setting.

18. A developer-only pseudo-localized locale may be used for layout testing, but
    it is not shown in the end-user Language selector.

## Validation

- Start Warp with no saved language and confirm the migrated surfaces remain
  unchanged in English.
- Select `简体中文`, confirm the restart notice appears, relaunch, and confirm
  the phase-one surfaces render in Simplified Chinese.
- Relaunch a second time and confirm the device-local preference persists.
- Switch back to `English`, relaunch, and confirm English is restored.
- Inject an invalid stored language in a test and confirm Warp starts in
  English with a usable Language selector.
- Force a missing Simplified Chinese message in a test and confirm the canonical
  English message is rendered.
- Verify variable and plural examples in both supported languages.
- Capture English and Simplified Chinese screenshots of Settings, the command
  palette, and the user menu at the default UI scale.
- Check translated screenshots for clipping, alignment, missing glyphs, and
  unchanged action behavior.

## Rollout

The first implementation is guarded by one localization feature flag while it
is validated in development and dogfood builds. The same flag controls the
Language selector and translated call sites so users never see a selector that
does not affect the advertised surfaces. The flag is removed after the initial
release is stable.

Later migrations are organized by complete product surface, with separate
translation and visual review. Security-sensitive surfaces require explicit
human copy review before they can be enabled in a non-English locale.

## Open questions

None for the first implementation. Automatic system-locale detection, live
language switching, cloud synchronization, and additional surfaces/locales are
explicit follow-up decisions.
