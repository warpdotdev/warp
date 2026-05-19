use std::collections::HashSet;

use super::*;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::blocklist::{AIQueryHistory, BlocklistAIPermissions};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::gallery::MCPGalleryManager;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::ai::outline::RepoOutlines;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::ai::AIRequestUsageModel;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::changelog_model::ChangelogModel;
use crate::cloud_object::model::persistence::CloudModel;
use crate::pricing::PricingInfoModel;
use crate::search::files::model::FileSearchModel;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::input::slash_command_model::SlashCommandEntryState;
use crate::terminal::input::slash_commands::SlashCommandsEvent;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use repo_metadata::RepoMetadataModel;
use watcher::HomeDirectoryWatcher;

use crate::editor::{EditorAction, TextStyleOperation};
use crate::input_suggestions::{HistoryOrder, Item};
use crate::network::NetworkStatus;
use crate::server::cloud_objects::{listener::Listener, update_manager::UpdateManager};
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;

use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::import::model::ImportedConfigModel;
use crate::settings::{AliasExpansionSettings, AppEditorSettings, InputBoxType, PrivacySettings};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
#[cfg(windows)]
use crate::system::SystemInfo;
use crate::system::SystemStats;
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::event::{BlockMetadataReceivedEvent, BootstrappedEvent};
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::local_shell::LocalShellState;
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;

use crate::terminal::block_list_viewport::ScrollPosition;
use crate::terminal::local_tty::shell::ShellStarter;
use crate::terminal::model::ansi::{Handler, PrecmdValue};
use crate::terminal::model::block::SerializedBlock;
use crate::terminal::model::blocks::{insert_block, BlockListPoint};
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::index::Side;
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model_events::ModelEvent;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use chrono::Local;
use warpui::text::SelectionType;

use crate::terminal::shell::ShellType;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::{ActiveSession, OneTimeModalModel, ToastStack, WorkspaceRegistry};
use crate::{
    editor::{DisplayPoint, Point},
    terminal::TerminalView,
};
use crate::{experiments, AgentNotificationsModel};
use fuzzy_match::FuzzyMatchResult;
use session_sharing_protocol::common::Role;
use smol_str::SmolStr;
use warp_completer::completer::{
    EngineFileType, Match, MatchStrategy, MatchedSuggestion, Priority, Suggestion,
    SuggestionResults, SuggestionType,
};
use warp_completer::meta::Span;

use unindent::Unindent;

#[cfg(feature = "voice_input")]
use voice_input::VoiceInputToggledFrom;
use warpui::platform::WindowStyle;
use warpui::{App, ReadModel, UpdateView, WindowId};

use crate::terminal::universal_developer_input::UniversalDeveloperInputButtonBarEvent;

use warp_util::user_input::UserInput;
use workflows::workflow::{Argument, ArgumentType, Workflow};

use crate::context_chips::prompt::Prompt;
use crate::terminal::general_settings::UserDefaultShellUnsupportedBannerState;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::view::inline_banner::ByoLlmAuthBannerSessionState;
use crate::terminal::writeable_pty::command_history::update_command_history;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider, ReferralThemeStatus};

pub fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    // Make sure we set up all necessary custom action bindings.
    app.update(init);

    // Initialize any global models required by the Input view.
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|ctx| ChangelogModel::new(ServerApiProvider::as_ref(ctx).get()));
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(|_| Prompt::mock());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(ImportedConfigModel::new);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(MCPGalleryManager::new);
    app.add_singleton_model(Listener::mock);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(|_| History::default());
    app.add_singleton_model(LocalWorkflows::new);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    // Pill bar model subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(|ctx| {
        crate::ai::blocklist::agent_view::orchestration_pill_bar_model::OrchestrationPillBarModel::new(
            Default::default(),
            ctx,
        )
    });
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(crate::remote_server::manager::RemoteServerManager::new);
    app.add_singleton_model(|_| crate::code_review::git_status_update::GitStatusUpdateModel::new());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(RepoOutlines::new_for_test);
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
    app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&crate::LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(|_| {
        crate::ai::document::ai_document_model::AIDocumentModel::new_for_test()
    });
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(SkillManager::new);

    // Add GlobalResourceHandlesProvider for persistence
    let tips_handle = app.add_model(|_| TipsCompleted::default());
    let referral_theme_status = app.add_model(ReferralThemeStatus::new);
    let user_default_shell_unsupported_banner_model_handle =
        app.add_model(|_| UserDefaultShellUnsupportedBannerState::default_value());
    app.add_singleton_model(move |_ctx| {
        GlobalResourceHandlesProvider::new(GlobalResourceHandles {
            model_event_sender: None, // No persistence in tests
            tips_completed: tips_handle,
            referral_theme_status,
            user_default_shell_unsupported_banner_model_handle,
            settings_file_error: None,
        })
    });

    #[cfg(windows)]
    {
        app.add_singleton_model(SystemInfo::new);
    }

    app.update(experiments::init);
    AltScreenReporting::register(app);
    app.add_singleton_model(|_| RestoredAgentConversations::new(vec![]));
    app.add_singleton_model(OneTimeModalModel::new);
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    app.add_singleton_model(|_| ToastStack);
    app.add_singleton_model(|_| PricingInfoModel::new());
    app.add_singleton_model(ByoLlmAuthBannerSessionState::new);
    app.add_singleton_model(|_| {
        crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier::new()
    });
    app.add_singleton_model(AgentConversationsModel::new);
    app.add_singleton_model(PersistedWorkspace::new_for_test);
    // `LocalShellState` captures the user's interactive login-shell PATH (used
    // for MCP/sbx executable resolution). Tests don't exercise that capture, so
    // register the singleton in its `NotLoaded` state to satisfy callers that
    // look it up via `LocalShellState::handle(ctx)`.
    app.add_singleton_model(|_| LocalShellState::NotLoaded);
}

fn bootstrap_terminal(
    terminal: &ViewHandle<TerminalView>,
    bootstrapped_event: BootstrappedEvent,
    app: &mut App,
) {
    let session_id = bootstrapped_event.session_info.session_id;
    terminal.update(app, |terminal, ctx| {
        terminal.model.lock().block_list_mut().set_bootstrapped();

        // Set session_id since precmd is not called in unit tests.
        terminal
            .model
            .lock()
            .block_list_mut()
            .active_block_for_test()
            .set_session_id(session_id);
        let model_event_dispatcher = terminal.model_event_dispatcher().clone();
        model_event_dispatcher.update(ctx, |dispatcher, _| {
            dispatcher.set_active_session_id(session_id);
        });

        terminal.sessions_model().update(ctx, |sessions, ctx| {
            let BootstrappedEvent {
                session_info,
                restored_block_commands,
                rcfiles_duration_seconds,
                spawning_command,
            } = bootstrapped_event;
            sessions.initialize_bootstrapped_session(
                *session_info,
                spawning_command,
                restored_block_commands,
                rcfiles_duration_seconds,
                ctx,
            );
        });
    });
}

fn enable_vim_mode(app: &mut App) {
    AppEditorSettings::handle(app).update(app, |editor_settings, ctx| {
        editor_settings
            .vim_mode
            .set_value(true, ctx)
            .expect("set value must succeed");
    });
}

pub async fn add_window_with_bootstrapped_terminal(
    app: &mut App,
    history_file_commands: Option<Vec<String>>,
    session_info: Option<SessionInfo>,
) -> ViewHandle<TerminalView> {
    add_window_with_bootstrapped_terminal_and_window_id(app, history_file_commands, session_info)
        .await
        .1
}

pub async fn add_window_with_bootstrapped_terminal_and_window_id(
    app: &mut App,
    history_file_commands: Option<Vec<String>>,
    session_info: Option<SessionInfo>,
) -> (WindowId, ViewHandle<TerminalView>) {
    let tips_model = app.add_model(|_| TipsCompleted::default());

    let shell_starter_source = ShellStarter::init(Default::default())
        .expect("Could not create a shell starter source or wsl name")
        .to_shell_starter_source()
        .await
        .expect("Could not create a shell starter source");
    let shell_type = shell_starter_source.shell_type();

    let session_info = session_info
        .unwrap_or_else(SessionInfo::new_for_test)
        .with_session_type(BootstrapSessionType::Local)
        .with_shell_type(shell_type);
    let history_file_commands = history_file_commands.unwrap_or_default();

    let (window_id, terminal) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        TerminalView::new_for_test(tips_model, None, ctx)
    });

    // TODO(vorporeal): There's a lot of fuckiness here.  `TerminalView::new_for_test`
    // calls `TerminalModel::new_for_test`, which fakes the InitShell and Bootstrapped
    // lifecycle events.  We then _also_ bootstrap the terminal here, which can and does
    // lead to inconsistent states.  We ought to only bootstrap the terminal once.
    let session_id = session_info.session_id;
    let bootstrapped_event = BootstrappedEvent {
        session_info: Box::new(session_info),
        restored_block_commands: history_file_commands
            .into_iter()
            .map(|command| HistoryEntry::command_at_time(command, Local::now(), None, true))
            .collect_vec(),
        rcfiles_duration_seconds: None,
        spawning_command: "test command".to_string(),
    };
    bootstrap_terminal(&terminal, bootstrapped_event, app);

    // Wait until history has been initialized for the session.
    let mut history_handle = History::handle(app);
    History::initialized_sessions(&mut history_handle, app, vec![session_id]).await;

    let input = terminal.read(app, |terminal, _| terminal.input().clone());
    // Notify the input that the session has bootstrapped
    input.update(app, |input, ctx| {
        input.set_active_block_metadata(BlockMetadata::new(Some(session_id), None), false, ctx);
    });
    (window_id, terminal)
}

/// Simulates being in a particular directory, for the purposes of completion
/// and syntax highlighting. The current directory is used to resolve
/// paths when parsing commands, and without it, completion/highlighting will
/// not run.
///
/// In particular, this sends precmd data and sets the active block's metadata.
pub fn simulate_directory_for_completion<A, S>(
    session_id: SessionId,
    terminal: &ViewHandle<TerminalView>,
    app: &mut A,
    directory: S,
) where
    A: UpdateView,
    S: Into<String>,
{
    let directory = directory.into();
    terminal.update(app, |terminal, ctx| {
        let block_metadata = BlockMetadata::new(Some(session_id), Some(directory.clone()));
        let block_index = {
            let mut model = terminal.model.lock();
            model.block_list_mut().precmd(PrecmdValue {
                pwd: Some(directory.clone()),
                session_id: Some(session_id.into()),
                ..Default::default()
            });
            model.block_list().active_block_index()
        };

        // Normally, the precmd message should be sufficient to also set this block metadata.
        // However, in unit tests the foreground executor does not relay the event, so notify
        // the dispatcher directly for models that observe active-session metadata.
        terminal
            .model_event_dispatcher()
            .update(ctx, |dispatcher, ctx| {
                dispatcher.set_active_session_id(session_id);
                ctx.emit(ModelEvent::BlockMetadataReceived(
                    BlockMetadataReceivedEvent {
                        block_metadata: block_metadata.clone(),
                        block_index,
                        is_after_in_band_command: false,
                        is_done_bootstrapping: true,
                    },
                ));
            });

        // Keep the input's block metadata in sync with the active-session metadata above.
        terminal.input().update(ctx, |input, ctx| {
            input.set_active_block_metadata(block_metadata, false, ctx);
        });
    });
}

fn argument_suggestion(name: impl Into<SmolStr>) -> MatchedSuggestion {
    let suggestion = Suggestion::with_same_display_and_replacement(
        name,
        None,
        SuggestionType::Argument,
        Priority::default(),
    );
    MatchedSuggestion::new(
        suggestion,
        Match::Prefix {
            is_case_sensitive: true,
        },
    )
}

/// Creates a [`MatchedSuggestion`] for a file completion result.
/// Specifically, we ensure the replacement is the entire path
/// while the display text is just the string after the last slash.
fn file_suggestion(path: impl Into<SmolStr>) -> MatchedSuggestion {
    let replacement = path.into();
    let display = replacement
        .rsplit(std::path::MAIN_SEPARATOR)
        .next()
        .map(Into::into)
        .unwrap_or_else(|| replacement.clone());

    let suggestion = Suggestion::new(
        display,
        replacement,
        None,
        SuggestionType::Argument,
        Priority::default(),
    )
    .with_file_type(EngineFileType::File);

    MatchedSuggestion::new(
        suggestion,
        Match::Prefix {
            is_case_sensitive: true,
        },
    )
}

fn case_insensitive_argument_suggestion(name: impl Into<SmolStr>) -> MatchedSuggestion {
    let suggestion = Suggestion::with_same_display_and_replacement(
        name,
        None,
        SuggestionType::Argument,
        Priority::default(),
    );
    MatchedSuggestion::new(
        suggestion,
        Match::Prefix {
            is_case_sensitive: false,
        },
    )
}

fn case_insensitive_exact_argument_suggestion(name: impl Into<SmolStr>) -> MatchedSuggestion {
    let suggestion = Suggestion::with_same_display_and_replacement(
        name,
        None,
        SuggestionType::Argument,
        Priority::default(),
    );
    MatchedSuggestion::new(
        suggestion,
        Match::Exact {
            is_case_sensitive: false,
        },
    )
}

fn fuzzy_argument_suggestion(
    name: impl Into<SmolStr>,
    matched_indices: Vec<usize>,
) -> MatchedSuggestion {
    let suggestion = Suggestion::with_same_display_and_replacement(
        name,
        None,
        SuggestionType::Argument,
        Priority::default(),
    );
    MatchedSuggestion::new(
        suggestion,
        Match::Fuzzy {
            match_result: FuzzyMatchResult {
                score: 1,
                matched_indices,
            },
        },
    )
}

fn editor_model_snapshot(input: &Input, ctx: &mut ViewContext<Input>) -> EditorSnapshot {
    input
        .editor()
        .read(ctx, |editor, ctx| editor.snapshot_model(ctx))
}

fn set_alias_expansion_setting(new_value: bool, app: &mut App) {
    AliasExpansionSettings::handle(app).update(app, |settings, ctx| {
        if let Err(e) = settings.alias_expansion_enabled.set_value(new_value, ctx) {
            panic!("Unable to set alias expansion setting in test, {e:?}");
        }
    });
}

/// Inserts block with dummy text and returns the block index.
fn insert_dummy_block(terminal: ViewHandle<TerminalView>, app: &mut App) -> BlockIndex {
    terminal.update(app, |terminal_view, _ctx| {
        let mut terminal_model = terminal_view.model.lock();
        let blocks = terminal_model.block_list_mut();
        // Add two lines to the command grid and output grid in a new block.
        insert_block(blocks, "cmd_a\ncmd_b\n", "output_a\noutput_b\n")
    })
}

/// Selects the first line in the command grid of given block.
fn select_first_command_line_of_block(
    block_index: BlockIndex,
    terminal: ViewHandle<TerminalView>,
    app: &mut App,
) {
    terminal.update(app, |terminal_view, _ctx| {
        let mut terminal_model = terminal_view.model.lock();
        let blocks = terminal_model.block_list_mut();
        let block = blocks.block_at(block_index).expect("block should exist");
        // Selections are inclusive of endpoint, hence we need to identify the last column to select the first command.
        let block_command_columns = block.prompt_and_command_grid().grid_handler().columns();
        let command_grid_offset = block.command_grid_offset();
        // Create a selection that just spans the first line of the command grid in the block.
        blocks.start_selection(
            BlockListPoint::new(command_grid_offset, 0),
            SelectionType::Simple,
            Side::Left,
        );
        blocks.update_selection(
            BlockListPoint::new(command_grid_offset, block_command_columns),
            Side::Right,
        );
        let selection = blocks.selection();
        assert!(selection.is_some());
    });
}

