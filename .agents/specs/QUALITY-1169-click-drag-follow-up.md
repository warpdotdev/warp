# QUALITY-1169 follow-up: centered click/drag annotations and gesture continuity

## PRODUCT

**Summary:** Correct the three dogfooding gaps observed after PR #14173
(`59bb73fa3c6e04a1c67a5ca0a134abd165f67338`): click ripples must be centered on
the captured cursor, drag trails must survive both canonical single-call and
split-call pointer sequences, and scrollbar interaction behavior must be
explicit rather than an accidental full-circle flash. The change remains
client-side in `warpdotdev/warp`; no server action schema or artifact API change
is required.

**User-visible invariants:**

1. A click ring's geometric center is the captured cursor coordinate after
   capture-space clamping and any screen/window coordinate mapping. The same
   invariant applies to drag start anchors and held indicators.
2. A drag represented by `MouseDown → MouseMove* → MouseUp` in one
   `UseComputer` call and the same sequence split over multiple calls produces
   one continuous trail with the recorded path, button identity, timestamps,
   release fade, and no click ring.
3. A stray move or release with no active press, a cancelled/failed call, and
   an incomplete press are handled deterministically: they cannot attach to a
   later unrelated click, and an incomplete press may render only the bounded
   held/anchor state until the recording ends.
4. Existing keyboard/type redaction, scroll labels, smart-cut timing, cursor
   capture, capture-dimension clamping, and best-effort original-video fallback
   remain unchanged.
5. A scrollbar press/scroll interaction has one documented outcome. The
   resolved outcome (confirmed by the requester after spec approval) is to *keep*
   the click ring on a scrollbar press and *center* it on the cursor exactly like
   any other click ring — do NOT suppress it. The centering fix in invariant 1
   applies uniformly to every ring/circle animation, including the full-circle
   drawn on a scrollbar press, so none of them render offset to the top-left. The
   action stream does not identify UI elements, so a true scrollbar drag with no
   nearby `MouseWheel` cannot be distinguished reliably and remains a normal drag
   annotation; no `MouseWheel`-adjacency suppression rule is implemented, since
   the requester chose to keep the ring. (The earlier recommended default was to
   suppress the ring for a click tightly associated with a wheel event; that
   recommendation is superseded by the keep-and-center decision below.)

**Key design choices:** Keep the renderer in ASS/libass, change origin-centered
circle dialogues to top-left alignment (`\an7`) so every ring/circle is centered
on the cursor, persist a recording-level pointer session so release coordinates
survive call boundaries, and classify one flattened recording pointer stream
rather than each `ActionLogEntry` independently. Keep the scrollbar click ring
(resolved product decision) and center it like every other ring; do not suppress
it and do not guess a scrollbar from coordinates.

## TECH

**Current context (all references are pinned to `88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e` unless noted):**

- `crates/computer_use/src/overlay.rs:557-624` defines
  `classify_pointer_gestures`; it receives one entry's events, so a
  `Down` in one entry cannot see a `Move` or `Up` in another.
- `crates/computer_use/src/overlay.rs:648-678` emits the click ring with an
  origin-centered `ass_circle_path` and `\an5\pos(cx,cy)`. The same
  origin-centered path/alignment appears for the drag anchor, held indicator,
  and their `\move` dialogue in `:680-744`.
- `app/src/ai/blocklist/action_model/execute/use_computer.rs:67-137` creates a
  fresh `PointerSink` and event buffer for every `UseComputer` call, then
  drains it into one `ActionLogEntry`.
- `crates/computer_use/src/linux/x11/mod.rs:228-380` creates per-call
  `last_capture` state. `record_down_move` updates that state, while
  `record_up` omits a release if the current call has no prior point. Therefore
  split `Down`/`Move`/`Up` calls currently commit `[Down]`, `[Move]`, and no
  release event; the renderer sees no complete drag.
- `app/src/ai/blocklist/action_model/recording_controller.rs:44-83,172-245`
  owns `ActiveRecording`, `PendingActionGroup`, and committed
  `ActionLogEntry` values. This is the recording-level lifetime needed for
  pointer-session state.
- `app/src/ai/blocklist/action_model/recording_finalize.rs:93-143` drains
  actions and calls `computer_use::post_process_recording`; the existing
  post-stop burn-in/cut path is the only artifact compositor.
- `crates/computer_use/src/overlay_tests.rs:430-560` covers nominal click/drag
  ASS strings but currently asserts `\an5` and has no libass pixel-center,
  split-entry, or scrollbar association fixture.

