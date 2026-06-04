# PRODUCT.md — Browser Pane for local previews and web lookup

Issue: https://github.com/warpdotdev/warp/issues/2164
Figma: none provided

## Summary

Warp adds support for opening web pages inside the app as a first-class Browser Pane. The v1 scope is intentionally narrow: local development previews, quick docs/search lookup, and explicitly opened web URLs, placed in Warp's existing pane system.

The main workflows are:

- Run a dev server in Warp, open the printed local URL in a split Browser Pane, keep the terminal or agent session visible beside it, and reload or open externally when needed.
- Open a Browser Pane from the command palette, type a docs URL or search query, and inspect the result without switching apps.

The Browser Pane behaves like a sibling of Warp's markdown/file/code viewer panes, not like a full browser product embedded inside Warp.

## Problem

Warp users often run local web apps, documentation sites, dashboards, and generated preview servers from terminal commands. When those commands print a URL, users must switch to an external browser to inspect the result. Users also switch away from Warp to search the web or read docs while working in the terminal. Both cases break the feedback loop between command output, file edits, agent activity, reference material, and rendered application state.

Issue #2164 asks for an in-app browser for docs lookup and web search. Later duplicate issues also ask for in-terminal app preview. The highest-value first cut is not a general-purpose browser replacement; it is a native Warp pane that keeps local preview and quick lookup state close to the terminal workspace.

## Goals

- Let users open local web previews inside Warp without leaving the active workspace.
- Let users type or paste a web search query or docs URL into Warp and view the result inside a Browser Pane.
- Match Warp's existing pane and viewer conventions: split pane as the primary route, new Warp tab as an explicit secondary route, and external browser available at all times.
- Keep existing URL-click behavior stable unless the user explicitly chooses an in-Warp browser action.
- Keep the v1 Browser Pane small: URL/search navigation, basic browser controls, pane lifecycle, reload, load/error states, copy URL, and open externally.
- Keep Browser Pane deployable in managed enterprise environments by making webview runtime availability explicit and recoverable.
- Define behavior that can be implemented and reviewed without first solving agent automation, browser profiles, developer tools, or general browser replacement.

## Non-goals

- Replacing the user's default browser for general browsing.
- Changing how Markdown files render in Warp. Markdown viewing remains a native rich-text/file-viewer experience; Browser Pane handles web URLs.
- Automatically routing every web link into Warp.
- Adding a setting that makes terminal URL primary-clicks open in Browser Pane.
- Shipping bookmarks, browser extensions, downloads management, password management, browser profiles, or an internal browser tab strip.
- Shipping responsive viewport presets in v1.
- Shipping console logs, network request inspection, DOM inspection, storage inspection, performance tools, or a developer-tools replacement in v1.
- Shipping agent-controllable browser automation in v1. Agent-visible screenshots, page state, click/type automation, and headless/background browser targets are follow-ups.
- Shipping cloud browsers, headless browser execution, browser recording, scheduled browser jobs, or test-run orchestration in v1.
- Shipping a spatial browser canvas, batch-opening pages, multi-page AI synthesis, or browser-level research workspace in v1.
- Bundling Chromium or a pinned webview runtime in the default v1 app package.
- Adding a user-facing browser-engine picker.
- Resolving arbitrary hostnames to determine whether a URL is local. V1 local-preview detection is syntactic only.

## Behavior

1. Warp exposes a Browser Pane as a first-class pane type for `http` and `https` URLs and web search queries.

2. Browser Pane can open in the active Warp tab as a split pane or in a new Warp tab.

3. The primary v1 layout for a local preview URL is a split pane placed to the right of the active pane, matching Warp's existing "Open in new pane" viewer convention.

4. Opening a URL in a new Warp tab creates a Warp tab whose primary content is the Browser Pane. It does not create an internal browser tab strip inside the pane.

5. Browser Pane supports the same pane-level lifecycle users expect from other pane types: focus, resize, close, and session restore where Warp already restores pane state.

6. Warp provides a command-palette action that lets the user type or paste a URL or search query before opening a Browser Pane.

7. When terminal output contains a web URL, the URL context menu offers at least: `Open in new pane`, `Open in new tab`, `Copy URL`, and `Open externally`.

8. For URL context menus, `Open in new pane` means "open this URL in a Browser Pane split in the current Warp tab." `Open in new tab` means "open this URL in a Browser Pane in a new Warp tab."

9. Primary-clicking a terminal URL keeps today's external-browser behavior in v1. Users opt into Browser Pane by choosing an explicit context-menu or command-palette action.

10. V1 local-preview URLs are identified syntactically: `localhost`, `127.0.0.1`, `[::1]`, and loopback IP literals. Warp does not perform DNS or network lookups to classify a URL as local in v1.

11. Local-preview URLs do not change primary-click behavior. Any in-Warp browser affordance for a local-preview URL is secondary to the existing click target.

12. If the user chooses `Open in new pane`, Warp opens a new Browser Pane split in the current Warp tab for the requested URL. V1 does not include open-or-reuse behavior.

13. If the user chooses `Open in new tab`, Warp creates a new Warp tab that hosts the requested URL in a Browser Pane. It does not reuse a split Browser Pane in the current tab for this action.

14. Browser Pane header uses Warp pane chrome plus a minimal browser toolbar.

15. The v1 toolbar includes an address/search field, back, forward, reload/stop, copy URL, and open externally.

16. The current URL or search-result URL is visible and selectable/copyable even when the page fails to load.

17. The address/search field accepts a typed or pasted `http` or `https` URL and navigates the Browser Pane to that URL.

