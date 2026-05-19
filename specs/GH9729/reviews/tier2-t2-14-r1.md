---
item: tier2-t2-14
commit: 46f0a2e
reviewer: R1-correctness
spec_ref: tech.md §697-699 (supplemental)
verdict: pass-with-nits
---

# Spec

`tech.md` §697-699 enumerates v1.1 follow-ups (animated GIF/WebP, zoom+pan,
status footer); they describe *features*, not chrome polish, so this commit
attaches as a supplemental polish row to the §698 zoom-control work landed
in t2-11/t2-12/t2-13. The tracker bullet calls out three issues observed at
427% zoom — (a) scrim alpha 230/255 leaks the new-tab page through, (b) the
zoom toolbar lacks its own background and disappears into the dim scrim,
(c) the t2-7-r1 visual-no-op gotcha — and scopes this commit to (a) and (b).
The implementation is a one-line bump of `scrim_color()` from alpha 230 to
250, with the (b) toolbar legibility addressed implicitly as a side-effect
of the darker scrim under the existing `themes::Secondary` button chrome.

# Findings

- [pass] One-line diff in `crates/ui_components/src/lightbox.rs:372`,
  `ColorU::new(0, 0, 0, 230)` → `ColorU::new(0, 0, 0, 250)`. No logic or
  layout change; risk surface is purely the rendered alpha.
- [pass] `scrim_color()` is module-private and has exactly one call site
  (line 711, `.with_background_color(scrim_color())`). Confirmed by
  ripgrep across `crates/` — no other consumer can drift, no other path
  hardcodes 230.
- [pass] No snapshot / insta / golden-image tests reference the scrim
  alpha or `scrim_color`; the commit message's "no test churn" claim
  checks out.
- [pass] Error path (`LightboxImageSource::Error`, lines 642-658) renders
  white text directly on the scrim; the bump only *improves* contrast,
  doesn't regress it.
- [pass] No floating-point or NaN exposure — pure `u8` constant, no
  arithmetic. Zero-size images, theme switching, light/dark mode are
  unaffected because the scrim is always opaque-black regardless of
  theme (this is true both before and after the commit; not introduced
  here).
- [nit] The commit message and the bullet claim toolbar prominence
  improves "as a side-effect" because `Button::Size::Small` +
  `themes::Secondary` already has its own button-chrome. I did not
  verify this visually and there is no code change that strengthens
  the toolbar background container directly. If a future round still
  finds the toolbar indistinct, the correct fix is an actual
  container/pill around the `[−] [100%] [+]` cluster (added to the
  follow-up backlog rather than landing covertly here). The commit
  is honest about this being a manual re-verification — flagging so
  the next reviewer with a build can confirm.
- [nit] Doc comment leaves the door open ("Set to 255 if a fully
  opaque modal is preferred") but doesn't say *who decides* or under
  what evidence. Low-stakes: this is a follow-up note, not a TODO that
  needs an issue. Mentioning here only to note that with alpha=250 the
  remaining 2% transparency is imperceptible to most users and the
  "faint hint of underlying surface for spatial context" rationale is
  weak — but also not load-bearing, so no action requested.
- [minor] The supplemental bullet explicitly defers issue (c) (the
  visual-no-op at >100% zoom reachable from cmd+scroll) to `t2-7-pan`,
  and t2-19 / t2-20 / t2-21 already landed `PanClippedImage` + pan
  state + 1.25x zoom-and-center on later commits. Worth confirming
  in the tracker that (c) is now covered by those follow-ups so this
  bullet's "deferred as `t2-7-pan`" caveat doesn't go stale. Out of
  scope to fix in *this* review file, but the next R2 pass over t2-14
  could note it in the tracker tick.

# What I checked

- Full `git show 46f0a2e` diff — exactly two hunks, one src + one
  tracker; src hunk is alpha 230 → 250 plus an updated doc comment.
- `tech.md` lines 696-710 to anchor §697-699 — confirmed they describe
  v1.1 features (animated playback, zoom+pan, status footer), so the
  "supplemental" framing in the spec_ref is appropriate.
- `crates/ui_components/src/lightbox.rs` lines 700-730 to confirm
  `scrim_color()` flows into a single `Container::with_background_color`
  call and nothing else reads it.
- ripgrep for `scrim_color`, alpha `230`, alpha `250`, and
  `ColorU::new(0, 0, 0` across `crates/` — only the lightbox
  references appear; the one unrelated `(230, 230, 255, 255)` hit in
  `warpui/examples/list/root_view.rs` is example code with a different
  RGBA tuple meaning.
- Error-render arm (`LightboxImageSource::Error`) — text-on-scrim
  contrast strictly improves.
- `Lightbox` struct invariants around zoom / pan / `drag_state` —
  none of those touch the scrim background, so persistent state from
  t2-19/t2-20/t2-21 isn't affected.

# Suggestions

- If a future iteration still finds the toolbar visually indistinct,
  prefer an explicit pill/container around the zoom toolbar
  (`themes::Secondary` with `with_uniform_padding` and a corner
  radius) over a further alpha bump — alpha 250 is already close to
  the 255 ceiling and additional opacity won't help toolbar
  contrast against a near-black surface.
- When ticking (c) on the tracker, cross-link to the t2-19/t2-20/t2-21
  pan-implementation commits so the "deferred as `t2-7-pan`" wording
  in this bullet doesn't read as still-open.