#[test]
fn test_input_tab() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // Note: we have similar boilerplate for many tests in this file - it would be nice to refactor this into a common helper!
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());
        // If there is no non-whitespace input, pass the tab to the editor
        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "    ");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "        ");
        });
        input.update(&mut app, |input, ctx| {
            input.input_shift_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "    ");
        });

        // Test that if there is a single cursor at the end, we do not pass tab to the editor.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("c", ctx);
            input.user_insert("d", ctx);
            input.user_insert(" ", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd ");
        });

        // Test that we don't pass the tab if the single cursor is in the middle either
        input.update(&mut app, |input, ctx| {
            input.user_insert("s", ctx);
            input.user_insert("o", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_left(/* stop at line start */ false, ctx);
            editor.move_left(/* stop at line start */ false, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd so");
        });

        // Test that if we select the entire buffer, we pass tab to the editor.
        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.select_all(ctx);
            })
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "    cd so");
        });
    });
}

#[test]
fn test_clear_selection_after_insert() {
    // When Agent Mode is inactive, we should clear the selection after inserting text into the
    // input box (both user-inserted and system-inserted text).
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let session_info = SessionInfo::new_for_test();
        let terminal: ViewHandle<TerminalView> = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let input = terminal.read(&app, |terminal, _ctx| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        let select_text = |app: &mut App| {
            let block_index = insert_dummy_block(terminal.clone(), app);
            select_first_command_line_of_block(block_index, terminal.clone(), app);
        };
        let user_insert = |app: &mut App, text: &str| {
            input.update(app, |input, ctx| {
                input.user_insert(text, ctx);
            });
        };
        let assert_selections_in_blocklist = |app: &mut App, expect_selections: bool| {
            terminal.read(app, |terminal_view, _ctx| {
                let terminal_model = terminal_view.model.lock();
                let blocks = terminal_model.block_list();
                let selection = blocks.selection();
                assert_eq!(selection.is_some(), expect_selections);
            });
        };

        // Shell Mode: Insert some text into the input box - this should clear the terminal selection!
        select_text(&mut app);
        user_insert(&mut app, "bar");
        assert_selections_in_blocklist(&mut app, false);

        // Shell Mode: System insert should also clear terminal selection.
        select_text(&mut app);
        user_insert(&mut app, "baz");
        assert_selections_in_blocklist(&mut app, false);

        // Activate Agent Mode, which should no longer allow text insertion to clear the selected text.
        terminal.update(&mut app, |terminal, ctx| {
            terminal.set_ai_input_mode_with_query(None, ctx)
        });

        // Agent Mode: Insert some text into the input box - this should no longer clear the terminal selection!
        select_text(&mut app);
        user_insert(&mut app, "bam");
        assert_selections_in_blocklist(&mut app, true);

        // Agent Mode: System insert should not clear terminal selection.
        select_text(&mut app);
        user_insert(&mut app, "bab");
        assert_selections_in_blocklist(&mut app, true);
    });
}

#[test]
fn test_merge_ai_and_command_history() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let now = Local::now();
        let current_session_id = SessionId::from(0);
        let other_session_id = SessionId::from(1);
        let all_live_session_ids = HashSet::from([current_session_id, other_session_id]);

        // Create entries in chronological order (from earliest to most recent)
        // Restored commands are now treated as CurrentSession
        let entry_30s = HistoryEntry::command_at_time(
            "echo 30 sec earlier [restored]".into(),
            now - Duration::from_secs(30),
            Some(current_session_id),
            true,
        );
        let entry_20s = HistoryEntry::command_at_time(
            "echo 20 sec earlier [different session]".into(),
            now - Duration::from_secs(20),
            None,
            false,
        );
        let entry_10s = HistoryEntry::command_at_time(
            "echo 10 sec earlier [current session]".into(),
            now - Duration::from_secs(10),
            Some(current_session_id),
            false,
        );
        let entry_5s = HistoryEntry::command_at_time(
            "echo 5 sec earlier [other session]".into(),
            now - Duration::from_secs(5),
            Some(other_session_id),
            false,
        );
        let entry_now =
            HistoryEntry::command_at_time("echo now [different session]".into(), now, None, false);

        let history_commands = vec![
            HistoryInputSuggestion::Command { entry: &entry_20s },
            HistoryInputSuggestion::Command { entry: &entry_now },
            HistoryInputSuggestion::Command { entry: &entry_30s },
            HistoryInputSuggestion::Command { entry: &entry_10s },
            HistoryInputSuggestion::Command { entry: &entry_5s },
        ];
        let only_history_commands = history_commands
            .clone()
            .into_iter()
            .sorted_by(|a, b| a.cmp(b, Some(current_session_id), &all_live_session_ids))
            .collect::<Vec<_>>();
        assert_eq!(only_history_commands.len(), 5);
        // DifferentSession items sorted by timestamp
        assert_eq!(
            only_history_commands[0].text(),
            "echo 20 sec earlier [different session]"
        );
        assert_eq!(
            only_history_commands[1].text(),
            "echo 5 sec earlier [other session]"
        );
        assert_eq!(
            only_history_commands[2].text(),
            "echo now [different session]"
        );
        // CurrentSession items sorted by timestamp (restored + current session)
        assert_eq!(
            only_history_commands[3].text(),
            "echo 30 sec earlier [restored]"
        );
        assert_eq!(
            only_history_commands[4].text(),
            "echo 10 sec earlier [current session]"
        );

        let ai_queries = vec![
            HistoryInputSuggestion::AIQuery {
                entry: AIQueryHistory::new_for_test(
                    "ai 35 sec earlier [different session]",
                    now - Duration::from_secs(35),
                    HistoryOrder::DifferentSession,
                ),
            },
            HistoryInputSuggestion::AIQuery {
                entry: AIQueryHistory::new_for_test(
                    "ai 25 sec earlier [different session]",
                    now - Duration::from_secs(25),
                    HistoryOrder::DifferentSession,
                ),
            },
            HistoryInputSuggestion::AIQuery {
                entry: AIQueryHistory::new_for_test(
                    "ai 15 sec earlier [current session]",
                    now - Duration::from_secs(15),
                    HistoryOrder::CurrentSession,
                ),
            },
            HistoryInputSuggestion::AIQuery {
                entry: AIQueryHistory::new_for_test(
                    "ai 7 sec earlier [current session]",
                    now - Duration::from_secs(7),
                    HistoryOrder::CurrentSession,
                ),
            },
        ];
        let only_ai_commands = ai_queries
            .clone()
            .into_iter()
            .sorted_by(|a, b| a.cmp(b, Some(current_session_id), &all_live_session_ids))
            .collect::<Vec<_>>();
        assert_eq!(only_ai_commands.len(), 4);
        // DifferentSession items sorted by timestamp
        assert_eq!(
            only_ai_commands[0].text(),
            "ai 35 sec earlier [different session]"
        );
        assert_eq!(
            only_ai_commands[1].text(),
            "ai 25 sec earlier [different session]"
        );
        // CurrentSession items sorted by timestamp
        assert_eq!(
            only_ai_commands[2].text(),
            "ai 15 sec earlier [current session]"
        );
        assert_eq!(
            only_ai_commands[3].text(),
            "ai 7 sec earlier [current session]"
        );

        let merged = history_commands
            .into_iter()
            .chain(ai_queries)
            .sorted_by(|a, b| a.cmp(b, Some(current_session_id), &all_live_session_ids))
            .collect::<Vec<_>>();
        assert_eq!(merged.len(), 9);
        // DifferentSession items sorted by timestamp
        assert_eq!(merged[0].text(), "ai 35 sec earlier [different session]");
        assert_eq!(merged[1].text(), "ai 25 sec earlier [different session]");
        assert_eq!(merged[2].text(), "echo 20 sec earlier [different session]");
        assert_eq!(merged[3].text(), "echo 5 sec earlier [other session]");
        assert_eq!(merged[4].text(), "echo now [different session]");
        // CurrentSession items sorted by timestamp
        assert_eq!(merged[5].text(), "echo 30 sec earlier [restored]");
        assert_eq!(merged[6].text(), "ai 15 sec earlier [current session]");
        assert_eq!(merged[7].text(), "echo 10 sec earlier [current session]");
        assert_eq!(merged[8].text(), "ai 7 sec earlier [current session]");
    });
}

#[test]
fn test_history_up() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd /".to_string(),
            "cd ~".to_string(),
            "git add .".to_string(),
            "ls cd".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let (input, editor, suggestions) = terminal.read(&app, |view, ctx| {
            let input = view.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            let input_suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());
            (input, editor, input_suggestions)
        });

        // Arrow up displays history in the correct order for an empty buffer
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 4);
            assert_eq!(suggestions.item_text(0).as_str(), "cd /");
            assert_eq!(suggestions.item_text(1).as_str(), "cd ~");
            assert_eq!(suggestions.item_text(2).as_str(), "git add .");
            assert_eq!(suggestions.item_text(3).as_str(), "ls cd");
        });

        // The buffer should contain the text of the last item
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "ls cd");
        });

        // The buffer contain the text of the second last item after another arrow-up
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git add .");
        });

        // Now put some text into the input and assert it has ctrl-r behavior on
        // arrow up
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("c", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "c");
        });
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        suggestions.read(&app, |suggestions, _ctx| {
            // Shouldn't contain the "ls cd"
            assert_eq!(suggestions.items().len(), 2);
            assert_eq!(suggestions.item_text(0).as_str(), "cd /");
            assert_eq!(suggestions.item_text(1).as_str(), "cd ~");
        });

        // The buffer should contain the text of the last item
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd ~");
        });

        // The buffer contain the text of the second last item after another arrow-up
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd /");
        });

        // Another editor-up is a no-op
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd /");
        });

        // Closing the history up has left the buffer unchanged
        input.update(&mut app, |input, ctx| {
            input.editor_escape(ctx);
        });
        input.read(&app, |input, ctx| {
            assert!(input.suggestions_mode_model.as_ref(ctx).is_closed());
            assert_eq!(input.buffer_text(ctx), "c");
        });
        editor.read(&app, |editor, ctx| {
            assert!(
                editor.single_cursor_on_first_row(ctx),
                "Should be single cursor on first row"
            );
        });

        // Test closing the history up menu again with the cursor in the
        // middle of the buffer.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("foo bar", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            for _ in 0..4 {
                editor.move_left(/* stop at line start */ false, ctx);
            }
        });
        editor.read(&app, |editor, ctx| {
            assert!(
                editor.single_cursor_on_first_row(ctx),
                "Should be single cursor on first row"
            );
            assert_eq!(
                editor.single_cursor_to_point(ctx).unwrap(),
                Point { row: 0, column: 3 },
            );
        });
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert!(
                input.suggestions_mode_model.as_ref(ctx).is_visible(),
                "Input suggestions should be visible",
            );
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert!(suggestions.items().is_empty());
        });
        input.update(&mut app, |input, ctx| {
            // This time use editor down to close the menu
            input.editor_down(ctx);
        });
        input.read(&app, |input, ctx| {
            assert!(
                !input.suggestions_mode_model.as_ref(ctx).is_visible(),
                "Input suggestions should be dismissed",
            );
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.single_cursor_to_point(ctx).unwrap(),
                Point { row: 0, column: 3 },
            );
        });
    });
}

#[test]
fn test_history_up_buffer_restoration() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd /".to_string(),
            "cd ~".to_string(),
            "git add .".to_string(),
            "ls cd".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let (input, suggestions) = terminal.read(&app, |view, _| {
            let input = view.input().clone();
            let input_suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());
            (input, input_suggestions)
        });

        // Arrow up displays history in the correct order for an empty buffer
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 4);
            assert_eq!(suggestions.item_text(0).as_str(), "cd /");
            assert_eq!(suggestions.item_text(1).as_str(), "cd ~");
            assert_eq!(suggestions.item_text(2).as_str(), "git add .");
            assert_eq!(suggestions.item_text(3).as_str(), "ls cd");
        });
        // The buffer should contain the text of the last item
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "ls cd");
        });

        // should_restore_buffer_before_history_up is true, so our buffer should go back to empty string.
        suggestions.update(&mut app, |suggestions, ctx| {
            suggestions.exit(true, ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "");
        });

        // History up again to the first history entry.
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "ls cd");
        });

        // should_restore_buffer_before_history_up is false, so our buffer should remain unchanged.
        suggestions.update(&mut app, |suggestions, ctx| {
            suggestions.exit(false, ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "ls cd");
        });
    });
}

#[test]
fn test_history_up_for_shared_session_executor() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Initialize as shared session executor
        // such that the history model isn't also initialized during bootstrapping
        // TODO(maggs): Improve testing utils for session sharing
        let tips_model = app.add_model(|_| TipsCompleted::default());
        let (_, terminal) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            TerminalView::new_for_test(tips_model, None, ctx)
        });
        terminal.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.block_list_mut().set_bootstrapped();
            model
                .block_list_mut()
                .active_block_for_test()
                .set_session_id(SessionId::from(0));
            model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                role: Role::Executor,
            });
        });

        let (input, suggestions) = terminal.read(&app, |view, _ctx| {
            let input = view.input().clone();
            let input_suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());
            (input, input_suggestions)
        });

        input.update(&mut app, |input, ctx| {
            // Initialize shared session history model
            let shared_session_history_model = ctx.add_model(|_| SharedSessionHistoryModel::new());

            // Simulate blocks
            shared_session_history_model.update(ctx, |history_model, _ctx| {
                history_model.push(HistoryEntry::for_completed_block(
                    "echo foo".into(),
                    &SerializedBlock::new_for_test("echo foo".as_bytes().to_vec(), vec![]),
                ));

                history_model.push(HistoryEntry::for_completed_block(
                    "cd ~".into(),
                    &SerializedBlock::new_for_test("cd ~".as_bytes().to_vec(), vec![]),
                ));
            });

            input.shared_session_input_state = Some(SharedSessionInputState {
                history_model: shared_session_history_model,
                pending_command_execution_request: None,
            });
            input.editor_up(ctx);
        });

        // Arrow up displays history in the correct order for an empty buffer
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 2);
            assert_eq!(suggestions.item_text(0).as_str(), "echo foo");
            assert_eq!(suggestions.item_text(1).as_str(), "cd ~");
        });

        // The buffer should contain the text of the last item
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd ~");
        });

        // Shared session executor should be able to navigate through history
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });

        // The buffer should contain the text of the second last item after another arrow-up
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "echo foo");
        });
    });
}

#[test]
fn test_history_up_multiline() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd ~\necho hello".to_string(),
            "git add .\n git rm .".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());

        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 2);
            assert_eq!(suggestions.item_text(1).as_str(), "git add .\n git rm .");
            assert_eq!(suggestions.item_text(0).as_str(), "cd ~\necho hello");
        });
        input.read(&app, |input, ctx| {
            assert_eq!("git add .\n git rm .", input.buffer_text(ctx));
        });
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!("cd ~\necho hello", input.buffer_text(ctx));
        });
        // Closing the history up menu restores the original buffer
        input.update(&mut app, |input, ctx| {
            input.editor_escape(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                &InputSuggestionsMode::Closed
            );
            assert!(input.buffer_text(ctx).is_empty());
        });
    });
}

