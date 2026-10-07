# OSC 8 hyperlinks — implementation plan

Source issue: [warpdotdev/warp#6393](https://github.com/warpdotdev/warp/issues/6393).
Behavior: see [`product.md`](product.md).

## Context

OSC 8 is the cross-terminal escape sequence for attaching a URL to a span of cells (`ESC ] 8 ; params ; URI ST` … visible text … `ESC ] 8 ; ; ST`). Warp parses OSC sequences via its alacritty/vte fork but currently has no `b"8"` arm — the bytes are dropped on the `_ => unhandled(params)` floor and the visible text becomes plain, unclickable output.

**ANSI processor** — `app/src/terminal/model/ansi/mod.rs:811-1230` is the OSC dispatch (`fn osc_dispatch`). It is a long match on the first param: `b"0" | b"2"`, `b"4"`, `b"9"`, `b"10" | b"11" | b"12"`, `b"50"`, `b"52"`, `b"104"`, `b"110-112"`, `b"133"` (prompt markers), `b"777"`, `b"1337"` (iTerm images), and several Warp-specific OSC numbers. Each arm calls into the `Handler` trait. The dispatcher itself stays stateless; per-stream state lives on the implementer.

**Handler trait** — `app/src/terminal/model/ansi/handler.rs:27-…`. Long trait with required and default methods. Default methods are used for hooks that not every implementor cares about (`prompt_marker`, `pluggable_notification`, the Warp DCS hook callbacks). Today's implementors: `GridHandler` (`app/src/terminal/model/grid/ansi_handler.rs:159`), `BlockGrid` (`app/src/terminal/model/blockgrid.rs:704`), `Block` (`app/src/terminal/model/block.rs:2906`), `AltScreen` (`app/src/terminal/model/alt_screen.rs:371`), `EarlyOutputHandler` (`app/src/terminal/model/early_output.rs:298`), and the test `MockHandler` (`app/src/terminal/model/ansi/mod_test.rs:24`).

**Shared ANSI types** — `crates/warp_terminal/src/model/ansi/control_sequence_parameters.rs`. `Attr`, `Mode`, `CursorStyle`, `PromptMarker` etc. live here and are re-exported by `app/src/terminal/model/ansi/mod.rs:21` (`pub use warp_terminal::model::ansi::control_sequence_parameters::*;`). New shared types (`Hyperlink`) belong here so both the parser and the trait can name them.

**Cell storage — both representations are in play.** A common simplification is "block list = `FlatStorage`, alt-screen = 2D `Cell+Row`," but the block list actually uses **both** at different points in a cell's lifecycle. Hyperlinks therefore have to be wired through both, with the migration path between them preserving the id.

- **Live (active) rows: 2D `Cell` + `Row`.** As output streams in, `GridHandler` writes into the visible 2D grid built from `Cell` (`crates/warp_terminal/src/model/grid/cell.rs:144`) and `Row`. `Cell` is 24 bytes and deliberately tightly packed (the comment at `cell.rs:122` calls out that adding bytes to `Cell` itself bumps the struct to 32 bytes — a 33% memory hit). Optional rare attributes go in `CellExtra` (`cell.rs:113-120`), a boxed side allocation that already holds `cell_with_zero_width` and `end_of_prompt`. Hyperlinks belong in `CellExtra`, not in the base `Cell`. This is also the only representation `AltScreen` uses — alt-screen apps don't migrate to scrollback.
- **Scrollback / finished rows: `FlatStorage`.** Once a block finishes (`GridHandler::finish` at `grid_handler.rs:1621-1643`, which calls `resize_storage(1, …)` to push everything in when the `MaximizeFlatStorage` flag is on) — and during normal operation, when rows scroll out of the visible 2D grid into history — content moves into `FlatStorage` (`crates/warp_terminal/src/model/grid/flat_storage/`). It uses a run-length-encoded `AttributeMap<T>` per attribute: `FgColorMap` and `BgAndStyleMap` (`flat_storage/style.rs:18-21`). RLE deduplicates long runs of cells that share the same value into a single map entry — a good fit for hyperlinks, where one OSC 8 span typically covers many adjacent cells with the same id.
- **Migration.** The 2D-to-flat path lives in `GridHandler::resize_storage` and the row-eviction code under `crates/warp_terminal/src/model/grid/`. Anywhere a `Cell` is converted into the flat representation (search for `From<&Cell>` impls in `flat_storage/style.rs:55-67` for the existing pattern), the conversion needs to read `cell.hyperlink_id()` and append it to the new `AttributeMap<Option<HyperlinkId>>` we're adding (§3a). The reverse path (flat → grid for resize/reflow that re-materializes rows) needs to do the inverse. Missing this in either direction silently strips clickability when a block scrolls or finishes, which is a regression on product invariant 12.

**Auto-detected URL flow — the model that the OSC 8 plumbing should mirror:**
- Detection lives in `app/src/terminal/model/grid/grid_handler.rs` via `pub fn url_at_point(&self, displayed_point: Point) -> Option<Link>` at line 596 and `Link` at line 143 (`{ range: RangeInclusive<Point>, is_empty: bool }`). `Link: ContainsPoint` (line 169).
- Higher-level wrappers: `app/src/terminal/model/blocks.rs:2460` (`url_at_point` returning `WithinBlock<Link>`), `app/src/terminal/model/alt_screen.rs:285`, and `app/src/terminal/model/terminal_model.rs:1815` (`link_at_range`) / `1844` (`url_at_point` returning `WithinModel<Link>`).
- View layer: `app/src/terminal/view/link_detection.rs` defines `GridHighlightedLink::{Url(WithinModel<Link>), File(WithinModel<FileLink>)}` (line 48), the hover state machine that calls `model.url_at_point(...)` → `self.highlighted_link.set(GridHighlightedLink::Url(url), ...)` at lines 299-310, and the click handler at lines 391-395 (`GridHighlightedLink::Url(url) => { let model = self.model.lock(); ctx.open_url(&model.link_at_range(url, ...)); }`). Right-click context menu and `OpenGridLink` action wiring is in `app/src/terminal/view.rs:24786`.
- Critically: the URL the auto-detector hands to `ctx.open_url` is recovered from the cell text via `link_at_range` — there is no notion of a URL stored separately from the cells. OSC 8 inverts that: the URI is independent of the visible text and must be carried alongside the cells.

A WIP foundation (`Hyperlink` type + parser + Handler hook + OSC 8 dispatch arm + parser unit tests) was scaffolded earlier on this branch and stashed; it is referenced where useful but is not the spec.

## Proposed changes

The implementation is broken into the layers below, listed bottom-up. Each layer can be merged independently behind the `OscHyperlinks` feature flag described in (6); the ordering in **Parallelization** below maps each layer to its dependencies.

### 1. ANSI types — `crates/warp_terminal/src/model/ansi/control_sequence_parameters.rs`

Add a `Hyperlink { id: Option<String>, uri: String }` value type, a `HyperlinkParseError`, and `Hyperlink::parse_osc_params(params: &[&[u8]]) -> Result<Option<Self>, HyperlinkParseError>`.

**Parser contract.** The OSC 8 grammar is `OSC 8 ; params ; URI ST`. The vte parser splits OSC bytes on `;` and hands `params: &[&[u8]]` to the dispatcher; this function takes the slice **after** the leading `b"8"` identifier:

1. **Field layout.** Treat the first slice element as the params field. Treat **all subsequent slice elements rejoined with `b";"`** as the URI field. This explicit rejoin is the parser's single most important rule: real-world URIs contain `;` (matrix params, query separators in some encodings, jsessionid, percent-encoded payloads), and the vte parser will split such URIs across multiple `params` entries. The contract — URI is always the rejoin of `params[1..]` — guarantees those URIs are reconstructed correctly by every implementation. Implementations that follow only the "two-field" mental shortcut **will silently drop valid URIs** and must not pass review.
2. **Close form.** `params == []`, or `params == [b""]`, or the rejoined URI field is empty → `Ok(None)`. Three accepted shapes because real emitters send all three.
3. **Open form.** Rejoined URI field is non-empty → parse the params field and return `Ok(Some(Hyperlink {...}))`. The params field is split on `:`; recognized keys are `id=...`, all others ignored. A params entry without `=` is `Err(MalformedParam)`.
4. **Error cases.** The error variants reflect the actual failure mode, not a catch-all:
    - Non-UTF-8 bytes in the URI → `Err(InvalidUtf8)`.
    - URI exceeding `MAX_URI_BYTES` (a `pub const` defined in this module — see §3) → `Err(UriTooLong { len })`, **checked on the raw `&[u8]` rejoin before allocating the URI `String`**, so a 1 GB OSC 8 sequence never produces a 1 GB allocation.
    - A param without `=` → `Err(MalformedParam)` (already covered in (3)).
    - Empty params slice with no opening byte at all isn't reachable because `osc_dispatch` already guards against `params.is_empty()`.

   ```rust
   #[derive(Debug, thiserror::Error)]
   pub enum HyperlinkParseError {
       #[error("hyperlink URI is not valid UTF-8")]
       InvalidUtf8,
       #[error("hyperlink URI exceeds the {} byte cap (got {len})", MAX_URI_BYTES)]
       UriTooLong { len: usize },
       #[error("hyperlink param is malformed (no '=' found)")]
       MalformedParam,
   }
   ```

**Parser unit tests** live next to the type in a `#[cfg(test)] mod hyperlink_parse_tests`. The required ones:

- **`uri_with_semicolons_is_rejoined`** (anti-regression for finding 5 in the third Oz pass): input `params == [b"", b"https://example.com/a?x=1", b"y=2"]`; assert URI is `"https://example.com/a?x=1;y=2"`. **Failing this test is the cardinal indicator of an implementation that took the two-field shortcut.**
- `open_with_no_params`, `open_with_id_param`, `close_canonical`, `close_single_empty_field`, `close_zero_fields`.
- `unknown_keys_in_params_are_ignored`, `multiple_params_separated_by_colons`, `malformed_param_without_equals_rejected`.
- `non_utf8_uri_rejected`.

Tradeoffs considered:
- **Storing `id` and `uri` directly vs. interning.** Interning at parse time would couple the parser to an app-level registry. The parser stays pure and small; deduplication happens at the storage layer (3).
- **`Hyperlink` as an enum vs. struct.** A `Hyperlink::Open(_)`/`Hyperlink::Close` enum is more self-describing than `Option<Hyperlink>`, but the close form has no payload, so `Option` is the smaller surface and matches `set_title(Option<String>)` and similar Handler signatures.

### 2. Handler trait + OSC 8 dispatch — `app/src/terminal/model/ansi/handler.rs` and `mod.rs`

Add a default-impl method on `Handler`:

```rust
fn set_hyperlink(&mut self, _hyperlink: Option<Hyperlink>) {}
```

The default-impl pattern matches `prompt_marker`, `pluggable_notification`, and the Warp DCS hook callbacks; existing implementors continue to compile unchanged and only the implementors that need to actually attach hyperlinks to cells (3) override it.

In `osc_dispatch`, add a `b"8"` arm next to `b"9"`/`b"133"`:

```rust
b"8" => match Hyperlink::parse_osc_params(&params[1..]) {
    Ok(hyperlink) => self.handler.set_hyperlink(hyperlink),
    Err(_) => unhandled(params),
},
```

### 3. Cell storage — attaching the hyperlink to cells

The implementation needs a deduplicated registry keyed by an opaque small-integer `HyperlinkId` so cells store a 4-byte handle rather than a per-cell `Arc<Hyperlink>`. The registry is per-grid (block-scoped on `BlockList`, screen-scoped on `AltScreen`).

```rust
// New module: crates/warp_terminal/src/model/grid/hyperlink_registry.rs
pub struct HyperlinkId(NonZeroU32);
pub struct HyperlinkRegistry { /* HashMap<Hyperlink, HyperlinkId> + Vec<Hyperlink> */ }
impl HyperlinkRegistry {
    /// Returns `Some(id)` for a successful intern, `None` if the
    /// distinct-entries cap is hit. The URI byte length cap is
    /// enforced earlier — in `parse_osc_params` (§1) — so a too-long
    /// URI never reaches `intern`. That keeps the failure-modes per
    /// caller simple: parse-time gives `Err(UriTooLong { ... })` or
    /// `Err(InvalidUtf8)`; intern-time gives `None`.
    pub fn intern(&mut self, h: Hyperlink) -> Option<HyperlinkId>;
    pub fn get(&self, id: HyperlinkId) -> Option<&Hyperlink>;
}
```

Both storage representations need wiring (the block list uses 2D `Cell+Row` for live rows and `FlatStorage` for scrollback; see Context). The order below matches the order a cell traverses them:

**3a. Live rows: extend `Cell` / `Row`.** Extend `CellExtra` (`crates/warp_terminal/src/model/grid/cell.rs:113`) with `hyperlink_id: Option<HyperlinkId>` and add accessors `Cell::hyperlink_id() / Cell::set_hyperlink_id()`. The 24-byte budget for `Cell` itself is preserved because the new field lives in the boxed extra. Resetting a cell preserves `EndOfPromptMarker`; the `hyperlink_id` slot is cleared along with the rest of the cell's content (per the §3d table). This is the only representation `AltScreen` ever uses, and the *first* representation block-list output uses while the block is live.

**3b. Scrollback: extend `FlatStorage`.** Add a third `AttributeMap<Option<HyperlinkId>>` parallel to `FgColorMap`/`BgAndStyleMap` in `crates/warp_terminal/src/model/grid/flat_storage/mod.rs`. RLE deduplicates long runs of cells that share the same id into one map entry. A new `flat_storage/hyperlink.rs` mirroring `flat_storage/style.rs` is the natural shape.

**3b-bis. Migration between the two representations.** When rows scroll out of the visible 2D grid into scrollback, or when a block finishes and `GridHandler::finish` (`grid_handler.rs:1621-1643`) calls `resize_storage(1, …)` to push everything to flat, every existing `Cell → flat_storage` conversion path needs to also carry the `hyperlink_id`. Concretely, the existing `From<&Cell> for BgAndStyle` impl (`flat_storage/style.rs:55-62`) is the model: we add an analogous `From<&Cell> for Option<HyperlinkId>` impl in the new `flat_storage/hyperlink.rs`, and update the row-flush / row-eviction code that calls these `From` impls so the new map gets populated alongside the existing two. The reverse path — flat → 2D row materialization for resize/reflow — already exists for color/style; we extend it to read the new map and stamp the resulting `Cell::set_hyperlink_id`. Missing either direction silently strips clickability when a block scrolls or finishes, which is a regression on product invariant 12; an integration test pumps an OSC 8 span into a small-height grid, scrolls it past the top of the visible region, and asserts hover/click still work in scrollback.

**3c. Single owner for active id and registry (delegation path).** Active hyperlink state and the registry both live in **one place**, the same place that `input(c: char)` stamps cells with attributes. That's `GridHandler` for `BlockList` flow and `AltScreen`'s own grid for the alt-screen flow. `BlockGrid` and `Block` do **not** carry independent copies; they delegate `set_hyperlink` (and cell stamping) to the inner `GridHandler`. Concretely:

| Type | Owns `active_hyperlink_id`? | Owns `HyperlinkRegistry`? | `set_hyperlink` impl |
| --- | --- | --- | --- |
| `GridHandler` (`grid/ansi_handler.rs:159`) | yes | yes | updates own state, interns into own registry |
| `BlockGrid` (`blockgrid.rs:704`) | no | no | forwards to `self.grid_handler.set_hyperlink(...)` (matches existing delegation pattern for `terminal_attribute`, `input`, etc.) |
| `Block` (`block.rs:2906`) | no | no | forwards via `BlockGrid` to the inner `GridHandler` |
| `AltScreen` (`alt_screen.rs:371`) | yes | yes | parallel to `GridHandler`, owns its own state because alt-screen doesn't share a grid with the block list |
| `EarlyOutputHandler` (`early_output.rs:298`) | no | no | inherits the default `Handler::set_hyperlink` no-op; OSC 8 in early-output is safely dropped |

This makes `set_hyperlink` and the cell-stamping path read the same field. The risk the reviewer flagged — `set_hyperlink` updating one copy while `input` reads another — is closed by ownership, not by careful synchronization.

```rust
// In GridHandler (and AltScreen with the same shape):
fn set_hyperlink(&mut self, hyperlink: Option<Hyperlink>) {
    self.active_hyperlink_id = hyperlink.and_then(|h| self.hyperlink_registry.intern(h));
}

// In BlockGrid and Block:
fn set_hyperlink(&mut self, hyperlink: Option<Hyperlink>) {
    self.grid_handler.set_hyperlink(hyperlink);
}
```

The `and_then` (vs `map`) is the second half of finding 3: when `intern` returns `None` because the registry is full, `active_hyperlink_id` stays `None` and the visible text from the OSC 8 sequence renders as plain non-clickable cells — there is no path where a cell's `hyperlink_id` references an entry that doesn't exist.

The grid's `input(&mut self, c: char)` path stamps `self.active_hyperlink_id` into each newly-written cell, in the same place SGR styling is applied today.

**3d. Behavior under cell-mutating operations.** OSC 8 makes `hyperlink_id` a per-cell attribute, so every operation that changes cell content has to make an explicit choice about it. The rule, applied uniformly:

| Operation | What happens to existing `hyperlink_id` on touched cells | Implementation note |
| --- | --- | --- |
| `input(c)` (write a char to cursor) | replaced with `self.active_hyperlink_id` (which may be `None`) | same place SGR is applied; never inherits the previous cell's id |
| `erase_chars(n)` | cleared to `None` along with all other cell attrs | erased cells render as default-state blanks; **must not** stay clickable |
| `clear_line(LineClearMode::*)` | cleared to `None` for the affected range | same |
| `clear_screen(ClearMode::*)` | cleared to `None` for the affected range | same |
| `delete_chars(n)` | cells are *removed* and following cells shift left; surviving cells keep their ids | id moves with the cell; the trailing inserted blanks have `None` |
| `insert_blank(n)` | newly inserted blanks have `None`; following cells shift right with their ids intact | the active id is **not** stamped onto the inserted blanks — `input` is the only writer that does that |
| `delete_lines(n)` / `insert_blank_lines(n)` | analogous: surviving rows keep ids; new blank rows have all `None` | |
| `scroll_up` / `scroll_down` | scrolled-out rows are dropped (their ids stop being referenced); newly exposed rows have all `None` | the registry itself does not shrink — see no-reclaim §3 |
| `reverse_index` (RI) | like `scroll_down` for boundary cases; otherwise cell content is preserved with its id | |
| `decaln` (DECALN — fill screen with `E`) | ids cleared along with content reset | |
| `reset_state` | `active_hyperlink_id = None`; **all cell ids cleared as a side effect of cell reset**; registry left intact (no-reclaim is fine here) | parallels how `terminal_attribute(Attr::Reset)` behaves, except scoped to the whole grid |
| Block boundary (`prompt_marker` start) | `active_hyperlink_id = None`; the previous block's cell ids are unchanged (block is now scrollback) | see invariant 10 |

Without this section, an implementer could plausibly leave erased blanks with stale ids (clickable empty space) or stamp the active id onto inserted blanks (a "Click to view scan report" link suddenly extending into the blank insert region). The table is the contract that prevents both.

**Bounded registry, no reclamation (security: DoS resistance).** Terminal output is untrusted and a hostile or buggy process can emit unlimited unique URIs. Two caps, enforced at different layers, and **a no-reclaim model**: entries are never freed while the registry is alive. The registry's lifetime is the grid's lifetime — when the grid (block, alt-screen, etc.) is dropped or replaced, the entire registry goes with it.

| Cap | Default | Enforced where | Behavior on hit |
| --- | --- | --- | --- |
| Max URI byte length | 4096 | `parse_osc_params` (§1) — checked before allocating the `String`, so a 1 GB OSC 8 sequence never produces a 1 GB allocation | parser returns `Err(UriTooLong { len })`; dispatcher passes the OSC to `unhandled(params)`; `set_hyperlink` is **not** called. The visible text continues to render (invariant 15). |
| Max distinct entries per registry | 4096 | `HyperlinkRegistry::intern` | returns `None`; `set_hyperlink` lands `active_hyperlink_id = None`; the still-active OSC 8 span renders as plain non-clickable cells. Existing entries stay valid; old links remain clickable. A `log::warn!` fires once per cap hit per registry. |
| Max referencing cells per entry | unbounded | n/a | bounded indirectly by the grid's row cap. |

**Why no reclamation.** A reclamation model would need consistent reference-count accounting across cell overwrites (replace `hyperlink_id`, dec/inc), row eviction from scrollback, RLE run splits and merges in `FlatStorage`, reflow on resize (rows rebuilt from underlying spans), and deserialization (incoming cells must bump counts on the loaded registry). Getting any one of those wrong leaks entries (memory creep) or under-counts (use-after-free of an `id` that still appears in some cell). For a feature where the steady-state working set per block is small (single-digit URLs in real-world output) and the cap (4096 entries × ~1 KB average ≈ 4 MB) is already small, the simpler "registry grows monotonically until grid is dropped" model is the right tradeoff — and trivially correct under all of the above transitions because the only mutation is "append at intern time."

**Block-grid lifetime.** Per product invariant 10, a `BlockList` block is the natural unit of registry ownership: hyperlinks are reset on prompt-marker boundaries, and when a block is fully evicted from scrollback the whole block (including its registry) is dropped. So the working set is bounded by the *current* block's distinct URLs, not the session's. `AltScreen` registries are cleared on screen reset.

**Caps are `pub const`** in the registry module so tests can override them via `#[cfg(test)] const MAX_DISTINCT_ENTRIES: usize = 4;` in a test-only build.

**`Clone` does not share id space.** `HyperlinkRegistry` derives `Clone`, and `GridHandler` clones its registry when the grid is cloned (e.g. via `GridHandler::clone` for snapshotting). The clone is a deep copy: the cloned registry resolves the same ids to the same URIs, but is then independent — if the original or clone interns a *new* URI, both registries hand out the next id from their own sequence and those ids do NOT mean the same thing across instances. Cross-instance cell copies are therefore unsafe and not performed anywhere in the current implementation. If a future change ever does need to splice cells from one grid into another, the cell's `hyperlink_id` must be re-interned through the destination grid's registry; copying the `HyperlinkId` raw will resolve to the wrong URI (or `None`) on the other side.

Tests (in `hyperlink_registry_tests.rs` and `parse_osc_params` parser tests):
- **Parser** (`parse_osc_params`): a URI exceeding `MAX_URI_BYTES` returns `Err(UriTooLong { len })` with `len` matching the input. **The parser does not allocate the URI `String` before checking length** — verified by a test that supplies `&[u8]` of length `MAX_URI_BYTES + 1` and asserts the function returns `Err` synchronously without holding a `String` of that size on the heap. Non-UTF-8 bytes return `Err(InvalidUtf8)` (separately tested).
- **Registry intern**: `intern` returns `Some(id)` up to `MAX_DISTINCT_ENTRIES` distinct interns; the first call past the cap returns `None`, all earlier ids stay valid. The cap-hit `log::warn!` fires exactly once.
- **`set_hyperlink` glue**: when `intern` returns `None`, `active_hyperlink_id` is `None` and subsequent `input(c)` writes plain non-clickable cells.
- **No-reclaim under churn**: overwrite all cells in a row that referenced a hyperlink; assert `registry.len_for_test()` does not decrease. Documented as the intended behavior, not a bug.
- **1 MB OSC 8 doesn't OOM**: feed a 1 MB OSC 8 sequence; assert no allocation of comparable size occurs (memory-tracked test allocator) and the visible text falls through to the next valid sequence.
- **Drop = free**: dropping a `BlockList` block drops its registry (asserted via a `Weak`/Drop counter in tests).
- **3d behavior table**: one test per row of the §3d table — write text under an active link, apply the operation, assert `hyperlink_id` matches the table.

### 4. Lookup helpers — model layer

Mirror the auto-detected URL flow.

The existing `Link` (`grid_handler.rs:143`) is a single `RangeInclusive<Point>` and represents one contiguous run. Per product invariant 5 (narrowed in this revision), an OSC 8 span is also a single contiguous run — the cells written between one `set_hyperlink(Some(_))` call and its matching `set_hyperlink(None)`/implicit close. We therefore reuse `Link` as-is for the v1 lookup. Cross-run grouping by `id` (a non-contiguous "link group" of the same `id`) is **not** supported in v1 because `Link` cannot represent multiple disjoint ranges; see Follow-ups for the multi-range path.

- `GridHandler::hyperlink_at_point(&self, point: Point) -> Option<Link>` (`grid_handler.rs`, alongside the existing `url_at_point` at line 596). Returns the contiguous `Link` for the OSC 8 span at `point`, expanded by walking left/right while the cell's `HyperlinkId` matches and the cells remain adjacent.
- `GridHandler::hyperlink_uri_at_point(&self, point: Point) -> Option<&str>` to read the URI without recovering it from cell text.
- Block / alt-screen / model-level wrappers paralleling `blocks.rs:2460`, `alt_screen.rs:285`, `terminal_model.rs:1844` (`hyperlink_at_point`, `hyperlink_uri_at_range`).

### 5. View layer — hover, click, copy, context menu

`app/src/terminal/view/link_detection.rs`:

- Extend `GridHighlightedLink` with a third variant: `Hyperlink(WithinModel<Link>, String /* uri */)`. The variant carries the URI directly because, unlike `Url`, it is not recoverable from the cell text.
- In the hover state machine (lines 299-339), call `model.hyperlink_at_point(position)` first; if it returns `Some`, set `GridHighlightedLink::Hyperlink(...)`. Only fall through to `url_at_point` (and the file-path scanner) when no OSC 8 span is found at that point — this implements product invariant 9 (OSC 8 wins over auto-detected URL on the same cell).
- In the click handler (lines 391-395), add the `Hyperlink` arm: open the URI via `ctx.open_url` (the same path the auto-detected `Url` arm uses). Same telemetry path (`TelemetryEvent::OpenLink`). URIs are not gated by a scheme allow-list (product invariant 16); `file://` works and is opened like any other scheme.
- Tooltip text: `GridHighlightedLink::tooltip_text` (line 63) returns "Open link" for the new variant.

`app/src/terminal/view.rs`:

- The `OpenGridLink(link)` action (line 24786) gets a new arm for `Hyperlink` that calls `ctx.open_url`.
- The right-click context menu wiring around line 15040 gets a `GridHighlightedLink::Hyperlink` branch with "Open link" / "Copy link" items. "Open link" dispatches `OpenGridLink`. "Copy link" copies the URI to the clipboard verbatim.

### 6. Feature flag

`FeatureFlag::OscHyperlinks` (added per `WARP.md`'s feature-flag guide, defaulted on for dogfood) gates **OSC 8 specific** behavior only: when off, `osc_dispatch`'s `b"8"` arm calls `unhandled(params)` and the rest of layers (3)–(5)/(6a)/(7) never sees an OSC 8 hyperlink.

The flag's compile-time wiring is restricted to the `b"8"` arm and the layer (5) `Hyperlink` variant of `GridHighlightedLink`. This lets the team revert OSC 8 in one place if a regression appears.

### 6a. Session persistence (Warp Drive, history, shared sessions)

The three persistence formats have different shapes. Rather than one strategy, here's a per-format compatibility table — both directions (old client reading new payload, and new client reading old payload) — with a concrete migration plan for each.

**Common type changes (apply to all formats).**
- `HyperlinkId` is `#[derive(Serialize, Deserialize)]` over a `NonZeroU32`.
- `CellExtra` gains `#[serde(default, skip_serializing_if = "Option::is_none")] hyperlink_id: Option<HyperlinkId>`. `serde(default)` → new client reading old payload deserializes cleanly. `skip_serializing_if` → new client writing for cells that don't carry an id produces bytes byte-identical to today's, so old-client reads are unaffected for all non-hyperlinked output.

#### 6a-i. sqlite history (`crates/persistence`)

Block output is already stored as `stylized_output: String` (a Diesel `Binary`-backed `Text` column at `crates/persistence/src/schema.rs:79-99`) containing the **raw ANSI byte stream from the PTY**. OSC 8 bytes — those the original program emitted, including the URI — are already present in this string today; they're just dropped on the parser floor on every load. Once layer (2) ships, the same string passes through a parser that recognizes OSC 8 and the cells become clickable on every load.

- **No schema migration needed.** The Diesel migrations directory does not gain a new entry for this feature.
- **Old client → new payload (forward compat):** trivially fine — old clients have always written ANSI byte streams that may include OSC 8 sequences from the program.
- **New client → old payload (backward compat):** also fine — new clients read the same ANSI string, OSC 8 sequences re-parse, hyperlinks are restored. Blocks written before the feature shipped come back clickable too, for free.
- The `HyperlinkRegistry` itself is **not** persisted in sqlite — it's reconstructed from the byte stream on load. Saves us a column and a migration, and matches how every other terminal-state attribute (color, bold, etc.) is handled today.

#### 6a-ii. Warp Drive (cloud objects, shared sessions)

Warp Drive blocks store the same `SerializedBlock` shape (`app/src/terminal/model/block.rs:474-480`), with `stylized_output: String`. Same reasoning as sqlite: OSC 8 bytes round-trip transparently in the string and re-parse on load.

- **Forward and backward compat both ride for free** for the byte-stream encoding. No protocol-version bump is needed for blocks whose hyperlink state can be recovered from `stylized_output`.
- **Risk: a future representation change.** If Warp Drive ever moves from the byte-stream encoding to a structured per-cell encoding (e.g. for performance), the cell-side `hyperlink_id` field becomes the wire field of record. The `serde(default, skip_serializing_if)` defaults above already cover both directions. We name this in the spec so anyone making that representation change tests both compatibility directions.

#### 6a-iii. Session-sharing protocol (`session-sharing-protocol` workspace crate)

> **Follow-up — out of scope for this PR.** This PR does not modify the
> `session-sharing-protocol` crate or snapshot/replay handling, so OSC 8
> hyperlinks are **not** preserved over live shared sessions yet. The plan
> below is the design for that follow-up. (Local history and Warp Drive
> persistence in §6a-i / §6a-ii need no protocol change — the OSC 8 bytes
> round-trip through the stored output and re-parse on load.)

The session-sharing protocol streams **events**, not byte streams (set cursor, write char, set SGR, etc.). Adding OSC 8 means a new event:

```rust
// session-sharing-protocol — new event variant
SetHyperlink { hyperlink: Option<Hyperlink> },
```

The protocol's serialization (whichever framing — protobuf, MessagePack, JSON-over-WS) must handle unknown variants on the consuming side. Concrete plan:

- **Old client reading new payload:** the `SetHyperlink` event is unknown. Whatever framing the protocol uses, the receiver must skip-and-continue rather than error. We audit and (if needed) fix this path as part of the layer's PR.
  - For protobuf: unknown fields are skipped by default — no fix needed.
  - For serde-tagged JSON / MessagePack: add `#[serde(other)]` or a `Unknown` variant with `#[serde(skip_serializing)]` on a new event-envelope enum so deserializers tolerate forward-compatible additions.
- **New client reading old payload:** old payload simply doesn't contain `SetHyperlink` events. Cells render plain — graceful degradation, exactly what we want. No code path needed.
- **Active-state replay.** The session-sharing replay must apply `SetHyperlink` events in stream order; a late-joiner gets a `state_snapshot` followed by the live tail, and the snapshot needs to include the active hyperlink id. We add `active_hyperlink: Option<Hyperlink>` to the snapshot frame (same `#[serde(default)]` posture).
- **Versioning fallback.** If the protocol turns out to error on unknown events instead of skipping, we bump the protocol version and gate `SetHyperlink` behind a "OSC 8 supported" client capability negotiated at session start. Older clients negotiate down; newer clients fall back to not emitting the event when paired with an older peer.

#### Tests

- **sqlite round-trip** (`app/src/persistence/block_list.rs` test module): persist a block with `stylized_output` containing an OSC 8 sequence; reload from sqlite; assert the reloaded `Block` has cells with non-None `hyperlink_id` and the URI matches.
- **Warp Drive round-trip:** simulate the cloud serialization round-trip and verify the same.
- **Old-client read-of-new-payload (session-sharing):** a CI matrix test that pins the old client to a tag-before-this-PR, runs the new client as the producer, and asserts the old client renders cells correctly (just not clickable). Lives in `crates/integration` and runs as part of the protocol-compat CI job.
- **New-client read-of-old-payload (session-sharing):** the inverse — new client consumes a recorded session from before the PR; asserts the deserializer doesn't error and cells render plain.

The sqlite and Warp Drive round-trips (§6a-i / §6a-ii) work in this PR because they ride the existing byte stream. The session-sharing event/snapshot work above is deferred to a follow-up.

### 7. AI context

> **Follow-up — out of scope for this PR.** The block→agent context formatter is not changed here, so OSC 8 URIs are not yet surfaced to the AI/agent as untrusted metadata. The requirement below (product invariant 14) defines the intended behavior for that follow-up.

- **AI context** (invariant 14). The block→agent context formatter inlines `visible text (URI)` for hyperlinked spans so an agent reading wizcli output sees the URI without losing the visible label.

Dedicated "copy as markdown" and "copy as terminal bytes" export actions are out of scope for this iteration (product invariant 13).

## End-to-end flow

```mermaid
sequenceDiagram
    participant PTY
    participant Processor as ansi::Performer
    participant Handler as GridHandler/BlockGrid/AltScreen
    participant Registry as HyperlinkRegistry
    participant Storage as FlatStorage / Cell+Row
    participant View as TerminalView
    participant Browser

    PTY->>Processor: ESC ] 8 ; id=foo ; https://x ESC \ "Click me" ESC ] 8 ; ; ESC \
    Processor->>Handler: set_hyperlink(Some(Hyperlink{id:"foo", uri:"https://x"}))
    Handler->>Registry: intern(hyperlink) -> HyperlinkId
    Handler->>Handler: active_hyperlink_id = Some(id)
    loop Each printable char "C","l","i","c","k"," ","m","e"
        Processor->>Handler: input(c)
        Handler->>Storage: write cell with hyperlink_id = active_hyperlink_id
    end
    Processor->>Handler: set_hyperlink(None)
    Handler->>Handler: active_hyperlink_id = None

    Note over View: User hovers a cell
    View->>Storage: hyperlink_at_point(p)
    Storage-->>View: Some(Link { range })
    View->>Registry: get(id) -> Hyperlink
    View->>View: cursor=PointingHand, tooltip="Open link" + uri

    Note over View: User Cmd-clicks
    View->>Browser: ctx.open_url("https://x")
```

## Testing and validation

Each numbered item below maps to a product invariant from `product.md`.

**Unit tests — parser** (`crates/warp_terminal/src/model/ansi/control_sequence_parameters.rs`, `#[cfg(test)] mod hyperlink_parse_tests`).
- Open / open-with-id / close / single-empty-field-close / unknown-keys-ignored / malformed-param / multi-`:` param separators / **`uri_with_semicolons_is_rejoined`** (mandatory; see §1) / non-UTF-8 URI → invariants 1, 2, 3, 15.

**Unit tests — dispatch** (`app/src/terminal/model/ansi/mod_test.rs`, alongside the existing `parse_osc9_*` tests).
- Open + close fires two `set_hyperlink` events on `MockHandler` (the mock gets a `hyperlink_events: Vec<Option<Hyperlink>>` field) → invariant 1.
- Bell-terminated and ESC-`\`-terminated forms both work → invariant 1.
- Malformed `OSC 8 ; foo` (no URI segment) does not panic, does not fire an event → invariant 15.
- Empty-URI close after open clears → invariant 2.

**Unit tests — registry + cell stamping** (new `hyperlink_registry_tests.rs`, plus extension of `cell_test.rs`).
- `intern` deduplicates: same `Hyperlink{id:"foo", uri:"x"}` returns the same `HyperlinkId`. (Used internally so adjacent runs with the same `(id, uri)` share a slot — *not* a behavior the lookup layer exposes; see Follow-ups for cross-run grouping.)
- Different URIs (regardless of `id`) produce different `HyperlinkId`s — `(id, uri)` is the registry key.
- `Cell::set_hyperlink_id`/`hyperlink_id` round-trip; `CellExtra` allocation only occurs when first set; cell reset clears the slot.

**Unit tests — `FlatStorage`** (`flat_storage/mod_tests.rs`).
- Writing 100 cells under one active id RLE-collapses to one `AttributeMap` entry.
- Removing a row that ended a span doesn't bleed the active id into later writes.
- **Migration round-trip** (anti-regression for §3b-bis): take a `Row` of `Cell`s with `hyperlink_id` set, push it into `FlatStorage` via the existing flush path, pull it back into a `Row` via the reverse path, assert each cell's `hyperlink_id` matches what went in. This is the single test that catches the most likely shipping bug ("clickability disappears when a block scrolls").

**Unit tests — model lookups.** `hyperlink_at_point` returns the contiguous run of cells around `point` that share the same `HyperlinkId`, expanding left and right while the next cell is adjacent and carries the same id → invariants 5, 10. The lookup explicitly does **not** jump across non-adjacent cells even when `HyperlinkId` matches; cross-run grouping is out of scope.

**Integration tests** (`crates/integration/`, following the patterns in the `warp-integration-test` skill).
- **`osc8_open_close.rs`** — pipe an OSC 8 open + visible text + close to a fake PTY, assert the cells carry a hyperlink, hover one, observe `PointingHand` cursor and tooltip showing the URI → invariants 1, 5, 17.
- **`osc8_implicit_close_at_block_boundary.rs`** — open a hyperlink before a `precmd` / new prompt, assert the next block's cells do not carry the hyperlink → invariant 10.
- **`osc8_copy_text_vs_link.rs`** — select across a hyperlink and copy: clipboard contains visible text (no OSC 8 escape bytes) → invariant 8.
- **`osc8_no_regression_on_url_autodetect.rs`** — output without OSC 8 still hyperlinks via auto-detection → invariant 18.

Cmd+click → `open_url` coverage previously relied on a per-cell position cache stamped during grid rendering; that cache was removed (too risky for the render hot path), so synthetic-click integration tests are not included. Click behavior needs a different test mechanism (follow-up).

**Manual verification (recorded in PR description with a short clip).**
- Run `printf '\e]8;;https://warp.dev\e\\Open Warp\e]8;;\e\\\n'` in a Warp block; hover, observe pointer; Cmd+click, observe browser open.
- Run `wizcli` (or any CLI that emits OSC 8 — `gcc`, `make`, modern `git`) and exercise the live link.
- Run `cat` on a file containing a hyperlink across a wrapped line to confirm reflow on resize keeps the click intact.
- Run a TUI in alt-screen mode that emits OSC 8 (e.g. `lazygit`) to confirm parity with block-list behavior.

## Risks and mitigations

- **Memory / DoS from the registry.** Designed in, not deferred — see "Bounded registry, no reclamation" in (3). The cap (4096 distinct entries × 4096-byte max URI ≈ 16 MB worst-case per block) plus the bounded URI byte length plus the registry's grid-scoped lifetime put a hard ceiling on the working set. Adopting no-reclaim avoids a class of refcount/use-after-free bugs across cell overwrite, RLE split/merge, scrollback eviction, reflow, and deserialization.
- **Cell-size budget.** `cell.rs:122` is explicit that growing `Cell` past 24 bytes is a 33% memory hit. The `HyperlinkId` lives in `CellExtra` exactly to avoid this; the only `Cell`-shaped change is to `CellExtra`'s box, which is already optional and pays only when present.
- **URIs containing `;` are addressed in the parser contract,** not as a deferred mitigation. See §1: the URI field is the `b";"`-rejoin of all `params[1..]` slice elements, and a `uri_with_semicolons_is_rejoined` unit test is required.
- **Existing handler implementors not overriding `set_hyperlink`.** Default no-op means OSC 8 is silently dropped on those surfaces (e.g. `EarlyOutputHandler`). Acceptable: those surfaces don't render clickable output today either. They can be wired later without a behavior change for users.
- **Persistence compatibility, three formats, three different stories.** sqlite history and Warp Drive round-trip OSC 8 transparently because the format is already the raw ANSI byte stream — no schema or protocol change. Session-sharing is event-streamed and gains a new `SetHyperlink` event; old clients must skip unknown events (audited per-framing in §6a-iii) and a CI matrix locks in both directions of the compat. If skip-unknown turns out to be unavailable, the fallback is a client-capability negotiation that downgrades when paired with a pre-PR client.

## Parallelization

- Layer (1) — parser + types — is a single warp_terminal-crate change with no app dependencies.
- Layer (2) — Handler hook + dispatch — depends on (1) only.
- Layer (3a) and (3b) can run in parallel after (1). (3c) depends on (2) and the chosen (3a/3b) for the surface it covers.
- Layer (4) depends on (3).
- Layer (5) depends on (4).
- Layer (6a) — session persistence — depends on (3) (the on-cell field shape) and on the session-sharing protocol crate. Independent of (4)/(5).
- Layer (7) (AI context) depends on (3) but is otherwise independent of (4)/(5)/(6a).

The natural agent split is: one on (1)+(2)+parser tests; one on (3a); one on (3b); then one each on (4), (5), (6a), (7).

## Follow-ups

- **Cross-run id grouping.** Treating two non-contiguous OSC 8 emissions with the same `id` as one logical link span. Requires a multi-range link type (something like `HyperlinkSpan { id: HyperlinkId, ranges: Vec<RangeInclusive<Point>> }`) and a generalization of `GridHighlightedLink` and the hover/click code to operate on a set of ranges. Deferred because (a) the common case is a single contiguous span, (b) Warp's existing `Link` and the highlighted-link UI assume one contiguous range, and (c) the user-facing benefit is small relative to the breadth of code that would change.
- **Underline-on-hover styling** for OSC 8 spans. Today the spec defers to existing SGR styling. Once the hover state is wired, an additional `Flags::HYPERLINK_HOVER` flag plus a small render change is a clean follow-on.
- **Copy as markdown / terminal bytes.** Dedicated export actions — "copy as markdown" (`[visible](uri)`) and "copy as terminal bytes" (re-emitting normalized OSC 8 wrappers) — are out of scope for this iteration (product invariant 13).
- **Outgoing OSC 8.** Emitting hyperlinks from Warp's own UI when piping output through the terminal is out of scope for this issue.
