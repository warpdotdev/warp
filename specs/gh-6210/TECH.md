# Tech Spec: Kitty Graphics Protocol — Unicode Placeholders

## Sources

- **Issue:** https://github.com/warpdotdev/Warp/issues/6210
- **Product spec:** `specs/gh-6210/PRODUCT.md`
- **Protocol reference:** https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders
- **Figma:** no UI changes (rendering follows the protocol, not a design)

## Context

Warp already parses the `U` key into `KittyControlData::unicode_placeholder` (`app/src/terminal/model/kitty.rs:714`) and then rejects it with `InvalidControlData::UnicodePlaceholderUnsupported` at two sites in `TryFrom<KittyMessage> for KittyAction`: `kitty.rs:322` (`a=T`, `StoreAndDisplay`) and `kitty.rs:353` (`a=p`, `DisplayStoredImage`). Because clients usually send `q=2`, the rejection is silent and the `U+10EEEE` placeholder cells paint as tofu.

Facts about the current code that shape the plan:

- The reply path already works. `end_kitty_action_receiving` (`app/src/terminal/model/terminal_model.rs:3497`) writes success and error replies through `create_kitty_error_reply`, honoring `KittyResponseVerbosity`. Accepting the action makes replies correct with no extra work.
- Model-side kitty handling is gated on `FeatureFlag::KittyImages` (`app/src/terminal/model/grid/ansi_handler.rs:1775`, `ansi/mod.rs:1593`). The image render block is gated on `FeatureFlag::ITermImages` (`app/src/terminal/grid_renderer.rs:555`). **Decision:** all new placeholder work is gated on `KittyImages`; the existing render block keeps its flag.
- `Cell` (`crates/warp_terminal/src/model/grid/cell.rs:146`) is held at exactly 24 bytes; adding a field is a 33% grid-memory regression. But a placeholder cell already carries everything the decode needs: `c` (base char `U+10EEEE`), `fg` (image id), and `extra.cell_with_zero_width` (combining diacritics for row/column). **Decision:** decode at render time, no `Cell`/`CellExtra` change.
- `Cell` stores no underline color, and no SGR 58/59 parsing exists anywhere in the ansi handler. The kitty spec encodes the placement id in the underline color. **Decision:** ignore placement ids in this issue; a placeholder cell resolves to the most recent virtual placement of its image (spec-permitted for placement id 0). SGR 58 is a follow-up.
- Existing placements are anchored: `AbsoluteImagePlacement` (`app/src/terminal/model/image_map.rs:289`) ties an image to one `AbsolutePoint`, and `ImagePlacementData` holds `z_index`, `height_cells`, `image_size`. A virtual placement has no anchor, so it needs separate storage.
- `render_image` (`grid_renderer.rs:1826`) draws a whole image at one offset via `scene.draw_image` (`crates/warpui_core/src/scene.rs:589`), which takes no source rectangle. Cropping per cell-run works with the existing `ClipBounds::BoundedByActiveLayerAnd(RectF)` (`scene.rs:51`) and `start_layer`/`stop_layer`, already used at `grid_renderer.rs:588-616`.
- `get_image_ids_in_range` (`app/src/terminal/model/grid/image.rs:7`) returns early when `has_displayed_output()` is true. Placeholder rendering does not use this function — it scans visible cells — so that early return is irrelevant here and stays untouched.
- `KittyImages` is compiled out on Windows (`app/src/features.rs:128`); placeholder support inherits that automatically.

## Changes

The steps are ordered so that each layer degrades well on its own: steps 1–3 make placeholder cells paint blank instead of tofu, and step 4 turns the blanks into the image.

### Step 1 — Model: accept `U=1` (`app/src/terminal/model/kitty.rs`)

1. Add `virtual_placement: bool` to `KittyPlacementData` (default `false`).
2. In `TryFrom<KittyMessage> for KittyAction`, delete both `UnicodePlaceholderUnsupported` rejections (`kitty.rs:322`, `:353`) and set `virtual_placement: message.control_data.unicode_placeholder` when building the `StoreAndDisplay` and `DisplayStoredImage` actions.
3. Keep `InvalidControlData::UnicodePlaceholderUnsupported` removed from the error enum (or mark deprecated) so no path can re-emit it.

### Step 2 — Storage: virtual placement map (`app/src/terminal/model/image_map.rs`, `grid/ansi_handler.rs`)

1. Add to `ImageMap`:
   ```rust
   pub struct VirtualPlacement {
       pub cols: usize,
       pub rows: usize,
       pub image_size: Vector2F, // scaled pixel size of the full placement
   }
   virtual_placements: HashMap<u32 /* image_id */, VirtualPlacement>,
   ```
   Keyed by `image_id` only (placement-id decision above); insertion overwrites, giving most-recent-wins.
2. In `handle_completed_kitty_action_internal` (`ansi_handler.rs:1803`, `:1905`): when `placement_data.virtual_placement` is set, compute `cols`/`rows` — from `c`/`r` when present, otherwise derived from `metadata.image_size` and `cell_width`/`cell_height` the way the existing `get_desired_dimensions` + `ceil` code does — then insert into `virtual_placements` and return. Do not call `images.place()`, do not emit newlines, do not move the cursor (independent of the `C` key). For `StoreAndDisplay`, still send `Event::ImageReceived` first so the pixel data reaches the `ImageCache`.
3. Eviction: clear `virtual_placements` in `evict_all_images` and remove the entry in `evict_image` (`image_map.rs:254`, `:261`), so `a=d` deletes and image-quota eviction also kill virtual placements. Placeholder cells whose lookup fails render blank (step 4), which satisfies PRODUCT.md invariant 13.
4. Add an accessor `ImageMap::virtual_placement(image_id) -> Option<&VirtualPlacement>` plus a grid-level passthrough next to `get_image_placement_data`.

