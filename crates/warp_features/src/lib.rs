use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use enum_iterator::{cardinality, Sequence};
#[cfg(feature = "test-util")]
pub use overrides::{get_overrides, set_overrides};

#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug, Sequence)]
pub enum FeatureFlag {
    Changelog,
    CocoaSentry,
    CrashReporting,
    DebugMode,
    Autoupdate,
    LogExpensiveFramesInSentry,
    WithSandboxTelemetry,
    RecordAppActiveEvents,

    WelcomeTips,
    ThinStrokes,
    WelcomeBlock,
    KnowledgeSidebar,

    RuntimeFeatureFlags,

    /// Enables cloud object related features for an explicit allowlist of team testers.
    CloudObjects,

    /// If `true`, fetch updated Warp channel versions from the Warp server endpoint instead of
    /// from GCP directly.
    FetchChannelVersionsFromWarpServer,

    /// Does grid storage go forwards or backwards
    SequentialStorage,

    /// If set, generators are executed in-band in all SSH sessions.
    InBandGeneratorsForSSH,

    /// If set, generators are executed using cmd.exe on Windows.
    RunGeneratorsWithCmdExe,

    /// Gates a bindable keyboard action for accepting command corrections.
    CommandCorrectionKey,

    /// If `true`, the "Show Initialization Block" menu item is added to the Blocks menu in the Mac
    /// menu bar.
    ToggleBootstrapBlock,

    /// A runtime flag to enable the creation of shared sessions.
    ///
    /// It is enabled if the logged in user is part of a paying team
    /// or part of the allowlist (via [`ServerExperiment::SessionSharingExperiment`]).
    ///
    /// We also use [`ServerExperiment::SessionSharingControl`] as a
    /// killswitch for abuse prevention.
    CreatingSharedSessions,

    /// Enables the joining / viewing of shared sessions (_not_ creation).
    ViewingSharedSessions,

    /// Enabling context chips functionality for prompt
    ContextChips,

    /// Ligature Support in the Editor and Grid
    Ligatures,

    /// When enabled, the `History` rule from the command_corrections crate
    /// will be enabled. When the `History` rule is enabled, the command_corrections
    /// lib will use the user's history as a last-ditch effort to find a reasonable correction.
    CommandCorrectionsHistoryRule,

    /// Used to gate an experiment we're doing on WarpDev ONLY
    /// to get a sense of PTY throughput over time.
    RecordPtyThroughput,

    /// Whether to fetch generic string objects from the server.
    FetchGenericStringObjects,

    /// Enables a setting on Intel Dual-GPU Macs to enable use of the integrated GPU over the
    /// discrete GPU.
    IntegratedGPU,

    /// Warp Agent Mode.
    AgentMode,

    /// Whether the user is part of the Warp Alpha Program (AI Trusted Testers).
    /// This is enabled automatically for local and dev builds.
    /// Collect conversation and input autodetection data for agent mode.
    /// Also collects block data for Next Command, if enabled.
    AgentModeAnalytics,

    /// A setting to enable a traditional completions experience.
    ClassicCompletions,

    /// Force enable classic completions.
    ForceClassicCompletions,

    /// If enabled, autosuggestions are hidden when the tab completions
    /// menu is open (except when using completions-as-you-type).
    RemoveAutosuggestionDuringTabCompletions,

    /// Feature flag for cursor reflow fix (fixes part of the Alacritty resizing logic).
    ResizeFix,

    /// Enable multiselect in Notebooks and Warp Text.
    RichTextMultiselect,

    /// If enabled, the default input mode is set to waterfall for new users.
    DefaultWaterfallMode,

    /// Makes the input editor's prompt selectable.
    SelectablePrompt,

    /// Enables the settings file feature.
    SettingsFile,

    /// Enables rect selection.
    RectSelection,

    /// Adds Alacritty as a supported terminal to import settings from.
    AlacrittySettingsImport,

    /// Enable dynamic enum parameter types for workflow arguments
    DynamicWorkflowEnums,

    /// Enables next action prediction within Warp, powered by AI.
    AgentPredict,

    /// Enables receiving shared Warp Drive objects.
    SharedWithMe,

    /// Enables workflows for use with Agent Mode.
    AgentModeWorkflows,

    /// Enables AI rules for use with Agent Mode.
    AIRules,

    /// Enables the shell selector, allowing us to open a new tab in
    /// a shell other than the default shell.
    ShellSelector,

    /// Enables writing to long-running commands in shared sessions.
    SharedSessionWriteToLongRunningCommands,

    /// Lazily builds scenes at render time instead of eagerly when a view
    /// changes.
    LazySceneBuilding,

    /// Enables support for ACLs in Session Sharing. Should be disabled if the
    /// corresponding `use_acls` flag in the session sharing server is disabled.
    /// https://github.com/warpdotdev/session-sharing-server/blob/b6590ebd0b0e7f6847d6b2228b4e77d63939ce22/server/Cargo.toml#L13
    SessionSharingAcls,

    /// Enables the full-screen "zen mode" setting, where we hide the tab bar if there's only one
    /// tab.
    FullScreenZenMode,

    /// Playground for reducing Warp UI clutter.
    MinimalistUI,

    /// Enables support for using native shell completions to supplement our
    /// completion specs.
    NativeShellCompletions,

    /// Adds avatar to the tab bar.
    AvatarInTabBar,

    /// Adds aliases for executing Warp Drive workflows.
    WorkflowAliases,

