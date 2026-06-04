# TECH.md - Browser Pane for local previews and web lookup

Product spec: `specs/GH2164/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/2164

## Context

Warp already has the right product surface for this feature: typed panes inside a `PaneGroup`, plus terminal/file context menus and command-palette routes that can open secondary content beside the active terminal.

- `CONTRIBUTING.md` defines spec PRs as `specs/GH<issue-number>/product.md` and `tech.md`, with the tech spec grounded in code and covering context, proposed changes, and validation.
- `WARP.md` requires new product features to be gated by a high-level `FeatureFlag` in `warp_core/src/features.rs`, with UI hidden behind the same flag until launch.
- `app/src/pane_group/pane/mod.rs:1` documents the pane contract: each pane has a `BackingView` for rendering/interactions and a `PaneContent` wrapper for pane-group lifecycle.
- `app/src/pane_group/pane/mod.rs:137` defines `IPaneType`; a Browser Pane needs its own pane type so focus, titles, drag/drop, and snapshot plumbing stay type-safe.
- `app/src/pane_group/pane/file_pane.rs:20` is the closest local example of a pane wrapper around a child view. It wraps `FileNotebookView` in `PaneView`, forwards pane events, snapshots the pane, and delegates focus.
- `app/src/app_state.rs:119` defines `LeafContents`, the persisted pane-content snapshot enum. Browser Pane needs a minimal snapshot that stores only the last full URL.
- `app/src/persistence/sqlite.rs:1117` maps persisted `LeafContents` variants to pane kinds before writing pane-specific rows, and `app/src/persistence/sqlite.rs:2463` maps those pane kinds back to snapshots during restore. Browser Pane persistence needs coverage in both directions, not just a new `LeafContents` variant.
- `app/src/util/openable_file_type.rs:24` defines `EditorLayout::{SplitPane, NewTab}` and `FileTarget::{MarkdownViewer, CodeEditor, ...}`. Browser Pane should mirror the same split/new-tab concept without overloading file-target semantics.
- `app/src/workspace/view.rs:5893` routes `FileTarget::MarkdownViewer(layout)` to `open_file_notebook`.
- `app/src/workspace/view.rs:7375` implements `open_file_notebook`: `EditorLayout::NewTab` calls `add_tab_from_existing_pane`, while `EditorLayout::SplitPane` calls `pane_group.add_pane_with_direction(Direction::Right, ..., focus_new_pane = true)`.
- `app/src/code/file_tree/view.rs:2324` exposes the user-facing "Open in new pane" and "Open in new tab" labels for existing viewer-like content.
- `app/src/terminal/view/link_detection.rs:391` keeps primary-click URL handling external by calling `ctx.open_url(...)` for terminal URLs.
- `app/src/terminal/view/link_detection.rs:425` does the same for rich-content URLs.
- `app/src/terminal/view.rs:15918` builds terminal block context menus. URL context menus currently only expose "Copy URL".
- `app/src/terminal/view/context_menu.rs:42` builds AI rich-content link context menus. URL context menus currently only expose "Copy URL".
- `app/src/terminal/view.rs:1374` defines `ContextMenuAction`, `app/src/terminal/view.rs:1676` defines terminal view events, `app/src/pane_group/mod.rs:496` defines pane-group events, and `app/src/workspace/view.rs:14144` handles pane-group events at the workspace layer. Terminal URL menu actions should flow through this existing event bridge rather than dispatching workspace actions directly from terminal views.
- `app/src/search/command_palette/mixer.rs:18` defines `CommandPaletteItemAction`; URL/search opening from command palette needs either a new action variant or a binding that launches a URL/search prompt.
- `app/src/search/command_palette/mixer.rs:86` maps `CommandPaletteItemAction` values to `ItemSummary`; browser URL/search actions should map to `ItemSummary::NoOp`.
- `app/src/search/command_palette/view.rs:722` dispatches command-palette actions and is the place to route a command-palette browser result into workspace/browser-pane opening.
- cmux is a useful local-dev-browser precedent. Its `SessionBrowserPanelSnapshot` stores `urlString`, `backHistoryURLStrings`, and `forwardHistoryURLStrings`, `Workspace` snapshots the browser panel with `preferredURLStringForSessionSnapshot()`, and `BrowserPanel` serializes/restores URL strings using the full `absoluteString`. Warp v1 does not need cmux's full browser history restore, but it should preserve the current full URL for exact dev-preview reloads.

There is no general-purpose Browser Pane in the app today. The implementation should add one as a new pane type instead of trying to route web URLs through the Markdown/file viewer. Markdown viewing remains native rich-text/file viewing; Browser Pane handles `http` and `https` content plus plain search queries that are converted into search-result URLs.

## Proposed changes

### Implementation sequence

The implementation should land in small reviewable slices behind `FeatureFlag::BrowserPane`:

1. Add browser input/layout parsing types, loopback classification, URL/search conversion, and telemetry redaction helpers with unit tests.
2. Add the Browser Pane shell, pane type, pane snapshot type, and a test/dummy `BrowserSurface` so pane creation, split/new-tab layout, focus, title, and restore plumbing can be reviewed without platform webview complexity.
3. Spike the native desktop webview path on macOS first because WKWebView is the most constrained system-webview baseline for Warp's current desktop app model. The spike should prove child-view embedding, URL navigation, back/forward/reload, load-state callbacks, focus handoff, nonpersistent storage, and download blocking. If the spike cannot satisfy a required v1 behavior, update this spec before widening implementation.
4. Add the terminal and rich-content URL context-menu routes through the existing terminal event -> pane-group event -> workspace bridge, keeping primary-click URL behavior unchanged.
5. Add the command-palette route, preferring an explicit URL/search prompt if inline query-dependent results would appear for ordinary command searches or pollute palette ranking.
6. Add SQLite/app-state persistence and restore with feature-flag-disabled and invalid-URL skip behavior.
7. Add Windows and Linux `BrowserSurface` implementations or hide Browser Pane on unsupported targets with a documented unavailable state and follow-up.
8. Complete accessibility, keyboard, telemetry, privacy, and manual validation before the feature is promoted beyond the initial flag.

### 1. Add a feature flag

Add `FeatureFlag::BrowserPane` in `warp_core/src/features.rs`.

All new user-visible routes should check the same flag:

- URL context-menu items.
- Command-palette result or URL/search prompt entrypoint.
- Browser Pane restoration from app state.
- Any toolbar controls specific to Browser Pane.

When the flag is disabled, primary-click URL behavior and existing context menus remain unchanged.

### 2. Add browser input and layout types

Add a small browser-pane module, for example `app/src/browser_pane/`, with shared types:

- `BrowserPaneLayout`
  - `SplitPane`
  - `NewTab`
- `BrowserPaneUrl`
  - Stores a parsed `url::Url`.
  - Accepts only `http` and `https`.
  - Normalizes display without dropping the user's entered URL.
  - Preserves path, query string, and fragment for navigation and local app/session restore.
- `BrowserPaneInput`
  - Represents either a direct URL or a plain search query.
  - Converts search queries to `https://www.google.com/search?q={query}` in v1 unless Warp already has a browser/search-engine setting by implementation time.
  - Rejects or externally routes unsupported non-`http`/`https` schemes instead of loading them in Browser Pane.
