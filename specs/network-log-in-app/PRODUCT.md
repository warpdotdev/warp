# Network log in-app pane
## Summary
The network activity log no longer writes to disk. When the user opens the network log console from Privacy settings or the existing keybinding, a read-only pane displays a snapshot of the most recent network requests and responses captured in memory.
## Figma
Figma: none provided
## Behavior
1. While the app is running, the most recent 50 network log items (request and response entries) are retained in memory. No network log file is written to disk anywhere on the user's system.
2. Each log item is a single formatted entry containing a timestamp plus the debug-formatted request or response, matching the format previously written to `warp_network.log`.
3. The feature is only exposed when `ContextFlag::NetworkLogConsole` is enabled. When the flag is disabled, the Privacy settings link, the keybinding, and the pane are all unavailable.
4. Clicking "View network logging" in Privacy settings opens the network log pane as a right-split of the active pane group.
5. Triggering the `input:insert_network_logging_workflow` keybinding ("Show Warp network log") opens the same pane with the same layout behavior.
6. If a network log pane already exists in the current window, both entrypoints focus that pane instead of opening a second one. At most one network log pane exists per window.
7. On open, the pane renders a one-shot snapshot of the in-memory items at that moment, in chronological order, one item per line. The pane does not update live as new requests are made.
8. Reopening the pane (closing and relaunching, or focusing from either entrypoint after new activity) re-seeds the view with the current snapshot.
9. When the in-memory log is empty at open time, the pane opens with an empty editor body. No error or placeholder toast is shown.
10. The pane content is read-only: the user cannot type into it, delete lines, or otherwise mutate the log. Standard read-only code editor affordances remain available: scroll, select, copy, and find-in-editor.
11. The pane header shows "Network log" as its title and supports the standard pane chrome (close, split, focus, drag between tabs) like other panes.
12. Closing the pane does not affect the underlying in-memory log. New requests continue to be captured and will appear the next time the pane is opened.
13. The pane is not restored across app restart. On relaunch, the pane is absent until the user explicitly reopens it via settings or the keybinding, and the in-memory log starts fresh.
14. The previously available "Tail Warp network log" workflow is removed from the command palette and workflow list. Users will no longer find a workflow that runs `tail -f` on a network log file.
15. The existing Privacy settings copy describing the "native console" for viewing network communications continues to apply; clicking the link opens the new pane rather than running a shell command in a terminal.