    SshDragAndDrop,
    DragTabsToWindows,

    /// Enables the overflow menu on AI blocks.
    AIBlockOverflowMenu,

    /// Enables cycling through the next command suggestions with down arrow.
    CycleNextCommandSuggestion,

    /// Enables multi-workspace selection.
    MultiWorkspace,

    /// Maximizes data in flat storage to reduce memory usage.
    MaximizeFlatStorage,

    ImeMarkedText,

    /// Enables partial next command suggestions with a prefix.
    PartialNextCommandSuggestions,

    AIGeneratedOnboardingSuggestions,

    /// Enables iTerm image rendering
    ITermImages,

    /// Enables validation of autosuggestions.
    ValidateAutosuggestions,

    /// Enables prompt suggestions sourced via MAA.
    PromptSuggestionsViaMAA,

    /// Enables using `esc` to clear autosuggestions.
    ClearAutosuggestionOnEscape,

    /// If enabled, the default theme is set to Adeberry for new users.
    DefaultAdeberryTheme,

    /// New, less intrusive autoupdate UI.
    AutoupdateUIRevamp,

    /// Enables Kitty image rendering
    KittyImages,

    /// Enables support for Warp Packs.
    WarpPacks,

    /// Enables the revised AI analytics policy banner.
    ///
    /// This does not gate actual collection of data under the new policy.
    GlobalAIAnalyticsBanner,

    /// Enables actual collection of AI analytics data per the revised AI analytics policy.
    GlobalAIAnalyticsCollection,

    /// Enables auto-generated AI memories.
    AIMemories,

    /// Enables the XML output system prompt for the primary (terminal) agent in Agent Mode.
    AgentModePrimaryXML,

    /// Enables the XML output system prompt for the pre-plan agent in Agent Mode.
    AgentModePrePlanXML,

    /// Enables Agent Mode onboarding.
    AgentOnboarding,

    /// Enables suggested rules.
    SuggestedRules,

    /// Enables suggested workflows for Agent Mode.
    SuggestedAgentModeWorkflows,

    /// Forces users to login.
    ForceLogin,

    /// Enables prediction of Agent Mode queries.
    PredictAMQueries,

    /// Enables full source code embedding of repos when using codebase context.
    FullSourceCodeEmbedding,

    /// Enables codebase indexing inside remote server daemon processes.
    RemoteCodebaseIndexing,

    /// If enabled, command palette searches will use Tantivy search instead of the default fuzzy search.
    UseTantivySearch,

    /// Allows AI to call the grep tool.
    GrepTool,

    /// MCP server v0 functionality.
    McpServer,

    /// Enables image as context for AM.
    ImageAsContext,

    /// UNIX shells running "natively" on Windows via MSYS2.
    MSYS2Shells,

    /// Allows AI to call the file retrieval tools.
    FileRetrievalTools,

    /// Reload files in an AI conversation to prevent stale files.
    ReloadStaleConversationFiles,

    /// Auto generate the title when creating a shared block.
    SharedBlockTitleGeneration,

    /// Retry truncated file edit responses from the coding agent.
    RetryTruncatedCodeResponses,

    /// Enables reading images with the `read_files` tool.
    ReadImageFiles,

    UsageBasedPricing,

    /// Enables cross-repo codebase context.
    CrossRepoContext,

    /// Persist codebase indices to disk.
    CodebaseIndexPersistence,

    /// Enables the AI context menu, or at-menu.
    AIContextMenuEnabled,

    /// Enables the AI context menu outside of AI input mode.
    AtMenuOutsideOfAIMode,

    /// Enables the resume button for cancelled AI conversations.
    AIResumeButton,

    /// Enables the agent to decide whether to execute a command.
    AgentDecidesCommandExecution,

    /// Show speed bump when enabling codebase indexing.
    CodebaseIndexSpeedbump,

    /// Enables inline review comments on specific lines of code.
    ContextLineReviewComments,

    /// Enables the fast-forward autoexecute button
    FastForwardAutoexecuteButton,

    /// Remembers the per-conversation fast-forward state across local session restoration.
    RememberFastForwardState,

    /// Enables the find/replace in code editor
    CodeFindReplace,

    /// Enables file search functionality in command palette
    CommandPaletteFileSearch,

    /// Enables the AI context menu nesting and commands
    AIContextMenuCommands,

    /// Enables sending stderr warnings in FileGlobV2 results.
    FileGlobV2Warnings,

    /// Enables code symbols in AI context menu
    AIContextMenuCode,

    /// Enables Warp Drive objects (like workflows) as context in AI context menu
    DriveObjectsAsContext,

    /// Expands code diff edits to replace the current pane instead of opening in a new tab.
    ExpandEditToPane,
    /// Enables fallback model load output messaging in the warping indicator.
    FallbackModelLoadOutputMessaging,

    /// Enables close button on left side of tabs
    TabCloseButtonOnLeft,

    /// Enables AI agent profile settings UI and functionality.
    ///
    /// TODO: When cleaning up this flag, also remove the `show_model_selectors_in_prompt`
    /// setting in [`SessionSettings`] (defined in `app/src/terminal/session_settings.rs`),
    /// as model selectors are always shown when this flag is enabled.
    ProfilesDesignRevamp,

    /// Enables new Search Codebase UI
    SearchCodebaseUI,

    /// Enables us to render linked code blocks
    LinkedCodeBlocks,

    /// Enables the tabbed file viewer
    TabbedEditorView,

