# Changelog Draft
**Channel:** stable
**Range:** v0.2026.04.29.08.56.stable_00 → v0.2026.05.06.09.12.stable_00
**Generated:** 2026-05-06T19:00:00Z
**Total PRs in range:** 211 | **Explicit markers:** 57 | **Unmarked:** 154

---

## New Features
- You can now drag tabs out of a window into their own window, or between windows, similar to Chrome. ([#9275](https://github.com/warpdotdev/warp/pull/9275))
- Added a `/set-tab-color` slash command for setting or clearing the current tab's color from the input bar. ([#9305](https://github.com/warpdotdev/warp/pull/9305))

## Improvements
- Added tab context menu actions to copy visible tab and pane metadata when available. ([#10120](https://github.com/warpdotdev/warp/pull/10120))
- The conversation details panel can now be opened and closed with a configurable keyboard shortcut. ([#9837](https://github.com/warpdotdev/warp/pull/9837))
- Conversation details side panel is now available for local Warp Agent conversations, not just cloud Oz runs. Click the info button in the pane header to open it for any active AI conversation. ([#9493](https://github.com/warpdotdev/warp/pull/9493))
- Reduced memory usage and CPU work in the agent runs management view while a conversation is streaming. ([#9866](https://github.com/warpdotdev/warp/pull/9866))
- Added support for drag-and-drop of image files into an active CLI agent session (e.g. Claude Code). ([#9553](https://github.com/warpdotdev/warp/pull/9553))
- Warp now renders inline local images and Mermaid diagrams in agent block output. ([#9993](https://github.com/warpdotdev/warp/pull/9993))
- Warp now silently falls back to a regular SSH session on remote hosts where the prebuilt remote-server binary is incompatible (e.g. glibc < 2.31), instead of attempting an install that would fail at runtime. ([#9681](https://github.com/warpdotdev/warp/pull/9681))
- HTML files using the .htm extension now open with HTML syntax highlighting in Warp's editor. ([#9360](https://github.com/warpdotdev/warp/pull/9360))
- Recognize Block's `goose` CLI agent — running `goose` now activates the CLI-agent toolbar, status, brand color, and icon like other recognized third-party agents. ([#9497](https://github.com/warpdotdev/warp/pull/9497))
- Added a `/continue-locally` slash command to continue cloud conversations locally. ([#9500](https://github.com/warpdotdev/warp/pull/9500))
- Added a "Show in Finder" (macOS) / "Show containing folder" (Linux/Windows) option to the tooltip that appears when clicking a detected file link. ([#9475](https://github.com/warpdotdev/warp/pull/9475))
- Tighten orchestration event subscription scope so SSE runs only for active parent and child agent runs. ([#9273](https://github.com/warpdotdev/warp/pull/9273))
- Fix macOS IME candidate popup positioning in code editor panes so it anchors to the editor caret instead of stale terminal/input positions. ([#9555](https://github.com/warpdotdev/warp/pull/9555))

## Bug Fixes
- Fixed /feedback recording "Unknown" instead of the installed Warp version on packaged builds. ([#10219](https://github.com/warpdotdev/warp/pull/10219))
- Fixed find (cmd+f) selection jumping to a different match when new output streams into the active block. ([#10057](https://github.com/warpdotdev/warp/pull/10057))
- Fix Japanese IME losing the last character of a phrase that ends right before a punctuation mark on macOS. ([#9730](https://github.com/warpdotdev/warp/pull/9730))
- Fixed local file tree blinking/reshuffling when connected to an SSH session ([#10184](https://github.com/warpdotdev/warp/pull/10184))
- Fixed terminal text selection not auto-scrolling when dragging beyond bounds ([#9448](https://github.com/warpdotdev/warp/pull/9448))
- Fixed Ctrl-G not closing CLI agent rich input on linux when editor is focused ([#10030](https://github.com/warpdotdev/warp/pull/10030))
- Pressing backspace in the agent view when the buffer is empty no longer resets the conversation. ([#10114](https://github.com/warpdotdev/warp/pull/10114))
- Fixed unnecessary reconnect attempts for remote SSH sessions after system sleep, reducing error noise ([#10096](https://github.com/warpdotdev/warp/pull/10096))
- Fixes issue with repeated TUI redraws for CLI agents on terminal pane resize. ([#9877](https://github.com/warpdotdev/warp/pull/9877))
- Fix new-session "+" dropdown alignment when the Tabs Panel is placed on the right side of the header toolbar. ([#9492](https://github.com/warpdotdev/warp/pull/9492))
- Copy keybinding now prioritizes selected text in the input over a selected block when both are active. ([#9491](https://github.com/warpdotdev/warp/pull/9491))
- [Windows] Fix hotkey window. ([#9891](https://github.com/warpdotdev/warp/pull/9891))
- [Windows] Symlink traversal fixed. ([#9863](https://github.com/warpdotdev/warp/pull/9863))
- Fixed a crash on Windows when handing off a Web conversation to the native client. ([#9987](https://github.com/warpdotdev/warp/pull/9987))
- Fixed a bug where multiple 'open skill' buttons shared hover state. ([#9437](https://github.com/warpdotdev/warp/pull/9437))
- Fixed the OSS Linux desktop entry so WarpOss launches through the packaged `warp-terminal-oss` command. ([#9424](https://github.com/warpdotdev/warp/pull/9424))
- Fixed Ctrl/Cmd shortcuts (e.g. copy, paste) failing on Windows when a non-Latin keyboard layout was active. ([#9476](https://github.com/warpdotdev/warp/pull/9476))
- Fixed background colour bleeding in alt screen programs (e.g. delta, diff-so-fancy). ([#9852](https://github.com/warpdotdev/warp/pull/9852))
- Clip the warping indicator's action chips onto a new line on narrow panes instead of overflowing. ([#9297](https://github.com/warpdotdev/warp/pull/9297))
- Inline `.bmp`, `.tiff` / `.tif`, and `.ico` images in agent block output now render correctly. ([#9397](https://github.com/warpdotdev/warp/pull/9397))
- If user attaches an image in block input we should lock in agent mode, without running the NLD classifier. ([#9366](https://github.com/warpdotdev/warp/pull/9366))
- Remote-server installs no longer fail when the staging-directory cleanup hits a race. ([#9681](https://github.com/warpdotdev/warp/pull/9681))
- `.command` shell scripts now open with shell syntax highlighting in Warp's editor. ([#9345](https://github.com/warpdotdev/warp/pull/9345))
- Fix git diff chip flickering between tracked-only and all-files count when untracked files are present ([#9244](https://github.com/warpdotdev/warp/pull/9244))
- `Open File → Default App` now opens files in the running Warp channel instead of routing to a different installed Warp. ([#9285](https://github.com/warpdotdev/warp/pull/9285))
- Fixed vertical tabs settings popup items being unclickable ([#9540](https://github.com/warpdotdev/warp/pull/9540))
- Fixed a macOS memory leak that occurred when Warp enumerated system fonts or built a font fallback chain. ([#9665](https://github.com/warpdotdev/warp/pull/9665))
- Executable shell scripts opened from a `file://` URL now run in the terminal instead of opening in the editor. ([#9503](https://github.com/warpdotdev/warp/pull/9503))
- Fixed Option+Enter, Option+Tab, and Option+Escape sending literal key names instead of correct escape sequences ([#9514](https://github.com/warpdotdev/warp/pull/9514))
- Fixed read_files tool showing an empty box when the LLM requests line ranges beyond the end of a file. ([#9326](https://github.com/warpdotdev/warp/pull/9326))
- Prevent Warp from consuming too much memory when identifying filepaths in long block outputs. ([#9617](https://github.com/warpdotdev/warp/pull/9617))
- Don't trigger the agent onboarding tutorial when Warp is running in headless SDK/CLI mode. ([#9590](https://github.com/warpdotdev/warp/pull/9590))
- Added `--version` flag support in the Oz CLI ([#9252](https://github.com/warpdotdev/warp/pull/9252))
- Fixed file tree flickering when transitioning to an SSH remote session ([#9320](https://github.com/warpdotdev/warp/pull/9320))
- Fixed scroll-to-start/end of selected block keybinding not working when the input is focused. ([#9332](https://github.com/warpdotdev/warp/pull/9332))
- Fix the terminal pane background appearing darker in horizontal tabs mode with background image or custom opacity. ([#9474](https://github.com/warpdotdev/warp/pull/9474))
- AI code blocks tagged `vue`, `xml`, `dockerfile`, `jsx`, `tsx`, etc. now render with syntax highlighting. ([#9471](https://github.com/warpdotdev/warp/pull/9471))
- Reopen Closed Session is now reachable from the new-session menu on Linux and Windows. ([#9347](https://github.com/warpdotdev/warp/pull/9347))
- Fixed missing syntax highlighting for C++ header files using `.hpp`, `.hxx`, or `.H` extensions. ([#9388](https://github.com/warpdotdev/warp/pull/9388))
- Fixed `/open-file` handling for relative WSL paths so Unix separators are preserved. ([#9322](https://github.com/warpdotdev/warp/pull/9322))

## Oz Updates
- Add Codex as a supported harness for local child agents. ([#10176](https://github.com/warpdotdev/warp/pull/10176))
- Configurable max context window per profile. ([#9352](https://github.com/warpdotdev/warp/pull/9352))

---

## Community
### Contributors
- @Abdalla-Eldoumani ✨
- @Akeuuh — [#9655](https://github.com/warpdotdev/warp/pull/9655) ✨
- @AntonVishal ✨
- @BennyWaitWhat ✨
- @Faizanq ✨
- @JamieMcMillan ✨
- @R3flector ✨
- @amriksingh0786 ✨
- @princepal9120 ✨
- @webdevtodayjason ✨
- @zerone0x ✨

### Issue Reporters
Thanks to the community members who reported issues fixed in this release:
- @user123 — [#5678](https://github.com/warpdotdev/warp/issues/5678) "Crash when opening large file"

---

*This draft was generated by the `changelog-draft` Oz skill. Needs Review and Skipped PRs are available in the JSON audit artifact.*