#[test]
fn test_history_up_multiline_vim() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd ~\necho hello".to_string(),
            "git add .\n git rm .".to_string(),
        ];

        // Create a terminal window with Vim mode enbled.
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let input = &terminal.read(&app, |terminal, _| terminal.input().clone());
        let suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());
        let editor = input.read(&app, |input, _ctx| input.editor.clone());
        AppEditorSettings::handle(&app).update(&mut app, |settings, settings_ctx| {
            let _ = settings.vim_mode.set_value(true, settings_ctx);
        });

        // Switch into Vim Normal mode.
        editor.update(&mut app, |editor, ctx| {
            editor.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });

        let vim_up_action = EditorAction::VimUserInsert(UserInput::new("k"));
        let vim_down_action = EditorAction::VimUserInsert(UserInput::new("j"));

        // Trigger the history menu.
        input.update(&mut app, |input, ctx| {
            input.handle_action(&InputAction::Up, ctx);
        });

        // The first suggestion should be inserted into the input buffer.
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 2);
            assert_eq!(suggestions.item_text(1).as_str(), "git add .\n git rm .");
            assert_eq!(suggestions.item_text(0).as_str(), "cd ~\necho hello");
        });
        input.read(&app, |input, ctx| {
            assert_eq!("git add .\n git rm .", input.buffer_text(ctx));
        });

        // Move up within the input buffer.
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&vim_up_action, ctx);
        });

        // The contents of the buffer should not change
        // because the cursor moved up one line.
        input.read(&app, |input, ctx| {
            assert_eq!("git add .\n git rm .", input.buffer_text(ctx));
        });

        // Attempt to move up from the first line in the input buffer.
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&vim_up_action, ctx);
        });

        // Now that we've reached the first line,
        // the upward motion takes us to the next suggestion.
        input.read(&app, |input, ctx| {
            assert_eq!("cd ~\necho hello", input.buffer_text(ctx));
        });

        // Move down from the bottom line of the second suggestion.
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&vim_down_action, ctx);
        });

        // Since the cursor was on the bottom line,
        // We now go back on the last suggestion.
        input.read(&app, |input, ctx| {
            assert_eq!("git add .\n git rm .", input.buffer_text(ctx));
        });

        // Move down from the bottom line of the last suggestion.
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&vim_down_action, ctx);
        });

        // Now that we've reached the last line,
        // This closes the history up menu and restores the original buffer.
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                &InputSuggestionsMode::Closed
            );
            assert!(input.buffer_text(ctx).is_empty());
        });
    });
}

/// TODO(andy) This test depends on [`terminal::writeable_pty::command_history::update_command_history`]
/// It should be moved into its own test module there, as that is really what's being tested here,
/// i.e. that is where the check for ignorespace is actually happening. I left it here due to the
/// complexity of setting up that test. As that module depends on a TerminalModel with a valid
/// BlockList, it was easier to utilize the boilerplate local to this module. Long-term, some of
/// these helpers should move into shared test utils to make setup easier.
#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_histignorespace_support_in_zsh() {
    let session_id: SessionId = 1.into();
    let session_info = SessionInfo::new_for_test()
        .with_id(session_id)
        .with_shell_type(ShellType::Zsh)
        .with_shell_options(HashSet::from(["histignorespace".into()]));

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;

        // Ensure history is in a known (empty) state.
        History::handle(&app).read(&app, |history, _ctx| {
            assert!(history.commands(session_id).unwrap().is_empty());
        });

        // Run "cd" to populate the history buffer.
        let input = terminal.read(&app, |view, _| view.input().clone());
        input.update(&mut app, |input, ctx| {
            input.try_execute_command("cd", ctx);
        });

        // Run "ls" with a leading space, which should prevent history insertion.
        input.update(&mut app, |input, ctx| {
            input.try_execute_command(" ls", ctx);
        });

        let (model, sessions) = terminal.read(&app, |terminal, _| {
            (terminal.model.clone(), terminal.sessions_model().clone())
        });

        app.update(|ctx| {
            update_command_history(
                &ExecuteCommandEvent {
                    command: "cd".into(),
                    session_id,
                    workflow_id: None,
                    workflow_command: None,
                    should_add_command_to_history: true,
                    source: CommandExecutionSource::User,
                },
                &model,
                None,
                &sessions,
                ctx,
            );

            update_command_history(
                &ExecuteCommandEvent {
                    command: " ls".into(),
                    session_id,
                    workflow_id: None,
                    workflow_command: None,
                    should_add_command_to_history: true,
                    source: CommandExecutionSource::User,
                },
                &model,
                None,
                &sessions,
                ctx,
            );
        });

        // Verify only "cd" made it into history.
        History::handle(&app).read(&app, |history, _ctx| {
            assert_eq!(
                history
                    .commands(session_id)
                    .unwrap()
                    .into_iter()
                    .map(|entry| entry.command.as_str())
                    .collect_vec(),
                vec!["cd"]
            );
        });
    });
}

fn build_suggestion_results<S: Into<Span>>(
    suggestions: Vec<MatchedSuggestion>,
    replacement_span: S,
    matcher: MatchStrategy,
) -> Option<SuggestionResults> {
    Some(SuggestionResults {
        replacement_span: replacement_span.into(),
        suggestions,
        match_strategy: matcher,
    })
}

#[test]
fn test_tab_completion_with_multibyte_chars() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |view, _| view.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("➤", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "➤");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "➤");
        });
    });
}

#[test]
fn test_tab_completion_with_cursor_movement() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let session_info = SessionInfo::new_for_test();
        let session_id = session_info.session_id;
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        // Simulate being in the /usr/bin directory.
        simulate_directory_for_completion(session_id, &terminal, &mut app, "/usr/bin");
        let input = terminal.read(&app, |view, _| view.input().clone());

        // Start the editor with the text "yarn a" and press tab to ensure tab completions are
        // showing.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("yarn a", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "yarn a");
        });
        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("add"),
                        argument_suggestion("audit"),
                        argument_suggestion("autoclean"),
                    ],
                    (5, 5),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
            // Somehow `completion_session_context` is yielding None for pwd
        });
        input.read(&app, |input, ctx| {
            // Tab completion menu should be open.
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ))
        });

        input.read(&app, |input, _ctx| {
            input
                .input_suggestions
                .read(&app, |input_suggestions, _ctx| {
                    assert!(input_suggestions
                        .items()
                        .iter()
                        .map(|item| item.text())
                        .eq(["add", "audit", "autoclean",]))
                });
        });

        // Add a character and ensure items are filtered down.
        input.update(&mut app, |input, ctx| {
            input.user_insert("u", ctx);
        });

        input.read(&app, |input, ctx| {
            input
                .input_suggestions
                .read(&app, |input_suggestions, _ctx| {
                    assert!(input_suggestions
                        .items()
                        .iter()
                        .map(|item| item.text())
                        .eq(["audit", "autoclean",]))
                });

            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ))
        });

        // Move cursor to the left--all the results should now appear.
        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.move_left(/* stop at line start */ false, ctx);
            })
        });

        input.read(&app, |input, ctx| {
            input
                .input_suggestions
                .read(&app, |input_suggestions, _ctx| {
                    assert!(input_suggestions
                        .items()
                        .iter()
                        .map(|item| item.text())
                        .eq(["add", "audit", "autoclean",]))
                });

            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ))
        });

        // Move cursor to the left one more time, the input suggestions menu should be closed.
        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.move_left(/* stop at line start */ false, ctx);
            })
        });

        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            ))
        });
    });
}

#[test]
fn test_tab_completion_with_leading_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |view, _| view.input().clone());
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert(" cd asdf", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), " cd asdf");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), " cd asdf");
        });
    });
}

#[test]
fn test_tab_completion_with_spaces() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd Documents/zed".to_string(),
            "curl https://app.warp.dev".to_string(),
            "cargo check\ncargo run".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let (editor, suggestions) = input.read(&app, |input, _| {
            let editor = input.editor().clone();
            let input_suggestions = input.input_suggestions.clone();
            (editor, input_suggestions)
        });

        // Single result tab completion should update buffer.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd A\\ p", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ p");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![argument_suggestion("A\\ path\\ with\\ spaces")],
                    (3, 7),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ path\\ with\\ spaces ");
        });

        // Multiple result tab completion should show menu and highlight the matches.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd A\\ ", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ ");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("A\\ dir\\ with\\ spaces"),
                        argument_suggestion("A\\ desktop"),
                    ],
                    (3, 6),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        // We should be highlighting the prefix matches from the last word.
        suggestions.read(&app, |suggestions, _| {
            let highlights = suggestions
                .items()
                .iter()
                .map(|item| item.matches())
                .collect::<Vec<_>>();
            assert_eq!(
                highlights,
                [
                    Some(&(0..4).collect::<Vec<_>>()),
                    Some(&(0..4).collect::<Vec<_>>())
                ]
            );
        });

        suggestions.update(&mut app, |suggestions, ctx| {
            suggestions.select_next(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ d");
        });

        // Closing the input suggestions menu leaves input buffer unchanged,
        // regardless of whether additional characters were inserted/removed from the original completion buffer text.
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ d");
        });
        suggestions.update(&mut app, |suggestions, ctx| {
            suggestions.exit(true, ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
            assert_eq!(input.buffer_text(ctx), "cd A\\ d");
        });

        // Inserting a character prefix-searches previous results.
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("A\\ dir\\ with\\ spaces"),
                        argument_suggestion("A\\ desktop"),
                    ],
                    (3, 7),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
            input.user_insert("e", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ de");
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 1);
            assert_eq!(suggestions.item_text(0), "A\\ desktop");
            let highlight = suggestions.items()[0].matches();
            assert_eq!(highlight, Some(&(0..5).collect::<Vec<_>>()));
        });

        // Typing out an entire suggestion should highlight the entire suggestion.
        input.update(&mut app, |input, ctx| {
            input.user_insert("sktop", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ desktop");
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 1);
            assert_eq!(suggestions.item_text(0), "A\\ desktop");
            let highlight = suggestions.items()[0].matches();
            assert_eq!(highlight, Some(&(0..10).collect::<Vec<_>>()));
        });

        // Deleting a character that wasn't part of the original completion buffer updates suggestions.
        editor.update(&mut app, |editor, ctx| {
            for _ in 0.."esktop".len() {
                editor.backspace(ctx);
            }
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ d");
            assert_ne!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 2);
            assert_eq!(suggestions.item_text(1), "A\\ desktop");
            assert_eq!(suggestions.item_text(0), "A\\ dir\\ with\\ spaces");
            let highlights = suggestions
                .items()
                .iter()
                .map(|item| item.matches())
                .collect::<Vec<_>>();
            assert_eq!(
                highlights,
                [
                    Some(&(0..4).collect::<Vec<_>>()),
                    Some(&(0..4).collect::<Vec<_>>())
                ]
            );
        });

        // Deleting a character that was part of the original completion buffer closes the suggestions menu
        editor.update(&mut app, |editor, ctx| editor.backspace(ctx));
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd A\\ ");
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
        });

        // Bring up suggestions one more time
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("A\\ dir\\ with\\ spaces"),
                        argument_suggestion("A\\ desktop"),
                    ],
                    (3, 6),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        // Use tab to select next element, tab-shift to go to the previous & enter to confirm
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, _| {
            // after first tab
            input.input_suggestions.read(&app, |suggestions, _| {
                assert_eq!(suggestions.get_selected_item_text().unwrap(), "A\\ desktop");
            });
        });
        input.update(&mut app, |input, ctx| {
            input.input_shift_tab(ctx);
            input.input_enter(ctx);
        });
        input.read(&app, |input, ctx| {
            // shift-tab, enter
            assert_eq!(input.buffer_text(ctx), "cd A\\ dir\\ with\\ spaces ");
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
        });
    });
}

#[test]
fn test_tab_completion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd Documents/zed".to_string(),
            "curl https://app.warp.dev".to_string(),
            "cargo check\ncargo run".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let (editor, suggestions) = input.read(&app, |input, _| {
            let editor = input.editor().clone();
            let input_suggestions = input.input_suggestions.clone();
            (editor, input_suggestions)
        });

        // Single result tab completion should update buffer.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("c", ctx);
            input.user_insert("d", ctx);
            input.user_insert(" ", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd ");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![argument_suggestion("Documents")],
                    (3, 3),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Documents ");
        });

        // Multiple result tab completion should show menu and highlight the matches.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("c", ctx);
            input.user_insert("d", ctx);
            input.user_insert(" ", ctx);
            input.user_insert("D", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd D");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("Downloads"),
                        argument_suggestion("Desktop"),
                    ],
                    (3, 4),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        // We should be highlighting the prefix matches from the last word.
        suggestions.read(&app, |suggestions, _| {
            let highlights = suggestions
                .items()
                .iter()
                .map(|item| item.matches())
                .collect::<Vec<_>>();
            assert_eq!(
                highlights,
                [
                    Some(&(0..1).collect::<Vec<_>>()),
                    Some(&(0..1).collect::<Vec<_>>())
                ]
            );
        });

        suggestions.update(&mut app, |suggestions, ctx| {
            suggestions.select_next(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd D");
        });

        // Closing the input suggestions menu leaves input buffer unchanged,
        // regardless of whether additional characters were inserted/removed from the original completion buffer text.
        input.update(&mut app, |input, ctx| {
            input.user_insert("o", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Do");
        });
        suggestions.update(&mut app, |suggestions, ctx| {
            suggestions.exit(true, ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
            assert_eq!(input.buffer_text(ctx), "cd Do");
        });

        // Inserting a character prefix-searches previous results.
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("Downloads"),
                        argument_suggestion("Documents"),
                    ],
                    (3, 5),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
            input.user_insert("c", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Doc");
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 1);
            assert_eq!(suggestions.item_text(0), "Documents");
            let highlight = suggestions.items()[0].matches();
            assert_eq!(highlight, Some(&(0..3).collect::<Vec<_>>()));
        });

        // Typing out an entire suggestion should highlight the entire suggestion.
        input.update(&mut app, |input, ctx| {
            input.user_insert("uments", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Documents");
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 1);
            assert_eq!(suggestions.item_text(0), "Documents");
            let highlight = suggestions.items()[0].matches();
            assert_eq!(highlight, Some(&(0..9).collect::<Vec<_>>()));
        });

        // Deleting a character that wasn't part of the original completion buffer updates suggestions.
        editor.update(&mut app, |editor, ctx| {
            for _ in 0.."cuments".len() {
                editor.backspace(ctx);
            }
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Do");
            assert_ne!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
        });
        suggestions.read(&app, |suggestions, _ctx| {
            assert_eq!(suggestions.items().len(), 2);
            assert_eq!(suggestions.item_text(1), "Documents");
            assert_eq!(suggestions.item_text(0), "Downloads");
            let highlights = suggestions
                .items()
                .iter()
                .map(|item| item.matches())
                .collect::<Vec<_>>();
            assert_eq!(
                highlights,
                [
                    Some(&(0..2).collect::<Vec<_>>()),
                    Some(&(0..2).collect::<Vec<_>>())
                ]
            );
        });

        // Deleting a character that was part of the original completion buffer closes the suggestions menu
        editor.update(&mut app, |editor, ctx| editor.backspace(ctx));
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd D");
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
        });

        // Bring up suggestions one more time
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("Desktop"),
                        argument_suggestion("Downloads"),
                        argument_suggestion("Documents"),
                    ],
                    (3, 4),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        // Use tab to select next element, tab-shift to go to the previous & enter to confirm
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, _| {
            // after first tab
            input.input_suggestions.read(&app, |suggestions, _| {
                assert_eq!(suggestions.get_selected_item_text().unwrap(), "Downloads");
            });
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, _| {
            // second tab
            input.input_suggestions.read(&app, |suggestions, _| {
                assert_eq!(suggestions.get_selected_item_text().unwrap(), "Documents");
            });
        });
        input.update(&mut app, |input, ctx| {
            input.input_shift_tab(ctx);
            input.input_enter(ctx);
        });
        input.read(&app, |input, ctx| {
            // shift-tab, enter
            // Accepting a suggestion inserts a space at the end
            assert_eq!(input.buffer_text(ctx), "cd Downloads ");
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
        });
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_tab_completion_with_selection() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd Documents/zed".to_string(),
            "curl https://app.warp.dev".to_string(),
            "cargo check\ncargo run".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // The buffer should have the text "cd Desktop" with "Desktop" selected.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd ", ctx);
            input.editor().update(ctx, |editor, ctx| {
                editor.insert_selected_text("Desktop/", ctx);
            });
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Desktop/");
        });

        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![argument_suggestion("Documents/")],
                    (3, 4),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Documents/");

            // The cursor should be at the end of the autocompleted text.
            let selection_range = input.editor().read(&app, |editor, ctx| {
                editor.start_byte_index_of_last_selection(ctx)
                    ..editor.end_byte_index_of_last_selection(ctx)
            });
            assert_eq!(selection_range, ByteOffset::from(13)..ByteOffset::from(13));
        });

        // Add more text after the inserted text and then reselect "Documents/". The editor will
        // ultimately have the text "cd Documents/foo/bar" with "Documents/" selected.
        input.update(&mut app, |input, ctx| {
            input.user_insert("foo/bar", ctx);
            input.editor().update(ctx, |editor, ctx| {
                editor
                    .select_ranges_by_byte_offset([ByteOffset::from(4)..ByteOffset::from(13)], ctx);
            });
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Documents/foo/bar");
        });

        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![argument_suggestion("Desktop/")],
                    (3, 4),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Desktop/foo/bar");

            // The cursor should be at the end of the autocompleted text (right after "Desktop/").
            let selection_range = input.editor().read(&app, |editor, ctx| {
                editor.start_byte_index_of_last_selection(ctx)
                    ..editor.end_byte_index_of_last_selection(ctx)
            });
            assert_eq!(selection_range, ByteOffset::from(11)..ByteOffset::from(11));
        });
    });
}