    /// Enables sending telemetry data to a file in addition to the server
    SendTelemetryToFile,

    /// Enables multiple agent profiles in settings for managing different AI agent configurations.
    MultiProfile,

    /// Enables displaying imported PR review comments in the blocklist.
    PRCommentsV2,

    /// Gates the bundled skill-based implementation of PR comment fetching.
    PRCommentsSkill,

    /// A new first-time user experience which prioritizes choosing a coding repository.
    GetStartedTab,

    /// Enables Projects and Project management
    Projects,

    /// Enables selection-as-context functionality in the code editor.
    SelectionAsContext,

    /// A context chip that shows when the PWD is inside of a git repository.
    CodeModeChip,

    /// Enables the prompt chip that displays the GitHub PR for the current branch.
    GithubPrPromptChip,

    /// A button on the homepage for easily creating new projects.
    CreateProjectFlow,

    /// Enables vim keybindings in the code editor.
    VimCodeEditor,

    /// Allows opening file links using the $EDITOR environment variable.
    AllowOpeningFileLinksUsingEditorEnv,

    /// Enables the ability to undo closed panes.
    UndoClosedPanes,

    /// Enables revert button for diff hunks in the gutter.
    RevertDiffHunk,

    /// Enables saving code review pane changes
    CodeReviewSaveChanges,

    /// Enables the file tree (with an entrypoint through code mode).
    FileTree,

    /// Enables ignoring input suggestions.
    AllowIgnoringInputSuggestions,

    /// Enables the one-time modal on app startup for existing users for the Code launch.
    CodeLaunchModal,

    /// Enables API key authentication for Agent SDK
    APIKeyAuthentication,

    /// Enables API key management UI in settings
    APIKeyManagement,

    /// Enables OAuth support for MCP.
    McpOauth,

    /// Enables attaching diff sets (multiple hunks from multiple files) as context in Agent Mode.
    DiffSetAsContext,

    /// Enables file- and diff set-level comments in the code review header.
    FileAndDiffSetComments,

    /// Enables discarding per-file and discarding all changes
    DiscardPerFileAndAllChanges,

    /// Enables UI zoom support (scaling the entire UI by a given percentage).
    UIZoom,

    /// Shows a confirmation dialog when cancelling an active summarization via Ctrl-C or stop.
    SummarizationCancellationConfirmation,

    /// Enables find/search in code review pane
    CodeReviewFind,

    /// Enables asynchronous find in terminal, running search on a background thread.
    AsyncFind,

    /// Enables using Agent Mode in shared sessions.
    AgentSharedSessions,

    /// Enables auto-opening code review pane on first agent change and its setting UI.
    AutoOpenCodeReviewPane,

    /// Enables the ambient agents command-line interface.
    AmbientAgentsCommandLine,

    /// Feature flags for the Build Plan Auto Reload experiment.
    BuildPlanAutoReloadBannerToggle,
    BuildPlanAutoReloadPostPurchaseModal,

    /// Enables inline code review functionality
    InlineCodeReview,

    /// Enables cloud environments management via CLI.
    CloudEnvironments,

    /// Enables the /create-environment slash command for setting up Warp Environments
    CreateEnvironmentSlashCommand,

    /// Enables the local docker sandbox entrypoints in the client.
    LocalDockerSandbox,

    /// Enables the /compact slash command.
    SummarizationConversationCommand,

    /// Enables the provider command for linking third-party services.
    ProviderCommand,

    /// Enables the integration command for managing agent integrations.
    IntegrationCommand,

    /// Enables the artifact command for uploading and downloading CLI artifacts.
    ArtifactCommand,

    /// Groups MCP tools and resources by their originating server when sending context to the AI backend.
    MCPGroupedServerContext,

    /// Enables the web search UI (when the model executes a web search).
    WebSearchUI,

    /// Enables the web fetch UI (when the model fetches content from URLs).
    WebFetchUI,

    /// Displays debugging IDs for MCP servers, installations, and gallery items.
    McpDebuggingIds,

    /// Enables rendering of images in markdown files and AI responses.
    MarkdownImages,
    /// Enables rendering Mermaid diagrams in markdown notebooks.
    MarkdownMermaid,
    /// Enables editable Mermaid diagrams to behave atomically in notebook and plan editors.
    EditableMarkdownMermaid,

    /// Enables rendering markdown tables in notebooks.
    MarkdownTables,

    /// Renders `.ipynb` (Jupyter) files as a formatted, read-only notebook in
    /// Warp's notebook viewer instead of showing the raw JSON in the code editor.
    JupyterNotebookRendering,

    /// Enables rendering markdown tables inline in AI block list responses.
    BlocklistMarkdownTableRendering,
    /// Enables rendering markdown images inline in AI block list responses.
    BlocklistMarkdownImages,

    /// Enables the /fork-from slash command.
    ForkFromCommand,

    /// Enables v2 of the context window usage UI.
    ContextWindowUsageV2,

    /// Dev-only: enables the expandable per-segment context window usage
    /// breakdown in the conversation usage card.
    ContextWindowUsageBreakdown,

    /// Enables global search
    GlobalSearch,

    /// Enables embedded code review comments.
    EmbeddedCodeReviewComments,

    /// Enables the revert to checkpoints feature.
    RevertToCheckpoints,

    /// Enables the /rewind slash command.
    RewindSlashCommand,

    /// Agent Management View.
    AgentManagementView,