18. If the address/search field input is not a URL, Browser Pane treats it as a search query and opens a search-results page. V1 uses Google search by default if Warp does not already expose a browser/search-engine setting.

19. The toolbar does not include bookmarks, extensions, downloads management, browser profiles, developer tools, automation controls, or an internal tab strip in v1.

20. Browser Pane shows visible page states for loading, loaded, failed to load, connection refused, unsupported URL scheme, certificate/security warning, and embedded browsing blocked.

21. If a local dev server is not running or refuses the connection, Browser Pane keeps the URL visible and offers retry plus open externally.

22. If a page requires an external browser for auth, permissions, unsupported protocol handling, or blocked embedding, Browser Pane offers open externally.

23. Reload is available from the Browser Pane toolbar and keyboard route.

24. Closing a Browser Pane does not stop the dev server or any terminal process that produced the URL.

25. Stopping the terminal process that produced the URL does not close the Browser Pane. The pane remains open and shows the appropriate reload or connection-failed state.

26. Links clicked inside the rendered web page navigate within the Browser Pane according to normal embedded-browser behavior, unless the browser engine or page policy requires opening externally.

27. Browser Pane restore is per pane. If Warp restores the Browser Pane after app/session restore, it restores the last full URL or search-result URL for that pane, including path, query string, and fragment, so dev-preview pages reload exactly. Search-result restore may therefore persist the raw search query locally inside Warp's app/session restore state. V1 does not need a separate workspace-level "last Browser Pane URL" memory.

28. Browser Pane does not emit page content, screenshots, cookies, credentials, request bodies, raw search queries, or full URLs with query strings into telemetry. Exact URL persistence is limited to local app/session restore state.

29. Telemetry is limited to coarse product events such as Browser Pane opened, opened externally, reload clicked, search submitted, or load failed, without sensitive URL/query/content payloads.

30. Browser Pane has accessible names for toolbar controls and exposes a meaningful pane title derived from the page title or URL.

31. Keyboard focus is clear when moving between terminal input, Browser Pane address/search field, and Browser Pane content. Existing pane navigation shortcuts continue to work, and page-level keyboard shortcuts apply only while the browser content is focused.

32. Browser Pane supports keyboard-first routes for opening a Browser Pane, focusing the address/search field, navigating back/forward, reloading, and returning focus to the terminal pane.

33. Non-`http`/`https` links are not loaded directly inside Browser Pane in v1. They are routed to the system/default handler or shown as unsupported with an external-open affordance.

34. Browser Pane uses the platform webview runtime available to Warp. V1 does not bundle Chromium or a pinned webview runtime by default.

35. If Browser Pane is unavailable because the platform webview runtime is missing, blocked by policy, unsupported, or fails startup, Warp does not change URL primary-click behavior. Browser Pane entrypoints are hidden or show a clear unavailable state with `Open externally`.

36. In managed enterprise environments, Warp can document the required platform webview runtime and expose a support/deployment escalation path for pinned-runtime packaging where the platform allows it. A pinned runtime is an enterprise deployment hatch, not a default user-facing mode.

37. If an enterprise-pinned runtime is used, Browser Pane behavior remains the same from the user's perspective: same toolbar, same pane lifecycle, same privacy boundaries, and no additional browser product features.

## Success Criteria

1. A user can run a local dev server in Warp, open the printed `localhost` URL in a split Browser Pane, keep working in the adjacent terminal pane, and reload the preview without leaving Warp.

2. A user can open a URL or search query in Browser Pane from the command palette even when the URL was not printed by terminal output.

3. A user can right-click a terminal URL and choose between opening it in a Browser Pane split, opening it in a new Warp tab, copying it, or opening it externally.

4. Browser Pane behavior matches Warp's existing viewer-pane mental model: split pane by default for local previews, new Warp tab as an explicit route, no internal browser tab strip.

5. Existing URL primary-click behavior remains unchanged in v1.

6. A user can open Browser Pane, focus the address/search field, search for documentation, navigate back/forward, reload, and return focus to the terminal without using the mouse.

7. A failed local server connection produces a useful in-pane error state with retry and open externally options.

8. Browser Pane restores its last full URL or search-result URL, including query string and fragment, when the pane is restored through Warp's normal app/session restore path.

9. General browsing features such as bookmarks, downloads management, browser extensions, browser profiles, developer tools, automation controls, cloud browsers, and internal browser tabs are absent from v1.

10. Browser Pane unavailable states are recoverable: users can still open links externally, and enterprise admins have a documented runtime requirement or escalation path.

11. No sensitive page content, screenshots, cookies, credentials, request bodies, or full URLs with query strings are emitted in telemetry.

## Follow-ups

- Agent-readable browser state: screenshot, current URL/title, and page text snapshot for a visible Browser Pane.
- Agent-controlled browser actions: navigate, click, type, scroll, reload, and form-submission policy.
- Responsive viewport presets for common app preview sizes.
- Console error and failed network request summaries.
- A user setting that makes local-preview URLs open in Browser Pane on primary click.
- Browser search-engine preference and search suggestions.
- Enterprise pinned-runtime packaging for environments that cannot use the default platform webview runtime.
- Private-network URL classification beyond loopback literals.
- Workspace-level Browser Pane URL memory.
- Full developer tools or deeper inspection surfaces.
- Browser recording, headless/cloud execution, and Playwright-compatible test generation.

## Open questions

None for v1 product behavior. Browser engine choice, storage isolation details, and platform-specific implementation constraints belong in the companion tech spec.