- `BrowserPaneUrlKind`
  - `LoopbackPreview`
  - `Web`

`BrowserPaneUrlKind::LoopbackPreview` should be syntactic only. It returns true for:

- `localhost`
- `127.0.0.1`
- `[::1]`
- other loopback IP literals parsed by `std::net::IpAddr::is_loopback`

It should not perform DNS or network lookups.

Do not reuse `EditorLayout` or `FileTarget` for browser URLs. Those names are file-oriented and already map to Markdown/code/editor behavior. Browser Pane can still intentionally mirror their split/new-tab variants.

### 3. Add the pane and backing view

Add a new pane pair following the existing pane contract:

- `app/src/pane_group/pane/browser_pane.rs`
- `app/src/browser_pane/view.rs`
- `app/src/browser_pane/model.rs`

The wrapper should be structurally similar to `FilePane`:

- `BrowserPane::new(input, ctx)` creates `BrowserPaneView`.
- `BrowserPane::from_view(view, ctx)` wraps it in `PaneView<BrowserPaneView>`.
- `PaneId::from_browser_pane_ctx` and `PaneId::from_browser_pane_view` create typed pane IDs.
- `IPaneType::Browser` renders as "Browser".
- `PaneContent::snapshot` returns `LeafContents::Browser(BrowserPaneSnapshot { url })`.
- `PaneContent::focus` focuses the browser toolbar or web content according to current browser focus state.
- `PaneContent::attach` subscribes to `BrowserPaneViewEvent::Pane` and title/load-state events, then forwards `PaneEvent` and `PaneTitleUpdated` to the pane group.