    /// Agent Management Details View - enables new details panel on card click.
    AgentManagementDetailsView,

    /// Enables scheduled ambient agents.
    ScheduledAmbientAgents,

    AgentView,

    /// Enables block context functionality in Agent View.
    AgentViewBlockContext,

    /// Enables the inline history menu for quickly accessing previous commands and conversations.
    InlineHistoryMenu,

    /// Enables the inline repo switcher menu for switching between indexed repos.
    InlineRepoMenu,

    /// Enables cloud mode functionality for ambient agents.
    CloudMode,

    /// Enables starting cloud mode from a local session.
    CloudModeFromLocalSession,

    /// Enables host selection in cloud mode.
    CloudModeHostSelector,

    /// Enables Warp Managed Secrets functionality.
    WarpManagedSecrets,

    /// Enables support for AM file diffs backed by the V4A patch format.
    V4AFileDiffs,

    /// Enables loading conversations in the Agent Management View.
    InteractiveConversationManagementView,

    /// Enables agent tips displayed below the warping indicator in Agent Mode.
    AgentTips,

    /// Allows agent mode to use computer use tools.
    AgentModeComputerUse,

    /// Enables computer use functionality in local clients.
    LocalComputerUse,

    /// Enables background, per-window computer use: driving a specific window directly without
    /// raising it or moving the cursor.  Currently only supported on macOS.
    BackgroundComputerUse,

    /// Enables video recording of computer-use sessions for cloud agents.
    VideoRecording,

    /// Enables team API key creation in the API key management UI.
    TeamApiKeys,

    /// Enables cloud conversation loading via the CLI --conversation flag.
    CloudConversations,

    /// Enables the "New agent" prompt chip in terminal mode when AgentView is enabled.
    ///
    /// When disabled (the default), the terminal message bar is shown instead.
    AgentViewPromptChip,

    /// Enables editing the agent input footer layout from the prompt context menu.
    AgentToolbarEditor,

    /// Enables configuring header toolbar item order, side placement, and visibility.
    ConfigurableToolbar,

    /// Enables real-time communication updates for ambient agent tasks.
    AmbientAgentsRTC,

    // Enables a side panel conversation list view for AgentView mode.
    AgentViewConversationListView,

    /// When enabled, the server will use message replacement + retroactive subtasks for
    /// summarization.
    SummarizationViaMessageReplacement,

    /// Enables pluggable notifications via OSC 9 and OSC 777 escape sequences.
    /// External programs can trigger system and in-app notifications.
    PluggableNotifications,

    /// Dev-only: simulate a GitHub-unauthed user in the Environments page flow.
    ///
    /// This is intended for developer testing and should have no effect in release builds.
    SimulateGithubUnauthed,

    /// When enabled, profile selection is displayed in an inline view above the Agent input (e.g. via /profile).
    InlineProfileSelector,

    /// Enables sending the server a list of Skills that the client has access to.
    ///
    /// If disabled, the server will send None as the SkillsContext.
    ListSkills,

    /// When enabled, we expose LSP as a tool to the agent
    LSPAsATool,

    /// Enables conversation artifacts.
    ConversationArtifacts,

    /// Enables auto-syncing ambient plans to Warp Drive.
    SyncAmbientPlans,

    /// Enables platform skills support (--skill flag) for agent runs.
    ///
    /// Skills are loaded from `.agents/skills/`, `.warp/skills/`, `.claude/skills/`, and `.codex/skills/`
    /// directories to provide base prompts for agent runs.
    OzPlatformSkills,
    /// Enables Oz identity federation commands.
    OzIdentityFederation,

    /// Gates populating/reading oz updates from channel versions in the changelog model.
    OzChangelogUpdates,

    /// Enables image upload for ambient agents.
    AmbientAgentsImageUpload,

    /// Enables image attachment support for cloud mode conversations.
    CloudModeImageContext,

    /// Enables loading and returning bundled skills in the SkillManager.
    BundledSkills,

    /// Enables the Oz launch modal for introducing cloud agent features.
    OzLaunchModal,

    /// Enables the OpenWarp launch modal announcing Warp going open-source.
    /// When enabled, the HOA onboarding flow is suppressed.
    OpenWarpLaunchModal,

    /// Enables the orchestration launch modal announcing multi-agent orchestration features.
    OrchestrationLaunchModal,

    /// Updated tab styling (background colors, border, close button positioning, margins).
    NewTabStyling,

    /// Enables file-based MCP server support via .mcp.json files in repo roots.
    FileBasedMcp,

    /// Enables passing user query arguments to skill invocations ($ARGUMENTS, $N).
    SkillArguments,

    /// When enabled, a conversation is only considered "active" once a new query has been
    /// sent since opening (rather than the moment its agent view is expanded).
    ActiveConversationRequiresInteraction,

    /// Enables attaching conversations as context in Agent Mode via the @ menu.
    ConversationsAsContext,

    /// Enables the rich input editor for CLI agents (e.g., Claude Code).
    /// Ctrl-G intercepts the keystroke and opens Warp's input editor instead of $EDITOR.
    CLIAgentRichInput,

    /// Enables incremental (diff-based) buffer updates for auto-reload instead of full replace.
    IncrementalAutoReload,

    /// Enables scroll position preservation in the code review pane when file
    /// content changes via auto-reload.
    CodeReviewScrollPreservation,

    /// Re-enables local Claude Code and Codex child harnesses in orchestration
    /// flows while the default behavior temporarily keeps them disabled.
    LocalClaudeCodexChildHarnesses,

