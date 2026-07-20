//! Deterministic color-and-glyph identity styling for orchestrated agents in
//! the TUI card: the design's theme-derived ANSI colors crossed with
//! a curated glyph set, plus the stable hash and per-request assignment
//! policy that keep identities stable across re-renders and edits.

use pathfinder_color::ColorU;
use warp_core::ui::theme::{Fill as ThemeFill, TerminalColors};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::elements::tui::TuiStyle;

/// Glyphs paired with themed colors to form deterministic agent identities.
const AGENT_IDENTITY_GLYPHS: [&str; 7] = ["⊹", "⟡", "✶", "◊", "⊛", "*", "✠"];

/// One deterministic color-and-glyph agent identity.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentIdentity {
    pub(crate) glyph: &'static str,
    pub(crate) style: TuiStyle,
}

impl Default for AgentIdentity {
    fn default() -> Self {
        Self {
            glyph: "⟡",
            style: TuiStyle::default(),
        }
    }
}

/// Builds the identity palette from the seven color roles in the design:
/// themed cyan, blue, magenta, lilac, pink, green, and yellow. Lilac uses
/// bright magenta while the remaining roles use their normal ANSI slots.
pub(crate) fn agent_identity_palette(colors: &TerminalColors) -> Vec<AgentIdentity> {
    let colors: [ColorU; 7] = [
        colors.normal.cyan.into(),
        colors.normal.blue.into(),
        colors.normal.magenta.into(),
        colors.bright.magenta.into(),
        colors.normal.red.into(),
        colors.normal.green.into(),
        colors.normal.yellow.into(),
    ];
    // Vary the color fastest so adjacent palette indices differ in color
    // before repeating a glyph.
    AGENT_IDENTITY_GLYPHS
        .iter()
        .flat_map(|glyph| {
            colors.iter().map(|color| AgentIdentity {
                glyph,
                style: TuiStyle::default().fg(CoreFill::from(ThemeFill::Solid(*color)).into()),
            })
        })
        .collect()
}

/// Stable FNV-1a hash of an agent name; must not vary across runs or
/// platforms so identities stay deterministic.
pub(crate) fn stable_hash(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Assigns a palette index to each agent name, starting from
/// `stable_hash(name) % len` and probing forward first-come. The palette is a
/// glyph × color grid, so the probe prefers a candidate whose glyph and color
/// are both unused, relaxing one dimension at a time as glyphs or colors run
/// out, and cycling deterministically by raw hash slot once every index is
/// taken.
pub(crate) fn assign_agent_identity_indices(
    names: impl IntoIterator<Item = impl AsRef<str>>,
    palette_len: usize,
) -> Vec<usize> {
    let mut assigned: Vec<usize> = Vec::new();
    if palette_len == 0 {
        return assigned;
    }
    // The palette lays glyph rows over color columns (color varies fastest);
    // degenerate palettes smaller than the glyph set collapse to one column.
    let color_count = (palette_len / AGENT_IDENTITY_GLYPHS.len()).max(1);
    let glyph_of = |index: usize| index / color_count;
    let color_of = |index: usize| index % color_count;
    let mut used_index = vec![false; palette_len];
    let mut used_glyph = vec![false; palette_len.div_ceil(color_count)];
    let mut used_color = vec![false; color_count];
    for name in names {
        let base =
            usize::try_from(stable_hash(name.as_ref()) % palette_len as u64).unwrap_or_default();
        let probe = |unused: &dyn Fn(usize) -> bool| {
            (0..palette_len)
                .map(|offset| (base + offset) % palette_len)
                .find(|candidate| unused(*candidate))
        };
        let index =
            probe(&|c| !used_index[c] && !used_glyph[glyph_of(c)] && !used_color[color_of(c)])
                .or_else(|| probe(&|c| !used_index[c] && !used_glyph[glyph_of(c)]))
                .or_else(|| probe(&|c| !used_index[c] && !used_color[color_of(c)]))
                .or_else(|| probe(&|c| !used_index[c]))
                .unwrap_or(base);
        used_index[index] = true;
        used_glyph[glyph_of(index)] = true;
        used_color[color_of(index)] = true;
        assigned.push(index);
    }
    assigned
}

#[cfg(test)]
#[path = "orchestrated_agent_identity_styling_tests.rs"]
mod tests;