`BrowserPaneView` should implement `BackingView` and expose:

- A title derived from page title when available, otherwise URL host/path.
- A minimal toolbar: address/search field, back, forward, reload/stop, copy URL, open externally.
- Loading, loaded, connection-refused, unsupported-scheme, certificate/security-warning, embedded-browsing-blocked, and generic load-failed states.
- Accessible labels for toolbar controls.
- Clear focus behavior when moving between terminal input, address/search field, browser content, and pane chrome.
- Keyboard actions or bindings for opening Browser Pane, focusing the address/search field, back, forward, reload, and returning focus to the terminal pane.

### 4. Use system webviews behind a narrow abstraction

V1 should use platform system webviews behind a narrow `BrowserSurface` abstraction rather than shipping Chromium or adopting Tauri/Electron as an app framework.

Reference points:

- Tauri's webview matrix is the model for platform choice, not a framework dependency: WebView2 on Windows, WKWebView/WebKit on macOS, and WebKitGTK on Linux. See `https://v2.tauri.app/reference/webview-versions/`.
- Wry is the closest Rust reference for mechanics: a webview attached to an existing window/event loop, child webviews where supported, and GTK container/event-loop integration for Linux X11/Wayland. See `https://docs.rs/wry/latest/wry/`.
- Electron is intentionally out of scope because it brings a Chromium/Node app process model rather than a pane-level native-webview primitive. See `https://www.electronjs.org/docs/latest/tutorial/process-model`.
- CEF/bundled Chromium is intentionally out of v1 because it brings Chromium framework binaries, helper processes, threading/message-passing complexity, packaging work, and app-owned security updates. See `https://chromiumembedded.github.io/cef/general_usage.html`.

Runtime/version policy:

- macOS: use WKWebView. Its WebKit version is supplied by macOS and updated through OS updates; Warp cannot pin a separate WKWebView engine version without abandoning the system-webview approach.
- Windows: use WebView2 Evergreen by default. Microsoft documents Fixed Version WebView2 as the pinning option, but it requires shipping a specific runtime with the app, disables automatic runtime updates for that packaged runtime, and adds more than 250 MB to the app package. Fixed Version is effectively bundling the Windows webview engine/runtime for Warp's use, even though it is not the same as adopting Electron or CEF as the app framework. V1 should not use Fixed Version unless a compatibility blocker is proven.
- Linux: use WebKitGTK only if it integrates cleanly with Warp's windowing stack. Its version comes from the target system/package set unless Warp chooses to bundle more of the GTK/WebKit stack, which is out of v1.

Decision matrix:

| Option | What Warp buys | What Warp pays | V1 decision |
| --- | --- | --- | --- |
| System webviews: WKWebView, WebView2 Evergreen, WebKitGTK | Smallest package, OS/runtime security updates, native integration, no app-owned browser-engine release train | Runtime differences across OSes, less deterministic rendering, feature detection needed for newer APIs | Default |
| WebView2 Fixed Version on Windows | Pinned Windows runtime, controlled rollout timing, reproducible Windows webview bugs, offline/locked-down install support | More than 250 MB package increase, Warp-owned runtime update cadence, Windows-only consistency, still leaves macOS/Linux unpinned | Escalation only |
| CEF/bundled Chromium | Cross-platform Chromium consistency, stronger control over engine version, better future fit for devtools/automation | Large binary/runtime footprint, helper processes, Chromium patch/update burden, packaging/signing complexity, larger security surface | Out of v1 |
| Electron | Full Chromium app platform with mature browser primitives | Replaces Warp's native Rust/custom UI model with a Chromium/Node process model | Not applicable |