    /// Gates client-side support for the `orchestrate` tool, which batches
    /// multiple child agents into a single tool call with an inline
    /// confirmation card. When enabled, the client advertises
    /// `RequestSettings.SupportsOrchestrate = true` and the server's
    /// orchestrate tool replaces `start_agent` / `start_agent_v2` for
    /// orchestration-capable conversations. Layered on top of
    /// `OrchestrationV2`; has no effect when v2 is off.
    RunAgentsTool,

    /// On `wait_for_events`, confirms parent status against the server and
    /// registers an orchestrator for the owner-side ancestor stream so it
    /// receives events for children created out-of-band (Oz CLI / web API).
    WaitForEventsParentRegistration,

    /// Shows a pending user query indicator during summarization when a follow-up
    /// prompt is queued via `/fork-and-compact` or `/compact-and`.
    PendingUserQueryIndicator,

    /// Gates the `/queue` slash command, which lets users queue a follow-up prompt
    /// while the agent is mid-response.
    QueueSlashCommand,
    /// Extends queued prompts to Cloud Mode setup and follow-up draining.
    QueuedPromptsV2,

    /// Enables an agent tool for the CLI subagent to explicitly transfer command control to the
    /// user.
    TransferControlTool,

    /// Enables Kitty keyboard protocol support (CSI u encoding, progressive enhancement).
    KittyKeyboardProtocol,

    /// Detects the word "figma" in the terminal input in real-time and shows a
    /// contextual button above the input.
    FigmaDetection,

    /// Enables header rows on all inline menus (label, tabs, resize handle).
    InlineMenuHeaders,
    /// Clears the current prompt when opening the inline model selector from the
    /// model chip, then restores that prompt when the selector closes.
    RestorePromptOnInlineModelSelectorSearch,

    /// Enables associating a tab color with a directory so tabs automatically
    /// adopt the configured color when their working directory matches.
    DirectoryTabColors,

    /// Enables the new settings to control visibility of Warp Drive, Code Review Panel,
    /// and Project Explorer & Global Search features.
    OpenWarpNewSettingsModes,

    /// Enables vertical tab layout as an alternative to the horizontal tab bar.
    VerticalTabs,

    /// Enables attaching code review comments, diff hunk, and attach as context
    /// from code review + code editor for House Of Agents work
    HoaCodeReview,

    /// Enables the `--harness` flag for `oz agent run`, allowing external agent
    /// CLIs (e.g. `claude`) to execute prompts instead of Warp's agent harness.
    AgentHarness,

    /// Enables workspace- and block-snapshot handoff between cloud agent runs
    /// and the local Warp client.
    /// When enabled:
    /// - The `AgentDriver` uploads a workspace snapshot (repo diffs + files) at the end of every
    ///   cloud agent run, regardless of harness.
    /// - Subsequent executions download the prior execution's handoff snapshot attachments.
    /// - Third-party harness conversations hydrate their terminal output inline by fetching a
    ///   block snapshot from the server.
    OzHandoff,

    /// Enables the upgraded CLI agent session tracking and notifications infrastructure.
    HOANotifications,

    /// Enables the install/update chip for the OpenCode Warp plugin.
    /// Requires HOANotifications to also be enabled.
    OpenCodeNotifications,

    /// Enables the install/update chip for the Codex Warp notification plugin.
    /// Requires HOANotifications to also be enabled.
    CodexNotifications,

    /// Enables the Codex Warp plugin marketplace integration.
    /// When disabled, Codex uses native OSC9 notifications.
    CodexPlugin,

    /// Enables the install/update chip for the Gemini CLI Warp extension.
    /// Requires HOANotifications to also be enabled.
    GeminiNotifications,

    /// When enabled, the "Skip for now" login flow does not create a Firebase
    /// anonymous user. The user remains fully logged out (no credentials) and
    /// login-gated features are disabled until they sign in.
    SkipFirebaseAnonymousUser,

    /// Enables tab configs — user-definable TOML templates for launching custom tab layouts.
    TabConfigs,

    /// Enables Warp local control through the standalone warpctrl CLI.
    WarpControlCli,

    /// Enables the ask_user_question tool allowing the agent to ask clarifying questions.
    AskUserQuestion,

    /// When enabled, solo users (not on a team) can use BYO API keys.
    SoloUserByok,

    /// Replaces the in-block warpification banner with a warpify footer.
    WarpifyFooter,

    /// Enables conversation retrieval via the CLI (oz run conversation get, oz run get --conversation).
    ConversationApi,

    /// Guided onboarding flow for existing users introducing HOA features
    /// (vertical tabs, agent inbox, tab configs).
    HOAOnboardingFlow,

    /// Enables commit, push, and create-PR actions in the code review panel.
    GitOperationsInCodeReview,

    /// Gates the remote control chip and `/remote-control` slash command in the CLI agent footer.
    HOARemoteControl,

    /// Trims trailing blank rows from CLI agent block output so unused vertical
    /// space is not rendered while the agent is running.
    TrimTrailingBlankLines,

    /// Gates the new SSH remote server flow that installs and connects to a
    /// persistent binary on the remote machine instead of using ControlMaster
    /// for command execution.
    SshRemoteServer,

    /// Redux of the setup/initial user query UI for cloud mode.
    CloudModeSetupV2,

    /// Enables summary mode in vertical tabs, showing condensed tab summaries
    /// instead of individual pane rows.
    VerticalTabsSummaryMode,

