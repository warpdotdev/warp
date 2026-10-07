use std::collections::HashSet;

use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui_core::elements::Fill as CoreFill;
use warpui_core::elements::tui::Color;

use super::{
    AGENT_IDENTITY_GLYPHS, agent_identity_palette, assign_agent_identity_indices, stable_hash,
};

fn test_colors() -> [ColorU; 7] {
    [
        ColorU::from_u32(0x010101FF),
        ColorU::from_u32(0x020202FF),
        ColorU::from_u32(0x030303FF),
        ColorU::from_u32(0x040404FF),
        ColorU::from_u32(0x050505FF),
        ColorU::from_u32(0x060606FF),
        ColorU::from_u32(0x070707FF),
    ]
}

#[test]
fn palette_crosses_the_seven_design_glyphs_and_colors() {
    assert_eq!(AGENT_IDENTITY_GLYPHS, ["⊹", "⟡", "✶", "◊", "⊛", "*", "✠"]);
    assert_eq!(agent_identity_palette(&test_colors()).len(), 49);
}

#[test]
fn palette_uses_the_themed_design_color_roles_in_order() {
    let colors = test_colors();
    let expected: Vec<Option<Color>> = colors
        .into_iter()
        .map(|color| Some(CoreFill::from(ThemeFill::Solid(color)).into()))
        .collect();
    let palette = agent_identity_palette(&colors);

    assert_eq!(
        palette[..expected.len()]
            .iter()
            .map(|identity| identity.style.fg)
            .collect::<Vec<_>>(),
        expected,
    );
}

#[test]
fn palette_entries_are_distinct_glyph_color_pairs() {
    let palette = agent_identity_palette(&test_colors());
    let unique: HashSet<String> = palette
        .iter()
        .map(|identity| format!("{}-{:?}", identity.glyph, identity.style.fg))
        .collect();
    assert_eq!(unique.len(), palette.len());
}

#[test]
fn stable_hash_is_deterministic_and_name_sensitive() {
    assert_eq!(stable_hash("researcher"), stable_hash("researcher"));
    assert_ne!(stable_hash("researcher"), stable_hash("reviewer"));
}

#[test]
fn assignment_is_deterministic_across_calls() {
    let names = ["alpha", "beta", "gamma", "delta"];
    assert_eq!(
        assign_agent_identity_indices(names, 40),
        assign_agent_identity_indices(names, 40),
    );
}

#[test]
fn assignment_keeps_identities_distinct_within_one_request() {
    // Two names that collide on a length-4 palette still get distinct slots
    // via the first-come probe fallback.
    let palette_len = 4;
    let names: Vec<String> = (0..palette_len).map(|i| format!("agent-{i}")).collect();
    let indices = assign_agent_identity_indices(&names, palette_len);
    let unique: HashSet<usize> = indices.iter().copied().collect();
    assert_eq!(unique.len(), palette_len);
}

#[test]
fn assignment_keeps_glyphs_and_colors_unique_until_exhausted() {
    // 7 glyph rows × 7 color columns.
    let palette_len = 49;
    let color_count = 7;
    let names: Vec<String> = (0..7).map(|i| format!("agent-{i}")).collect();
    let indices = assign_agent_identity_indices(&names, palette_len);
    // All seven agents get distinct glyph rows and color columns.
    let glyphs: HashSet<usize> = indices.iter().map(|index| index / color_count).collect();
    assert_eq!(glyphs.len(), names.len());
    let colors: HashSet<usize> = indices.iter().map(|index| index % color_count).collect();
    assert_eq!(colors.len(), color_count);
}

#[test]
fn assignment_cycles_deterministically_beyond_palette_exhaustion() {
    let palette_len = 3;
    let names: Vec<String> = (0..palette_len + 2).map(|i| format!("agent-{i}")).collect();
    let indices = assign_agent_identity_indices(&names, palette_len);
    assert_eq!(indices.len(), palette_len + 2);
    // The first `palette_len` assignments cover every slot; overflow entries
    // reuse slots by raw hash without panicking or omitting agents.
    let first: HashSet<usize> = indices[..palette_len].iter().copied().collect();
    assert_eq!(first.len(), palette_len);
    for index in &indices[palette_len..] {
        assert!(*index < palette_len);
    }
}

#[test]
fn assignment_handles_an_empty_palette() {
    assert!(assign_agent_identity_indices(["alpha"], 0).is_empty());
}