### Step 3 — Decode: placeholder cells at render time (new module `app/src/terminal/model/kitty_placeholder.rs`)

1. `pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';`
2. The kitty row/column diacritic table (297 codepoints, from kitty's `rowcolumn-diacritics.txt`), as a sorted `static [char; 297]` with a binary-search `fn diacritic_index(c: char) -> Option<u16>`. Mechanical; generate once and check in with a unit test pinning first/last entries.
3. `pub fn decode_placeholder(cell: &Cell) -> Option<DecodedPlaceholder>` returning:
   ```rust
   pub struct DecodedPlaceholder {
       pub image_id: u32,    // fg: Color::Spec rgb → low 24 bits; Color::Indexed(i) → i as u32;
                             // a 3rd diacritic supplies bits 24–31, folded in by the decoder
       pub row: Option<u16>, // 1st diacritic in cell_with_zero_width
       pub col: Option<u16>, // 2nd diacritic
   }
   ```
   `None` when `cell.c != PLACEHOLDER_CHAR` or the fg is not `Spec`/`Indexed`. Invalid diacritics decode as absent, letting continuation rules apply (blank if they can't).
4. Continuation inference (missing `row`/`col`) lives in `placeholder_runs_in_row(row: &[Cell]) -> Vec<PlaceholderRun>` in the same module: it scans one grid row, resolves omitted diacritics against the cell to the left (same `image_id` → next column, same row; only a row diacritic → column 0 of that row; nothing to continue from → row 0, column 0), and groups the cells into maximal runs (same image, same source row, consecutive source columns). Keeping this in the model module makes it unit-testable with synthetic rows.

### Step 4 — Render: draw fragments and suppress tofu (`app/src/terminal/grid_renderer.rs`)

Tofu suppression:

1. In `render_cell_glyph`, when `FeatureFlag::KittyImages.is_enabled()` and `cell.c == PLACEHOLDER_CHAR`, return before any glyph work — no shaped glyph, no `native_glyph_for_cell` fallback — so the cell paints only its background. This is also the degraded state for every failed lookup, matching ghostty. In `render_grid_with_ligatures`, the same condition appends a layout placeholder to the row's attributed string (like the `\t` case) instead of the cell content, so placeholder cells never shape as text.

Fragment drawing:

2. Both grid render paths (`render_grid_without_ligatures` and `render_grid_with_ligatures`) collect `(offset_row, PlaceholderRun)` pairs by calling `placeholder_runs_in_row` on each visible row inside their existing row loop, gated on `FeatureFlag::KittyImages`.
3. After the merged-background flush and decoration drawing at the end of each render function — so fragments paint above their cells' backgrounds — each run draws via `render_placeholder_run`, which looks up `grid.virtual_placement(image_id)` and the `StoredImageMetadata` (miss → skip; cells stay blank; out-of-range tiles clamp or skip). Then:
   - `full_size = cell_size * vec2f(cols as f32, rows as f32)` (logical px).
   - `run_rect = RectF::new(grid_origin + cell_size * vec2f(run_start_col, offset_row), cell_size * vec2f(visible_len, 1))`.
   - `image_origin = run_rect.origin() - cell_size * vec2f(src_col as f32, src_row as f32)`.
   - `ctx.scene.start_layer(ClipBounds::BoundedByActiveLayerAnd(run_rect))`, fetch the image the way `render_image` does (`AssetSource::Raw`, `FitType::Stretch`, bounds = `full_size * scale_factor`), `draw_image(RectF::new(image_origin, logical_image_size), …)`, `stop_layer()`.
   This reuses `draw_image` unchanged; no wgpu work.
4. Performance check before merging: a full-screen placeholder image produces one layer per visible row (worst case ~60–100 layers). Measure frame time against a full-screen anchored kitty image; if layer churn is measurable, coalesce runs that span full rows of the same image into one taller clip rect.

## Testing

- `kitty_tests.rs` unit tests (new test module for `kitty.rs`): `U=1` on `a=T` and `a=p` produces actions with `virtual_placement: true` instead of an error, and its absence produces non-virtual actions.
- `blockgrid_tests.rs` (with `FeatureFlag::KittyImages.override_enabled(true)`): a virtual placement — on both the `a=T` and `a=p` paths — leaves the cursor unmoved and adds no anchored placement; `c`/`r` omitted derives dimensions from the image and cell size.
- `image_map_tests.rs` (new test module for `ImageMap`): a second virtual placement for the same image replaces the first; `evict_image` and `evict_all_images` clear virtual placements.
- `kitty_placeholder.rs` unit tests: diacritic table pins (first entry `U+0305`, count 297), fg decoding for `Color::Spec` and `Color::Indexed`, third-diacritic high byte, invalid input → `None`.
- `kitty_placeholder_tests.rs` run-building tests feed synthetic cell rows to `placeholder_runs_in_row`: explicit consecutive tiles form one run; omitted diacritics continue the left neighbor; row-only diacritics start at column 0; different image ids, non-placeholder gaps, and duplicated tiles split runs.
- Manual acceptance (PRODUCT.md invariant 20): `kitten icat --unicode-placeholder <png>` shows the image; scroll it out and back; resize to force reflow; delete with `a=d` and confirm blanks, not tofu.

## Out of scope

- SGR 58/59 parsing and underline-color placement ids (follow-up; unblocks multiple virtual placements per image).
- Placeholder relay for tmux/SSH passthrough beyond ordinary cell text.
- Kitty animation (`a=f`, `a=a`), z-index compositing for virtual placements, and any change to anchored placements or iTerm images.
- Feature-flag promotion; `KittyImages` rollout state is unchanged by this work.