    CloudModeInputV2,

    /// Enables continuing cloud mode conversations in the cloud after an execution ends.
    HandoffCloudCloud,

    /// Enables the local-to-cloud Oz handoff entry points (footer chip and
    /// `/move-to-cloud` slash command) that fork the active local Oz
    /// conversation into a fresh cloud agent run with the current workspace
    /// snapshot attached. Requires `OzHandoff` to also be enabled.
    HandoffLocalCloud,

    /// Enables creating API keys scoped to named agents in the API key
    /// management UI. When enabled the "Team" option in the key-type
    /// selector is replaced with "Agent" and users can pick which agent
    /// identity the key authenticates as.
    NamedAgents,
    /// Gates the driver behavior that writes GitHub credentials to disk
    /// (`~/.git-credentials`, `~/.config/gh/hosts.yaml`) and runs the
    /// background refresh loop that keeps them fresh during a task run.
    GitCredentialRefresh,

    /// Gates the v2 billing and usage page redesign.
    BillingAndUsagePageV2,
    /// Enables configurable expanded context windows for eligible GPT models.
    GPTConfigurableContextWindow,

    /// Replaces the raw harness CLI command with a styled header showing CLI name + status icon.
    HarnessSessionHeader,

    /// Enables the code review view for remote sessions.
    RemoteCodeReview,

    /// Gates the Grouped Tabs feature.
    GroupedTabs,

    /// Gates the Pinned Tabs feature, which lets users pin individual tabs
    /// and whole tab groups so they stay at the front of the tab list and
    /// are protected from reordering.
    PinnedTabs,

    /// Gates the SuperGrok feature, which lets users
    /// connect a Grok subscription instead of pasting an API key.
    SuperGrok,

    /// Gates Gemini Enterprise (GEAP) BYOLLM, which lets users
    /// route eliglible models to GEAP instead of Warp-managed inference.
    GeminiEnterprise,

    /// Gates NLD input classification matching the buffer against agent
    /// prompt history (in addition to shell command history). Still in
    /// development, so enabled only for dev/dogfood builds.
    NldPromptHistoryMatch,

    /// Gates the custom model router feature, which allows users to define
    /// their own model routers.
    CustomModelRouters,

    /// Enables state-mutating recovery for abnormal terminal lifecycle sequences.
    TerminalLifecycleRecovery,

    /// Shows a warning in the agent view when the active conversation's
    /// provider-side prompt cache has expired.
    PromptCacheExpiryWarning,

    /// Enables the `--runner` flag on `run-cloud`, which overrides an agent's
    /// compute (docker image, instance shape, setup commands) by runner ID.
    CloudRunners,

    /// Renders MCP tool-call request and response JSON as an interactive
    /// collapsible tree with typed colors and per-row Copy JSON, instead of
    /// a flat pretty-printed blob.
    McpJsonTreeView,

    /// Test feature flag.
    MaggsTest,
}

static FLAG_STATES: [AtomicBool; cardinality::<FeatureFlag>()] =
    [const { AtomicBool::new(false) }; { cardinality::<FeatureFlag>() }];

/// This map is populated by UserPreferences, which take precedence
/// over the global feature flag state.
static USER_PREFERENCE_MAP: [AtomicTriState; cardinality::<FeatureFlag>()] =
    [const { AtomicTriState::new() }; { cardinality::<FeatureFlag>() }];

/// Flag for whether or not feature flags have been globally initialized. Outside
/// of tests, this ensures that feature flags are only used after they're set
/// up by the app's `run_internal` function.
#[cfg(debug_assertions)]
static FEATURES_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Features used in debugging.
pub const DEBUG_FLAGS: &[FeatureFlag] = &[FeatureFlag::DebugMode, FeatureFlag::RuntimeFeatureFlags];
/// Features enabled only for the WarpLocal developer build.
pub const LOCAL_FLAGS: &[FeatureFlag] = &[FeatureFlag::LocalClaudeCodexChildHarnesses];

/// Features enabled for the development team.  The expectation is that, over
/// time, these will move on to PREVIEW_FLAGS before being launched.
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::LogExpensiveFramesInSentry,
    FeatureFlag::ToggleBootstrapBlock,
    FeatureFlag::CreatingSharedSessions,
    FeatureFlag::RemoveAutosuggestionDuringTabCompletions,
    FeatureFlag::ResizeFix,
    FeatureFlag::AgentModeWorkflows,
    FeatureFlag::AgentModeAnalytics,
    FeatureFlag::LazySceneBuilding,
    FeatureFlag::SshDragAndDrop,
    FeatureFlag::MultiWorkspace,
    FeatureFlag::ImeMarkedText,
    FeatureFlag::MSYS2Shells,
    FeatureFlag::RetryTruncatedCodeResponses,
    FeatureFlag::ContextLineReviewComments,
    FeatureFlag::RunGeneratorsWithCmdExe,
    FeatureFlag::Projects,
    FeatureFlag::ProviderCommand,
    FeatureFlag::MarkdownImages,
    FeatureFlag::FileAndDiffSetComments,
    FeatureFlag::FileGlobV2Warnings,
    FeatureFlag::SummarizationViaMessageReplacement,
    FeatureFlag::LocalComputerUse,
    FeatureFlag::VideoRecording,
    FeatureFlag::OzLaunchModal,
    // These are enabled via 100% experiment on prod warp-server,
    // but we need to enable here for dogfood builds.
    FeatureFlag::CrossRepoContext,
    FeatureFlag::CodebaseIndexPersistence,
    FeatureFlag::FullSourceCodeEmbedding,
    FeatureFlag::CodebaseIndexSpeedbump,
    // End manually enabled Code features.
    FeatureFlag::EditableMarkdownMermaid,
    FeatureFlag::CodeReviewScrollPreservation,
    FeatureFlag::RememberFastForwardState,
    FeatureFlag::GeminiNotifications,
    FeatureFlag::LocalDockerSandbox,
    #[cfg(not(windows))]
    FeatureFlag::SshRemoteServer,
    FeatureFlag::RemoteCodebaseIndexing,
    FeatureFlag::GPTConfigurableContextWindow,
    FeatureFlag::RestorePromptOnInlineModelSelectorSearch,
    FeatureFlag::WarpControlCli,
    FeatureFlag::NldPromptHistoryMatch,
    FeatureFlag::TerminalLifecycleRecovery,
    FeatureFlag::PromptCacheExpiryWarning,
    FeatureFlag::BackgroundComputerUse,
    FeatureFlag::ContextWindowUsageBreakdown,
    FeatureFlag::JupyterNotebookRendering,
    FeatureFlag::CloudRunners,
    FeatureFlag::WaitForEventsParentRegistration,
    FeatureFlag::McpJsonTreeView,
];