Adoption criteria:

- Use system webviews for v1 if they support URL/search navigation, local HTTP previews, basic navigation controls, load/error states, focus, and bounded storage.
- Escalate to a CEF/bundled-Chromium design only if a concrete implementation spike shows a required v1 behavior cannot be met on a supported platform. Examples: no workable child-webview integration in Warp's windowing stack, no viable private/ephemeral storage mode, or a core local-preview API missing from the platform webview.
- Do not cite hypothetical future automation/devtools needs as a reason to ship Chromium in v1; those are follow-ups by product scope.

Add `BrowserSurface` rather than leaking webview-engine APIs through the pane:

- `BrowserSurface`
  - `load_url(url: &Url)`
  - `reload()`
  - `stop_loading()`
  - `go_back()`
  - `go_forward()`
  - `can_go_back()`
  - `can_go_forward()`
  - `current_url()`
  - `title()`
  - load-state callbacks
  - policy callback for external navigation or blocked embedding

The implementation can use direct platform bindings or a small Rust webview wrapper, but that choice should stay behind `BrowserSurface`. Start with a macOS WKWebView spike because it is the clearest system-webview baseline for Warp's current native desktop app model. Before widening to Windows or Linux, the spike should prove child-view embedding, URL/search navigation, local HTTP preview loading, basic navigation controls, load/error callbacks, focus handoff, nonpersistent storage, and download blocking. If one target cannot support embedded browsing in the first pass, hide Browser Pane on that target behind the feature flag, show the unavailable state from the product spec, and document the platform gap in the PR.

Do not build developer tools, request interception, console log streaming, agent-readable screenshots, agent-controllable actions, browser recording, Playwright-compatible automation, cloud/headless browser execution, scheduled browser jobs, multi-page research canvases, or AI synthesis surfaces into this abstraction for v1.

### 5. Add workspace opening APIs

Add a workspace-level entrypoint:

- `WorkspaceAction::OpenBrowserPaneFromInput { input: String, layout: BrowserPaneLayout }`
- `Workspace::open_browser_pane_from_input(input, layout, ctx) -> Result<(), BrowserPaneOpenError>`

`open_browser_pane_from_input` should:

1. Parse the input as either a supported `http`/`https` URL or a plain search query.
2. Construct `BrowserPane`.
3. Match the existing viewer layout behavior:
   - `BrowserPaneLayout::SplitPane`: add a right split to the active tab with `focus_new_pane = true`.
   - `BrowserPaneLayout::NewTab`: add a Warp tab containing the Browser Pane, using the same new-tab placement setting used by `open_file_notebook`.
4. Emit a toast or in-pane error for invalid/unsupported URL schemes.
5. Send only coarse telemetry without full URL/query/page data.

The new-tab route creates a Warp tab. It does not create an internal browser tab strip.

### 6. Extend terminal URL context menus

Keep the primary-click path unchanged in `app/src/terminal/view/link_detection.rs`: `ctx.open_url(...)` remains the default click behavior.

When `FeatureFlag::BrowserPane` is enabled, add URL context-menu items in both terminal URL surfaces:

- `app/src/terminal/view.rs:15937` for grid-highlighted terminal URLs.
- `app/src/terminal/view/context_menu.rs:42` for rich-content URLs.

The menu order should be:

1. `Open in new pane`
2. `Open in new tab`
3. `Copy URL`
4. `Open externally`

Add terminal/context-menu actions and events using the existing bridge:

