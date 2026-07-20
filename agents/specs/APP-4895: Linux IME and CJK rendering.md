# APP-4895: Linux IME and CJK rendering

*Spec: Make Linux Wayland IME input reliable and prevent malformed CJK fallback fonts from breaking glyph rendering.*

## Evidence and current state

The canonical report is [Warp #11524](https://github.com/warpdotdev/warp/issues/11524). It contains two reproducible user-visible symptoms:

1. On Ubuntu/GNOME Wayland with Fcitx5, configuring Left Shift as the input-mode toggle does not change the active input mode in Warp, although the same shortcut works in GNOME Terminal and other terminals. Follow-up reports also reproduce this on X11 and with Ctrl+Space.
2. CJK text such as `你好世界` is rendered garbled or bloated instead of using a valid system fallback font.

Code-level investigation was attempted before this spec:

- The pinned client checkout is `ebe3d363e053f2791a6682c7f80b124319be1c86`. Comparing its parent with this commit reproduces the first defect's activation gap: before merged PR [#13819](https://github.com/warpdotdev/warp/pull/13819), Linux only called `set_ime_allowed(true)` on the X11 path; the current commit enables it for every Linux window and adds Wayland cursor-area/event-loop guards. The existing modifier conversion tests pass (`6 passed`), but there is no regression test for the actual IME lifecycle or Fcitx5 shortcut.
- The current environment has no `fcitx5` binary and only a virtual X display, so a native Wayland/Fcitx5 end-to-end run could not be performed here. The implementation must not claim the issue is fixed until the required Wayland and X11 exercises below are run.
- The CJK failure has an executable prior reproduction in open PR [#13144](https://github.com/warpdotdev/warp/pull/13144): `DroidSansFallback.ttf`/`DroidSansFallbackFull.ttf` contain reserved bit 7 in simple-glyph coordinate flags; strict `skrifa`/`swash` parsing rejects their outlines and the fallback path renders `.notdef`/empty glyphs. The PR's review identified that sampling only a few flags, checking only the first face of a TTC, and removing a whole path can miss or discard valid faces. The current `fonts.rs` has no reserved-bit validation.

## PRODUCT

### Desired behavior

1. Every Linux Warp window opts into the platform IME path. On native Wayland, Fcitx5 can activate the configured input method, and a modifier-only Left Shift shortcut toggles the input mode without being swallowed by Warp.
2. IME preedit and commit events are delivered once to the focused editor/terminal. A preedit updates marked text; a commit clears marked text before inserting committed characters. Repeated compositor `Enabled`/cursor-area events do not create a feedback loop, duplicate text, or broken-pipe behavior.
3. Existing X11 behavior remains supported, including Fcitx5 shortcuts that use Left Shift or Ctrl+Space. The Wayland fix must not remove the X11 cursor-position cache workaround.
4. CJK characters render with a valid fallback glyph. A malformed font face is never selected for fallback, while valid faces in the same TTC and later valid fallback families remain eligible.
5. A malformed fallback face must not cause the whole glyph layer or unrelated glyphs to disappear. ASCII/Latin text, emoji/color glyphs, valid CJK fonts, and non-font raster errors retain their existing behavior.

### Key design choices

- Preserve the merged winit text-input-v3 approach from #13819 rather than adding a second IME backend; only change the event-loop path if the required end-to-end test demonstrates a remaining failure.
- Keep strict TrueType parsing and quarantine only faces whose complete simple-glyph flag stream contains reserved bit 7; do not make `swash` accept malformed outlines or hide unrelated rasterization errors.
- Validate and cache by `(font path, face index)`, not only by path, so a corrupted face in a collection cannot discard clean siblings.

### Design alternatives

- **IME: add a direct Fcitx5/IBus DBus backend.** Rejected for this ticket: it duplicates winit's native text-input-v3 integration, adds platform-specific dependencies and lifecycle ownership, and does not address the already-merged activation/loop fixes.
- **IME: force XWayland/XIM.** Rejected: it loses native Wayland behavior, fails on compositors without a useful XIM path, and does not match the reporter's native Wayland setup.
- **Fonts: accept reserved bit 7 in `ttf-parser`/`swash`.** Rejected: it weakens strict parsing for malformed fonts and risks undefined or incorrect outlines.
- **Fonts: return an empty glyph or continue after every raster error.** Rejected as the primary fix: it masks the malformed-face selection problem and could hide unrelated renderer failures. The renderer may continue to other glyphs only when the selected font face has been explicitly quarantined.
- **Fonts: copy #13144's background scanner unchanged.** Rejected as a default: the PR's global path-level state, partial sampling, TTC handling, and asynchronous removal create races and can remove valid faces. Reuse its raw-table detection idea only with complete parsing and per-face ownership.

## TECH

### Context

- `crates/warpui/src/windowing/winit/window.rs:1457 @ ebe3d363e053f2791a6682c7f80b124319be1c86` creates Linux windows and now calls `set_ime_allowed(true)` unconditionally.
- `crates/warpui/src/windowing/winit/event_loop/mod.rs:1270 @ ebe3d363e053f2791a6682c7f80b124319be1c86` converts modifier-only `KeyboardInput` events into `ModifierKeyChanged`; `:1526` handles `Ime::Enabled`, `Preedit`, `Commit`, and `Disabled`; `:1801` updates the active IME cursor area and distinguishes Wayland from X11.
- `crates/warpui/src/windowing/winit/event_loop/key_events.rs:58 @ ebe3d363e053f2791a6682c7f80b124319be1c86` converts ordinary key events. Its existing tests cover key normalization but not IME lifecycle or modifier-only Fcitx5 activation.
- `crates/warpui/src/windowing/winit/fonts.rs:188 @ ebe3d363e053f2791a6682c7f80b124319be1c86` owns the font database; `:323` inserts a font source and maps every loaded face; `:534` loads Linux fallback families. The current implementation validates parseability but does not inspect `glyf` simple-glyph flags.
- `crates/warpui/src/windowing/winit/fonts/swash_rasterizer.rs` rasterizes the selected face through the cosmic-text/swash path. `crates/warpui/src/rendering/wgpu/renderer/glyph.rs:177-215` currently returns from the whole layer on a glyph-cache/raster error, so malformed-face selection must be prevented before this path.
- `crates/warpui/Cargo.toml:157-165 @ ebe3d363e053f2791a6682c7f80b124319be1c86` pins the relevant stack: `owned_ttf_parser` 0.25.0, `fontdb` 0.23.0, the Warp cosmic-text revision, and winit.

### Proposed changes

#### IME activation and event routing

1. Keep Linux `set_ime_allowed(true)` for both X11 and Wayland windows, and retain the #13819 Wayland cursor-area deduplication and disabled→enabled edge guard.
2. Add a deterministic event-loop test seam (pure state helper or equivalent) that verifies:
   - `Ime::Enabled` reports the cursor position once per disabled→enabled transition;
   - repeated `Enabled` events do not re-enter an update loop;
   - identical cursor areas are deduplicated on Wayland, while an equivalent X11 update still performs the existing nudge;
   - switching focus to another window with the same rectangle still updates that window;
   - preedit is ignored while disabled and is dispatched as marked text while enabled;
   - commit dispatches `ClearMarkedText` before exactly one `TypedCharacters` event.
3. If the native Wayland/Fcitx5 exercise still fails on the pinned #13819 code, make the smallest change in `event_loop/mod.rs`/`key_events.rs` needed to preserve modifier-only Left Shift for IME activation and to consume each preedit/commit sequence once. Do not add a second IME backend or change shell/keybinding semantics.

#### Malformed CJK fallback faces

1. Add a testable validator for TrueType `glyf` simple glyph flags. It must parse `head`, `loca`, and `glyf`, handle both short and long `loca` formats, inspect every simple glyph, and honor repeat flags. Any flag byte with reserved bit 7 set marks that face malformed; composite/empty glyphs and fonts without `glyf` are not rejected by this rule.
2. Integrate validation at `TextLayoutSystem::insert_font` (or an equivalent single ownership boundary) before a face can enter the fallback maps. Cache results by `(Source::File path, face index)`; binary/bundled fonts remain supported and are validated when their bytes are available.
3. For a TTC, inspect every loaded face and remove or skip only malformed face IDs. Keep clean siblings, update `font_id_map`, `loaded_fonts`, `font_selections`, family membership, and non-Windows fallback lists consistently, and return a clear error only when the requested source has no valid face left.
4. Preserve the existing strict parser/rasterizer behavior for valid fonts. Do not classify a face as malformed merely because a transient swash image lookup or unrelated raster operation returns `None`/an error.
5. Add a deterministic CJK fixture/test that sets bit 7 on a late coordinate flag and on a repeated flag, proves both are rejected, proves a clean sibling face in a TTC remains selectable, and proves `你好世界` selects a valid fallback and produces non-empty raster bounds/atlas data. Include a test for the no-valid-face error path.

### Open questions resolved

- **Is the IME problem still an unimplemented Linux activation path?** No. On the spec commit, #13819 is merged and current `master` unconditionally enables Linux IME; the remaining work is regression coverage and a narrowly scoped event-loop correction only if the end-to-end test still reproduces.
- **Which framework is in scope?** Native winit text-input-v3 on Wayland plus the existing X11 path. Fcitx5 is the required acceptance framework; IBus is an adjacent smoke check, not a new backend.
- **Should malformed-font handling be global or per-face?** Per `(path, face index)`, because TTC files can contain both malformed and valid faces.
- **Should the implementation weaken `ttf-parser`?** No; quarantine malformed faces and preserve strict parsing.
- **What happens when the environment cannot run Wayland/Fcitx5?** The implementation must record the exact environment mismatch and cannot claim the user-facing defect is verified; the code-level tests and `./script/presubmit` remain mandatory, and a human must run the missing visual exercise before merge.

## Validation & verification criteria

All criteria must pass before merge:

1. **Baseline reproduction and current-state check.** Run the report's exact input steps on a native Linux Wayland session with Fcitx5 and Left Shift configured as the toggle, then repeat in a reference terminal. Warp must toggle modes exactly as the reference terminal does; no `Ime::Enabled`/preedit/commit storm or duplicate committed text may occur. Record the environment, Warp build, Fcitx5 version, compositor, and `fcitx5-diagnose` output.
2. **X11 compatibility.** Repeat Left Shift and Ctrl+Space input-mode switching under X11/Fcitx5 (including the issue-reported `XMODIFIERS=@im=fcitx` setup). Candidate selection must commit text to Warp, and the existing X11 cursor-area nudge must remain active.
3. **IME regression tests.** Add and pass focused unit tests for the `Ime` state sequence and Wayland/X11 cursor-area behavior described in TECH. The tests must fail against the pre-#13819 Linux window behavior or an equivalent regression fixture and pass on the implementation branch.
4. **IME UI proof.** Exercise a running Warp window with computer use on native Wayland/Fcitx5, capture screenshots or video showing (a) Left Shift toggling the indicator/mode, (b) preedit text, and (c) committed Chinese text in the terminal/editor. Attach the visual proof to the task record and PR; compare the same text and shortcut in a reference terminal.
5. **Malformed-glyph parser regression.** Add a regression test that fails before the change and passes after it for reserved bit 7 in a late simple-glyph flag and a repeated flag, using deterministic font bytes/fixtures. The test must cover both short/long `loca` handling and all faces in a TTC.
6. **Fallback and raster output.** With a malformed primary CJK face installed or represented by the fixture, verify that the malformed face is absent from candidate/fallback selection, a valid sibling/next fallback is selected, `你好世界` has non-zero glyph IDs and raster bounds, and one malformed glyph cannot abort rendering of neighboring ASCII/CJK glyphs.
7. **No collateral damage.** Run the existing `warpui` key-event and text-layout tests, including `cargo test --manifest-path Cargo.toml -p warpui --lib key_events` and `cargo test --manifest-path Cargo.toml -p warpui --lib text_layout`, plus the new IME/font tests. Valid bundled fonts, valid TTC siblings, Latin text, emoji, and X11 behavior must remain green.
8. **Full repository gate.** From the repository root, `./script/presubmit` passes with no new warnings or failures.
9. **Final reproduction.** Re-run the exact Wayland/Fcitx5 and CJK steps from criteria 1 and 6 after all tests and presubmit complete. The observed result must remain correct in a fresh Warp process, not only in a warmed font cache or after restarting Fcitx5.