**Trace/reproduction evidence:** The supplied MAA URL
`https://staging.warp.dev/conversation/a2b2ba95-0b15-4f94-b813-4cb3d0656607`
was opened directly and in a browser. It redirects to
`https://staging.warp.dev/login?redirect_to=...`; no action blocks,
coordinates, or artifact preview are accessible without credentials. Do not
claim that this particular trace contained a split sequence. The merged client
path and a deterministic replay establish the defensive fix: the same-call
sequence is supported, and split calls must be reconstructed regardless of the
unobservable trace grouping. The libass replay of the merged ASS vector at
`(640,360)` measured an approximately `(603.5,323.5)` center with `\an5` and
`(639.5,359.5)` with `\an7`; a 128×128 reproduction is clipped at the top-left
with `\an5` and centered around `(64,64)` with `\an7`.
The steerable investigation run is
`https://oz.staging.warp.dev/runs/019f8f43-6380-752b-8bbc-26da28a52e5c`.

### Proposed changes

1. **Correct vector alignment.**
   - Keep `ass_circle_path` centered around the drawing origin.
   - Change the click ring dialogue from `\an5\pos(cx,cy)` to
     `\an7\pos(cx,cy)`.
   - Make the same alignment change for the drag anchor and held indicator
     circles. Keep the trail polyline's existing `\an7\pos(0,0)`.
   - Continue using `clamp_point` and `remap_source_interval`; alignment must
     not alter coordinate scaling, smart-cut remapping, clipping, color, or
     animation timing.

2. **Persist pointer state across action calls.**
   - Add a recording-scoped `PointerSession` owned by `ActiveRecording`, shared
     with each call's `PointerSink`. It stores the currently pressed button(s)
     and the last resolved capture-space point. The state is updated atomically
     as events are pushed; it is not a log of sensitive text.
   - On `MouseDown`, resolve the point using the existing screen/window mapping,
     replace or reject an already-active same-button press deterministically, and
     emit `Down`.
   - On `MouseMove`, emit `Move` at the resolved point and retain the active
     button/last point. A move on a surface that does not match the recorded
     target clears the active pointer state rather than reusing stale
     coordinates.
   - On `MouseUp`, use the session's last resolved point when the action carries
     no coordinate, emit `Up` only for the matching active button, then clear
     that button. This makes a release in a new call observable at the last
     capture-space point.
   - On failed/cancelled `UseComputer`, discard the pending group and reset the
     session state so a later click cannot inherit an abandoned press. On
     successful completion, keep an incomplete press only until the recording
     ends; do not synthesize a release.
   - Preserve per-event `Instant` offsets from the recording start and the
     existing `Target::Screen`/matching `Target::Window` coordinate rules.
     Non-Linux actors may leave the sink unused as today, but shared types and
     no-op paths must continue compiling.

3. **Classify a recording-level stream.**
   - At burn-in, concatenate all `pointer_events` from all committed entries,
     stable-sort by event offset (preserving insertion order for equal
     timestamps), and classify the resulting stream once. Do not classify each
     entry separately.
   - Extend the classifier to track button identity. A matching `Down`,
     zero-or-more `Move`, and matching `Up` is one gesture; a press with no
     move is a click, a press with a move is a drag, and an unmatched
     move/release is ignored. A new `Down` while a button is active closes the
     prior incomplete gesture deterministically before starting the new one.
   - Render the resulting gestures through the existing segment remapping and
     bounded animation durations. A split-call drag therefore receives one
     trail dialogue, one anchor, and one held indicator, with no ring.
   - Keep pointer-only action groups in the smart-cut timeline even when their
     labels are empty. Mixed keyboard/pointer entries continue to render both
     redacted pills and pointer geometry.

4. **Scrollbar/scroll association (resolved: keep and center).**
   - The requester confirmed the “keep” outcome: a click on a scrollbar renders
     the normal click ring, and that ring is centered on the cursor via the same
     `\an7` centering fix as every other click ring (invariant 1). No
     `MouseWheel`-adjacency suppression rule is implemented, so no click ring is
     suppressed for being near a wheel event. The implementation therefore adds
     no scroll-association branch and no suppression test.
   - A drag with no nearby `MouseWheel` remains a drag trail because the current
     protocol has no scrollbar-element identity.
   - Keep `MouseWheel` itself free of pointer geometry; the existing scroll pill
     remains unchanged. The selected outcome (keep-and-center) is recorded here
     and in the fixture names so the behavior cannot regress silently.

**Design alternatives:**

- **Alignment:** (a) selected `\an7` with the origin-centered path; it matches
  libass's drawing-origin semantics and needs no radius compensation; (b) retain
  `\an5` and offset `\pos` by the radius, which is fragile for scaled rings,
  different circle sizes, and moving indicators; (c) rewrite every path with a
  top-left origin, which duplicates geometry and makes animation math harder.
- **Gesture continuity:** (a) selected recording-scoped pointer session plus
  flattened classification; it fixes missing release coordinates and preserves
  exact event offsets across entries; (b) keep an actor alive across calls,
  which would couple recording lifetime to platform input state and change
  existing actor ownership; (c) infer drags from summaries/cursor positions,
  which loses button identity and is not reliable for window-target coordinates.
