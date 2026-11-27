# Disable file path links in terminal output — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/12890
Figma: none provided

## Summary
Warp should add a user setting that disables automatic file and directory path linkification in terminal content while leaving regular URL links clickable. The default remains today’s behavior so existing users keep clickable file paths unless they opt out.

## Problem
Warp currently highlights existing file and directory paths as links when the pointer moves over terminal output. That hover styling and clickable behavior is useful for users who open files from terminal output, but it is distracting for users who do not want ordinary path-shaped text to become interactive. The existing `general.link_tooltip` setting only controls whether clicking a link opens a tooltip; it does not disable file path detection, pointing-hand cursor changes, hover highlighting, context-menu file actions, or direct file opening.

## Goals
- Provide one visible, persisted setting that lets users disable automatic file and directory path links in terminal content.
- Preserve regular URL link behavior when file path links are disabled.
- Preserve the existing default behavior for users who do not change the setting.
- Keep the setting cross-platform where Warp can detect file paths, with no separate per-shell or per-session configuration.
- Make the setting discoverable from Settings search and user config.

## Non-goals
- Disabling URL links such as `https://example.com`, markdown links, or explicit terminal hyperlink sequences.
- Changing how file paths are parsed, normalized, or opened when file path links remain enabled.
- Removing file paths from terminal output, changing copy/selection text, or changing command output rendering.
- Disabling file-related actions in dedicated file UIs, editors, file explorers, code review panes, or other non-terminal surfaces.
- Replacing `general.link_tooltip`; tooltip behavior and file-path detection remain separate settings.

## Behavior
1. Warp exposes a boolean setting named in user-facing language as “Link file paths in terminal output” or equivalent. The enabled state means Warp automatically detects existing file and directory paths in terminal content and makes them interactive. The disabled state means Warp does not make file or directory paths interactive.

2. The setting defaults to enabled for new and existing users. Users who never change the setting experience no visible behavior change.

3. When the setting is enabled, current file path link behavior is preserved:
   - Hovering over a detected file or directory path changes the cursor to the link cursor and applies the existing hover treatment.
   - Clicking or modifier-clicking the path follows the existing open-file/open-folder behavior.
   - The link tooltip and context menu continue to offer the existing file actions, subject to the separate tooltip setting.
   - Line and column suffix handling such as `src/main.rs:10` continues to work as it does today.

4. When the setting is disabled, plain file and directory paths in terminal content remain ordinary selectable text:
   - Hovering over a path does not change the cursor to the link cursor.
   - The path does not receive link hover highlighting.
   - Clicking, modifier-clicking, or middle-clicking the path does not open the file or directory.
   - No file-link tooltip appears for that path.
   - Right-clicking the path does not show file-link-specific menu items such as copy path, show containing folder, open in editor, or open in Warp unless another explicit surface already provides those actions independently of terminal path linkification.

5. URL behavior is unchanged when file path links are disabled:
   - URLs in terminal grids and block output are still detected as links.
   - URL hover styling, link cursor behavior, click/modifier-click behavior, tooltips, and URL context-menu actions continue to follow the existing URL-link rules.
   - A string that is both part of a URL and path-shaped is treated as a URL, not as a file path, matching the current URL-over-file overlap rule.

6. Explicit links remain links. Markdown hyperlinks in rendered AI or rich terminal content and terminal hyperlink escape-sequence URLs continue to be interactive even when file path links are disabled, as long as they are represented as URLs rather than implicit filesystem path detections.

7. The setting applies consistently across terminal-rendered content where Warp currently performs implicit local file path detection:
   - Command output in the block list.
   - Alt-screen terminal content.
   - Rendered terminal rich content that currently uses the same detected-link system, including AI block text, requested-action text, and user-query text shown inside the terminal pane.
   - Restored terminal blocks and restored rich content after they re-render.

8. The setting is evaluated live. If a user disables file path links while a terminal pane is open, existing file-path hover state clears and subsequent hovers do not reopen file links. If a background path-detection job was already running, its results must not reintroduce file path links after the setting has been disabled.

9. If a user re-enables file path links, future hovers and future rich-content link-detection passes behave as they did before the setting was disabled. Warp is not required to immediately scan every visible line, but the next normal hover or rendered-content refresh should detect paths again.

10. Selection and copy are unaffected. Users can still select and copy path text whether the setting is enabled or disabled. Disabling file path links must not change block copy, selected text copy, copy-on-select, or pasted command output.

11. Remote-session behavior remains consistent with current rules. If Warp already avoids local file path detection for a remote block or surface, the new setting does not make remote file paths interactive. Disabling the setting also prevents any file path linkification that would otherwise be allowed for a local session.

12. The setting is persisted and synced like comparable general user preferences. Changing it in Settings or user config updates the same underlying value and survives app restart.

13. The setting is searchable. Searching Settings for terms such as “file path”, “link”, “clickable path”, “terminal output”, and “open file” should find the control.

## Success criteria
1. With the setting enabled, hovering and opening an existing local file path in terminal output works the same as it does today.
2. With the setting disabled, hovering the same local file path produces no file-link hover style, no pointing-hand cursor, no file tooltip, and no file-opening behavior.
3. With the setting disabled, hovering and opening `https://example.com` still works.
4. Toggling the setting off while a file path is currently highlighted clears the highlight and prevents stale background detection from restoring it.
5. The setting appears in Settings, is discoverable by Settings search, and persists through restart or config reload.
6. Existing text selection and copy behavior for path text is unchanged.

## Validation
- Add unit coverage for the terminal hover path showing that URL detection still runs while file path scanning is skipped when the setting is disabled.
- Add unit coverage for rich-content link detection showing that `DetectedLinkType::Url` remains present and `DetectedLinkType::FilePath` is omitted when the setting is disabled.
- Add settings tests or view-level coverage for the new Settings control, action, search terms, persisted default, and toggle behavior.
- Add a regression test for stale background file-path detection results not reintroducing a file link after the setting turns off.
- Manually validate in a local terminal block containing an existing file path and a URL on the same line: toggle on, toggle off, toggle on again, and verify only the file path behavior changes.
- Manually validate in alt screen and restored block-list output when practical.

## Open questions
- Exact final label text can be bikeshedded during implementation. The spec assumes the setting’s intent remains “Link file paths in terminal output” and that the default stays enabled.
