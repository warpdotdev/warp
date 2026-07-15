use warp::tui_export::Appearance;
use warpui_core::App;

use super::TuiStatusState;
use crate::tui_builder::TuiUiBuilder;

#[test]
fn status_glyphs_match_the_shared_transcript_contract() {
    assert_eq!(TuiStatusState::Constructing.glyph(), "○");
    assert_eq!(TuiStatusState::Pending.glyph(), "○");
    assert_eq!(TuiStatusState::Blocked.glyph(), "■");
    assert_eq!(TuiStatusState::Running.glyph(), "●");
    assert_eq!(TuiStatusState::Succeeded.glyph(), "✓");
    assert_eq!(TuiStatusState::Failed.glyph(), "×");
    assert_eq!(TuiStatusState::Cancelled.glyph(), "■");
}

#[test]
fn status_styles_reuse_semantic_builder_styles() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            assert_eq!(
                TuiStatusState::Pending.glyph_style(&builder),
                builder.dim_text_style()
            );
            assert_eq!(
                TuiStatusState::Blocked.glyph_style(&builder),
                builder.attention_glyph_style()
            );
            assert_eq!(
                TuiStatusState::Running.glyph_style(&builder),
                builder.attention_glyph_style()
            );
            assert_eq!(
                TuiStatusState::Succeeded.glyph_style(&builder),
                builder.success_glyph_style()
            );
            assert_eq!(
                TuiStatusState::Failed.glyph_style(&builder),
                builder.error_text_style()
            );
            assert_eq!(
                TuiStatusState::Cancelled.glyph_style(&builder),
                builder.muted_text_style()
            );
        });
    });
}
