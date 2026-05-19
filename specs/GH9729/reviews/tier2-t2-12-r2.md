---
item: tier2-t2-12
commit: 65b2f56
reviewer: R2-quality
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-12 supplements one bullet in `specs/GH9729/tech.md` (Follow-ups not
in v1), the same line t2-7 and t2-11 attached to.

§698 (`tech.md:698`, verbatim):

> - **Zoom and pan controls**: extend `lightbox::Params` with zoom
>   state and `lightbox_view.rs` keybindings (`+`, `-`, `0`,
>   drag-to-pan).

t2-12 is the third pass on that bullet: it deletes the t2-11
`cmdorctrl-=/--/-0` keybindings (which manual testing proved don't
dispatch because `LightboxView` doesn't claim keyboard focus) and
replaces them with a three-button toolbar plus cmd+scroll-wheel. The
action surface (`LightboxViewAction::ZoomIn/Out/Reset`) and the
`step_zoom` / footer logic from t2-11 are unchanged. R1 covers
correctness against the keybindings-deleted / actions-preserved
contract; this review is quality-only.

# Findings

**1. `ZoomDirection` enum placement. Right place — paired with
`NavigationDirection`.** Sitting at lines 129-136 of
`crates/ui_components/src/lightbox.rs`, directly under
`NavigationDirection` (line 100-104) and `NavigateHandler` (line 107),
is the right call. The alternative — pulling it into a shared
`crates/warp_core/src/ui/` module — would split a tightly-coupled
duo: the only consumer of `ZoomDirection` is `Lightbox` (the producer)
and `LightboxView` (the dispatcher). Moving it out would also leave
`NavigationDirection` orphaned next door, inviting the inconsistency
of "navigation lives here, zoom lives elsewhere" with no behavioural
reason. Keep it in `lightbox.rs`.

**2. `ZoomHandler` type alias. Right shape.**
`pub type ZoomHandler = Arc<dyn Fn(ZoomDirection, &mut EventContext, &AppContext)>`
(line 140) is a verbatim mirror of `NavigateHandler` (line 107) with
the direction enum swapped. Same `Arc<dyn Fn>` wrapper, same arg
order. The mirroring is a feature: it lets a reader of `Options`
predict the shape of every per-action handler on the struct without
re-checking. No reason to deviate.

**3. Six button fields on `Lightbox`. Verbose but right.** The struct
now carries `close_button` + `prev_button` + `next_button` +
`zoom_{in,out,reset}_button`. Six fields, ~12 lines. The two
alternatives in the prompt:

- `Vec<button::Button>` indexed by enum/purpose: replaces named
  field access with index discipline and an enum→index map. Loses
  the compile-time guarantee that you can't ask for a zoom button
  before it exists. Adds runtime panics for index-out-of-bounds. Net
  negative for six well-known buttons.
- Drop persistent button state and reconstruct each render via
  `button::Button::default()`: would break any hover / pressed /
  focus animation state stored on `Button` between frames. The
  existing `close_button` / `prev_button` / `next_button` fields
  exist precisely because `Component`s in this codebase hold their
  own UI state across `render` calls — sidestepping that convention
  just for the zoom trio would be a one-off.

Verbose-but-uniform wins over clever-but-different. Keep as-is.

**4. Toolbar offsets vs `SCRIM_PADDING`. Consistent with the existing
close button; both magic numbers could share a name.** The close
button at line 408 uses `vec2f(-12., 12.)` (top-right, 12 px inset);
the new zoom toolbar at line 536 uses `vec2f(12., -12.)` (bottom-left,
12 px inset). Same magnitude, mirrored signs — the symmetry is
correct and matches a user's mental model of "12 px inset from each
corner."

The 12 is not a multiple of `SCRIM_PADDING = 48.` (lines 16) and isn't
related to it — `SCRIM_PADDING` is image-vs-scrim padding, the 12 is
button-vs-scrim-edge inset. They're different distances, so a shared
constant would mislead. The nit is that 12 now appears literally four
times in the file (close + toolbar + prev arrow `vec2f(12., 0.)` line
439 + next arrow `vec2f(-12., 0.)` line 467). A
`SCRIM_BUTTON_INSET = 12.` constant near `SCRIM_PADDING` would clean
that up without conflating semantics. Not blocking; worth a follow-up.

**5. `Icon::Refresh` for zoom-reset. Best of a thin field, but worth
revisiting.** Searching `Icon` (`crates/warp_core/src/ui/icons.rs`):
the only candidates are `Refresh`, `RefreshCcw`, `RefreshCw04`,
`Maximize`, `Minimize`, `ExpandUp/Down`, `ClockRefresh`. There is
**no** `ZoomIn` / `ZoomOut` / `One-to-One` / `Square` / `Hundred` /
`Original` icon in the enum. The remaining options are:

- `Refresh` (chosen): a circular-arrow reload glyph. Semantically
  "reload" rather than "fit to 100%". Mild mismatch — a user who
  doesn't know the keybinding could read it as "reload image."
- `Maximize`: the four-corner arrows glyph. Reads as "go fullscreen,"
  not "reset zoom." Worse.
- A `"1:1"` or `"100%"` text label via `button::Content::Text`:
  semantically perfect, but breaks visual rhythm with the Minus / Plus
  icon-buttons on either side. Mixed icon+text toolbars look untidy
  at button::Size::Small.
- Drop the reset button entirely (cmd+scroll covers in/out; reset is
  the rarest action): smallest UI surface, but reset-to-100% is the
  highest-value zoom interaction precisely because it's hard to land
  by mouse alone. Removing it would push users into "click minus
  eight times" territory.

`Refresh` is the least-bad of the available icons. The right
follow-up is to add a `ZoomReset` / `OneToOne` icon to the icon enum
rather than to change the choice here. Not blocking.

**6. `SCROLL_ZOOM_DEAD_ZONE = 1.0`. Plausible but not load-bearing —
verify on a real device.** `Event::ScrollWheel.delta`
(`crates/warpui_core/src/event.rs:102-107`) is a `Vector2F`. On macOS
(`crates/warpui/src/platform/mac/event.rs:235-246`) the delta is
populated directly from `NSEvent.scrollingDeltaX/Y` with
`precise: hasPreciseScrollingDeltas()`. NSEvent's `scrollingDeltaY`
reports **pixels** when `hasPreciseScrollingDeltas` is true (trackpad
/ Magic Mouse) and **lines** (typically ~1 unit per detent) when
false (classic wheel mouse). So:

- Precise (trackpad): a deliberate gesture is tens-to-hundreds of
  pixels per frame. A resting trackpad reporting < 1 px / frame is
  exactly what `SCROLL_ZOOM_DEAD_ZONE = 1.0` is supposed to swallow.
  Plausible.
- Non-precise (classic wheel): one detent reports a delta on the
  order of ~1.0 to ~10.0 (line units). Here `1.0` is right at the
  threshold — a slow wheel-tick at delta = 0.9 would be dropped,
  delta = 1.1 would fire. Borderline.

The 1.0 dead-zone is safe-ish but not principled. A cleaner design
would branch on `precise`: `if precise { 1.0 } else { 0.0 }` (or
similar), since on a classic wheel **every** event is intentional and
a dead-zone is counterproductive. Worth a comment to that effect, or
a follow-up to thread `precise` into the handler. Not blocking — the
trackpad case is the common one and 1.0 handles it. The doc comment
("macOS continuous-touch scroll events report values well below 1.0
at rest") is empirically defensible but worth caveating with the
classic-wheel case.

**7. `if !modifiers.cmd && !modifiers.ctrl`. Right call — keep as
"either is fine."** Treating cmd-or-ctrl as interchangeable matches
macOS Preview (cmd+scroll) and the cross-platform `cmdorctrl-` idiom
used throughout the FixedBinding layer. Pressing both cmd and ctrl
simultaneously is a non-issue: there is no plausible chord where the
user wants cmd-ctrl-scroll to do anything *other* than zoom, and an
xor (`modifiers.cmd ^ modifiers.ctrl`) would surprise the dual-key
user by **not** zooming in exactly the case where the user clearly
intends to zoom. Keep the inclusive `||` (written as a guard:
`!cmd && !ctrl → propagate`). The current form is correct.

**8. The 20-line history comment in `init()`. Useful, keep it.** The
comment at `app/src/workspace/lightbox_view.rs:27-46` reads as
heavy-handed compared to the surrounding two-line comments, but the
*content* is load-bearing: it records two separate failed approaches
(t2-7 bare keys, t2-11 modifier keys) and explains *why* the file no
longer contains zoom keybindings. Without it, a future contributor
will inevitably try to re-add `cmdorctrl-=` and re-create the bug.
The file as a whole is 758 lines with 199 comment lines (~26%); a
20-line block in a 758-line file is well within the codebase's comment
density. Specifically: this is exactly the kind of "why the code
*doesn't* do the obvious thing" comment that pays for itself the
first time it prevents a regression. Keep verbatim.

**9. Naming. Consistent — no notes.** `on_zoom` / `ZoomDirection` /
`ZoomHandler` mirrors `on_navigate` / `NavigationDirection` /
`NavigateHandler` and `on_dismiss` / `DismissHandler`. Each handler is
named after the user-facing verb and each direction enum is named
after the noun. No deviation needed.

**10. No new tests. Acceptable; one extraction would help.** GUI
rendering and mouse-event dispatch genuinely aren't unit-testable at
this layer (R1 will scrutinise). The one piece that **is** pure logic
is the scroll-delta→zoom-direction decision (lines 309-319 of
`crates/ui_components/src/lightbox.rs`):

```rust
let dy = delta.y();
if dy.abs() < SCROLL_ZOOM_DEAD_ZONE { return StopPropagation; }
let direction = if dy > 0.0 { ZoomDirection::In } else { ZoomDirection::Out };
```

Extracting this to a `fn scroll_delta_to_zoom(dy: f32) -> Option<ZoomDirection>`
free function would let the test surface cover the three branches
(positive over threshold, negative over threshold, |dy| < threshold).
Three trivial tests, zero behaviour change, real regression value the
next time someone touches `SCROLL_ZOOM_DEAD_ZONE`. Worth a follow-up,
not a blocker.

**11. `on_zoom` dispatch closure shape vs `on_navigate`. Consistent.**
Both closures use the identical `|direction, ctx, _|` arg pattern in
`lightbox_view.rs:404` (navigate) and `lightbox_view.rs:427` (zoom),
ignoring the third `&AppContext` arg with `_`. Both use an exhaustive
`match` on the direction enum. Mirrors cleanly.

**12. SHA hygiene.** Pre-amend SHA is `65b2f56`; the tracker (and any
later loop-hygiene amend) may show a different post-amend SHA, matching
the t2-10 / t2-11 pattern. The frontmatter captures `65b2f56` at
review time; not a quality concern.

# Verdict

**pass-with-nits.**

The architecture is sound: ZoomDirection/ZoomHandler mirror the
existing NavigationDirection/NavigateHandler convention exactly, the
button-field verbosity is justified, the corner-inset magic numbers
are consistent with the existing close-button positioning, the 20-line
history comment is the right amount of "why the obvious thing isn't
here," and the cmd-or-ctrl modifier check is correctly inclusive. The
nits are all follow-ups:

- (4) extract `SCRIM_BUTTON_INSET = 12.` to deduplicate the four
  literal `12.` offsets in `lightbox.rs`.
- (5) add a proper `ZoomReset` / `OneToOne` glyph to the icon enum and
  swap it in for `Icon::Refresh`.
- (6) make `SCROLL_ZOOM_DEAD_ZONE` aware of `Event::ScrollWheel.precise`
  (or document why 1.0 is acceptable for the non-precise / classic-
  wheel case).
- (10) extract the dead-zone-plus-sign decision to a pure helper and
  cover with three trivial unit tests.

None of these block the change. The implementation faithfully captures
the lesson from t2-7 → t2-11 (you can't bind keyboard zoom in a
LightboxView, because it doesn't claim focus) and routes around it
with a GUI surface that's idiomatic for this codebase.
