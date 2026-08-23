use warpui::App;

use super::*;
use crate::ai::llms::LLMInfo;
use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Regression test: this selector used to duplicate `model_leading_icon`'s fallback
/// logic instead of calling it, so server-provided Kimi models (provider `Unknown`)
/// always rendered the generic agent glyph here even though the inline `/model`
/// picker already showed the Kimi logo. `oz_model_icon` must resolve identically to
/// `model_leading_icon` for a Kimi model.
#[test]
fn kimi_model_gets_kimi_logo_not_generic_fallback() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(|ctx| {
            let llm = LLMInfo::new_for_test("kimi-k26-fireworks");
            assert_eq!(oz_model_icon(&llm, false, ctx), Icon::KimiLogo);
        });
    });
}

/// A custom-endpoint model whose config-key id happens to start with "kimi" must not
/// render Kimi's mark: `is_custom_endpoint: true` should suppress the id/name-based
/// heuristic so a user-controlled alias can't impersonate a third-party provider.
#[test]
fn custom_endpoint_named_kimi_keeps_generic_presentation() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(|ctx| {
            let llm = LLMInfo::new_for_test("kimi-proxy");
            assert_ne!(oz_model_icon(&llm, true, ctx), Icon::KimiLogo);
        });
    });
}

/// A non-Kimi model is unaffected and keeps falling back to the generic agent glyph
/// (its provider is `Unknown` in `LLMInfo::new_for_test`).
#[test]
fn non_kimi_model_keeps_generic_fallback() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(|ctx| {
            let llm = LLMInfo::new_for_test("gpt-test");
            assert_eq!(oz_model_icon(&llm, false, ctx), Icon::Agent);
        });
    });
}
