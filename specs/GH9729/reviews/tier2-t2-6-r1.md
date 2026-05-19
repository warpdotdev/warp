---
item: tier2-t2-6
commit: f077496
reviewer: R1-correctness
spec_ref: tech.md §697
verdict: pass-with-nits
---

# Spec

§697 (verbatim from `specs/GH9729/tech.md:697`):

> **Animated GIF / WebP continuous playback in the Lightbox**. Wire `Image::enable_animation_with_start_time(Instant)` into the Lightbox image element and drive a per-frame redraw loop on the focused entry. **Play/pause control** is the next layer on top.

§544 (last sentence of step 7) says, prior to t2-6, that "Animated GIF and animated WebP files render their first frame statically because the Lightbox does not call `enable_animation_with_start_time` (no v1 change to this)." That sentence is intentionally a v1 limitation; §697 is the follow-up that supersedes it for tier-2. The §544 prose itself is not edited (per the review's hard rules), and that is fine — §544 documents the v1 end-to-end flow, while §697 is the explicit follow-up that lifts the limitation.

# Findings

- **[pass]** §697 (a) "wire `enable_animation_with_start_time` into the Lightbox image element" is correctly implemented. `lightbox.rs:172-181` builds the `Image` element and conditionally calls `.enable_animation_with_start_time(start)` when `params.animation_start_time` is `Some`. `lightbox_view.rs:274` passes `Some(self.animation_start_time)` from the live view.
- **[pass]** §697 (a) "drive a per-frame redraw loop on the focused entry" — no explicit timer or `ctx.spawn` is needed because GPUI's `Image::paint_animated_image` (`crates/warpui_core/src/elements/image.rs:213-215`) already calls `ctx.repaint_after(Duration::from_millis(remaining_delay))` on each paint when `started_at.is_some()`. The diff's reliance on this implicit self-loop is correct, and the inline comment at `lightbox.rs:172-176` documents it accurately.
- **[pass]** §697 (b) "play/pause control is the next layer on top" — explicitly deferred. The TIER2_TODO entry "t2-6-pause" (lines 106-117) and the commit message both spell out a coherent reason: real pause-resume needs a `paused_at`/frozen-elapsed primitive on the upstream `Image` element; the two call-site-only workarounds (rebuild `started_at = now() - paused_elapsed`, or drop `enable_animation_with_start_time` while paused) either silently keep advancing the frame or jump back to frame 0 on resume. The spec phrasing "next layer on top" treats pause as a sub-step, not a same-iteration deliverable, so this deferral is on-spec.
- **[pass]** `Instant` type matches the upstream API. `Image::enable_animation_with_start_time` takes `instant::Instant` (`image.rs:11,128`); a `std::time::Instant` would be a type mismatch. The diff imports `instant::Instant` in both `lightbox.rs:3` and `lightbox_view.rs:3`. Workspace dep declaration is correct: `Cargo.toml:167` already declares `instant = { version = "0.1.12", features = ["wasm-bindgen"] }`, and the new `crates/ui_components/Cargo.toml:9` line `instant.workspace = true` picks it up.
- **[pass]** Static-image regression — none. `paint_static_image` (`image.rs:359-360,209`) never reads `started_at`. The animated branch (`image.rs:362-363`) is only taken for `Image::Animated(...)` payloads. Static PNG/JPEG/SVG paths are unaffected by setting `animation_start_time`.
- **[pass]** Reset-on-navigation. `lightbox_view.rs:301-303,308` resets `animation_start_time` to `Instant::now()` on `NavigatePrevious`/`NavigateNext`, so a newly-focused image plays from frame 0 of its own loop rather than continuing the previous image's clock. Matches the commit message claim and the spec's "per focused entry" wording.
- **[pass]** Re-render survival. `animation_start_time` is owned by `LightboxView` (a struct field at `lightbox_view.rs:72`), not reconstructed in `render`. A `ctx.notify()` for a non-navigation reason (theme change, asset state change, etc.) re-runs `render` but reads the existing field, so the animation timeline survives. Confirmed by reading the `render` body (`lightbox_view.rs:265-294`); only `params.images.get(self.current_index)` and the field itself are read on each render.
- **[pass]** Reset on `update_params`. `lightbox_view.rs:97` resets the anchor when `LightboxView::update_params` replaces the image set — correct, because that path corresponds to the user dismissing and re-opening (or programmatically replacing) the lightbox.
- **[pass]** Concurrency / cancellation. Dismissal flows through `LightboxViewAction::Dismiss → emit(LightboxViewEvent::Close)` (`lightbox_view.rs:296-297`), handled at `view.rs:7483-7484` where `me.lightbox_view = None`. Dropping the view drops the `Image` element; without a paint pass `ctx.repaint_after` is never re-armed, so the per-frame loop stops cleanly. No leaked `ctx.spawn` task is created at this layer.
- **[pass]** Performance / DoS bound. The decode-time cap from §259 (`MAX_ANIMATED_FRAMES`, `MAX_ANIMATED_TOTAL_PIXELS ≈ 256 MB`) bounds frame count and total pixels at asset-cache load time, before `AnimatedBitmap` exists. The per-paint cost is one `get_current_frame(elapsed_ms as u32)` call plus one `paint_static_image` of the resolved frame, scheduled by `ctx.repaint_after(remaining_delay)`. A pathological 1 ms inter-frame GIF would request ≤ 1000 repaints/sec, but the per-call cost is bounded by the static-paint cost of one frame of bounded pixel count — acceptable, and identical to the changelog animation surface that already runs this same pattern in production.
- **[pass]** Examples and tests. Both `lightbox::Params { ... }` literals in `crates/ui_components/examples/library.rs:559-588,593-621` pass `animation_start_time: None`. A workspace-wide `grep -rn "lightbox::Params\s*{"` returns exactly three hits (the two example literals plus `lightbox_view.rs:267`); no other call site was missed. Build-pass corroborates this — a missed literal would have failed compile because the new field is non-optional in the struct definition.
- **[nit]** **Reset on load completion is not done.** The post-load callback at `lightbox_view.rs:166-175` calls `ctx.notify()` and rewrites `slot.source` for `Error` cases but does not reset `self.animation_start_time`. User-visible effect: if a 5 MB animated GIF takes ~600 ms to read off disk and decode, then once it pops in, `paint_animated_image` computes `elapsed_time = 600 ms` and the animation visibly starts ~600 ms into the loop rather than at frame 0. For a sub-second-loop GIF this can be ½ to a full revolution of out-of-phase. The fix is one line: `me.animation_start_time = Instant::now();` inside the spawn closure (after the `ctx.notify()`), gated on the new state being `AssetState::Loaded` so non-load updates don't bump it. This is a polish nit, not a blocker — the animation still plays continuously and from the correct frames; it just doesn't begin at frame 0 of the loop. Recommend filing as an R2 follow-up if not addressed in this round.
- **[nit]** The doc comment on the new `Params::animation_start_time` field (`lightbox.rs:93-100`) describes the `None` case as "first frame only (legacy behaviour, kept for callers that don't want animation, e.g. inert example/test surfaces)." That is technically correct for `None`, but it underplays the upstream contract: when `started_at` is `None`, `paint_animated_image` falls back to `Instant::now()` for the elapsed-time calc and *also* skips `ctx.repaint_after` (`image.rs:200-215`). So `None` means "first frame frozen forever", not "first frame, then animation idle from then on" — which the wording does not contradict but could clarify for a future reader.

# What I checked

- `git show f077496` and `git show --stat f077496`: 6 files, +63/-6, all changes confined to lightbox surface plus TIER2_TODO ledger update.
- `specs/GH9729/tech.md:697` — quoted verbatim above.
- `specs/GH9729/tech.md:544` — confirmed step-7 prose flags v1 limitation, superseded by §697; not edited by this commit.
- `specs/GH9729/tech.md:256-316` — §259-area decode caps confirmed (`MAX_ANIMATED_FRAMES`, `MAX_ANIMATED_TOTAL_PIXELS ≈ 256 MB`).
- `crates/warpui_core/src/elements/image.rs:11,128,192-221,353-365` — confirmed `enable_animation_with_start_time` shape, `paint_animated_image` self-loop via `ctx.repaint_after`, `started_at`-gated repaint, and the static-vs-animated branch.
- `crates/ui_components/src/lightbox.rs:3,93-100,172-181` — new field, doc comment, opt-in builder call.
- `crates/ui_components/Cargo.toml:9` — `instant.workspace = true` added.
- `Cargo.toml:167` — workspace `instant = { version = "0.1.12", features = ["wasm-bindgen"] }` already present.
- `app/src/workspace/lightbox_view.rs:3,72,85,97,267-274,296-297,301-303,308` — view field, init, `update_params` reset, render-pass plumbing, navigation resets.
- `app/src/workspace/lightbox_view.rs:155-178` — post-load spawn closure does NOT reset `animation_start_time` (basis for first nit).
- `app/src/workspace/view.rs:7478-7495` — `LightboxViewEvent::Close → me.lightbox_view = None` confirms clean teardown.
- `crates/ui_components/examples/library.rs:559-621` — both literals pass `animation_start_time: None`.
- Workspace-wide search `grep -rn "lightbox::Params\s*{"`: 3 matches, all updated.
- `specs/GH9729/TIER2_TODO.md:48-58,77-81,103-117` — t2-6 marked `[x]` with pause-deferral rationale; new `t2-6-pause` Deferred R2 entry has a coherent technical justification.

# Suggestions

- Consider the one-line load-completion reset (gated on `AssetState::Loaded`) inside `start_asset_load`'s spawn closure to make animated images start at frame 0 after the disk-read latency. Defensible to defer to R2; flagging now so it doesn't get lost.
- Optional: tighten the `animation_start_time: None` doc comment to spell out that `None` ⇒ first-frame static (no per-paint repaint), not "starts animating from first paint." The current wording is non-misleading but a future reader picking up t2-6-pause may benefit from the precise contract being one read away.