1. Add context-menu action variants for opening the URL in Browser Pane split/new-tab and for opening externally.
2. Handle those `ContextMenuAction` variants in `TerminalView::context_menu_action`.
3. Emit terminal events that carry the URL string and desired `BrowserPaneLayout`.
4. Forward those terminal events from `terminal_pane.rs` into new `pane_group::Event` variants.
5. Handle those pane-group events in `Workspace` by calling `open_browser_pane_from_input`.

`Open externally` should keep using `ctx.open_url`.

Use `RespectObfuscatedSecrets::Yes` when deriving the menu URL string from terminal output, matching the current context-menu copy behavior. Do not log or send the full URL as telemetry.

### 7. Add command-palette URL/search opening

Add a command-palette route for users who want to paste/type a URL or search query even when it did not appear in terminal output.

Preferred implementation:

- Add a command-palette action titled `Open URL or Search in Browser Pane`.
- Accepting that action opens a small URL/search prompt, or an equivalent focused input, with split-pane layout as the default.
- Submitting the prompt dispatches `WorkspaceAction::OpenBrowserPaneFromInput { layout: SplitPane }`.
- Add `CommandPaletteItemAction::OpenBrowserPaneFromInput { input: String }` only if the implementation can create query-dependent results without showing the Browser Pane result for ordinary command searches.
- Map this action to `ItemSummary::NoOp` so arbitrary URLs and search queries do not pollute command-palette recents.

The command palette should not show a Browser Pane search result for every generic command query. If inline query-dependent results are used, gate them to explicit URL-looking inputs, explicit search prefixes, or another ranking rule that avoids competing with normal command results. The product invariant is the same: a user can paste/type an `http` or `https` URL or search query and open it in Browser Pane.

### 8. Persist pane restore state

Add:

- `LeafContents::Browser(BrowserPaneSnapshot)`
- `BrowserPaneSnapshot { url: String }`

Browser Pane should be persisted by `LeafContents::is_persisted` when the feature flag is enabled. The implementation should add:

- A Browser Pane kind constant for SQLite pane leaves.
- A pane-specific table or compatible persisted field that stores only the last full URL or search-result URL.
- `save_pane_state` coverage that writes the Browser Pane kind and URL snapshot.
- `read_node` coverage that reconstructs `LeafContents::Browser(BrowserPaneSnapshot { url })`.
- A migration for any new schema used to store Browser Pane snapshots.

Restoration should:

1. Parse and validate the stored URL.
2. Recreate `BrowserPane`.
3. Restore the last full URL or search-result URL only, including path, query string, and fragment.
4. If the URL is invalid or the feature flag is disabled, skip restoring that pane with a warning rather than failing the whole tab.

Do not persist page content, screenshots, scroll position, cookies, credentials, request bodies, form values, browser history stack, automation state, or per-page storage in app-state snapshots.

Browser-engine storage must be bounded:

- App-state/SQLite persistence stores only `BrowserPaneSnapshot { url }`, where `url` is the current full `http`/`https` URL or search-result URL, including path, query string, and fragment. Search-result URLs may include the raw search query, so this data is allowed only in local app/session restore state and never in telemetry. This is intentional for a developer browser so local previews reload exactly, and matches cmux's browser-session precedent of restoring browser URL strings from app-owned session state.
- macOS should configure WKWebView with a nonpersistent `WKWebsiteDataStore`, which Apple documents as in-memory website data that is not written to disk.
- Windows should create Browser Pane webviews with a dedicated WebView2 profile/user-data scope and private mode when available; WebView2 exposes profile metadata including `IsInPrivateModeEnabled`, profile path, and browsing-data clearing APIs.
- Linux should use an ephemeral WebKitGTK context or website data manager when available; WebKitGTK documents ephemeral contexts/managers whose webviews do not store website data in client storage.
- If any supported platform cannot provide a private/ephemeral mode, the implementation must isolate Browser Pane data in a Browser Pane-specific user-data directory under Warp app data and clear cookies/cache/storage on pane close and app shutdown. It must not use the user's default external-browser profile.
- Downloads are out of v1. If the engine tries to start a download, Browser Pane should block it or route externally rather than silently writing files.