#[test]
fn test_tab_completion_longest_common_prefix() {
    // We need to check that we fill longest common prefix in two cases
    // Case 1: When user triggers a tab completion
    // Case 2: When user types to filter the completion results and then triggers tab completion again
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());

        // Case 1: When user triggers a tab completion, fill buffer with longest common prefix
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open Cha", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("Charlie1.txt"),
                        argument_suggestion("Charlie2.txt"),
                        argument_suggestion("Charlie3.txt"),
                        argument_suggestion("Charlie111_1.txt"),
                        argument_suggestion("Charlie111_2.txt"),
                        argument_suggestion("Charlie111_3.txt"),
                    ],
                    (5, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open Charlie");
        });

        // Case 2: When user types to filter the completion results and then triggers tab completion again,
        // fill buffer with longest common prefix of the filtered results
        input.update(&mut app, |input, ctx| {
            input.user_insert("11", ctx);
        });
        suggestions.update(&mut app, |suggestions, _| {
            suggestions.set_items(vec![
                Item::from_text("Charlie111_1.txt".to_string()),
                Item::from_text("Charlie111_2.txt".to_string()),
                Item::from_text("Charlie111_3.txt".to_string()),
            ]);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open Charlie111_");
        });
    });
}

#[test]
fn test_tab_completion_longest_common_prefix_with_fuzzy_suggestions_and_completions_open() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open c", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("charlie.txt"),
                        argument_suggestion("charlotte.txt"),
                        fuzzy_argument_suggestion("bobcha.txt", (3..=4).collect()),
                    ],
                    (5, 6),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            // Tab completion menu should be open.
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ))
        });
        input.update(&mut app, |input, ctx| {
            // Trigger tab completion when the completion menu is open.
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            // The common prefix between the two prefix matches should be inserted.
            assert_eq!(input.buffer_text(ctx), "open charl");
        });
    });
}

#[test]
fn test_tab_completion_hides_autosuggestion() {
    let _test = FeatureFlag::RemoveAutosuggestionDuringTabCompletions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open-file ", ctx);
            input.set_autosuggestion(
                "sesame",
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![argument_suggestion("a.txt"), argument_suggestion("b.txt")],
                    (5, 5),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            // Tab completion menu should be open.
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ));

            // Autosuggestion should be closed.
            assert!(input
                .editor
                .as_ref(ctx)
                .current_autosuggestion_text()
                .is_none());
        });
    });
}

#[test]
fn test_completions_while_typing_doesnt_hide_autosuggestion() {
    let _test = FeatureFlag::RemoveAutosuggestionDuringTabCompletions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        InputSettings::handle(&app).update(&mut app, |input_settings, ctx| {
            let _ = input_settings
                .completions_open_while_typing
                .set_value(true, ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open-file ", ctx);
            input.set_autosuggestion(
                "sesame",
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            )
        });

        // Autosuggestion should be active.
        input.read(&app, |input, ctx| {
            assert!(input
                .editor
                .as_ref(ctx)
                .current_autosuggestion_text()
                .is_some());
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![argument_suggestion("a.txt"), argument_suggestion("b.txt")],
                    (5, 5),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            // Tab completion menu should be open.
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ));

            assert!(input
                .editor
                .as_ref(ctx)
                .current_autosuggestion_text()
                .is_some());
        });
    });
}

#[test]
fn test_agent_mode_set_while_typing_slash_command() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Start with natural language detection
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            assert!(!input.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });

        // Open slash commands menu by typing "/"
        input.update(&mut app, |input, ctx| {
            input.user_insert("/", ctx);
        });

        // Verify slash commands menu is open and agent mode is forced
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::SlashCommands
            ));
            // Should be in agent mode now
            assert!(input.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });

        // Add a command with a space
        input.update(&mut app, |input, ctx| {
            input.user_insert("plan ", ctx);
        });

        // Verify menu is closed and we're still in agent mode
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            ));
            assert!(input.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });
    });
}

#[test]
fn test_plan_slash_command_argument_with_slash_does_not_disable_slash_command_parsing() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            input.user_insert("/plan investigate app/src/main.rs", ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(
                !matches!(
                    input.slash_command_model.as_ref(ctx).state(),
                    SlashCommandEntryState::DisabledUntilEmptyBuffer
                ),
                "slash command parsing should not be disabled when the argument contains '/'"
            );
        });
    });
}

#[test]
fn test_open_slash_command_triggers_completions_on_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let session_id: SessionId = 1.into();
        let session_info = SessionInfo::new_for_test().with_id(session_id);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        simulate_directory_for_completion(session_id, &terminal, &mut app, "/tmp");

        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert("/", ctx);
            input.user_insert("open-file ", ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "/open-file ");
            assert!(!matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::SlashCommands
            ));
            assert!(input.completions_abort_handle.is_some());
        });
    });
}

#[test]
fn test_open_slash_command_triggers_completions_when_selected() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let session_id: SessionId = 1.into();
        let session_info = SessionInfo::new_for_test().with_id(session_id);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        simulate_directory_for_completion(session_id, &terminal, &mut app, "/tmp");

        input.update(&mut app, |input, ctx| {
            input.user_insert("/", ctx);
            input.handle_slash_commands_menu_event(
                &SlashCommandsEvent::SelectedStaticCommand {
                    id: COMMAND_REGISTRY
                        .get_command_id_with_name(commands::EDIT.name)
                        .copied()
                        .expect("open command should exist"),
                    cmd_or_ctrl_enter: false,
                },
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "/open-file ");
            assert!(input.completions_abort_handle.is_some());
        });
    });
}

#[test]
fn test_open_slash_command_requires_path() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("/open-file ", ctx)
            });
        });

        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });
    });
}

#[test]
fn test_changelog_slash_command_clears_buffer_on_success() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(commands::CHANGELOG.name, ctx)
            });
        });

        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "");
        });
    });
}
#[test]
fn test_open_slash_command_opens_files_palette_when_entered_from_slash_menu() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.user_insert("/", ctx);
            input.user_insert("open-file", ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "/open-file");
        });

        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_open_slash_command_clears_buffer_on_success() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_file.txt");
        std::fs::File::create(&file_path).unwrap();

        let session_id: SessionId = 1.into();
        let session_info = SessionInfo::new_for_test().with_id(session_id);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        simulate_directory_for_completion(
            session_id,
            &terminal,
            &mut app,
            temp_dir.to_string_lossy(),
        );

        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("/open-file test_file.txt", ctx)
            });
        });

        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
        });

        let _ = std::fs::remove_file(file_path);
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_open_slash_command_expands_tilde() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let home_dir = dirs::home_dir().expect("home directory must exist");
        let file_path = home_dir.join("warp_tilde_test_file.txt");
        std::fs::File::create(&file_path).unwrap();

        let session_id: SessionId = 1.into();
        let session_info = SessionInfo::new_for_test().with_id(session_id);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Simulate being in a directory that is NOT the home directory so we can
        // verify that ~ expansion takes priority over cwd joining.
        let temp_dir = std::env::temp_dir();
        simulate_directory_for_completion(
            session_id,
            &terminal,
            &mut app,
            temp_dir.to_string_lossy(),
        );

        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("/open-file ~/warp_tilde_test_file.txt", ctx)
            });
        });

        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });

        // Buffer should be cleared on success, indicating the file was found.
        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
        });

        let _ = std::fs::remove_file(file_path);
    });
}

#[test]
fn test_shell_lock_respected_when_slash_command_typed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Explicitly lock to shell mode
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_terminal(true, ctx);
        });

        // Verify locked in shell mode
        input.read(&app, |input, ctx| {
            let ai_model = input.ai_input_model.as_ref(ctx);
            assert!(ai_model.is_input_type_locked());
            assert!(!ai_model.is_ai_input_enabled());
        });

        // Type a slash command - should NOT force agent mode when locked
        input.update(&mut app, |input, ctx| {
            input.user_insert("/plan ", ctx);
        });

        // Should still be in shell mode because it was locked
        input.read(&app, |input, ctx| {
            let ai_model = input.ai_input_model.as_ref(ctx);
            assert!(!ai_model.is_ai_input_enabled());
            assert!(ai_model.is_input_type_locked());
        });
    });
}

#[test]
fn test_new_conversation_keybinding_requires_double_press_in_non_empty_agent_view() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.send_user_query_in_conversation(
                    "hello".to_owned(),
                    conversation_id,
                    None,
                    ctx,
                );
            });
        });

        let is_non_empty = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .is_some_and(|conversation| !conversation.is_empty())
        });
        assert!(is_non_empty);

        input.update(&mut app, |input, ctx| {
            input.user_insert("draft", ctx);
            input.handle_action(
                &InputAction::TriggerSlashCommandFromKeybinding(commands::AGENT.name),
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                Some(conversation_id),
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "draft");
        });

        input.update(&mut app, |input, ctx| {
            input.handle_action(
                &InputAction::TriggerSlashCommandFromKeybinding(commands::AGENT.name),
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active");
            assert_ne!(active_conversation_id, conversation_id);
        });
    });
}

#[test]
fn test_new_conversation_keybinding_does_not_require_confirmation_in_empty_agent_view() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        let is_empty = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .is_some_and(|conversation| conversation.is_empty())
        });
        assert!(is_empty);

        input.update(&mut app, |input, ctx| {
            input.user_insert("draft", ctx);
            input.handle_action(
                &InputAction::TriggerSlashCommandFromKeybinding(commands::AGENT.name),
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active");
            assert_ne!(active_conversation_id, conversation_id);
        });
    });
}

#[test]
fn test_new_conversation_input_trigger_remains_single_step_in_non_empty_agent_view() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.send_user_query_in_conversation(
                    "hello".to_owned(),
                    conversation_id,
                    None,
                    ctx,
                );
            });
        });

        let command = COMMAND_REGISTRY
            .get_command_with_name(commands::NEW.name)
            .expect("/new command should exist");
        input.update(&mut app, |input, ctx| {
            let handled = input.execute_slash_command(
                command,
                None,
                SlashCommandTrigger::input(),
                /*is_queued_prompt*/ false,
                ctx,
            );
            assert!(handled);
        });

        terminal.read(&app, |view, ctx| {
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active");
            assert_ne!(active_conversation_id, conversation_id);
        });
    });
}

#[test]
fn test_create_docker_sandbox_slash_command_executes_and_clears_buffer() {
    App::test((), |mut app| async move {
        let _docker_sandbox_flag = FeatureFlag::LocalDockerSandbox.override_enabled(true);
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.user_insert("draft text", ctx);
            let handled = input.execute_slash_command(
                &commands::CREATE_DOCKER_SANDBOX,
                None,
                SlashCommandTrigger::input(),
                /*is_queued_prompt*/ false,
                ctx,
            );
            assert!(handled);
        });

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
        });
    });
}

#[test]
fn test_agent_mode_set_when_block_attached() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Start with natural language detection
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            assert!(!input.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });

        // Attach a block
        input.update(&mut app, |input, ctx| {
            input.user_insert("<plan:398bf127-b3ca-47ab-b15c-f569dd982651>", ctx);
        });

        // Should be in agent mode now
        input.read(&app, |input, ctx| {
            assert!(input.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });

        // Add a prompt
        input.update(&mut app, |input, ctx| {
            input.user_insert(" implement this plan", ctx);
        });

        // Verify we're still in agent mode
        input.read(&app, |input, ctx| {
            assert!(input.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });
    });
}

#[test]
fn test_tab_completion_single_prefix_suggestion_with_fuzzy_suggestions() {
    // If there is a single prefix suggestion with other fuzzy suggestions,
    // we should insert that prefix suggestion directly into the buffer
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open cha", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("cha.txt"),
                        fuzzy_argument_suggestion("bobcha.txt", (3..=5).collect()),
                    ],
                    (5, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha.txt ");
        });
    });
}

#[test]
fn test_tab_completion_only_fuzzy_suggestions() {
    // If there are only fuzzy suggestions, we don't insert a prefix even if there is a common prefix
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open cha", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        fuzzy_argument_suggestion("bobcha1.txt", (3..=5).collect()),
                        fuzzy_argument_suggestion("bobcha2.txt", (3..=5).collect()),
                    ],
                    (5, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha");
        });
    });
}