/// Features enabled for feature preview build users (e.g.: Friends of Warp).
/// All PREVIEW_FLAGS are also automatically added to dogfood builds (WarpDev).
pub const PREVIEW_FLAGS: &[FeatureFlag] = &[FeatureFlag::AsyncFind, FeatureFlag::PinnedTabs];

/// Features enabled for all release builds (i.e.: everything but WarpLocal).
/// NOTE: if you are promoting a feature from Preview to launch, you'll likely
/// want to enable the feature by default in app/Cargo.toml, rather than add it to RELEASE_FLAGS.
pub const RELEASE_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::Autoupdate,
    FeatureFlag::Changelog,
    FeatureFlag::CrashReporting,
    // Marked text is currently only supported on MacOS.
    #[cfg(target_os = "macos")]
    FeatureFlag::ImeMarkedText,
    // Remote server binary is not yet supported on Windows.
    #[cfg(not(windows))]
    FeatureFlag::SshRemoteServer,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    FeatureFlag::DragTabsToWindows,
];

/// Flags that we want to allow to switch at runtime (assuming RuntimeFeatureFlags is set)
pub const RUNTIME_FEATURE_FLAGS: &[FeatureFlag] = &[FeatureFlag::LocalClaudeCodexChildHarnesses];

impl FeatureFlag {
    pub fn is_enabled(&self) -> bool {
        #[cfg(all(debug_assertions, not(feature = "test-util")))]
        {
            use std::sync::atomic::Ordering;
            assert!(
                FEATURES_INITIALIZED.load(Ordering::Relaxed),
                "Tried to check FeatureFlag::{self:?} before feature flags were initialized"
            );
        }

        overrides::get_override(*self)
            .or(USER_PREFERENCE_MAP[*self as usize].get())
            .or(Some(FLAG_STATES[*self as usize].load(Ordering::Relaxed)))
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn set_enabled(self, enabled: bool) {
        // Allow calling this in integration tests because we sometimes use it in the app
        // during flows that integration tests cover.
        if cfg!(test) && cfg!(not(feature = "integration_tests")) {
            panic!(
                "Tried to globally enable {self:?} in a test. Use FeatureFlag::{self:?}.override_enabled instead"
            );
        }
        FLAG_STATES[self as usize].store(enabled, Ordering::Relaxed);
    }

    /// Sets a user preference for this flag. User preferences take precedence
    /// over the global feature flag state, and can be used to allow explicit opt-in
    /// and explicit opt-out behavior.
    pub fn set_user_preference(self, enabled: bool) {
        USER_PREFERENCE_MAP[self as usize].set(enabled);
    }

    /// Sets a thread-local test override for this flag. The override lasts
    /// until the returned guard is dropped.
    ///
    /// **Warning**: overrides do not work for tests of multi-threaded code. If
    /// you need to test multi-threaded code that's behind a feature flag, you'll
    /// need to set an override in _each_ thread.
    ///
    /// Tests should create overrides early on and allow them to be
    /// dropped automatically when they finish. This keeps overrides scoped to
    /// the duration of the test, since Rust doesn't have test lifecycle hooks.
    #[cfg(feature = "test-util")]
    pub fn override_enabled(self, enabled: bool) -> overrides::OverrideGuard {
        overrides::override_flag(self, enabled)
    }

    pub fn flag_description(&self) -> Option<&'static str> {
        use FeatureFlag::*;

        // Note: many feature flags are purposefully omitted from this list, in order to avoid blowing up
        // the Preview changelog. Features below which are enabled for Preview via PREVIEW_FLAGS, will be added to the changelog.
        // Features which are added to Stable should ideally have their feature flag removed entirely, but at the
        // very least, the feature flag should be removed from the Preview changelog by removing it from PREVIEW_FLAGS.
        // ** ONLY Preview-exclusive features should be added to this list! **
        match self {
            AgentSharedSessions => {
                Some("Enables viewing agent conversations within shared sessions.")
            }
            CodeReviewFind => Some("Enables the find bar in the code review pane."),
            BlocklistMarkdownImages => {
                Some("Enables rendering markdown images inline in AI block list responses.")
            }
            CloudEnvironments => {
                Some("Enables creating and managing Warp Environments via the CLI.")
            }
            CreateEnvironmentSlashCommand => Some(
                "Enables the /create environment slash command for setting up Warp Environments with custom configurations.",
            ),
            GlobalSearch => Some("Enables global search in the left panel"),
            BlocklistMarkdownTableRendering => {
                Some("Enables rendering markdown tables inline in AI block list responses.")
            }
            MarkdownTables => {
                Some("Enables rendering and interaction support for markdown tables in notebooks.")
            }
            SettingsFile => Some(
                "Enables configuring Warp via a user-editable `settings.toml` file, with hot reload and error reporting for invalid values.",
            ),
            GitOperationsInCodeReview => Some(
                "Enables commit, push, and create-PR actions directly from the code review panel.",
            ),
            PinnedTabs => Some("Enables pinning individual tabs and tab groups to the front of the tab bar."),
            AsyncFind => Some(
                "Runs terminal find on a background thread to keep the UI responsive while searching large outputs.",
            ),
            _ => None,
        }
    }
}