### 9. Telemetry and privacy

Add coarse telemetry events only:

- Browser Pane opened.
- Browser Pane opened externally.
- Browser Pane reload clicked.
- Browser Pane search submitted.
- Browser Pane load failed.

Allowed metadata:

- Open source: terminal context menu, rich-content context menu, command palette, restore.
- Layout: split pane or new Warp tab.
- URL kind: loopback preview or web.
- Input kind: direct URL or search query.
- Failure kind: connection refused, unsupported scheme, certificate/security warning, blocked embedding, generic failure.

Forbidden metadata:

- Full URL.
- Query string.
- Fragment.
- Raw search query.
- Page title.
- Page text.
- Screenshot.
- Cookies or credentials.
- Request or response body.

### 10. Error and external-open behavior

Browser Pane should handle these cases in-pane:

- Invalid URL typed into toolbar or command palette.
- Unsupported scheme.
- Plain search query typed into toolbar or command palette.
- Local dev server not running or connection refused.
- TLS/certificate/security warning.
- Page blocks embedded browsing or requires an external auth/permission flow.

Every failure state keeps the URL visible and offers:

- Retry, when retry makes sense.
- Open externally.
- Copy URL.

Non-`http`/`https` links clicked inside page content should not load directly in Browser Pane in v1. Route them to the system/default handler or show unsupported with `Open externally`.

Plain search query input should navigate to the v1 search URL rather than producing an invalid URL error.

## End-to-end flow

```mermaid
flowchart TD
    TerminalURL["Terminal URL"] --> ContextMenu["URL context menu"]
    RichURL["Rich-content URL"] --> ContextMenu
    Palette["Command palette URL/search result"] --> WorkspaceAction["WorkspaceAction::OpenBrowserPaneFromInput"]
    ContextMenu --> TerminalAction["ContextMenuAction -> Terminal Event -> PaneGroup Event"]
    TerminalAction --> WorkspaceAction
    WorkspaceAction --> Validate["Parse http/https URL or search query"]
    Validate --> BrowserPane["BrowserPane"]
    BrowserPane --> Split["Split right in active Warp tab"]
    BrowserPane --> Tab["New Warp tab"]
    BrowserPane --> Surface["BrowserSurface"]
    Surface --> State["Loading/loaded/error states"]
```

## Testing and validation

### Unit tests

Add focused unit tests for:

- `BrowserPaneUrl` accepts only `http` and `https`.
- `BrowserPaneInput` converts plain search text into the v1 Google search URL.
- `BrowserPaneInput` rejects or externally routes unsupported schemes.
- Loopback classification returns true for `localhost`, `127.0.0.1`, `[::1]`, and loopback IP literals.
- Loopback classification returns false for non-loopback hosts and does not perform DNS.
- URL telemetry redaction strips query, fragment, raw search query, page title, and full URL.
- Browser Pane snapshot restore accepts valid `http`/`https` URLs and search-result URLs, preserves query strings and fragments, and rejects unsupported schemes without panicking.
- Command-palette URL/search route opens the URL/search prompt, or creates a narrowly gated inline result only for explicit URL/search inputs.

### Pane and workspace tests

Add the smallest available pane/workspace tests covering:

- `WorkspaceAction::OpenBrowserPaneFromInput` with `SplitPane` adds a right split to the active tab and focuses it.
- `WorkspaceAction::OpenBrowserPaneFromInput` with `NewTab` creates a new Warp tab, respecting the existing new-tab placement setting.
- Restoring `LeafContents::Browser` recreates a Browser Pane with the last full URL or search-result URL when the flag is enabled.
- Restoring `LeafContents::Browser` is skipped gracefully when the flag is disabled.
- Persisting and reading Browser Pane snapshots round-trips the full URL through SQLite without dropping the surrounding tab.
- Closing a Browser Pane does not affect terminal sessions or terminal processes.

### Terminal and command-palette tests