#[test]
fn test_tab_completion_prioritizes_longest_common_prefix_with_fuzzy_suggestions() {
    // If there are multiple prefix suggestions with any number of fuzzy suggestions,
    // the common prefix is inserted.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open cha", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("charlie1.txt"),
                        argument_suggestion("charlie2.txt"),
                        fuzzy_argument_suggestion("bobcha1.pdf", (3..=5).collect()),
                        fuzzy_argument_suggestion("bobcha11.pdf", (3..=5).collect()),
                    ],
                    (5, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open charlie");
        });

        // We also just check that we don't insert the common prefix when typing
        // to filter if there isn't a common prefix or the replacement
        // does not start the common prefix.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open cha1", ctx);
        });
        suggestions.update(&mut app, |suggestions, _| {
            suggestions.set_items(vec![
                Item::from_text("charlie1.txt".to_string()),
                Item::from_text("bobcha1.pdf".to_string()),
                Item::from_text("bobcha11.pdf".to_string()),
            ]);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha1");
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert("p", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha1p");
        });
        suggestions.update(&mut app, |suggestions, _| {
            suggestions.set_items(vec![
                Item::from_text("bobcha1.pdf".to_string()),
                Item::from_text("bobcha11.pdf".to_string()),
            ]);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha1p");
        });
    });
}

#[test]
fn test_tab_completion_single_prefix_suggestion_after_fuzzy_suggestions() {
    // If there is a single prefix suggestion ordered after other fuzzy suggestions, we
    // insert that prefix suggestion directly into the buffer.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("git a", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        fuzzy_argument_suggestion("dab", vec![4]),
                        argument_suggestion("add"),
                    ],
                    (4, 5),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git add ");
        });
    });
}

#[test]
fn test_tab_completion_case_sensitive_single_suggestion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open ab", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("abc.txt"),
                        case_insensitive_argument_suggestion("Abcd.txt"),
                    ],
                    (5, 6),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            // There is only 1 case-sensitive prefix suggestion, so we insert it
            assert_eq!(input.buffer_text(ctx), "open abc.txt ");
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open ab", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        case_insensitive_argument_suggestion("Abc.txt"),
                        fuzzy_argument_suggestion("bobabc.txt", (3..=4).collect()),
                    ],
                    (5, 6),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            // There are no case-sensitive prefixes, but 1 case-insensitive prefix,
            // suggestion, so we insert it.
            assert_eq!(input.buffer_text(ctx), "open Abc.txt ");
        });
    });
}

#[test]
fn test_tab_completion_case_sensitivity_common_prefix() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open ab", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("abcdef.txt"),
                        argument_suggestion("abcdag.txt"),
                        case_insensitive_argument_suggestion("Abcd.txt"),
                    ],
                    (5, 6),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            // Insert the common prefix for the case-sensitive suggestions.
            assert_eq!(input.buffer_text(ctx), "open abcd");
        });
    });
}

#[test]
fn test_tab_completion_case_insensitive_exact_match() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("abc", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("abcdef"),
                        case_insensitive_exact_argument_suggestion("Abc"),
                    ],
                    (0, 3),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            // Single case-sensitive prefix suggestions are inserted even if there's
            // a case-insensitive exact match.
            assert_eq!(input.buffer_text(ctx), "abcdef ");
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("abc", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("abcdef"),
                        argument_suggestion("abcdeg"),
                        case_insensitive_exact_argument_suggestion("Abc"),
                    ],
                    (0, 3),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            // Case-sensitive common prefixes are inserted even if there's a
            // case-insensitive exact match.
            assert_eq!(input.buffer_text(ctx), "abcde");
        });
    });
}

#[test]
fn test_tab_completion_longest_common_prefix_with_fuzzy_suggestions() {
    // We want to test the following behaviour:
    // 1. If there is a single prefix suggestion with other fuzzy suggestions,
    //    we should insert that prefix suggestion directly into the buffer
    // 2. If there are only fuzzy suggestions, we don't insert a prefix even if there is a common prefix
    // 3. If there is a single prefix suggestion ordered after other fuzzy suggestions, we
    //     insert that prefix suggestion directly into the buffer.
    // We also check that this behaviour works when typing to filter.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let suggestions = input.read(&app, |input, _ctx| input.input_suggestions.clone());

        // Case 1. If there is a single prefix suggestion with other fuzzy suggestions, we should insert that prefix suggestion
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open cha", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("cha.txt"),
                        fuzzy_argument_suggestion("bobcha.txt", (3..=5).collect()),
                    ],
                    (5, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha.txt ");
        });

        // Case 2. If there are only fuzzy suggestions, we don't insert a prefix even if there is a common prefix
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("open cha", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        fuzzy_argument_suggestion("bobcha1.txt", (3..=5).collect()),
                        fuzzy_argument_suggestion("bobcha2.txt", (3..=5).collect()),
                    ],
                    (5, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha");
        });

        // We also just check that we don't insert the common prefix when typing
        // to filter if there isn't a common prefix or the replacement
        // does not start the common prefix.
        input.update(&mut app, |input, ctx| {
            input.user_insert("1", ctx);
        });
        suggestions.update(&mut app, |suggestions, _| {
            suggestions.set_items(vec![
                Item::from_text("charlie1.txt".to_string()),
                Item::from_text("bobcha1.pdf".to_string()),
                Item::from_text("bobcha11.pdf".to_string()),
            ]);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha1");
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert("p", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha1p");
        });
        suggestions.update(&mut app, |suggestions, _| {
            suggestions.set_items(vec![
                Item::from_text("bobcha1.pdf".to_string()),
                Item::from_text("bobcha11.pdf".to_string()),
            ]);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "open cha1p");
        });

        // Case 3: Ensure that the prefix suggestion is inserted, even if it's not the first
        // ordered suggestion.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("git a", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        fuzzy_argument_suggestion("dab", vec![4]),
                        argument_suggestion("add"),
                    ],
                    (4, 5),
                    MatchStrategy::Fuzzy,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            )
        });

        input.update(&mut app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git add ");
        });
    });
}

#[test]
fn test_tab_completion_common_prefix_shorter() {
    // We need to check the same two cases as the 'longest_common_prefix' test, however we want
    // to verify that if the longest common prefix is _shorter_ than what the user typed, we
    // don't insert it
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let suggestions = input.read(&app, |input, _| input.input_suggestions.clone());

        // Case 1: When a user triggers a tab completion, ensure longest common prefix is
        // longer than the text
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd foo/b", ctx);
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("foo/Bar"),
                        argument_suggestion("foo/bazz"),
                    ],
                    (3, 8),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd foo/b");
        });

        // Case 2: When user types to filter the completion results and then triggers tab
        // completion again, we still want to ensure the longest common prefix is longer
        // than the text
        input.update(&mut app, |input, ctx| {
            input.close_input_suggestions(/*should_focus_input=*/ true, ctx);
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd f", ctx);
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("far"),
                        argument_suggestion("foo/Bar"),
                        argument_suggestion("foo/bazz"),
                    ],
                    (3, 4),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
            input.user_insert("oo/b", ctx);
        });
        suggestions.update(&mut app, |suggestions, _| {
            suggestions.set_items(vec![
                Item::from_text("foo/Bar".into()),
                Item::from_text("foo/bazz".into()),
            ]);
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd foo/b");
        });
    });
}

#[test]
fn test_cursor_movement() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "cd Documents/zed".to_string(),
            "curl https://app.warp.dev".to_string(),
            "cargo check\ncargo run".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor.clone());
        // Test cursor movement
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("c", ctx);
            input.user_insert("d", ctx);
            input.user_insert(" ", ctx);
            input.user_insert("D", ctx);
        });

        // XXX Note that it's necessary to put `input_tab` in a separate call.
        // Otherwise, there's a race where we crash because editor:cursor is not set.
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd D");
        });

        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("Downloads"),
                        argument_suggestion("Documents"),
                    ],
                    (3, 4),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        let expected_completion = InputSuggestionsMode::CompletionSuggestions {
            replacement_start: 3,
            buffer_text_original: "cd D".to_string(),
            completion_results: SuggestionResults {
                suggestions: vec![
                    argument_suggestion("Downloads"),
                    argument_suggestion("Documents"),
                ],
                replacement_span: Span::new(3, 4),
                match_strategy: MatchStrategy::CaseInsensitive,
            },
            trigger: CompletionsTrigger::Keybinding,
            menu_position: TabCompletionsMenuPosition::AtLastCursor,
        };
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Do");
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                expected_completion
            );
        });
        // move back 1 character, and we're still showing the completion, except ignoring the
        // characters _after_ the cursor
        editor.update(&mut app, |editor, ctx| {
            editor.move_left(/* stop at line start */ false, ctx)
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd Do");
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                expected_completion
            );
        });
        editor.read(&app, |editor, ctx| {
            assert!(editor.is_single_cursor_only(ctx));
            let column = editor.start_byte_index_of_last_selection(ctx).as_usize();
            assert_eq!(column, 4);
        });

        // Put the cursor back at the end
        editor.update(&mut app, |editor, ctx| {
            editor.move_right(/* stop at line end */ false, ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert!(editor.is_single_cursor_only(ctx));
            let column = editor.start_byte_index_of_last_selection(ctx).as_usize();
            assert_eq!(column, 5);
        });
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_newline_insertion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        // Fill in the buffer with `ls \`
        editor.update(&mut app, |editor, ctx| {
            editor.user_insert(r"ls \", ctx);
        });

        // There should only be one line.
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.max_point(ctx).row(), 0);
        });

        // Move cursor to the end of the first line
        editor.update(&mut app, |input, ctx| {
            let line_0_end = DisplayPoint::new(0, input.line_len(0, ctx).unwrap());
            input
                .select_ranges(Some(line_0_end..line_0_end), ctx)
                .unwrap();
        });

        // Handle a return
        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });

        // We should have inserted a newline
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.max_point(ctx).row(), 1);
        });
    })
}

#[test]
fn test_should_not_insert_newline_on_enter_in_empty_buffer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
            assert!(!input.should_insert_newline_on_enter(ctx));
        });
    })
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_should_insert_newline_on_enter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = r"
            1 slash \
            2 slashes \\
            3 slashes \\\
            4 slashes \\\\
            no slashes
        "
        .unindent();

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content(base_text.as_str(), ctx);
            input.editor.update(ctx, |editor, ctx| {
                editor
                    .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                    .unwrap();
            })
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), base_text);
            assert!(input.editor.as_ref(ctx).single_cursor_on_first_line(ctx));
        });

        input.update(&mut app, |input, ctx| {
            // Move cursor to end of first line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_to_line_end(ctx);
            });
            assert!(input.should_insert_newline_on_enter(ctx));

            // Move cursor to end of second line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));

            // Move cursor to end of third line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(input.should_insert_newline_on_enter(ctx));

            // Move cursor to end of fourth line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));

            // Move cursor to end of fifth line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));
        });
    })
}

