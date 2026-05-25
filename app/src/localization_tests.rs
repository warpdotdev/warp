use super::environment_locale_candidates_from;
use crate::settings::{init_and_register_user_preferences, AppLanguage, LanguageSettings};
use parking_lot::RwLock;
use settings::Setting as _;
use settings::SettingsManager;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use warp_localization::LocaleId;
use warpui::SingletonEntity as _;
use warpui::{
    elements::Empty, platform::WindowStyle, App, AppContext, Element, Entity, TypedActionView, View,
};

fn register_language_settings(ctx: &mut AppContext, language: AppLanguage) {
    init_and_register_user_preferences(ctx);
    ctx.add_singleton_model(|_| SettingsManager::default());
    let language_settings = LanguageSettings::register(ctx);
    language_settings.update(ctx, |settings, ctx| {
        settings
            .app_language
            .set_value(language, ctx)
            .expect("test app language should be configurable");
    });
}

struct RenderCounterView {
    counter: Arc<AtomicUsize>,
}

impl Entity for RenderCounterView {
    type Event = ();
}

impl View for RenderCounterView {
    fn ui_name() -> &'static str {
        "RenderCounterView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        self.counter.fetch_add(1, Ordering::Relaxed);
        Empty::new().finish()
    }
}

impl TypedActionView for RenderCounterView {
    type Action = ();
}

#[test]
fn environment_locale_candidates_prioritize_language_override() {
    let candidates = environment_locale_candidates_from(|key| match key {
        "LANGUAGE" => Some("zh_CN:fr_FR::en_US".to_owned()),
        "LC_ALL" => Some("de_DE.UTF-8".to_owned()),
        "LC_MESSAGES" => Some("it_IT.UTF-8".to_owned()),
        "LANG" => Some("en_GB.UTF-8".to_owned()),
        _ => None,
    });

    assert_eq!(
        candidates,
        vec![
            "zh_CN",
            "fr_FR",
            "en_US",
            "de_DE.UTF-8",
            "it_IT.UTF-8",
            "en_GB.UTF-8",
        ]
    );
}

#[test]
fn environment_locale_candidates_ignore_blank_values() {
    let candidates = environment_locale_candidates_from(|key| match key {
        "LANGUAGE" => Some("  :  ".to_owned()),
        "LC_ALL" => Some("   ".to_owned()),
        "LC_MESSAGES" => None,
        "LANG" => Some(" zh_CN.UTF-8 ".to_owned()),
        _ => None,
    });

    assert_eq!(candidates, vec!["zh_CN.UTF-8"]);
}

#[test]
fn replace_system_locale_candidates_reports_whether_cache_changed() {
    let cache = RwLock::new(vec!["en_US".to_owned()]);

    assert!(!super::replace_system_locale_candidates(
        &cache,
        vec!["en_US".to_owned()]
    ));
    assert!(super::replace_system_locale_candidates(
        &cache,
        vec!["zh_CN".to_owned(), "en_US".to_owned()]
    ));
    assert_eq!(*cache.read(), vec!["zh_CN".to_owned(), "en_US".to_owned()]);
}

#[test]
fn text_for_app_uses_configured_language() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_language_settings(ctx, AppLanguage::SimplifiedChinese);
        });

        app.read(|ctx| {
            assert_eq!(
                super::text_for_app(
                    ctx,
                    "settings.appearance.language.option.simplified_chinese"
                ),
                "简体中文"
            );
            assert_eq!(
                super::text_for_app(ctx, "settings.appearance.language.option.english"),
                "英语"
            );
        });
    });
}

#[test]
fn text_for_app_or_uses_fallback_only_when_key_is_missing() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_language_settings(ctx, AppLanguage::SimplifiedChinese);
        });

        app.read(|ctx| {
            assert_eq!(
                super::text_for_app_or(ctx, "settings.appearance.language.option.system", "System"),
                "跟随系统"
            );
            assert_eq!(
                super::text_for_app_or(ctx, "missing.test.key", "Fallback value"),
                "Fallback value"
            );
        });
    });
}