Add UI/action tests where existing harnesses make them cheap:

- Primary-click terminal URL still calls the external URL path.
- URL context menu contains `Open in new pane`, `Open in new tab`, `Copy URL`, and `Open externally` when the flag is enabled.
- URL context menu remains unchanged when the flag is disabled.
- Rich-content URL context menu gets the same Browser Pane actions.
- Submitting the command-palette URL/search route dispatches `OpenBrowserPaneFromInput` with split-pane layout.
- Browser keyboard actions focus the address/search field, navigate back/forward, reload, and return focus to the terminal.

### Manual validation

Run Warp locally with `FeatureFlag::BrowserPane` enabled and validate:

1. Start a local server from Warp, for example a docs site, web app dev server, or `python3 -m http.server`.
2. Right-click the printed `localhost` URL and choose `Open in new pane`.
3. Confirm the Browser Pane opens to the right of the terminal, the terminal stays usable, and reload works.
4. Stop the server and reload the pane. Confirm the in-pane connection-failed state keeps the URL visible and offers retry/open externally.
5. Start the server again and retry. Confirm the page loads.
6. Right-click the same URL and choose `Open in new tab`. Confirm Warp creates a new Warp tab without an internal browser tab strip.
7. Primary-click the URL. Confirm it still opens externally as it does today.
8. Paste an `https` URL into the command palette route. Confirm it opens a Browser Pane.
9. Type a docs/search query into the command palette route or Browser Pane address/search field. Confirm it opens a search-results page.
10. Use keyboard routes to focus the address/search field, navigate back/forward, reload, and return focus to the terminal.
11. Try a non-`http` URL. Confirm it does not load directly in Browser Pane.
12. Quit and restore Warp. Confirm restored Browser Panes reload their last full URL or search-result URL, including query string and fragment, and do not restore page content or browser history stack.

### Behavior-to-verification mapping

| Product behavior | Verification |
| --- | --- |
| #1 | Unit-test `BrowserPaneUrl`/`BrowserPaneInput` for supported `http`/`https` URLs and search queries; manually open both URL and search-query inputs. |
| #2, #3, #4, #12, #13 | Workspace tests cover `SplitPane` and `NewTab`; manual validation confirms split-right local preview, new Warp tab, and no internal browser tab strip. |
| #5 | Pane/workspace restore tests cover focus, resize/close lifecycle through the pane group, and Browser Pane restore through `LeafContents::Browser`. |
| #6 | Command-palette test accepts a typed/pasted URL or search query and dispatches `OpenBrowserPaneFromInput`. |
| #7, #8 | Terminal and rich-content context-menu tests assert `Open in new pane`, `Open in new tab`, `Copy URL`, and `Open externally`, with the split/new-tab actions carrying the expected layout. |
| #9, #11 | Terminal URL primary-click regression test still calls the existing external URL path for normal and loopback-preview URLs. |
| #10 | Unit tests cover syntactic loopback classification for `localhost`, `127.0.0.1`, `[::1]`, other loopback literals, non-loopback hosts, and no DNS lookup. |
| #14, #15, #16, #19 | Browser Pane view tests or visual/manual validation cover pane chrome, toolbar controls, URL visibility on failure, and absence of bookmarks/extensions/downloads/profiles/devtools/automation/tab strip. |
| #17, #18 | Unit tests cover address/search parsing; manual validation confirms direct URL navigation and search-result navigation. |
| #20, #21, #22 | Browser surface/load-state tests cover loading, loaded, failed load, connection refused, unsupported scheme, certificate/security warning, blocked embedding, retry, copy URL, and open externally; manual validation covers stopped local server retry. |
| #23 | UI/action tests and manual keyboard validation cover reload/stop from toolbar and keyboard route. |
| #24, #25 | Pane/workspace tests and manual validation confirm closing Browser Pane or stopping the producing terminal process does not terminate the other surface. |
| #26, #33 | Browser navigation policy tests cover in-pane `http`/`https` link navigation and system/default-handler or unsupported-state handling for non-`http`/`https` links. |
| #27 | SQLite/app-state round-trip tests persist and restore the current full URL, including path, query string, and fragment, without restoring page content or history stack. |
| #28, #29 | Telemetry unit/review tests assert coarse events only and reject full URL, query string, fragment, raw search query, page title, page text, screenshots, cookies/credentials, and request/response bodies. |
| #30, #31, #32 | Accessibility and keyboard manual validation cover toolbar accessible names, meaningful pane title, visible focus, browser-content focus boundaries, back/forward/reload shortcuts, address-field focus, and terminal focus return. |
| #34, #35 | Platform/runtime checks verify Browser Pane uses the platform webview runtime, hides or shows an unavailable state when the runtime is missing/blocked/unsupported, and leaves primary-click URL behavior unchanged. |
| #36, #37 | Release/docs review verifies enterprise runtime requirements and pinned-runtime escalation guidance; manual/platform validation confirms pinned-runtime deployments keep the same user-visible Browser Pane behavior. |

