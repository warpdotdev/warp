# Warp Swift

Native macOS port using AppKit, SwiftTerm, and the system zsh. Open
`WarpSwift.xcodeproj` in Xcode to run or debug it.

Build the local Apple Silicon release from the repository root:

```sh
xcodebuild -project macos-swift/WarpSwift.xcodeproj -scheme WarpSwift \
  -configuration Release -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath macos-swift/build build
open macos-swift/build/Build/Products/Release/WarpSwift.app
```

The app is ad hoc signed and has a separate bundle identifier. Xcode may ask
you to trust SwiftTerm's build tool plugin. The resolved package revisions are
in the project workspace's `Package.resolved`.

Enter submits a command, Shift-Enter inserts a newline, and Cmd-Shift-Enter
moves focus to direct shell input. At the next command the integration resumes.
Up (in a single-line draft) or Ctrl-R opens command history. Type to filter it,
use Up and Down to preview matches directly in the editor, and press Enter to
keep the selected command for editing. Escape restores the original draft. Right Arrow accepts an inline history
suggestion at the end of the command. History is local to Warp Swift and
skips commands that begin with whitespace.

Tab requests completion from the current integrated zsh session. A single
result is inserted. With multiple results, Tab or the arrow keys select a
candidate and Enter inserts it. Escape closes the list. Completion uses zsh
definitions and is available in the editor for local and integrated nested
zsh sessions.
Cmd-T opens a native tab, Cmd-D splits the window, and Cmd-F searches the
focused terminal. Cmd-Shift-[ and Cmd-Shift-] navigate command boundaries in
scrollback. Run `warpify-zsh` to open an integrated nested local zsh.

## Comparison status

The reference is Warp at `c6f07bb45` with the existing local desktop changes.
The checked-out source provides the reference for the rows below. The installed
Warp app is a different build. The reference build attempt stopped at an uncached
Rust dependency, so timing results are pending.

| Scenario | Reference source | Swift status |
| --- | --- | --- |
| Editable command, ANSI/Unicode, exit code | `app/src/terminal/input/terminal.rs` | Local zsh. Submission and focus restoration exercised on macOS. |
| Command recall and suggestions | `app/src/terminal/input/inline_history` | Persistent local recall, inline prefix suggestions, and current-session priority. Tab requests native zsh candidates for commands, paths and supported flags. Shell history import is pending. |
| Command/output grouping and navigation | `app/src/terminal/view` | A bottom-pinned timeline renders commands and captured output as separate blocks and highlights nonzero exits. Interactive programs remain attached to the live terminal. Block actions, Git prompt chips, and Warp’s custom tab strip are pending. |
| Interactive foreground programs | `app/src/terminal/view` | Direct PTY input. Verified with vim and a scripted cat/SIGINT run. |
| Warpify | `app/src/terminal/warpify` | Explicit nested local zsh integration. SSH uses direct terminal input. Remote integration, bash, and fish are pending. |
| Tabs, splits, search, themes, settings | `app/src/workspace` | Native tabs and splits and SwiftTerm search. Theme and settings parity are pending. |

Run `python3 macos-swift/Tests/shell_integration_test.py` to exercise multiline
input, exit status, nested zsh, and Ctrl-C in a real PTY.

For baseline performance, run both release builds on the same Mac with equal
window size, font, and scrollback. After one warmup, record five runs for input
latency (p95, idle and with output in a neighboring pane), time to render the
final block of fixed ANSI output, peak RSS, and scroll frame time (p95).
When SSH integration lands, also record first remote prompt to editor-ready.
Profile any degradation over 20% in Instruments and rerun the same workload.