#[test]
fn test_powershell_should_insert_newline_on_enter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = r"
            1 slash \
            1 backtick with space `
            1 backtick no space f`
            no backtick
            2 backticks ``
            3 backticks ```
        "
        .unindent();

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content(base_text.as_str(), ctx);
            input.editor.update(ctx, |editor, ctx| {
                editor
                    .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                    .unwrap();
                editor.set_shell_family(ShellFamily::PowerShell);
            })
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), base_text);
            assert!(input.editor.as_ref(ctx).single_cursor_on_first_line(ctx));
        });

        input.update(&mut app, |input, ctx| {
            // Move cursor to end of first line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));

            // Move cursor to end of second line.
            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(input.should_insert_newline_on_enter(ctx));

            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));

            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));

            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));

            input.editor.update(ctx, |editor, ctx| {
                editor.move_down(ctx);
                editor.move_to_line_end(ctx);
            });
            assert!(!input.should_insert_newline_on_enter(ctx));
        });
    })
}

#[test]
fn test_workflow_selected() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.user_insert("hello", ctx);
        });

        let workflow = Workflow::new(
            "test",
            "{{p1}} {{parameter_2}} {{p3}} foo {{p1}} {{parameter_2}}",
        )
        .with_arguments(vec![
            Argument::new("p1", ArgumentType::Text),
            Argument::new("parameter_2", ArgumentType::Text),
            Argument::new("p3", ArgumentType::Text),
        ]);

        input.update(&mut app, |input, ctx| {
            input.show_workflows_info_box_on_workflow_selection(
                WorkflowType::Local(workflow),
                WorkflowSource::Global,
                WorkflowSelectionSource::Undefined,
                None,
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "p1 parameter_2 p3 foo p1 parameter_2"
            );
        });
    });
}

#[test]
fn test_workflow_selected_with_default_value() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let workflow = Workflow::new("test", "{{p1}}/{{parameter_2}}").with_arguments(vec![
            Argument {
                name: "p1".into(),
                description: None,
                default_value: Some("default_parameter_1".into()),
                arg_type: Default::default(),
            },
            Argument {
                name: "parameter_2".into(),
                description: None,
                default_value: Some("default_parameter_2".into()),
                arg_type: Default::default(),
            },
        ]);

        input.update(&mut app, |input, ctx| {
            input.show_workflows_info_box_on_workflow_selection(
                WorkflowType::Local(workflow),
                WorkflowSource::Global,
                WorkflowSelectionSource::Undefined,
                None,
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "default_parameter_1/default_parameter_2"
            );
        });
    });
}

#[test]
fn test_multiple_workflows_selected() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let workflow = Workflow::new("test", "p1 {{foo}} bar")
            .with_arguments(vec![Argument::new("foo", ArgumentType::Text)]);

        input.update(&mut app, |input, ctx| {
            input.show_workflows_info_box_on_workflow_selection(
                WorkflowType::Local(workflow.clone()),
                WorkflowSource::Global,
                WorkflowSelectionSource::Undefined,
                None,
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "p1 foo bar");
        });

        // "foo" should be the only range highlighted.
        input.update(&mut app, |input, ctx| {
            let text_style_runs = input.editor.read(ctx, |editor, ctx| {
                editor
                    .text_style_runs(ctx)
                    .filter_map(|text_run| {
                        text_run
                            .text_style()
                            .background_color
                            .map(|_| text_run.text().to_owned())
                    })
                    .collect::<Vec<_>>()
            });

            assert_eq!(text_style_runs, ["foo"]);
        });

        // Input the workflow again.
        input.update(&mut app, |input, ctx| {
            input.show_workflows_info_box_on_workflow_selection(
                WorkflowType::Local(workflow),
                WorkflowSource::Global,
                WorkflowSelectionSource::Undefined,
                None,
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "p1 foo bar");
        });

        // "foo" should be the only range highlighted.
        input.update(&mut app, |input, ctx| {
            let text_style_runs = input.editor.read(ctx, |editor, ctx| {
                editor
                    .text_style_runs(ctx)
                    .filter_map(|text_run| {
                        text_run
                            .text_style()
                            .background_color
                            .map(|_| text_run.text().to_owned())
                    })
                    .collect::<Vec<_>>()
            });

            assert_eq!(text_style_runs, ["foo"]);
        });
    });
}

#[test]
fn test_workflow_argument_tab_with_syntax_highlighting() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let workflow = Workflow::new("test", "yarn {{cwd}} {{flags}}").with_arguments(vec![
            Argument {
                name: "cwd".into(),
                description: None,
                default_value: Some("--cwd ./".into()),
                arg_type: Default::default(),
            },
            Argument::new("flags", ArgumentType::Text),
        ]);

        input.update(&mut app, |input, ctx| {
            input.show_workflows_info_box_on_workflow_selection(
                WorkflowType::Local(workflow.clone()),
                WorkflowSource::Global,
                WorkflowSelectionSource::Undefined,
                None,
                ctx,
            );

            // Simulates syntax highlighting highlighting a portion of an argument
            input.editor.update(ctx, |editor, ctx| {
                let theme = Appearance::as_ref(ctx).theme();
                let terminal_colors_normal = theme.terminal_colors().normal.to_owned();
                editor.update_buffer_styles(
                    vec![ByteOffset::from(5)..ByteOffset::from(10)],
                    TextStyleOperation::default().set_syntax_color(
                        AnsiColorIdentifier::Yellow
                            .to_ansi_color(&terminal_colors_normal)
                            .into(),
                    ),
                    ctx,
                )
            })
        });

        // Even though there are 2 args, there will be 3 runs
        input.read(&app, |input, ctx| {
            // Buffer text should equal our command w/ defaults inserted
            assert_eq!(input.buffer_text(ctx), "yarn --cwd ./ flags");

            let selected_text = input
                .editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));

            // Currently selected text should be the text for the first arg
            assert_eq!(selected_text, "--cwd ./");

            let text_style_runs = input.editor.read(ctx, |editor, ctx| {
                editor
                    .text_style_runs(ctx)
                    .filter_map(|text_run| {
                        text_run
                            .text_style()
                            .background_color
                            .map(|_| text_run.text().to_owned())
                    })
                    .collect::<Vec<_>>()
            });

            // Even though we have only 2 args, there will be 3 runs b/c of syntax highlighting
            assert_eq!(text_style_runs, ["--cwd", " ./", "flags"]);
        });

        input.update(&mut app, |input, ctx| {
            input.input_shift_tab(ctx);
        });

        input.read(&app, |input, ctx| {
            let selected_text = input
                .editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));

            // Tab moves over to next argument
            assert_eq!(selected_text, "flags");
        })
    })
}

#[test]
fn test_workflow_view_does_not_panic() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let workflows = vec![
            Workflow::new("Test Workflow", "echo \"Hello World\""),
            Workflow::new("Test Workflow with Description", "echo \"Hello World\"")
                .with_description("This is a test workflow that prints Hello World!".into()),
            Workflow::new("Test Workflow with Args", "echo \"Hello {{person}}\"")
                .with_arguments(vec![Argument::new("person", ArgumentType::Text)
                    .with_description("The person you want to say hello to".to_string())]),
            Workflow::new("test", "echo \"Hello {{person}}\"")
                .with_description("This is a test workflow that prints Hello {{person}}!".into())
                .with_arguments(vec![Argument::new("person", ArgumentType::Text)
                    .with_description("The person you want to say hello to".to_string())]),
        ];

        for workflow in workflows {
            let command = workflow.content().to_string();
            input.update(&mut app, |input, ctx| {
                input.show_workflows_info_box_on_workflow_selection(
                    WorkflowType::Local(workflow),
                    WorkflowSource::Global,
                    WorkflowSelectionSource::Undefined,
                    None,
                    ctx,
                );
            });

            input.read(&app, |input, ctx| {
                // Buffer text should equal our command w/ defaults inserted
                assert_eq!(
                    input.buffer_text(ctx),
                    command.replace("{{", "").replace("}}", "")
                );
            });
        }
    })
}

#[test]
fn test_system_insert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.system_insert("hello world", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "hello world",
                "Should have inserted 'hello world'"
            );
        });
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
        });
        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty(), "Input should be empty");
        });
        input.update(&mut app, |input, ctx| {
            input.system_insert("hello\nworld", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "hello\nworld",
                "Should have inserted 'hello\nworld'"
            );
        });
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
        });
        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty(), "Input should be empty");
        });
        input.update(&mut app, |input, ctx| {
            input.system_insert("héłló worlḏ", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "héłló worlḏ",
                "Should have inserted 'héłló worlḏ'"
            );
        });
    });
}

#[test]
fn test_is_cursor_in_valid_position_for_completions_while_typing() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });
        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // If cursor is at end of line, show completions menu
        input.update(&mut app, |input, ctx| {
            input.user_insert("gi", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Buffer now looks like "gi|"
        });
        input.update(&mut app, |input, ctx| {
            assert!(input.is_cursor_in_valid_position_for_completions_while_typing(ctx));
        });

        // If cursor is not at end of line, don't show completions menu
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_start(ctx);
            // Buffer now looks like "|gi"
        });
        input.update(&mut app, |input, ctx| {
            assert!(!input.is_cursor_in_valid_position_for_completions_while_typing(ctx));
        });

        // Even if cursor is at end of line when there's multiple lines, don't show
        // completions unless its at the end of the last line.
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Buffer now looks like " gi|"
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert("\ngi", ctx);
            // Buffer currently looks like "gi\ngi|"
            assert_eq!(input.buffer_text(ctx), "gi\ngi");
            assert!(input.is_cursor_in_valid_position_for_completions_while_typing(ctx));
        });

        editor.update(&mut app, |editor, ctx| {
            // Close the tab completion menu if open
            editor.escape(ctx);
            editor.move_up(ctx);
            // Buffer now looks like "gi|\ngi"
        });

        input.update(&mut app, |input, ctx| {
            assert!(!input.is_cursor_in_valid_position_for_completions_while_typing(ctx));
        });

        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Buffer now looks like "gi\ngi|"
        });
        input.update(&mut app, |input, ctx| {
            assert!(input.is_cursor_in_valid_position_for_completions_while_typing(ctx));
        });
    });
}

#[test]
fn test_last_word_insertions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // last word insertion looks for preceding whitespace character
        let history_file_commands = vec![
            "https://app.warp.dev".to_string(),
            "cargo check\ncargo run --features".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;

        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("git test", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git test");
        });

        // Insert while selecting the word `test`
        editor.update(&mut app, |editor, ctx| {
            editor.select_word(&DisplayPoint::new(0, 4), ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git --features");
        });

        // Next insert replaces inserted word (not all of current text), with word from second last history command
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git https://app.warp.dev");
        });

        // Insert is temporary, undo goes back to initial state before first insertion
        // After undo, `test` is currently selected
        editor.update(&mut app, |editor, ctx| {
            editor.undo(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git test");
        });

        // After system edit action (undo), subsequent inserts will insert last word of most recent command
        // After insert, `--features` is currently selected
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git --features");
        });

        // After user edit action (input), subsequent inserts will insert last word of most recent command
        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("f", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git f");
        });
        // Cursor after `f`
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git f--features");
        });

        // After non-edit action (move left), subsequent inserts will insert last word of most recent command
        editor.update(&mut app, |editor, ctx| {
            editor.move_left(/* stop at line start */ false, ctx);
            editor.move_left(/* stop at line start */ false, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git --featuresf--features");
        });
    });
}

#[test]
fn test_last_word_insertions_multiline() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "git status".to_string(),
            "cargo check\ncargo run".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;

        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("git test\ngit two\ngit three", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git test\ngit two\ngit three");
        });

        editor.update(&mut app, |editor, ctx| {
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 4)..DisplayPoint::new(0, 6),
                        DisplayPoint::new(1, 4)..DisplayPoint::new(1, 6),
                        DisplayPoint::new(2, 4)..DisplayPoint::new(2, 6),
                    ],
                    ctx,
                )
                .unwrap();
        });
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "git runst\ngit runo\ngit runree");
        });

        // Insert again.
        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "git statusst\ngit statuso\ngit statusree"
            );
        });

        // On selection change, reset to inserting latest in history.
        editor.update(&mut app, |editor, ctx| {
            editor
                .select_ranges(vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 6)], ctx)
                .unwrap();
        });
        editor.update(&mut app, |editor, ctx| {
            editor.delete(ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 4)..DisplayPoint::new(0, 6),
                        DisplayPoint::new(1, 4)..DisplayPoint::new(1, 6),
                        DisplayPoint::new(2, 4)..DisplayPoint::new(2, 6),
                    ],
                    ctx,
                )
                .unwrap();
        });

        input.update(&mut app, |input, ctx| {
            input.insert_last_word_previous_command(ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(
                input.buffer_text(ctx),
                "git runtusst\ngit runatuso\ngit runatusree"
            );
        });
    });
}

#[test]
fn test_alias_expansion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let aliases = HashMap::from_iter([("gco".into(), "git checkout".into())]);
        let session_info = SessionInfo::new_for_test().with_aliases(aliases);

        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });
        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // Commands are expanded when cursor is at end of line
        input.update(&mut app, |input, ctx| {
            input.user_insert("gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "gco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "git checkout ");
        });

        // Commands are expanded when cursor is in middle of the line
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("gco test", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            use crate::editor::EditorAction;
            editor.move_to_buffer_end(ctx);
            editor.handle_action(&EditorAction::MoveBackwardOneWord, ctx);
            // Cursor is now at "gco |test"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "git checkout test");
        });
    });
}

#[test]
fn test_alias_expansion_multiple_commands_in_input() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let aliases = HashMap::from_iter([("gco".into(), "git checkout".into())]);
        let session_info = SessionInfo::new_for_test().with_aliases(aliases);

        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });
        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // Multilined commands are expanded
        input.update(&mut app, |input, ctx| {
            input.user_insert("test \ngco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "test \ngco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "test \ngit checkout ");
        });

        // Mulitlined commands with multiple cursors are not expanded
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("gco \ngco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            use crate::editor::EditorAction;
            editor.move_to_buffer_end(ctx);
            editor.handle_action(&EditorAction::AddCursorAbove, ctx);
            // Cursor is now at "gco |\ngco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "gco \ngco ");
        });

        // Chained commands are expanded
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("vim && gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "vim && gco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "vim && git checkout ");
        });

        // Nested commands are expanded
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd $(gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "cd $(gco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "cd $(git checkout ");
        });
    });
}

#[test]
fn test_alias_expansion_when_invalid_expansion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let aliases = HashMap::from_iter([("gco".into(), "git checkout".into())]);
        let session_info = SessionInfo::new_for_test().with_aliases(aliases);

        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });
        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // No expansion if the token is an argument
        input.update(&mut app, |input, ctx| {
            input.user_insert("test gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "test gco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "test gco ");
        });

        // No expansion if the token is not an alias
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("test ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "test |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "test ");
        });
    });
}

#[test]
fn test_alias_expansion_when_alias_includes_itself() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let aliases =
            HashMap::from_iter([("g".into(), "git".into()), ("ls".into(), "ls -G".into())]);
        let session_info = SessionInfo::new_for_test().with_aliases(aliases);

        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });
        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // An alias that includes itself is not expanded
        input.update(&mut app, |input, ctx| {
            input.user_insert("ls ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "ls |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "ls ");
        });

        // Aliases that are only a substring of the alias value are still expanded
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("g ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "g |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "git ");
        });
    });
}

#[test]
fn test_alias_expansion_with_abbreviations() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let abbreviations = HashMap::from_iter([("g".into(), "git log".into())]);
        let aliases = HashMap::from_iter([("g".into(), "git".into())]);
        let session_info = SessionInfo::new_for_test()
            .with_aliases(aliases)
            .with_abbreviations(abbreviations);

        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // Abbreviations are expanded and take priority over aliases
        input.update(&mut app, |input, ctx| {
            input.user_insert("g ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "g |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "git log ");
        });
    });
}

#[test]
fn test_alias_expansion_when_alias_expansion_is_disabled() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let abbreviations = HashMap::from_iter([("gco".into(), "git checkout".into())]);
        let aliases =
            HashMap::from_iter([("g".into(), "git".into()), ("vi".into(), "nvim".into())]);
        let session_info = SessionInfo::new_for_test()
            .with_aliases(aliases)
            .with_abbreviations(abbreviations);

        set_alias_expansion_setting(false, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // Aliases are not expanded
        input.update(&mut app, |input, ctx| {
            input.user_insert("g ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "g |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "g ");
        });

        // Abbreviations are still expanded
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "gco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            assert_eq!(input.buffer_text(ctx), "git checkout ");
        });
    });
}

#[test]
fn test_alias_expansion_disabled_in_ai_input_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let aliases = HashMap::from_iter([("gco".into(), "git checkout".into())]);
        let session_info = SessionInfo::new_for_test().with_aliases(aliases);

        // Enable alias expansion setting
        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // Set input type to AI mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::AI,
                        is_locked: true,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Aliases should NOT be expanded when in AI input mode, even with setting enabled
        input.update(&mut app, |input, ctx| {
            input.user_insert("gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "gco |"
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            // Alias should NOT be expanded since we're in AI input mode
            assert_eq!(input.buffer_text(ctx), "gco ");
        });

        // Now switch back to Shell mode and verify expansion works
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: true,
                    },
                    false, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("gco ", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.run_expansion_on_space(ctx);
            // Alias should now be expanded since we're in Shell mode
            assert_eq!(input.buffer_text(ctx), "git checkout ");
        });
    });
}

#[test]
fn test_get_expanded_command_on_execute() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let aliases = HashMap::from_iter([("gco".into(), "git checkout".into())]);
        let session_info = SessionInfo::new_for_test().with_aliases(aliases);

        set_alias_expansion_setting(true, &mut app);
        let terminal = add_window_with_bootstrapped_terminal(
            &mut app,
            None, /* history_file_commands */
            Some(session_info),
        )
        .await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        input.update(&mut app, |input, ctx| {
            input.set_active_block_metadata(
                BlockMetadata::new(Some(SessionId::from(0)), Some("~".into())),
                false,
                ctx,
            )
        });

        // Expansion happens at the end of the line
        input.update(&mut app, |input, ctx| {
            input.user_insert("gco", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "gco|"
        });
        input.update(&mut app, |input, ctx| {
            let result = input.get_expanded_command_on_execute(ctx);
            assert_eq!(result, Some("git checkout".into()));
        });

        // Commands are expanded when cursor is in middle of the line
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("gco test", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            use crate::editor::EditorAction;
            editor.move_to_buffer_start(ctx);
            editor.handle_action(&EditorAction::MoveForwardOneWord, ctx);
            // Cursor is now at "gco| test"
        });
        input.update(&mut app, |input, ctx| {
            let result = input.get_expanded_command_on_execute(ctx);
            assert_eq!(result, Some("git checkout test".into()));
        });

        // Returns None if there is no alias to be expanded.
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("echo Hello", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            // Cursor is now at "echo Hello|"
        });
        input.update(&mut app, |input, ctx| {
            let result = input.get_expanded_command_on_execute(ctx);
            assert_eq!(result, None);
        });
    });
}

#[test]
fn test_tab_completions_menu_for_regular_completions() {
    let _flag = FeatureFlag::ClassicCompletions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd Do", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![file_suggestion("Downloads"), file_suggestion("Documents")],
                    (3, 5),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        let expected_menu_position = TabCompletionsMenuPosition::AtLastCursor;
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { menu_position, .. } if menu_position == &expected_menu_position
            ))
        });
    })
}

#[test]
fn test_tab_completions_menu_for_classic_completions() {
    let _flag = FeatureFlag::ClassicCompletions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        app.update(|ctx| {
            InputSettings::handle(ctx).update(ctx, |setting, ctx| {
                setting
                    .classic_completions_mode
                    .toggle_and_save_value(ctx)
                    .expect("Able to turn on classic completions");
            })
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd Do", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![file_suggestion("Downloads"), file_suggestion("Documents")],
                    (3, 5),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            // The menu should be docked after `cd `.
            assert_eq!(
                input.editor.as_ref(ctx).get_cached_buffer_point(COMPLETIONS_START_OF_REPLACEMENT_SPAN_POSITION_ID),
                Some(Point { row: 0, column: 3 })
            );
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { menu_position, .. } if menu_position == &TabCompletionsMenuPosition::AtStartOfReplacementSpan
            ))
        });
    })
}

#[test]
fn test_tab_completions_menu_for_classic_completions_with_files() {
    let _flag = FeatureFlag::ClassicCompletions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        app.update(|ctx| {
            InputSettings::handle(ctx).update(ctx, |setting, ctx| {
                setting
                    .classic_completions_mode
                    .toggle_and_save_value(ctx)
                    .expect("Able to turn on classic completions");
            })
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("cd foo/Do", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        file_suggestion("foo/Downloads"),
                        file_suggestion("foo/Documents"),
                    ],
                    (3, 9),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            // The menu should be docked after `cd foo/`.
            assert_eq!(
                input.editor.as_ref(ctx).get_cached_buffer_point(COMPLETIONS_START_OF_REPLACEMENT_SPAN_POSITION_ID),
                Some(Point { row: 0, column: 7 })
            );
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { menu_position, .. } if menu_position == &TabCompletionsMenuPosition::AtStartOfReplacementSpan
            ))
        });
    })
}

#[test]
fn test_vim_escape_with_history_menu() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        enable_vim_mode(&mut app);
        let history_file_commands = vec!["cd ~".to_string(), "ls".to_string()];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let (input, editor) = terminal.read(&app, |view, ctx| {
            let input = view.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        // Arrow up displays history in the correct order for an empty buffer
        input.update(&mut app, |input, ctx| {
            input.editor_up(ctx);
        });
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::HistoryUp { .. }
            ));
        });

        // If input suggestions are history, Esc key should exit normal mode before dismissing the
        // history menu.
        editor.update(&mut app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Insert));
            editor.escape(ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::HistoryUp { .. }
            ));
        });

        editor.update(&mut app, |editor, ctx| {
            editor.escape(ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            ));
        });
    });
}

#[test]
fn test_vim_escape_with_completions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        enable_vim_mode(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;

        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Insert));
        });
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("c", ctx);
            input.user_insert("d", ctx);
            input.user_insert(" ", ctx);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd ");
        });
        input.update(&mut app, |input, ctx| {
            input.input_tab(ctx);
            input.handle_completion_suggestions_results(
                build_suggestion_results(
                    vec![
                        argument_suggestion("Documents"),
                        argument_suggestion("Pictures"),
                    ],
                    (3, 3),
                    MatchStrategy::CaseInsensitive,
                ),
                CompletionsTrigger::Keybinding,
                editor_model_snapshot(input, ctx),
                ctx,
            );
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cd ");
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::CompletionSuggestions { .. }
            ));
        });

        // If input suggestions are completions, Esc key should dismiss that before exiting normal
        // mode.
        editor.update(&mut app, |editor, ctx| {
            editor.escape(ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Insert));
        });
        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            ));
        });

        editor.update(&mut app, |editor, ctx| {
            editor.escape(ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn test_voice_input_toggle_preserves_lock_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Start in shell mode with input locked
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: true,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Verify we're in locked shell mode
        let initial_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(initial_config.input_type, InputType::Shell);
        assert!(initial_config.is_locked);

        // Toggle voice input (should switch to AI mode but preserve lock state)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::ToggleVoiceInput(
                    VoiceInputToggledFrom::Button,
                ),
                ctx,
            );
        });

        // Verify we're now in AI mode but still locked
        let after_voice_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(after_voice_config.input_type, InputType::AI);
        assert!(
            after_voice_config.is_locked,
            "Input mode lock state should be preserved when toggling voice input"
        );

        // Test the reverse: start unlocked and ensure it stays unlocked
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: false, // Unlocked (auto-detection enabled)
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Toggle voice input again
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::ToggleVoiceInput(
                    VoiceInputToggledFrom::Button,
                ),
                ctx,
            );
        });

        // Verify we're in AI mode but still unlocked
        let final_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(final_config.input_type, InputType::AI);
        assert!(
            !final_config.is_locked,
            "Input mode should remain unlocked (auto-detection) when toggling voice input"
        );
    });
}

#[test]
fn test_input_type_button_explicit_lock() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Start in unlocked shell mode (auto-detection enabled)
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: false,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Verify initial state
        let initial_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(initial_config.input_type, InputType::Shell);
        assert!(!initial_config.is_locked);

        // Explicitly click AgentMode button - should lock to AI mode
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::InputTypeSelected(InputType::AI),
                ctx,
            );
        });

        // Verify we're now locked in AI mode
        let after_click_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(after_click_config.input_type, InputType::AI);
        assert!(
            after_click_config.is_locked,
            "Input should be locked when user explicitly clicks AgentMode button"
        );

        // Explicitly click Terminal button - should lock to Shell mode
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::InputTypeSelected(InputType::Shell),
                ctx,
            );
        });

        // Verify we're now locked in Shell mode
        let final_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(final_config.input_type, InputType::Shell);
        assert!(
            final_config.is_locked,
            "Input should be locked when user explicitly clicks Terminal button"
        );
    });
}

#[test]
fn test_auto_detection_toggle() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Start in locked AI mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::AI,
                        is_locked: true,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Verify initial locked state
        let initial_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(initial_config.input_type, InputType::AI);
        assert!(initial_config.is_locked);

        // Toggle auto-detection (should unlock and switch to Shell mode for empty buffer)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::EnableAutoDetection,
                ctx,
            );
        });

        // Verify we're now unlocked and switched to Shell mode (empty buffer defaults to Shell)
        let after_toggle_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(after_toggle_config.input_type, InputType::Shell);
        assert!(
            !after_toggle_config.is_locked,
            "Input should be unlocked after toggling auto-detection"
        );

        // Toggle auto-detection again (should do nothing, since auto-detection is already enabled)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::EnableAutoDetection,
                ctx,
            );
        });

        // Verify we're still unlocked in Shell mode
        let second_toggle_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(second_toggle_config.input_type, InputType::Shell);
        assert!(
            !second_toggle_config.is_locked,
            "Input should remain unlocked after toggling auto-detection again"
        );

        // Switch to AI mode manually and test toggle behavior
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::InputTypeSelected(InputType::AI),
                ctx,
            );
        });

        // Verify we're locked in AI mode
        let locked_ai_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(locked_ai_config.input_type, InputType::AI);
        assert!(
            locked_ai_config.is_locked,
            "Input should be locked when manually set to AI"
        );

        // Toggle auto-detection from locked AI mode
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::EnableAutoDetection,
                ctx,
            );
        });

        // Verify we're unlocked and defaults to Shell mode (empty buffer)
        let final_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(final_config.input_type, InputType::Shell);
        assert!(
            !final_config.is_locked,
            "Input should be unlocked after enabling auto-detection"
        );
    });
}

#[test]
fn test_input_mode_setting_methods() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        InputSettings::handle(&app).update(&mut app, |input_settings, ctx| {
            let _ = input_settings
                .input_box_type
                .set_value(InputBoxType::Universal, ctx);
        });

        // Test setting input mode to agent mode
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_agent(true, ctx);
        });

        let config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config.input_type, InputType::AI);
        assert!(config.is_locked, "Input should be locked to AI mode");

        // Test setting input mode to terminal mode
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_terminal(true, ctx);
        });

        let config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config.input_type, InputType::Shell);
        assert!(config.is_locked, "Input should be locked to Shell mode");
    });
}

fn run_input_mode_prefix_test(udi_enabled: bool, input_type: InputType) {
    let input_prefix = match input_type {
        InputType::Shell => super::TERMINAL_INPUT_PREFIX,
        InputType::AI => super::AI_INPUT_PREFIX,
    };

    App::test((), |mut app| async move {
        let _am_flag = FeatureFlag::AgentMode.override_enabled(true);

        initialize_app(&mut app);

        // Ensure the AI autodetection is enabled.
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(true, ctx);
            // Make sure the autodetection is actually enabled, in practice.
            assert!(ai_settings.is_ai_autodetection_enabled(ctx));
        });
        // Set the input box type based on the test configuration.
        InputSettings::handle(&app).update(&mut app, |input_settings, ctx| {
            let input_box_type = if udi_enabled {
                InputBoxType::Universal
            } else {
                InputBoxType::Classic
            };
            let _ = input_settings.input_box_type.set_value(input_box_type, ctx);
        });

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        for c in format!("{input_prefix}some text").chars() {
            input.update(&mut app, |input, ctx| {
                input.user_insert(&c.to_string(), ctx);
            });
        }

        input.read(&app, |input, ctx| {
            // The input prefix should be stripped.
            assert_eq!(input.buffer_text(ctx), "some text");

            app.read_model(input.ai_input_model(), |input_model, _| {
                assert_eq!(input_model.input_type(), input_type);

                // Prefixes represent an explicit mode selection, so they lock the input type in
                // both classic input and UDI.
                assert!(input_model.is_input_type_locked());

                // We should treat this as the mode having been set while the buffer was empty.
                assert!(input_model.was_lock_set_with_empty_buffer());
            })
        });
    });
}

macro_rules! input_mode_prefix_tests {
    ($($name:ident: ($udi_enabled:literal, $input_mode:expr),)*) => {
        $(
            #[test]
            fn $name() {
                run_input_mode_prefix_test($udi_enabled, $input_mode);
            }
        )*
    };
}

input_mode_prefix_tests! {
    test_ai_input_prefix_with_udi: (true, InputType::AI),
    test_ai_input_prefix_with_no_udi: (false, InputType::AI),
    test_shell_input_prefix_with_udi: (true, InputType::Shell),
    test_shell_input_prefix_with_no_udi: (false, InputType::Shell),
}
fn enter_fullscreen_agent_view_for_test(terminal: &ViewHandle<TerminalView>, app: &mut App) {
    terminal.update(app, |view, ctx| {
        view.agent_view_controller().update(ctx, |controller, ctx| {
            controller
                .try_enter_agent_view(
                    None,
                    AgentViewEntryOrigin::Input {
                        was_prompt_autodetected: false,
                    },
                    ctx,
                )
                .expect("Should be able to enter agent view");
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_remains_text_when_handoff_flag_disabled() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(false);

        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), CLOUD_HANDOFF_INPUT_PREFIX);
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::None);
            assert!(!input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_activates_when_handoff_flags_enabled() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
            assert!(input.handoff_compose_state.as_ref(ctx).is_active());
            app.read_model(input.ai_input_model(), |input_model, _| {
                assert_eq!(input_model.input_type(), InputType::AI);
                assert!(input_model.is_input_type_locked());
                assert!(input_model.was_lock_set_with_empty_buffer());
            });
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_normal_deletion_does_not_exit() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert(" ", ctx);
        });

        // Normal backspace that deletes the space (cursor at end) should NOT exit.
        input.update(&mut app, |input, ctx| {
            input
                .editor
                .update(ctx, |editor, ctx| editor.backspace(ctx));
        });

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
            assert!(input.handoff_compose_state.as_ref(ctx).is_active());
        });

        // Backspace on the now-empty buffer exits & mode.
        input.update(&mut app, |input, ctx| {
            input
                .editor
                .update(ctx, |editor, ctx| editor.backspace(ctx));
        });

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::None);
            assert!(!input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_exits_on_backspace_at_beginning_of_buffer() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert("fix tests", ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
        });

        // Move cursor to the beginning, then backspace.
        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, ctx| {
                editor.handle_action(&EditorAction::MoveToLineStart, ctx);
                editor.backspace(ctx);
            });
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::None);
            assert!(!input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_keeps_shell_prefix_as_query_text() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert(super::TERMINAL_INPUT_PREFIX, ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), super::TERMINAL_INPUT_PREFIX);
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
            assert!(input.handoff_compose_state.as_ref(ctx).is_active());
            app.read_model(input.ai_input_model(), |input_model, _| {
                assert_eq!(input_model.input_type(), InputType::AI);
                assert!(input_model.is_input_type_locked());
            });
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_escape_exits_mode_preserving_prompt_text() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert("fix tests", ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
        });

        input.update(&mut app, |input, ctx| {
            input.editor_escape(ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::None);
            assert!(!input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_remains_text_in_powershell_with_nld_enabled() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(true, ctx);
        });

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);
        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, _| {
                editor.set_shell_family(ShellFamily::PowerShell);
            });
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), CLOUD_HANDOFF_INPUT_PREFIX);
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::None);
            assert!(!input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}

#[test]
fn test_cloud_handoff_prefix_activates_in_powershell_when_nld_disabled() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);
        input.update(&mut app, |input, ctx| {
            input.editor.update(ctx, |editor, _| {
                editor.set_shell_family(ShellFamily::PowerShell);
            });
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
            assert!(input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}
#[test]
fn test_cloud_handoff_prefix_vim_escape_exits_insert_before_handoff_mode() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        enable_vim_mode(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.activate_cloud_handoff_compose(HandoffEntryPoint::Ampersand, ctx);
            input.user_insert("fix tests", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Insert));
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.escape(ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.escape(ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "fix tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::None);
            assert!(!input.handoff_compose_state.as_ref(ctx).is_active());
        });
    });
}
#[test]
fn test_cloud_handoff_prefix_ignores_terminal_input_mode_toggle() {
    App::test((), |mut app| async move {
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
        let _oz_handoff_flag = FeatureFlag::OzHandoff.override_enabled(true);
        let _handoff_local_cloud_flag = FeatureFlag::HandoffLocalCloud.override_enabled(true);

        initialize_app(&mut app);
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx);
        });
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        enter_fullscreen_agent_view_for_test(&terminal, &mut app);

        input.update(&mut app, |input, ctx| {
            input.user_insert(CLOUD_HANDOFF_INPUT_PREFIX, ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert("run tests", ctx);
            input.set_input_mode_terminal(true, ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "run tests");
            assert_eq!(input.prefix_mode(ctx), InputPrefixMode::CloudHandoff);
            assert!(input.handoff_compose_state.as_ref(ctx).is_active());
            app.read_model(input.ai_input_model(), |input_model, _| {
                assert_eq!(input_model.input_type(), InputType::AI);
                assert!(input_model.is_input_type_locked());
            });
        });
    });
}
#[test]
fn test_image_attachment_preserves_lock_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Test with locked Shell mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: true,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Select image (should switch to AI mode but preserve lock state)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::SelectFile,
                ctx,
            );
        });

        // Verify we're in AI mode but still locked
        let locked_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(locked_config.input_type, InputType::AI);
        assert!(
            locked_config.is_locked,
            "Lock state should be preserved when selecting image"
        );

        // Test with unlocked Shell mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: false,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Select image again
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::SelectFile,
                ctx,
            );
        });

        // Verify we're in AI mode but still unlocked
        let unlocked_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(unlocked_config.input_type, InputType::AI);
        assert!(
            !unlocked_config.is_locked,
            "Auto-detection should be preserved when selecting image"
        );
    });
}

#[test]
fn test_ai_context_menu_closes_when_space_immediately_after_at_symbol() {
    let _ai_context_menu_enabled = FeatureFlag::AIContextMenuEnabled.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::SetAIContextMenuOpen(true),
                ctx,
            );
        });

        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::AIContextMenu { .. }
            ));
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert(" ", ctx);
        });

        input.read(&app, |input, ctx| {
            assert_eq!(
                *input.suggestions_mode_model().as_ref(ctx).mode(),
                InputSuggestionsMode::Closed
            );
            assert_eq!(input.buffer_text(ctx), "@ ");
        });
    });
}

#[test]
fn test_ai_context_menu_preserves_lock_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Start in locked Shell mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: true,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Open AI context menu (should no longer switch to AI mode)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::SetAIContextMenuOpen(true),
                ctx,
            );
        });

        // Verify we stay in Shell mode with lock state preserved
        let config_after_open = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config_after_open.input_type, InputType::Shell);
        assert!(
            config_after_open.is_locked,
            "Lock state should be preserved when opening AI context menu"
        );

        // Test with unlocked mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: false,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Open AI context menu again
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::SetAIContextMenuOpen(true),
                ctx,
            );
        });

        // Verify we stay in Shell mode and unlocked (@ button no longer switches to AI mode)
        let config_after_second_open = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config_after_second_open.input_type, InputType::Shell);
        assert!(
            !config_after_second_open.is_locked,
            "Auto-detection should be preserved when opening AI context menu"
        );

        // Closing context menu should not change input config
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::SetAIContextMenuOpen(false),
                ctx,
            );
        });

        let config_after_close = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config_after_close.input_type, InputType::Shell);
        assert!(!config_after_close.is_locked);
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn test_input_config_transitions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Test sequence: Shell(locked) -> VoiceInput -> AutoDetection -> AgentMode(locked)

        // Start in locked Shell mode
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: true,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Toggle voice input (should go to AI mode, preserve lock)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::ToggleVoiceInput(
                    VoiceInputToggledFrom::Button,
                ),
                ctx,
            );
        });

        let config_after_voice = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config_after_voice.input_type, InputType::AI);
        assert!(config_after_voice.is_locked);

        // Toggle auto-detection (should unlock and switch to Shell mode for empty buffer)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::EnableAutoDetection,
                ctx,
            );
        });

        let config_after_auto = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config_after_auto.input_type, InputType::Shell);
        assert!(!config_after_auto.is_locked);

        // Explicitly click AgentMode button (should lock in AI mode)
        input.update(&mut app, |input, ctx| {
            input.handle_universal_developer_input_button_bar_event(
                &UniversalDeveloperInputButtonBarEvent::InputTypeSelected(InputType::AI),
                ctx,
            );
        });

        let final_config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(final_config.input_type, InputType::AI);
        assert!(final_config.is_locked);
    });
}

#[test]
fn test_should_show_completions_in_ai_input() {
    // Test cases where the function should return true
    // i.e. we should trigger completions-as-you-type in AI input.
    assert!(should_show_completions_in_ai_input("/"));
    assert!(should_show_completions_in_ai_input("/foo"));
    assert!(should_show_completions_in_ai_input("some text /foo"));

    assert!(should_show_completions_in_ai_input("./"));
    assert!(should_show_completions_in_ai_input("./foo"));
    assert!(should_show_completions_in_ai_input("some text ./foo"));

    assert!(should_show_completions_in_ai_input("foo/"));
    assert!(should_show_completions_in_ai_input("~/"));
    assert!(should_show_completions_in_ai_input("foo/bar"));
    assert!(should_show_completions_in_ai_input("some text foo/bar"));

    assert!(should_show_completions_in_ai_input("../"));
    assert!(should_show_completions_in_ai_input("../foo"));
    assert!(should_show_completions_in_ai_input("bar ../foo"));

    // Test cases where the function should return false
    // i.e. we should NOT trigger completions-as-you-type in AI input.
    assert!(!should_show_completions_in_ai_input("foo"));
    assert!(!should_show_completions_in_ai_input("some text/ foo"));
    assert!(!should_show_completions_in_ai_input("./bar foo"));
    assert!(!should_show_completions_in_ai_input("some text / bar foo"));
    assert!(!should_show_completions_in_ai_input(""));
    assert!(!should_show_completions_in_ai_input("../foo bar"));
    // Space at the end invalidates triggering completions.
    assert!(!should_show_completions_in_ai_input("../foo "));
}

#[test]
fn test_remove_ignored_suggestion_on_command_execution() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |view, _| view.input().clone());

        // First, add a command to ignored suggestions
        let test_command = "echo hi";
        IgnoredSuggestionsModel::handle(&app).update(&mut app, |model, ctx| {
            model.add_ignored_suggestion(
                test_command.to_string(),
                crate::suggestions::ignored_suggestions_model::SuggestionType::ShellCommand,
                ctx,
            );
        });

        // Verify the command is ignored
        let is_ignored_before = IgnoredSuggestionsModel::handle(&app).read(&app, |model, _| {
            model.is_ignored(
                test_command,
                crate::suggestions::ignored_suggestions_model::SuggestionType::ShellCommand,
            )
        });
        assert!(is_ignored_before, "Command should be ignored initially");

        // Execute the command
        input.update(&mut app, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert(test_command, ctx);
            input.try_execute_command(test_command, ctx);
        });

        // Verify the command is no longer ignored
        let is_ignored_after = IgnoredSuggestionsModel::handle(&app).read(&app, |model, _| {
            model.is_ignored(
                test_command,
                crate::suggestions::ignored_suggestions_model::SuggestionType::ShellCommand,
            )
        });
        assert!(
            !is_ignored_after,
            "Command should no longer be ignored after execution"
        );
    });
}

#[test]
fn test_remove_ignored_suggestion_on_ai_query_execution() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |view, _| view.input().clone());

        // First, add an AI query to ignored suggestions
        let test_query = "what is the current date";
        IgnoredSuggestionsModel::handle(&app).update(&mut app, |model, ctx| {
            model.add_ignored_suggestion(
                test_query.to_string(),
                crate::suggestions::ignored_suggestions_model::SuggestionType::AIQuery,
                ctx,
            );
        });

        // Verify the query is ignored
        let is_ignored_before = IgnoredSuggestionsModel::handle(&app).read(&app, |model, _| {
            model.is_ignored(
                test_query,
                crate::suggestions::ignored_suggestions_model::SuggestionType::AIQuery,
            )
        });
        assert!(is_ignored_before, "AI query should be ignored initially");

        // Set up AI input mode and execute the query
        input.update(&mut app, |input, ctx| {
            input.ai_input_model.update(ctx, |ai_input, ctx| {
                ai_input.set_input_type(InputType::AI, ctx);
            });
            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert(test_query, ctx);
            input.submit_ai_query(None, ctx);
        });

        // Verify the query is no longer ignored
        let is_ignored_after = IgnoredSuggestionsModel::handle(&app).read(&app, |model, _| {
            model.is_ignored(
                test_query,
                crate::suggestions::ignored_suggestions_model::SuggestionType::AIQuery,
            )
        });
        assert!(
            !is_ignored_after,
            "AI query should no longer be ignored after execution"
        );
    });
}

#[test]
fn test_agent_view_terminal_only_initial_input_config_unlocked_when_autodetection_enabled() {
    App::test((), |mut app| async move {
        let _am_flag = FeatureFlag::AgentMode.override_enabled(true);
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);

        initialize_app(&mut app);

        // Ensure autodetection is enabled in terminal mode.
        // When AgentView is enabled, terminal-only mode uses nld_in_terminal_enabled_internal.
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .nld_in_terminal_enabled_internal
                .set_value(true, ctx);
            assert!(ai_settings.is_nld_in_terminal_enabled(ctx));
        });

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });

        assert_eq!(config.input_type, InputType::Shell);
        assert!(
            !config.is_locked,
            "Expected terminal-only AgentView input to start unlocked when autodetection is enabled"
        );
    });
}

#[test]
fn test_terminal_only_ai_enter_enters_agent_view_and_clears_buffer() {
    use crate::ai::blocklist::agent_view::AgentViewState;
    use crate::ai::blocklist::InputConfig;

    App::test((), |mut app| async move {
        let _am_flag = FeatureFlag::AgentMode.override_enabled(true);
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);

        initialize_app(&mut app);

        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(true, ctx);
            assert!(ai_settings.is_ai_autodetection_enabled(ctx));
        });

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Put the input into (unlocked) AI mode while agent view is inactive.
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::AI,
                        is_locked: false,
                    },
                    false, /* is_input_buffer_empty */
                    ctx,
                );
            });

            input.clear_buffer_and_reset_undo_stack(ctx);
            input.user_insert("what is the current date", ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.input_enter(ctx);
        });

        // Buffer should be cleared.
        input.read(&app, |input, ctx| {
            assert!(input.buffer_text(ctx).is_empty());
        });

        // Agent view should now be active.
        terminal.read(&app, |terminal, _| {
            let state = terminal
                .model
                .lock()
                .block_list()
                .agent_view_state()
                .clone();
            assert!(matches!(state, AgentViewState::Active { .. }));
        });
    });
}

#[test]
fn test_terminal_only_escape_locks_shell_mode() {
    use crate::ai::blocklist::InputConfig;

    App::test((), |mut app| async move {
        let _am_flag = FeatureFlag::AgentMode.override_enabled(true);
        let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);

        initialize_app(&mut app);

        // Autodetection on; we still expect Esc to explicitly lock to shell.
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .ai_autodetection_enabled_internal
                .set_value(true, ctx);
            assert!(ai_settings.is_ai_autodetection_enabled(ctx));
        });

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let editor = input.read(&app, |input, _| input.editor().clone());

        // Start in AI mode (unlocked) while agent view is inactive.
        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::AI,
                        is_locked: false,
                    },
                    true, /* is_input_buffer_empty */
                    ctx,
                );
            });
        });

        // Hit Esc (via editor) and ensure we end up locked to shell.
        editor.update(&mut app, |editor, ctx| {
            editor.escape(ctx);
        });

        let config = input.read(&app, |input, _| {
            app.read_model(input.ai_input_model(), |ai_input, _| {
                ai_input.input_config()
            })
        });
        assert_eq!(config.input_type, InputType::Shell);
        assert!(config.is_locked);
    });
}

#[test]
fn test_page_up_and_down_scroll_terminal_from_prompt() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        terminal.update(&mut app, |terminal, _| {
            terminal
                .model
                .lock()
                .simulate_block("ls", &"\n".repeat(1000));
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert("echo first line\necho second line", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_buffer_end(ctx);
            editor.handle_action(&EditorAction::PageUp, ctx);
        });

        assert_eq!(
            input.read(&app, |input, ctx| input.buffer_text(ctx)),
            "echo first line\necho second line"
        );
        let scroll_position_after_page_up =
            terminal.read(&app, |terminal, _| terminal.scroll_position());
        assert!(matches!(
            scroll_position_after_page_up,
            ScrollPosition::FixedAtPosition { .. }
        ));

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::PageDown, ctx);
        });

        assert_eq!(
            input.read(&app, |input, ctx| input.buffer_text(ctx)),
            "echo first line\necho second line"
        );
        let scroll_position_after_page_down =
            terminal.read(&app, |terminal, _| terminal.scroll_position());
        assert_ne!(
            scroll_position_after_page_down,
            scroll_position_after_page_up
        );
    });
}

#[test]
fn test_page_up_and_down_do_not_scroll_terminal_when_suggestions_are_visible() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_file_commands = vec![
            "echo alpha\necho beta".to_string(),
            "git status\ngit diff".to_string(),
        ];
        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, Some(history_file_commands), None)
                .await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        terminal.update(&mut app, |terminal, _| {
            terminal
                .model
                .lock()
                .simulate_block("ls", &"\n".repeat(1000));
        });

        input.update(&mut app, |input, ctx| {
            input.handle_action(&InputAction::Up, ctx);
            assert!(input.suggestions_mode_model.as_ref(ctx).is_visible());
        });

        let initial_scroll_position = terminal.read(&app, |terminal, _| terminal.scroll_position());
        let initial_buffer = input.read(&app, |input, ctx| input.buffer_text(ctx));

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::PageUp, ctx);
            editor.handle_action(&EditorAction::PageDown, ctx);
        });

        terminal.read(&app, |terminal, _| {
            assert_eq!(terminal.scroll_position(), initial_scroll_position);
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), initial_buffer);
            assert!(input.suggestions_mode_model.as_ref(ctx).is_visible());
        });
    });
}

#[test]
fn test_page_up_and_down_scroll_terminal_with_vim_mode_enabled() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        terminal.update(&mut app, |terminal, _| {
            terminal
                .model
                .lock()
                .simulate_block("ls", &"\n".repeat(1000));
        });

        AppEditorSettings::handle(&app).update(&mut app, |settings, settings_ctx| {
            let _ = settings.vim_mode.set_value(true, settings_ctx);
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert("echo first line\necho second line", ctx);
        });
        editor.update(&mut app, |editor, ctx| {
            editor.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        });

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::PageUp, ctx);
        });

        assert_eq!(
            input.read(&app, |input, ctx| input.buffer_text(ctx)),
            "echo first line\necho second line"
        );
        let scroll_position_after_page_up =
            terminal.read(&app, |terminal, _| terminal.scroll_position());
        assert!(matches!(
            scroll_position_after_page_up,
            ScrollPosition::FixedAtPosition { .. }
        ));

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::PageDown, ctx);
        });

        assert_eq!(
            input.read(&app, |input, ctx| input.buffer_text(ctx)),
            "echo first line\necho second line"
        );
        let scroll_position_after_page_down =
            terminal.read(&app, |terminal, _| terminal.scroll_position());
        assert_ne!(
            scroll_position_after_page_down,
            scroll_position_after_page_up
        );
    });
}

#[test]
fn test_custom_terminal_page_scroll_binding_applies_when_prompt_is_focused() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (window_id, terminal) =
            add_window_with_bootstrapped_terminal_and_window_id(&mut app, None, None).await;
        let (input, editor) = terminal.read(&app, |terminal, ctx| {
            let input = terminal.input().clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input, editor)
        });

        terminal.update(&mut app, |terminal, _| {
            terminal
                .model
                .lock()
                .simulate_block("ls", &"\n".repeat(1000));
        });

        app.update(|ctx| {
            ctx.set_custom_trigger(
                "terminal:scroll_up_one_page".to_owned(),
                warpui::keymap::Trigger::Keystrokes(
                    vec![Keystroke::parse("shift-pageup").unwrap()],
                ),
            );
        });

        let focus_path = [terminal.id(), input.id(), editor.id()];

        let handled = app
            .dispatch_keystroke(
                window_id,
                &focus_path,
                &Keystroke::parse("pageup").unwrap(),
                false,
            )
            .unwrap();
        assert!(!handled);
        terminal.read(&app, |terminal, _| {
            assert_eq!(
                terminal.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
        });

        let handled = app
            .dispatch_keystroke(
                window_id,
                &focus_path,
                &Keystroke::parse("shift-pageup").unwrap(),
                false,
            )
            .unwrap();
        assert!(handled);
        terminal.read(&app, |terminal, _| {
            assert!(matches!(
                terminal.scroll_position(),
                ScrollPosition::FixedAtPosition { .. }
            ));
        });
    });
}
