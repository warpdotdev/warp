//! Deterministic color-and-glyph agent identities for the TUI orchestration
//! card: a theme-derived palette of ANSI colors crossed with
//! a curated glyph set, plus the stable hash and per-request assignment
//! policy that keep identities stable across re-renders and edits.

use pathfinder_color::ColorU;
use warp_core::ui::theme::{Fill as ThemeFill, TerminalColors};
use warpui_core::elements::tui::TuiStyle;
use warpui_core::elements::Fill as CoreFill;

/// Glyphs paired with themed colors to form deterministic agent identities.
const AGENT_IDENTITY_GLYPHS: [&str; 8] = ["⟡", "⊹", "✶", "◊", "⊛", "*", "✠", "●"];

/// Minimum luma distance from the resolved background for a palette color
/// to count as readable.
const AGENT_IDENTITY_MIN_CONTRAST: f32 = 32.0;

/// The identity palette must offer at least this many combinations; use the
/// unfiltered ANSI set when contrast filtering would drop below it.
const AGENT_IDENTITY_MIN_COMBOS: usize = 32;

/// One deterministic color-and-glyph agent identity.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentIdentity {
    pub(crate) glyph: &'static str,
    pub(crate) style: TuiStyle,
}

/// Builds the identity palette from the themed ANSI colors (normal + bright,
/// excluding low-contrast slots against `background`) crossed with the glyph
/// set, yielding at least [`AGENT_IDENTITY_MIN_COMBOS`] combinations. All
/// colors derive from the theme; no raw design hex.
pub(crate) fn agent_identity_palette(
    colors: &TerminalColors,
    background: ColorU,
) -> Vec<AgentIdentity> {
    let all_colors: [ColorU; 16] = [
        colors.normal.black.into(),
        colors.normal.red.into(),
        colors.normal.green.into(),
        colors.normal.yellow.into(),
        colors.normal.blue.into(),
        colors.normal.magenta.into(),
        colors.normal.cyan.into(),
        colors.normal.white.into(),
        colors.bright.black.into(),
        colors.bright.red.into(),
        colors.bright.green.into(),
        colors.bright.yellow.into(),
        colors.bright.blue.into(),
        colors.bright.magenta.into(),
        colors.bright.cyan.into(),
        colors.bright.white.into(),
    ];
    let background_luma = luma(background);
    let readable: Vec<ColorU> = all_colors
        .iter()
        .copied()
        .filter(|color| (luma(*color) - background_luma).abs() >= AGENT_IDENTITY_MIN_CONTRAST)
        .collect();
    // Guarantee the minimum combination count even for unusual themes
    // where filtering strips too many slots.
    let colors = if readable.len() * AGENT_IDENTITY_GLYPHS.len() >= AGENT_IDENTITY_MIN_COMBOS {
        readable
    } else {
        all_colors.to_vec()
    };
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

/// Rec. 709 luma of a solid color, for background-contrast filtering.
fn luma(color: ColorU) -> f32 {
    0.2126 * f32::from(color.r) + 0.7152 * f32::from(color.g) + 0.0722 * f32::from(color.b)
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

/// Assigns a palette index to each agent name: `stable_hash(name) % len`,
/// with a first-come linear-probe fallback so identities within one request
/// stay distinct, cycling deterministically once the palette is exhausted.
pub(crate) fn assign_agent_identity_indices(
    names: impl IntoIterator<Item = impl AsRef<str>>,
    palette_len: usize,
) -> Vec<usize> {
    let mut assigned: Vec<usize> = Vec::new();
    if palette_len == 0 {
        return assigned;
    }
    let mut used = vec![false; palette_len];
    let mut used_count = 0;
    for name in names {
        let base =
            usize::try_from(stable_hash(name.as_ref()) % palette_len as u64).unwrap_or_default();
        let index = if used_count >= palette_len {
            // Palette exhausted: cycle deterministically by raw hash slot.
            base
        } else {
            (0..palette_len)
                .map(|offset| (base + offset) % palette_len)
                .find(|candidate| !used[*candidate])
                .unwrap_or(base)
        };
        if !used[index] {
            used[index] = true;
            used_count += 1;
        }
        assigned.push(index);
    }
    assigned
}

#[cfg(test)]
#[path = "agent_identity_tests.rs"]
mod tests;