#[test]
fn language_setting_change_invalidates_existing_views() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_language_settings(ctx, AppLanguage::English);
            super::register_localization_updater(ctx);
        });

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| RenderCounterView {
            counter: render_counter.clone(),
        });

        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        LanguageSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .app_language
                .set_value(AppLanguage::SimplifiedChinese, ctx)
                .expect("test app language should be configurable");
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 2);
    });
}

#[test]
fn system_language_setting_change_invalidates_views_when_locale_cache_is_unchanged() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_language_settings(ctx, AppLanguage::English);
            super::register_localization_updater(ctx);
        });

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| RenderCounterView {
            counter: render_counter.clone(),
        });

        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        LanguageSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .app_language
                .set_value(AppLanguage::System, ctx)
                .expect("test app language should be configurable");
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 2);
    });
}

#[test]
fn bundled_catalogs_include_new_agent_and_onboarding_keys() {
    let keys = [
        "agent.warping.status.warping",
        "agent.warping.status.generating_plan",
        "agent.warping.status.searching_codebase",
        "agent.warping.status.executing_command",
        "agent.warping.waiting_for_instructions",
        "agent.warping.next_check_in",
        "agent.orchestration.config.header",
        "agent.orchestration.config.description",
        "agent.orchestration.config.view_details",
        "agent.orchestration.run_agents.title",
        "agent.orchestration.run_agents.spawned_partial",
        "agent.orchestration.controls.opencode_cloud_unsupported",
        "agent.block.debug.copy_debug_id",
        "agent.codebase_index_speedbump.title",
        "agent.codebase_index_speedbump.description",
        "agent.codebase_index_speedbump.index_codebase",
        "agent.codebase_index_speedbump.allow_automatic_indexing",
        "agent.codebase_index_speedbump.dont_show_again",
        "agent.codebase_index_speedbump.indexing_title",
        "agent.codebase_index_speedbump.view_status",
        "agent.block.recommended",
        "agent.input_footer.full_terminal_default_model",
        "agent.output.permissions.read_files",
        "agent.output.permissions.search_codebase",
        "agent.output.permissions.search_directory",
        "agent.output.permissions.write_to_running_command",
        "agent.requested_command.warning.always_ask_permission",
        "agent_sdk.api_key.confirm.expire",
        "agent_sdk.api_key.confirm.expire_cancelled",
        "agent_sdk.api_key.confirm.expire_help",
        "agent_sdk.api_key.error.create_failed",
        "agent_sdk.api_key.error.expire_failed",
        "agent_sdk.api_key.error.expire_non_interactive_requires_force",
        "agent_sdk.api_key.error.multiple_matches_specify_uid",
        "agent_sdk.api_key.error.not_found",
        "agent_sdk.api_key.output.created",
        "agent_sdk.api_key.output.expired",
        "agent_sdk.api_key.output.multiple_matches",
        "agent_sdk.api_key.output.not_expired",
        "agent_sdk.api_key.output.raw_api_key",
        "agent_sdk.api_key.output.secret_shown_once",
        "agent_sdk.api_key.output.uid",
        "agent_sdk.api_key.prompt.select_key_to_expire",
        "agent_sdk.secret.confirm.delete",
        "agent_sdk.secret.confirm.delete_cancelled",
        "agent_sdk.secret.confirm.delete_help",
        "agent_sdk.secret.error.bedrock_access_key_non_interactive_required",
        "agent_sdk.secret.error.bedrock_access_key_update_value",
        "agent_sdk.secret.error.bedrock_api_key_update_value",
        "agent_sdk.secret.error.bedrock_non_interactive_required",
        "agent_sdk.secret.error.delete_non_interactive_requires_force",
        "agent_sdk.secret.error.not_found",
        "agent_sdk.secret.error.read_value_file_failed",
        "agent_sdk.secret.output.created",
        "agent_sdk.secret.output.deleted",
        "agent_sdk.secret.output.updated",
        "agent_sdk.secret.prompt.aws_access_key_id",
        "agent_sdk.secret.prompt.aws_region",
        "agent_sdk.secret.prompt.aws_secret_access_key",
        "agent_sdk.secret.prompt.aws_session_token_optional",
        "agent_sdk.secret.prompt.bedrock_api_key",
        "agent_sdk.secret.prompt.openai_base_url",
        "agent_sdk.secret.prompt.openai_base_url_help",
        "agent_sdk.secret.prompt.secret_value",
        "agent_sdk.secret.scope.personal",
        "agent_sdk.secret.scope.team",
        "agent_management.agent_type_selector.title",
        "ai_document.message.updated_plan",
        "ai_document.tooltip.synced_to_warp_drive",
        "code.file_tree.project_explorer_unavailable",
        "code_review.comment.from_github",
        "code_review.comment.outdated",
        "conversation_details.field.artifacts",
        "conversation_details.creator.created_by",
        "conversation_details.field.conversation_id",
        "conversation_details.field.created_on",
        "conversation_details.field.credits_used",
        "conversation_details.field.directory",
        "conversation_details.field.environment_details",
        "conversation_details.field.environment_setup_commands",
        "conversation_details.field.error",
        "conversation_details.field.harness",
        "conversation_details.field.initial_query",
        "conversation_details.field.run_id",
        "conversation_details.field.run_time",
        "conversation_details.field.status",
        "editor.suggestions.cycle_suggestions",
        "search.ai_context_menu.code_symbols_indexing",
        "search.ai_context_menu.loading_results",
        "terminal.input.dynamic_enum.failure",
        "terminal.input.dynamic_enum.generate_message",
        "terminal.input.dynamic_enum.no_results",
        "terminal.input.dynamic_enum.pending",
        "terminal.input.dynamic_enum.run_command",
        "terminal.input.hint.tell_agent_what_to_build",
        "terminal.input.hint.kick_off_cloud_agent",
        "terminal.input.hint.ai_command_search",
        "terminal.input.hint.run_commands",
        "terminal.input.hint.steer_running_agent",
        "terminal.input.hint.steer_running_agent_classic",
        "terminal.input.hint.ask_follow_up",
        "terminal.input.hint.ask_follow_up_classic",
        "terminal.input.models.spec.billed_to_api",
        "terminal.input.models.spec.cost",
        "terminal.input.models.spec.intelligence",
        "terminal.input.models.spec.speed",
        "terminal.input.voice.listening",
        "terminal.input.voice.transcribing",
        "terminal.input.a11y.label",
        "terminal.input.a11y.helper",
        "terminal.block_onboarding.prompt.line_one",
        "terminal.block_onboarding.prompt.line_two",
        "terminal.block_onboarding.prompt.learn_more",
        "terminal.block_onboarding.prompt.no_existing_ps1",
        "terminal.block_onboarding.prompt.shell_prompt",
        "terminal.block_onboarding.prompt.look_incorrect",
        "terminal.block_onboarding.prompt.let_us_know",
        "terminal.block_onboarding.prompt.warp_prompt",
        "terminal.block_onboarding.prompt.customizable_settings",
        "terminal.block_onboarding.agentic.welcome",
        "terminal.inline_banner.agent_mode_setup.title",
        "terminal.inline_banner.agent_mode_setup.description",
        "terminal.inline_banner.agent_mode_setup.optimize",
        "terminal.ambient_agent.status.failed",
        "terminal.init_environment.mode_selector.title",
        "terminal.rewind.no_code_to_restore",
        "terminal.shared_session.snapshot.description",
        "terminal.shared_session.snapshot.title",
        "terminal.shared_session.metadata.skill",
        "terminal.skills.project_skill",
        "theme_chooser.title",
        "view_components.action_button.beta",
        "workflow.arg_selector.new",
    ];

    for locale in [LocaleId::EnUs, LocaleId::ZhCn] {
        for key in keys {
            assert_ne!(super::text_for_locale(locale, key), key);
        }
    }
}
