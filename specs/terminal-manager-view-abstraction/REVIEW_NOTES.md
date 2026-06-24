# Terminal manager view abstraction review notes
This is a temporary tracker for open review questions and decisions while iterating on the implementation.
## Large architecture questions
### 1. Should `TerminalManager` keep the default `TerminalView` type parameter?
Status: Resolved
Decision: Remove the default type parameter.
Rationale: `TerminalManager<S = TerminalView>` hides the distinction between the generic manager and the current GUI instantiation. Direct concrete references and downcasts should say `TerminalManager<TerminalView>` explicitly so future non-GUI instantiations are clear.
Affected review comments:
- Default type parameter skepticism on `pub struct TerminalManager<S = TerminalView>`.
### 2. What belongs in `TerminalManager<TerminalView>::create_model` versus extracted helpers/modules?
Status: Resolved
Decision: Move toward one shared generic local-terminal construction pipeline, with `TerminalManager<TerminalView>::create_model` acting as a GUI adapter.
Resolution:
- Remove `view() -> ViewHandle<TerminalView>` from the object-safe `crate::terminal::TerminalManager` trait so the trait describes terminal model/session lifecycle instead of a GUI view.
- Keep a lightweight `impl crate::terminal::TerminalManager for TerminalManager<TerminalView>` for the GUI manager; this is still needed for model access, detach behavior, and downcasting in current pane code.
- Change GUI constructors to return the `TerminalView` separately alongside the boxed manager, instead of recovering the view through `terminal_manager.as_ref(ctx).view()`.
- Add a real generic `TerminalManager<S>::new(surface, state, ...)` path that assembles manager-owned session state around any `TerminalSurface`.
- Keep `TerminalManager<TerminalView>::create_model` as the compatibility adapter that resolves GUI-specific inputs, constructs `TerminalView`, wires GUI-specific behavior, boxes the manager, and returns `(manager, terminal_view)`.
- Do not add a generic object-safe trait impl for every `TerminalManager<S>` yet; the current detach behavior is still GUI/session-sharing-specific.
Rationale: A generic struct alone is not enough. GUI and future TUI constructors should share the same local terminal model/session/controller construction path, while frontend-specific view construction and wiring stay in adapters.
### 3. Should `SessionCore` and `SessionState` be separate concepts, and should they live in their own module?
Status: Resolved
Decision: Remove both current structs and replace them with one private parts bundle.
Resolution:
- Delete the current `SessionCore` / `SessionState` split.
- Add one private `LocalTerminalSessionParts` struct that bundles the constructed model/session/controller/channel pieces needed to assemble `TerminalManager<S>` and create the surface.
- Add one helper, likely `build_local_terminal_session_parts(...) -> LocalTerminalSessionParts`, that creates channels, `Sessions`, `ModelEventDispatcher`, `TerminalModel`, `PtyController`, `RemoteServerController`, colors, PTY-read receivers, and shell-starter source in one pass.
- Do not add a separate config struct unless argument growth makes it necessary.
- Consider moving `LocalTerminalSessionParts` and its builder into a private `local_tty/session_state.rs` or similarly named module if it makes `terminal_manager.rs` easier to scan.
Rationale: The `SessionCore` / `SessionState` distinction is not self-explanatory. A single `Parts` struct is enough to avoid a giant tuple while making clear that this is a construction-output bundle, not a domain-level session state machine.
### 4. Which shared-session code is actually GUI-specific?
Status: Resolved
Decision: Keep session sharing GUI-specific for this PR, but isolate it behind a clearly named helper boundary.
Resolution:
- Do not try to make session sharing TUI-ready in this PR.
- Move the current local sharer setup out of the generic constructor flow into a named `TerminalViewSessionSharing` helper (or equivalent).
- Keep using the existing reusable protocol/model pieces such as `shared_session::sharer::Network`, model ordered-terminal-event flow, and existing shared handlers.
- Leave `shared_session::manager::Manager` GUI-shaped for now; it still stores `TerminalView` handles and supports app-level share/rejoin/stop UI.
- Keep `stream_historical_agent_conversations` in the `TerminalViewSessionSharing` boundary for now. It is not purely rendering UI, but it currently depends on `terminal_view.id()`, selected conversation ownership, and sharer presence data.
Rationale: We are not building TUI session sharing in this PR. The goal is to make the GUI-specific session-sharing setup easy to identify and work on later, without over-abstracting protocol/UI boundaries prematurely.
### 5. Can the `TerminalSurface` / `PtyIntent` API bounds be cleaner?
Status: Resolved
Decision: Keep the `From<&SurfaceEvent> for Option<PtyIntent>` pattern, but hide the higher-ranked bound behind a marker trait.
Resolution:
- Add an internal `PtyIntentEvent` marker trait in `terminal_surface.rs`.
- Implement it blanket-style for event types where `for<'a> Option<PtyIntent>: From<&'a T>`.
- Make `TerminalSurface` require `Self::Event: PtyIntentEvent`.
- Remove repeated `for<'a> Option<PtyIntent>: From<&'a <S as Entity>::Event>` bounds from generic call sites.
- Use `Self::Event` instead of `<Self as Entity>::Event` where possible.
Rationale: This preserves the original event projection pattern while quarantining Rust type-system plumbing in one place. The surface trait then reads as “this surface’s event type can produce a PTY intent.”
## Small questions to revisit
- Whether `model_events` should be cfg-gated or otherwise avoid `allow(dead_code)`.
- Whether the `TerminalView` pass-through lifecycle methods should be inlined into the trait impl and the old methods removed.
- Whether `should_poll_for_password_prompt` should move out of `TerminalView` into a helper taking settings plus `is_ssh_uploader`.
- Whether the `From<&Event> for Option<PtyIntent>` mapping can be protected by tests so new PTY-driving events are not missed.
- Whether `Event` in the `From` impl should be documented or named as `TerminalViewEvent` for clarity.
## Nits / likely direct fixes after architecture decisions
- Remove “this PR” language from code comments.
- Remove future-reader-unhelpful wording like “future non-GUI constructor” and “without constructing `TerminalView`.”
- Rewrite comments to describe what a type/function contains and why it exists, not the history of the refactor.
- Reorder `should_poll_for_password_prompt` in the trait so it is separated from the `on_*` hooks.
- Inline or remove tiny helpers if they only exist to support an abstraction we decide not to keep.