/// Marks that feature flags have been globally initialized.
pub fn mark_initialized() {
    #[cfg(debug_assertions)]
    FEATURES_INITIALIZED.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(feature = "test-util"))]
mod overrides {
    #[inline(always)]
    pub fn get_override(_flag: super::FeatureFlag) -> Option<bool> {
        None
    }
}

/// Thread-local feature flag overrides for unit tests. For isolation, tests
/// should use overrides instead of globally modifying flags with [`super::FeatureFlag::set_enabled`].
#[cfg(feature = "test-util")]
mod overrides {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::FeatureFlag;

    thread_local! {
        static FLAG_OVERRIDES: RefCell<HashMap<FeatureFlag,bool>> = RefCell::new(HashMap::new());
    }

    /// RAII guard to set feature flag overrides in tests. When the guard is
    /// dropped, it reverts to the global flag state.
    #[must_use = "if unused the override will be immediately cleared"]
    pub struct OverrideGuard {
        flag: FeatureFlag,
    }

    /// Gets the overridden state for a flag, if set.
    pub fn get_override(flag: FeatureFlag) -> Option<bool> {
        FLAG_OVERRIDES.with(|overrides| overrides.borrow().get(&flag).copied())
    }

    /// Gets the set of overridden flags.
    pub fn get_overrides() -> HashMap<FeatureFlag, bool> {
        FLAG_OVERRIDES.with(|overrides| overrides.borrow().clone())
    }

    /// Applies a set of overrides.
    ///
    /// This is intended to be used with [`get_overrides`] to apply a set of
    /// existing overrides to a newly-spawned thread.  If you are trying to
    /// override a single feature flag, use [`FeatureFlag::override_enabled`]
    /// instead.
    pub fn set_overrides(new_overrides: HashMap<FeatureFlag, bool>) {
        FLAG_OVERRIDES.with(|overrides| *overrides.borrow_mut() = new_overrides);
    }

    /// Set a thread-local override for a flag.
    pub fn override_flag(flag: FeatureFlag, enabled: bool) -> OverrideGuard {
        set_override(flag, enabled);
        OverrideGuard { flag }
    }

    fn set_override(flag: FeatureFlag, enabled: bool) {
        FLAG_OVERRIDES.with(|overrides| {
            let previous = overrides.borrow_mut().insert(flag, enabled);
            // We could support nested overrides, but it requires some care around
            // out-of-order drops - if overrides are set and then cleared out of
            // order, what should the state after each drop be?
            if previous.is_some() {
                panic!("Multiple overrides set for {flag:?}");
            }
        });
    }

    fn clear_override(flag: FeatureFlag) {
        FLAG_OVERRIDES.with(|overrides| {
            let previous = overrides.borrow_mut().remove(&flag);
            if previous.is_none() {
                panic!("Cleared override for {flag:?}, but none was set");
            }
        });
    }

    impl Drop for OverrideGuard {
        fn drop(&mut self) {
            clear_override(self.flag);
        }
    }
}

/// An atomic tri-state value.
///
/// This is initially unset, and can be set to a true or false value.
///
/// Writes and reads use [`Ordering::Relaxed`], so should not be used for
/// synchronization.
struct AtomicTriState(AtomicU8);

impl AtomicTriState {
    const fn new() -> Self {
        Self(AtomicU8::new(TriState::Unset as u8))
    }

    fn get(&self) -> Option<bool> {
        TriState::from(self.0.load(Ordering::Relaxed)).into()
    }

    fn set(&self, value: bool) {
        self.0.store(TriState::from(value) as u8, Ordering::Relaxed);
    }
}

/// A simple enum representing a tristate, to be used as the backing type
/// for [`AtomicTriState`].
enum TriState {
    Unset = 0,
    False = 1,
    True = 2,
}

impl From<bool> for TriState {
    fn from(value: bool) -> Self {
        if value {
            TriState::True
        } else {
            TriState::False
        }
    }
}

impl From<u8> for TriState {
    fn from(value: u8) -> Self {
        match value {
            0 => TriState::Unset,
            1 => TriState::False,
            2 => TriState::True,
            _ => unreachable!(),
        }
    }
}

impl From<TriState> for Option<bool> {
    fn from(value: TriState) -> Self {
        match value {
            TriState::Unset => None,
            TriState::False => Some(false),
            TriState::True => Some(true),
        }
    }
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