- **Scrollbar behavior:** (a) suppress a click ring tightly associated with a
  wheel event, avoiding a misleading flash while retaining ordinary clicks — the
  earlier recommended default, now superseded; (b) selected: keep every click
  ring (protocol-simple) and center it on the cursor via the same `\an7` fix, so
  the scrollbar full-circle no longer renders offset to the top-left — this is
  the requester-confirmed outcome; (c) classify scrollbar elements from
  coordinates, rejected because the client has no stable UI hit-test or element
  metadata.

**Open questions resolved:**

- The MAA trace's exact action grouping is unresolved because staging requires
  authentication in this environment; the implementation must support both
  same-call and split-call sequences and add fixtures for each.
- The requester confirmed the scrollbar outcome during spec approval: *keep*
  the click ring on a scrollbar press and *center* it on the cursor (the same
  `\an7` centering as any other click ring), rather than suppress it. The
  recommended adjacent-wheel suppression rule is therefore NOT implemented; no
  suppression branch or suppression assertions are added. This decision is
  recorded in invariant 5 and proposed change 4 above.
- No server or artifact API change is needed: the structured client action
  stream already carries the required pointer primitives and the client owns
  burn-in.

**Risks / mitigations:**

- Stale pointer state could connect unrelated gestures; reset on failed/cancelled
  calls, reject unmatched releases, and test interleaved/abandoned sequences.
- Cross-entry sorting could reorder equal-time events; use stable ordering and
  test same-offset click/drag boundaries.
- ASS alignment fixes can regress clipping or window-target scaling; retain
  coordinate/clamp tests and the libass image-level center test.
- Heuristic scroll association may miss a scrollbar drag or suppress a nearby
  intentional click; keep the threshold explicit, document it, and make the
  product choice easy to switch.

## Validation & verification criteria (all must pass before merge)

1. Reproduce the merged alignment defect with the existing ASS vector and
   libass/ffmpeg: at a known coordinate, the pre-change `\an5` output is
   measurably above-left and the new `\an7` output's rendered geometry is
   centered within one pixel (or one rasterization tolerance) of the requested
   coordinate. This is the first bug-fix check.
2. Add/update a renderer regression test (for example
   `click_ring_and_drag_circles_are_centered_under_libass`) that fails against
   the merged implementation and passes after the change. It must exercise the
   generated ring, drag anchor, and held circle—not merely search for an ASS
   substring—and assert the pixel bounding-box center against the requested
   point.
3. Add a pure ASS-generation assertion that all origin-centered circle
   dialogues use `\an7`, while the trail remains `\an7\pos(0,0)`, and retain
   clipping at `(0,0,width,height)`.
4. Add a recording-level fixture for canonical same-call
   `Down → Move → Up`; assert one drag, one trail path containing every
   non-zero segment, one anchor, one held indicator, correct release fade, and
   zero click rings.
5. Add a split-entry fixture with `Down` in entry A, `Move` events in entries B
   and C, and `Up` in entry D; assert the same single gesture/timing/path as
   the canonical fixture, including the release point recovered from the
   recording-scoped pointer session.
6. Add button and boundary fixtures: right/middle clicks remain clicks; a
   matching button release closes only its gesture; unmatched releases and
   stray moves render nothing; a second press while held and a failed/cancelled
   call reset state deterministically; an incomplete press never creates a
   later click ring.
7. Add screen and window-target coordinate fixtures proving that split events
   preserve capture-space mapping, clamping, and target mismatch invalidation;
   mixed keyboard/pointer entries still redact printable key/type payloads and
   preserve scroll labels.
8. Add scrollbar replay coverage for the requester-selected outcome. Under the
   recommended default, a click followed by a nearby wheel suppresses only that
   click ring, while a distant/standalone click still emits the normal
   900 ms ring; under “keep,” assert the normal ring timing/fade for the same
   replay. A scrollbar drag without a wheel remains a deterministic drag trail.
9. Verify smart-cut remapping and animation bounds remain unchanged: click ring
   and drag fade stay within retained segments, pointer-only entries still keep
   their action windows, and no annotation is emitted outside capture dimensions.
10. Verify failures and lifecycle behavior: a failed/cancelled pointer call
    discards its pending group and resets session state; finalization with
    valid actions still uploads the processed artifact; any burn-in failure
    still uploads the original recording and cleans temporary files.
11. Run the focused Rust tests for `computer_use` overlay/recording behavior and
    the app recording-controller/finalization tests, then run the repository's
    documented `./script/presubmit` checks on Linux. The focused regression suite
    must pass without requiring a live MAA login.
12. Exercise representative published artifacts through the existing recording
    verification flow: Linux click-only, canonical drag, split-call drag,
    right/middle click, scrollbar click+scroll, and mixed keyboard/pointer
    recordings. Download each artifact, confirm center/path/timing visually and
    with frame inspection, confirm no typed payload is visible, and confirm the
    original-video fallback on an induced burn-in failure.