### Commands

For the spec-only PR, validate Markdown and repository state:

- `git add specs/GH2164/product.md specs/GH2164/tech.md`
- `git diff --cached -- specs/GH2164/product.md specs/GH2164/tech.md`
- `git -c filter.lfs.process= -c filter.lfs.required=false -c filter.lfs.clean= status --short`

For implementation PRs, run the repo-required checks before review:

- `cargo fmt`
- `cargo clippy`
- Targeted unit or pane tests added for this feature.
- Relevant manual validation from this spec.
- Update `specs/GH2164/product.md` or `specs/GH2164/tech.md` in the same PR if implementation changes user-facing behavior, architecture, module boundaries, or validation strategy.

## Risks and mitigations

### Embedded webview dependency and platform behavior

Risk: Adding a native embedded webview can introduce platform-specific dependencies, packaging constraints, and inconsistent behavior.

Mitigation: Put all engine-specific code behind `BrowserSurface`, gate the feature by `FeatureFlag::BrowserPane`, and hide the feature on unsupported targets until the platform path is reviewed.

### Browser scope creep

Risk: Browser Pane can easily become a general browser, developer-tools replacement, test runner, research canvas, or agent-automation surface.

Mitigation: Keep v1 to local preview and quick lookup ergonomics: pane lifecycle, URL/search navigation, minimal toolbar, load/error states, reload, copy URL, and open externally. Defer responsive presets, console/network summaries, devtools, screenshots, recording, cloud/headless execution, multi-page synthesis, and agent control.

### Sensitive URL or page data leakage

Risk: URLs can contain credentials, tokens, local service paths, query strings, or page content.

Mitigation: Store only the last full URL or search-result URL in local pane restore state, prefer ephemeral browser-engine storage, avoid full URL/search telemetry, and never emit page content, screenshots, cookies, credentials, request bodies, raw search queries, or query strings.

### Focus conflicts with terminal input

Risk: Browser content can capture keyboard shortcuts users expect to apply to terminal panes.

Mitigation: Keep pane-level navigation shortcuts owned by Warp chrome, expose clear focus state, support explicit browser keyboard routes, and apply page-level shortcuts only while browser content is focused.

### Restore failures causing tab loss

Risk: A bad persisted URL or disabled feature could fail pane restoration and drop a tab.

Mitigation: Treat Browser Pane restore as best-effort. Invalid or disabled Browser Pane snapshots should be skipped with a warning, following the app-state pattern that avoids failing the surrounding tab for non-restorable panes.

## Follow-ups

- Agent-readable browser state for a visible Browser Pane.
- Agent-controlled browser actions.
- Responsive viewport presets.
- Console error and failed network request summaries.
- User setting for primary-click local-preview URLs to open in Browser Pane.
- Browser search-engine preference and search suggestions.
- Private-network URL classification beyond loopback literals.
- Workspace-level Browser Pane URL memory.
- Full developer tools.
- Browser recording, headless/cloud execution, and Playwright-compatible test generation.
- Spatial/multi-page research workspace and AI synthesis over open pages.
