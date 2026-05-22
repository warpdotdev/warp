use crate::settings::{DisplayLanguage, LanguageSettings};
use settings::Setting as _;
use std::borrow::Cow;
use strum_macros::EnumIter;
use warpui::{AppContext, SingletonEntity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    EnUs,
    ZhCn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, EnumIter)]
pub enum I18nKey {
    SettingsSearchPlaceholder,
    SettingsSearchNoResultsTitle,
    SettingsSearchNoResultsDescription,
    SettingsNavAbout,
    SettingsNavAccount,
    SettingsNavAgents,
    SettingsNavBillingAndUsage,
    SettingsNavCode,
    SettingsNavCloudPlatform,
    SettingsNavTeams,
    SettingsNavAppearance,
    SettingsNavFeatures,
    SettingsNavKeybindings,
    SettingsNavWarpify,
    SettingsNavReferrals,
    SettingsNavSharedBlocks,
    SettingsNavWarpDrive,
    SettingsNavPrivacy,
    SettingsNavMcpServers,
    SettingsNavWarpAgent,
    SettingsNavAgentProfiles,
    SettingsNavAgentMcpServers,
    SettingsNavKnowledge,
    SettingsNavThirdPartyCliAgents,
    SettingsNavCodeIndexing,
    SettingsNavEditorAndCodeReview,
    SettingsNavCloudEnvironments,
    SettingsNavOzCloudApiKeys,
    TeamsCreateTeam,
    TeamsJoinExistingTeam,
    TeamsTeamMembers,
    TeamsInviteByLink,
    TeamsInviteByEmail,
    TeamsRestrictByDomain,
    TeamsMakeTeamDiscoverable,
    TeamsFreePlanUsageLimits,
    TeamsPlanUsageLimits,
    TeamsSharedNotebooks,
    TeamsSharedWorkflows,
    TeamsNeedMoreSeatsPrefix,
    TeamsUpgradeToEnterprise,
    TeamsUpgradeToBusiness,
    TeamsUnlimitedTeamMemberLimitSuffix,
    TeamsHigherTeamMemberLimitSuffix,
    MenuWhatsNew,
    MenuSettings,
    MenuKeyboardShortcuts,
    MenuDocumentation,
    MenuFeedback,
    MenuViewWarpLogs,
    MenuSlack,
    MenuSignUp,
    MenuBillingAndUsage,
    MenuUpgrade,
    MenuInviteAFriend,
    MenuLogOut,
    MenuUpdateAndRelaunchWarp,
    MenuUpdatingTo,
    MenuUpdateWarpManually,
    MenuRearrangeToolbarItems,
    MenuNewWindow,
    MenuNewTerminalTab,
    MenuNewAgentTab,
    MenuPreferences,
    MenuPrivacyPolicy,
    MenuDebug,
    MenuSetDefaultTerminal,
    MenuFile,
    MenuOpenRecent,
    MenuLaunchConfigurations,
    MenuSaveNew,
    MenuOpenRepository,
    MenuCloseFocusedPanel,
    MenuCloseWindow,
    MenuEdit,
    MenuUseWarpsPrompt,
    MenuCopyOnSelectWithinTerminal,
    MenuSynchronizeInputs,
    MenuView,
    MenuToggleMouseReporting,
    MenuToggleScrollReporting,
    MenuToggleFocusReporting,
    MenuCompactMode,
    MenuTab,
    MenuAi,
    MenuBlocks,
    MenuDrive,
    MenuWindow,
    MenuHelp,
    MenuSendFeedback,
    MenuWarpDocumentation,
    MenuGithubIssues,
    MenuWarpSlackCommunity,
    CommonAgent,
    CommonTerminal,
    CommonCloudOz,
    CommonLocalDockerSandbox,
    CommonNewWorktreeConfig,
    CommonNewTabConfig,
    CommonReopenClosedSession,
    CommonSplitPaneRight,
    CommonSplitPaneLeft,
    CommonSplitPaneDown,
    CommonSplitPaneUp,
    CommonClosePane,
    CommonCopyLink,
    CommonRename,
    CommonShare,
    CommonCollapseAll,
    CommonRetry,
    CommonRevertToServer,
    CommonAttachToActiveSession,
    CommonCopyId,
    CommonCopyVariables,
    CommonLoadInSubshell,
    CommonOpenOnDesktop,
    CommonDuplicate,
    CommonExport,
    CommonTrash,
    CommonOpen,
    CommonEdit,
    CommonRestore,
    CommonDeleteForever,
    CommonUntitled,
    CommonRefresh,
    CommonCancel,
    CommonDelete,
    CommonReject,
    CommonOverwrite,
    CommonPrevious,
    CommonNext,
    CommonUndo,
    CommonRemove,
    CommonSave,
    CommonVariables,
    CommonEditing,
    CommonViewing,
    CommonMoveTo,
    TabStopSharing,
    TabShareSession,
    TabStopSharingAll,
    TabRenameTab,
    TabResetTabName,
    TabMoveTabDown,
    TabMoveTabRight,
    TabMoveTabUp,
    TabMoveTabLeft,
    TabCloseTab,
    TabCloseOtherTabs,
    TabCloseTabsBelow,
    TabCloseTabsRight,
    TabSaveAsNewConfig,
    CommonCopy,
    CommonCut,
    CommonPaste,
    CommonSelectAll,
    CommonCopyPath,
    CommonOpenInWarp,
    CommonOpenInEditor,
    CommonShowInFinder,
    CommonShowContainingFolder,
    TerminalCopyUrl,
    TerminalInsertIntoInput,
    TerminalCopyCommand,
    TerminalCopyCommands,
    TerminalFindWithinBlock,
    TerminalFindWithinBlocks,
    TerminalScrollToTopOfBlock,
    TerminalScrollToTopOfBlocks,
    TerminalScrollToBottomOfBlock,
    TerminalScrollToBottomOfBlocks,
    TerminalShare,
    TerminalShareBlock,
    TerminalSaveAsWorkflow,
    TerminalAskWarpAi,
    TerminalCopyOutput,
    TerminalCopyFilteredOutput,
    TerminalToggleBlockFilter,
    TerminalToggleBookmark,
    TerminalForkFromHere,
    TerminalRewindToBeforeHere,
    TerminalCopyPrompt,
    TerminalCopyRightPrompt,
    TerminalCopyWorkingDirectory,
    TerminalCopyGitBranch,
    TerminalEditCliAgentToolbelt,
    TerminalEditAgentToolbelt,
    TerminalEditPrompt,
    TerminalCommandSearch,
    TerminalAiCommandSearch,
    TerminalCopyOutputAsMarkdown,
    TerminalSaveAsPrompt,
    TerminalShareConversation,
    TerminalCopyConversationText,
    DriveFolder,
    DriveNotebook,
    DriveWorkflow,
    DrivePrompt,
    DriveEnvironmentVariables,
    DriveNewFolder,
    DriveNewNotebook,
    DriveNewWorkflow,
    DriveNewPrompt,
    DriveNewEnvironmentVariables,
    DriveImport,
    DriveRemove,
    DriveEmptyTrash,
    DriveTrashTitle,
    DriveCreateTeamText,
    DriveTeamZeroStateText,
    DriveOfflineBannerText,
    DriveCopyPrompt,
    DriveCopyWorkflowText,
    DriveCopyToPersonal,
    DriveCopyAll,
    DriveRestoreNotebookTooltip,
    DriveCopyNotebookToPersonalTooltip,
    DriveCopyNotebookToClipboardTooltip,
    DriveRestoreWorkflowTooltip,
    DriveRefreshNotebookTooltip,
    WorkflowSignInToEditTooltip,
    EnvVarsCommand,
    EnvVarsClearSecret,
    EnvVarsNoAccess,
    EnvVarsMovedToTrash,
    EnvVarsRestoreTooltip,
    CodeHunk,
    CodeOpenInNewPane,
    CodeOpenInNewTab,
    CodeOpenFile,
    CodeNewFile,
    CodeCdToDirectory,
    CodeRevealInFinder,
    CodeRevealInExplorer,
    CodeRevealInFileManager,
    CodeAttachAsContext,
    CodeCopyRelativePath,
    CodeOpeningUnavailableRemoteTooltip,
    CodeGoToDefinition,
    CodeFindReferences,
    CodeDiscardThisVersion,
    CodeAcceptAndSave,
    CodeCloseSaved,
    CodeCopyFilePath,
    CodeViewMarkdownPreview,
    CodeSearchDiffSetsPlaceholder,
    CodeLineNumberColumnPlaceholder,
    CodeReviewCopyText,
    CodeReviewSendToAgent,
    CodeReviewViewInGithub,
    CodeReviewOneComment,
    CodeReviewCommentSingular,
    CodeReviewCommentPlural,
    CodeReviewOutdatedCommentSingular,
    CodeReviewOutdatedCommentPlural,
    CodeReviewCommit,
    CodeReviewPush,
    CodeReviewPublish,
    CodeReviewCreatePr,
    CodeReviewAddDiffSetAsContext,
    CodeReviewAddFileDiffAsContext,
    CodeReviewShowSavedComment,
    CodeReviewAddComment,
    CodeReviewDiscardAll,
    CodeReviewStashChanges,
    CodeReviewNoChangesToCommit,
    CodeReviewNoGitActionsAvailable,
    CodeReviewMaximize,
    CodeReviewShowFileNavigation,
    CodeReviewHideFileNavigation,
    CodeReviewInitializeCodebase,
    CodeReviewInitializeCodebaseTooltip,
    CodeReviewOpenRepository,
    CodeReviewOpenRepositoryTooltip,
    CodeReviewDiscardChanges,
    CodeReviewFileLevelCommentsCantBeEdited,
    CodeReviewOutdatedCommentsCantBeEdited,
    SettingsPageTitleAccount,
    SettingsPageTitlePrivacy,
    SettingsPageTitleMcpServers,
    SettingsPageTitleInviteFriend,
    SettingsCategoryBillingAndUsage,
    SettingsCategoryFeaturesGeneral,
    SettingsCategoryFeaturesSession,
    SettingsCategoryFeaturesKeys,
    SettingsCategoryFeaturesTextEditing,
    SettingsCategoryFeaturesTerminalInput,
    SettingsCategoryFeaturesTerminal,
    SettingsCategoryFeaturesNotifications,
    SettingsCategoryFeaturesWorkflows,
    SettingsCategoryFeaturesSystem,
    SettingsCategoryCodebaseIndexing,
    SettingsCategoryCodeEditorAndReview,
    SettingsCategoryWarpifySubshells,
    SettingsCategoryWarpifySubshellsDescription,
    SettingsCategoryWarpifySsh,
    SettingsCategoryWarpifySshDescription,
    WarpDriveSignUpPrompt,
    WarpDriveSignUpButton,
    WarpDriveLabel,
    WarpDriveDescription,
    WarpifyTitle,
    WarpifyDescription,
    WarpifyLearnMore,
    WarpifyCommandPlaceholder,
    WarpifyHostPlaceholder,
    WarpifyAddedCommands,
    WarpifyDenylistedCommands,
    WarpifySshSessions,
    WarpifyInstallSshExtension,
    WarpifyInstallSshExtensionDescription,
    WarpifyUseTmuxWarpification,
    WarpifyTmuxWarpificationDescription,
    WarpifyDenylistedHosts,
    WarpifyInstallModeAlwaysAsk,
    WarpifyInstallModeAlwaysInstall,
    WarpifyInstallModeNeverInstall,
    AccountSignUp,
    AccountFreePlan,
    AccountComparePlans,
    AccountContactSupport,
    AccountManageBilling,
    AccountUpgradeTurboPlan,
    AccountUpgradeLightspeedPlan,
    AccountSettingsSync,
    AccountReferralCta,
    AccountReferFriend,
    AccountVersion,
    AccountLogOut,
    AccountUpdateUpToDate,
    AccountUpdateCheckForUpdates,
    AccountUpdateChecking,
    AccountUpdateDownloading,
    AccountUpdateAvailable,
    AccountUpdateRelaunchWarp,
    AccountUpdateUpdating,
    AccountUpdateInstalled,
    AccountUpdateUnableToInstall,
    AccountUpdateManual,
    AccountUpdateUnableToLaunch,
    PrivacySafeModeTitle,
    PrivacySafeModeDescription,
    PrivacyCustomRedactionTitle,
    PrivacyCustomRedactionDescription,
    PrivacyTelemetryTitle,
    PrivacyTelemetryDescriptionEnterprise,
    PrivacyTelemetryDescription,
    PrivacyTelemetryFreeTierNote,
    PrivacyTelemetryDocsLink,
    PrivacyDataManagementTitle,
    PrivacyDataManagementDescription,
    PrivacyDataManagementLink,
    PrivacyPolicyTitle,
    PrivacyPolicyLink,
    PrivacyAddRegexPatternTitle,
    PrivacySecretDisplayAsterisks,
    PrivacySecretDisplayStrikethrough,
    PrivacySecretDisplayAlwaysShow,
    PrivacyTabPersonal,
    PrivacyTabEnterprise,
    PrivacyEnterpriseReadonly,
    PrivacyNoEnterpriseRegexes,
    PrivacyRecommended,
    PrivacyAddAll,
    PrivacyEnabledByOrganization,
    PrivacySecretVisualMode,
    PrivacySecretVisualModeDescription,
    PrivacyAddRegex,
    PrivacyZdrTooltip,
    PrivacyManagedByOrganization,
    PrivacyCrashReportsTitle,
    PrivacyCrashReportsDescription,
    PrivacyCloudConversationStorageTitle,
    PrivacyCloudConversationStorageEnabledDescription,
    PrivacyCloudConversationStorageDisabledDescription,
    PrivacyNetworkLogTitle,
    PrivacyNetworkLogDescription,
    PrivacyNetworkLogLink,
    PrivacyModalNameOptional,
    PrivacyModalNamePlaceholder,
    PrivacyModalRegexPattern,
    PrivacyModalInvalidRegex,
    PrivacyModalCancel,
    CodeIndexNewFolder,
    CodeFeatureName,
    CodeInitializationSettingsHeader,
    CodebaseIndexingLabel,
    CodebaseIndexingDescription,
    CodeWarpIndexingIgnoreDescription,
    CodeAutoIndexFeatureName,
    CodeAutoIndexDescription,
    CodeIndexingDisabledAdmin,
    CodeIndexingWorkspaceEnabledAdmin,
    CodeIndexingDisabledGlobalAi,
    CodebaseIndexLimitReached,
    CodeSubpageCodebaseIndexing,
    CodeSubpageEditorAndCodeReview,
    CodeInitializedFolders,
    CodeNoFoldersInitialized,
    CodeOpenProjectRules,
    CodeIndexingSection,
    CodeNoIndexCreated,
    CodeIndexDiscoveredChunks,
    CodeIndexSyncingProgress,
    CodeIndexSyncing,
    CodeIndexSynced,
    CodeIndexTooLarge,
    CodeIndexStale,
    CodeIndexFailed,
    CodeIndexNotBuilt,
    CodeLspServersSection,
    CodeLspInstalled,
    CodeLspInstalling,
    CodeLspChecking,
    CodeLspAvailableForDownload,
    CodeLspRestartServer,
    CodeLspViewLogs,
    CodeLspAvailable,
    CodeLspBusy,
    CodeLspStopped,
    CodeLspNotRunning,
    CodeAutoOpenCodeReviewPanel,
    CodeAutoOpenCodeReviewPanelDescription,
    CodeShowCodeReviewButton,
    CodeShowCodeReviewButtonDescription,
    CodeShowDiffStats,
    CodeShowDiffStatsDescription,
    CodeProjectExplorer,
    CodeProjectExplorerDescription,
    CodeGlobalFileSearch,
    CodeGlobalFileSearchDescription,
    CodeExternalDefaultApp,
    CodeExternalSplitPane,
    CodeExternalNewTab,
    CodeExternalChooseFileLinksEditor,
    CodeExternalChooseCodePanelEditor,
    CodeExternalChooseLayout,
    CodeExternalTabbedFileViewer,
    CodeExternalTabbedFileViewerDescription,
    CodeExternalMarkdownViewerDefault,
    KeybindingsSearchPlaceholder,
    KeybindingsConflictWarning,
    KeybindingsDefault,
    KeybindingsCancel,
    KeybindingsClear,
    KeybindingsSave,
    KeybindingsPressNewShortcut,
    KeybindingsDescription,
    KeybindingsUse,
    KeybindingsReferenceInSidePane,
    KeybindingsNotSyncedTooltip,
    KeybindingsConfigureTitle,
    KeybindingsCommandColumn,
    PlatformApiKeyDeletedToast,
    PlatformNewApiKeyTitle,
    PlatformSaveYourKeyTitle,
    PlatformApiKeysTitle,
    PlatformCreateApiKeyButton,
    PlatformDescriptionPrefix,
    PlatformDocumentationLink,
    PlatformHeaderName,
    PlatformHeaderKey,
    PlatformHeaderScope,
    PlatformHeaderCreated,
    PlatformHeaderLastUsed,
    PlatformHeaderExpiresAt,
    PlatformNever,
    PlatformScopePersonal,
    PlatformScopeTeam,
    PlatformNoApiKeys,
    PlatformNoApiKeysDescription,
    PlatformApiKeyTypePersonalDescription,
    PlatformApiKeyTypeTeamDescription,
    PlatformExpirationOneDay,
    PlatformExpirationThirtyDays,
    PlatformExpirationNinetyDays,
    PlatformExpirationNever,
    PlatformTeamKeyNoCurrentTeamError,
    PlatformCreateApiKeyFailed,
    PlatformSecretKeyInfo,
    PlatformCopied,
    PlatformCopy,
    PlatformDone,
    PlatformNameLabel,
    PlatformTypeLabel,
    PlatformExpirationLabel,
    PlatformCreating,
    PlatformCreateKey,
    PlatformSecretKeyCopiedToast,
    PlatformDeleteApiKeyFailed,
    FeaturesPinToTop,
    FeaturesPinToBottom,
    FeaturesPinToLeft,
    FeaturesPinToRight,
    FeaturesActiveScreen,
    FeaturesDefault,
    FeaturesNewTabAfterAllTabs,
    FeaturesNewTabAfterCurrentTab,
    FeaturesGlobalHotkeyDisabled,
    FeaturesGlobalHotkeyDedicatedWindow,
    FeaturesGlobalHotkeyShowHideAllWindows,
    FeaturesCtrlTabActivatePrevNextTab,
    FeaturesCtrlTabCycleMostRecentSession,
    FeaturesCtrlTabCycleMostRecentTab,
    FeaturesTabBehaviorOpenCompletions,
    FeaturesTabBehaviorAcceptAutosuggestion,
    FeaturesTabBehaviorUserDefined,
    FeaturesDefaultSessionTerminal,
    FeaturesDefaultSessionAgent,
    FeaturesDefaultSessionCloudAgent,
    FeaturesDefaultSessionTabConfig,
    FeaturesDefaultSessionDockerSandbox,
    FeaturesChangesApplyNewWindows,
    FeaturesCurrentBackend,
    FeaturesOpenLinksInDesktopApp,
    FeaturesOpenLinksInDesktopAppTooltip,
    FeaturesRestoreSession,
    FeaturesWaylandRestoreWarning,
    FeaturesSeeDocs,
    FeaturesStickyCommandHeader,
    FeaturesLinkTooltip,
    FeaturesQuitWarning,
    FeaturesStartWarpAtLoginMac,
    FeaturesStartWarpAtLogin,
    FeaturesQuitWhenAllWindowsClosed,
    FeaturesShowChangelogToast,
    FeaturesAllowedValuesOneToTwenty,
    FeaturesMouseScrollLines,
    FeaturesMouseScrollTooltip,
    FeaturesAutoOpenCodeReviewPanel,
    FeaturesAutoOpenCodeReviewPanelDescription,
    FeaturesWarpDefaultTerminal,
    FeaturesMakeWarpDefaultTerminal,
    FeaturesMaximumRowsInBlock,
    FeaturesBlockMaximumRowsDescription,
    FeaturesSshWrapper,
    FeaturesSshWrapperNewSessions,
    FeaturesReceiveDesktopNotifications,
    FeaturesNotifyAgentTaskCompleted,
    FeaturesNotifyCommandNeedsAttention,
    FeaturesPlayNotificationSounds,
    FeaturesShowInAppAgentNotifications,
    FeaturesNotificationWhenCommandLongerThan,
    FeaturesNotificationSecondsToComplete,
    FeaturesToastNotificationsStayVisibleFor,
    FeaturesSeconds,
    FeaturesDefaultShellForNewSessions,
    FeaturesWorkingDirectoryForNewSessions,
    FeaturesConfirmCloseSharedSession,
    FeaturesLeftOptionKeyMeta,
    FeaturesRightOptionKeyMeta,
    FeaturesLeftAltKeyMeta,
    FeaturesRightAltKeyMeta,
    FeaturesGlobalHotkeyLabel,
    FeaturesNotSupportedOnWayland,
    FeaturesWidthPercent,
    FeaturesHeightPercent,
    FeaturesAutohideKeyboardFocus,
    FeaturesKeybinding,
    FeaturesClickSetGlobalHotkey,
    FeaturesChangeKeybinding,
    FeaturesAutocompleteSymbols,
    FeaturesErrorUnderlining,
    FeaturesSyntaxHighlighting,
    FeaturesOpenCompletionsTyping,
    FeaturesSuggestCorrectedCommands,
    FeaturesExpandAliasesTyping,
    FeaturesMiddleClickPaste,
    FeaturesVimMode,
    FeaturesVimUnnamedRegisterClipboard,
    FeaturesVimStatusBar,
    FeaturesAtContextMenuTerminalMode,
    FeaturesSlashCommandsTerminalMode,
    FeaturesOutlineCodebaseSymbolsContextMenu,
    FeaturesShowTerminalInputMessageLine,
    FeaturesShowAutosuggestionKeybindingHint,
    FeaturesShowAutosuggestionIgnoreButton,
    FeaturesArrowAcceptsAutosuggestions,
    FeaturesKeyAcceptsAutosuggestions,
    FeaturesCompletionsOpenTyping,
    FeaturesCompletionsOpenTypingOrKey,
    FeaturesCompletionMenuUnbound,
    FeaturesKeyOpensCompletionMenu,
    FeaturesTabKeyBehavior,
    FeaturesCtrlTabBehaviorLabel,
    FeaturesEnableMouseReporting,
    FeaturesEnableScrollReporting,
    FeaturesEnableFocusReporting,
    FeaturesUseAudibleBell,
    FeaturesWordCharactersLabel,
    FeaturesDoubleClickSmartSelection,
    FeaturesShowHelpBlockNewSessions,
    FeaturesCopyOnSelect,
    FeaturesNewTabPlacement,
    FeaturesDefaultModeForNewSessions,
    FeaturesShowGlobalWorkflowsCommandSearch,
    FeaturesHonorLinuxSelectionClipboard,
    FeaturesLinuxSelectionClipboardTooltip,
    FeaturesPreferIntegratedGpu,
    FeaturesUseWaylandWindowManagement,
    FeaturesWaylandTooltip,
    FeaturesWaylandSecondaryText,
    FeaturesWaylandRestart,
    FeaturesPreferredGraphicsBackend,
    AiWarpAgentTitle,
    AiRemoteSessionOrgPolicy,
    AiSignUpPrompt,
    AiUsage,
    AiResetsDate,
    AiRestrictedBilling,
    AiUnlimited,
    AiUsageLimitDescription,
    AiCredits,
    AiUpgrade,
    AiComparePlans,
    AiContactSupport,
    AiUpgradeMoreUsage,
    AiForMoreUsage,
    AiActiveAi,
    AiNextCommand,
    AiNextCommandDescription,
    AiPromptSuggestions,
    AiPromptSuggestionsDescription,
    AiSuggestedCodeBanners,
    AiSuggestedCodeBannersDescription,
    AiNaturalLanguageAutosuggestions,
    AiNaturalLanguageAutosuggestionsDescription,
    AiSharedBlockTitleGeneration,
    AiSharedBlockTitleGenerationDescription,
    AiCommitPrGeneration,
    AiCommitPrGenerationDescription,
    AiAgents,
    AiAgentsDescription,
    AiProfiles,
    AiProfilesDescription,
    AiModels,
    AiContextWindowTokens,
    AiPermissions,
    AiApplyCodeDiffs,
    AiReadFiles,
    AiExecuteCommands,
    AiPermissionsManagedByWorkspace,
    AiInteractWithRunningCommands,
    AiCommandDenylist,
    AiCommandDenylistDescription,
    AiCommandAllowlist,
    AiCommandAllowlistDescription,
    AiDirectoryAllowlist,
    AiDirectoryAllowlistDescription,
    AiShowModelPickerInPrompt,
    AiBaseModel,
    AiBaseModelDescription,
    AiCodebaseContext,
    AiCodebaseContextDescription,
    AiLearnMore,
    AiCallMcpServers,
    AiMcpZeroStatePrefix,
    AiAddServer,
    AiMcpZeroStateMiddle,
    AiMcpZeroStateLearnMore,
    AiMcpAllowlist,
    AiMcpAllowlistDescription,
    AiMcpDenylist,
    AiMcpDenylistDescription,
    AiInput,
    AiShowInputHintText,
    AiShowAgentTips,
    AiIncludeAgentCommandsHistory,
    AiIncorrectDetectionPrefix,
    AiLetUsKnow,
    AiAutodetectAgentPrompts,
    AiAutodetectTerminalCommands,
    AiNaturalLanguageDetectionDescription,
    AiIncorrectInputDetectionPrefix,
    AiNaturalLanguageDetection,
    AiNaturalLanguageDenylist,
    AiNaturalLanguageDenylistDescription,
    AiMcpServers,
    AiMcpServersDescription,
    AiAutoSpawnThirdPartyServers,
    AiFileBasedMcpDescription,
    AiSeeSupportedProviders,
    AiManageMcpServers,
    AiRules,
    AiRulesDescription,
    AiSuggestedRules,
    AiSuggestedRulesDescription,
    AiWarpDriveAgentContext,
    AiWarpDriveAgentContextDescription,
    AiKnowledge,
    AiManageRules,
    AiVoiceInput,
    AiVoiceInputDescriptionPrefix,
    AiVoiceInputDescriptionSuffix,
    AiKeyForActivatingVoiceInput,
    AiPressAndHoldToActivate,
    AiVoice,
    AiOther,
    AiShowOzChangelog,
    AiShowUseAgentFooter,
    AiUseAgentFooterDescription,
    AiShowConversationHistory,
    AiAgentThinkingDisplay,
    AiAgentThinkingDisplayDescription,
    AiPreferredConversationLayout,
    AiThirdPartyCliAgents,
    AiShowCodingAgentToolbar,
    AiCodingAgentToolbarDescriptionPrefix,
    AiCodingAgentToolbarDescriptionMiddle,
    AiCodingAgentToolbarDescriptionSuffix,
    AiAutoShowHideRichInput,
    AiRequiresWarpPlugin,
    AiAutoOpenRichInput,
    AiAutoDismissRichInput,
    AiCommandsEnableToolbar,
    AiToolbarCommandDescription,
    AiOrgEnforcedTooltip,
    AiEnableAgentAttribution,
    AiAgentAttribution,
    AiAgentAttributionDescription,
    AiComputerUseCloudAgents,
    AiExperimental,
    AiCloudComputerUseDescription,
    AiOrchestration,
    AiOrchestrationDescription,
    AiApiKeys,
    AiByokDescription,
    AiOpenAiApiKey,
    AiAnthropicApiKey,
    AiGoogleApiKey,
    AiContactSales,
    AiContactSalesByok,
    AiUpgradeBuildPlan,
    AiUseOwnApiKeys,
    AiAskAdminUpgradeBuild,
    AiWarpCreditFallback,
    AiWarpCreditFallbackDescription,
    AiAwsBedrock,
    AiAwsManagedDescription,
    AiAwsDescription,
    AiUseAwsBedrockCredentials,
    AiLoginCommand,
    AiAwsProfile,
    AiAutoRunLoginCommand,
    AiAutoRunLoginDescription,
    AiRefresh,
    AiAddProfile,
    AiEdit,
    AiAuto,
    AiModelsUpper,
    AiPermissionsUpper,
    AiBaseModelColon,
    AiFullTerminalUseColon,
    AiComputerUseColon,
    AiApplyCodeDiffsColon,
    AiReadFilesColon,
    AiExecuteCommandsColon,
    AiInteractWithRunningCommandsColon,
    AiAskQuestionsColon,
    AiRunAgentsColon,
    AiCallMcpServersColon,
    AiCallWebToolsColon,
    AiAutoSyncPlansColon,
    AiNone,
    AiAgentDecides,
    AiAlwaysAllow,
    AiAlwaysAsk,
    AiUnknown,
    AiAskOnFirstWrite,
    AiNever,
    AiNeverAsk,
    AiAskUnlessAutoApprove,
    AiOn,
    AiOff,
    AiDirectoryAllowlistColon,
    AiCommandAllowlistColon,
    AiCommandDenylistColon,
    AiMcpAllowlistColon,
    AiMcpDenylistColon,
    AiSelectMcpServers,
    AiReadOnly,
    AiSupervised,
    AiAllowSpecificDirectories,
    AiDefault,
    AiDisabled,
    AiProfileDefault,
    AiFreePlanFrontierModelsUnavailable,
    AiProfileEditorTitle,
    AiProfileEditorName,
    AiDefaultProfileNameReadonly,
    AiDeleteProfile,
    AiDirectoryPathPlaceholder,
    AiCommandAllowPlaceholder,
    AiCommandDenyPlaceholder,
    AiCommandsCommaSeparatedPlaceholder,
    AiCommandRegexPlaceholder,
    AiProfileNamePlaceholder,
    AiFullTerminalUseModel,
    AiFullTerminalUseModelDescription,
    AiComputerUse,
    AiComputerUseModel,
    AiComputerUseModelDescription,
    AiContextWindow,
    AiContextWindowDescription,
    AiPermissionAgentDecidesDescription,
    AiPermissionAlwaysAllowDescription,
    AiPermissionAlwaysAskDescription,
    AiWriteToPtyAskOnFirstWriteDescription,
    AiWriteToPtyAlwaysAskDescription,
    AiComputerUseNeverDescription,
    AiComputerUseAlwaysAskDescription,
    AiComputerUseAlwaysAllowDescription,
    AiUnknownSettingDescription,
    AiAskQuestions,
    AiRunOrchestratedAgents,
    AiRunAgentsNeverDescription,
    AiRunAgentsAlwaysAllowDescription,
    AiRunAgentsAlwaysAskDescription,
    AiAskQuestionAskUnlessAutoApproveDescription,
    AiAskQuestionNeverDescription,
    AiAskQuestionAlwaysAskDescription,
    AiMcpAllowlistProfileDescription,
    AiMcpDenylistProfileDescription,
    AiPlanAutoSync,
    AiPlanAutoSyncDescription,
    AiCallWebTools,
    AiCallWebToolsDescription,
    AiToolbarLayout,
    AiSelectCodingAgent,
    AiShowAndCollapse,
    AiAlwaysShow,
    AiNeverShow,
    AiLeft,
    AiRight,
    AppearanceCategoryLanguage,
    AppearanceCategoryThemes,
    AppearanceCategoryIcon,
    AppearanceCategoryWindow,
    AppearanceCategoryInput,
    AppearanceCategoryPanes,
    AppearanceCategoryBlocks,
    AppearanceCategoryText,
    AppearanceCategoryCursor,
    AppearanceCategoryTabs,
    AppearanceCategoryFullscreenApps,
    AppearanceLanguageLabel,
    AppearanceLanguageDescription,
    AppearanceThemeCreateCustom,
    AppearanceThemeLight,
    AppearanceThemeDark,
    AppearanceThemeCurrent,
    AppearanceThemeSyncWithOs,
    AppearanceThemeSyncWithOsDescription,
    AppearanceAppIconLabel,
    AppearanceAppIconDefault,
    AppearanceAppIconBundleWarning,
    AppearanceAppIconRestartWarning,
    AppearanceWindowCustomSizeLabel,
    AppearanceWindowColumns,
    AppearanceWindowRows,
    AppearanceWindowOpacityLabel,
    AppearanceWindowTransparencyUnsupported,
    AppearanceWindowTransparencyWarning,
    AppearanceWindowTransparencySettingsSuggestion,
    AppearanceWindowBlurRadiusLabel,
    AppearanceWindowBlurTextureLabel,
    AppearanceWindowHardwareTransparencyWarning,
    AppearanceToolsPanelConsistentAcrossTabs,
    AppearanceInputTypeLabel,
    AppearanceInputRadioWarp,
    AppearanceInputRadioShellPs1,
    AppearanceInputPositionLabel,
    AppearancePanesDimInactive,
    AppearancePanesFocusFollowsMouse,
    AppearanceBlocksCompactMode,
    AppearanceBlocksJumpToBottomButton,
    AppearanceBlocksShowDividers,
    AppearanceTextAgentFont,
    AppearanceTextDefaultSuffix,
    AppearanceTextMatchTerminal,
    AppearanceTextLineHeight,
    AppearanceTextResetToDefault,
    AppearanceTextTerminalFont,
    AppearanceTextViewAllFonts,
    AppearanceTextFontWeight,
    AppearanceTextFontSizePx,
    AppearanceTextNotebookFontSize,
    AppearanceTextUseThinStrokes,
    AppearanceTextMinimumContrast,
    AppearanceTextShowLigatures,
    AppearanceTextLigaturesTooltip,
    AppearanceCursorType,
    AppearanceCursorTypeDisabledVim,
    AppearanceCursorBlinking,
    AppearanceCursorBar,
    AppearanceCursorBlock,
    AppearanceCursorUnderline,
    AppearanceTabsCloseButtonPosition,
    AppearanceTabsShowIndicators,
    AppearanceTabsShowCodeReviewButton,
    AppearanceTabsPreserveActiveColor,
    AppearanceTabsUseVerticalLayout,
    AppearanceTabsShowVerticalPanelRestored,
    AppearanceTabsShowVerticalPanelRestoredDescription,
    AppearanceTabsUseLatestPromptTitle,
    AppearanceTabsUseLatestPromptTitleDescription,
    AppearanceTabsHeaderToolbarLayout,
    AppearanceTabsDirectoryColors,
    AppearanceTabsDirectoryColorsDescription,
    AppearanceTabsDefaultNoColor,
    AppearanceTabsShowTabBar,
    AppearanceFullscreenUseCustomPadding,
    AppearanceFullscreenUniformPaddingPx,
    AppearanceZoomLabel,
    AppearanceZoomDescription,
    AppearanceInputModePinnedBottom,
    AppearanceInputModePinnedTop,
    AppearanceInputModeWaterfall,
    AppearanceOptionNever,
    AppearanceOptionLowDpiDisplays,
    AppearanceOptionHighDpiDisplays,
    AppearanceOptionAlways,
    AppearanceContrastOnlyNamedColors,
    AppearanceWorkspaceDecorationsAlways,
    AppearanceWorkspaceDecorationsWindowed,
    AppearanceWorkspaceDecorationsHover,
    AppearanceOptionRight,
    AppearanceOptionLeft,
    LanguagePreferenceSystem,
    LanguagePreferenceEnglish,
    LanguagePreferenceChineseSimplified,
}

pub fn current_locale(app: &AppContext) -> Locale {
    match *LanguageSettings::as_ref(app).display_language.value() {
        DisplayLanguage::System => system_locale(),
        DisplayLanguage::English => Locale::EnUs,
        DisplayLanguage::ChineseSimplified => Locale::ZhCn,
    }
}

pub fn tr(app: &AppContext, key: I18nKey) -> &'static str {
    let locale = current_locale(app);
    let value = match locale {
        Locale::EnUs => en_us(key),
        Locale::ZhCn => zh_cn(key),
    };
    debug_assert!(
        !value.trim().is_empty(),
        "missing i18n translation for {key:?} in {locale:?}"
    );
    value
}

pub fn tr_static(app: &AppContext, text: &'static str) -> &'static str {
    match current_locale(app) {
        Locale::EnUs => text,
        Locale::ZhCn => zh_cn_static(text).unwrap_or(text),
    }
}

pub fn tr_text<'a>(app: &AppContext, text: &'a str) -> Cow<'a, str> {
    match current_locale(app) {
        Locale::EnUs => Cow::Borrowed(text),
        Locale::ZhCn => Cow::Borrowed(zh_cn_static(text).unwrap_or(text)),
    }
}

fn system_locale() -> Locale {
    #[cfg(target_os = "macos")]
    if let Some(locale) = macos_system_locale() {
        return locale;
    }

    let lang = std::env::var("LANG")
        .unwrap_or_default()
        .to_ascii_lowercase();
    locale_from_identifier(&lang)
}

fn locale_from_identifier(identifier: &str) -> Locale {
    if identifier.to_ascii_lowercase().starts_with("zh") {
        Locale::ZhCn
    } else {
        Locale::EnUs
    }
}

#[cfg(target_os = "macos")]
fn macos_system_locale() -> Option<Locale> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use warpui::platform::mac::utils::nsstring_as_str;

    unsafe {
        let locale_class = class!(NSLocale);
        let locale: *const Object = msg_send![locale_class, currentLocale];
        if locale.is_null() {
            return None;
        }

        let is_language_code_supported: bool =
            msg_send![locale, respondsToSelector: sel!(languageCode)];
        let locale_identifier: *const Object = if is_language_code_supported {
            msg_send![locale, languageCode]
        } else {
            msg_send![locale, localeIdentifier]
        };

        if locale_identifier.is_null() {
            return None;
        }

        let locale_identifier = nsstring_as_str(locale_identifier).ok()?;
        Some(locale_from_identifier(locale_identifier))
    }
}

fn zh_cn_static(text: &str) -> Option<&'static str> {
    Some(match text {
        "Accept" => "接受",
        "Add" => "添加",
        "Add a workflow argument" => "添加工作流参数",
        "Add diff set as context" => "将 diff 集添加为上下文",
        "Add environment variables" => "添加环境变量",
        "Add file diff as context" => "将文件 diff 添加为上下文",
        "Add repo" => "添加仓库",
        "Advanced" => "高级",
        "Agent location" => "智能体位置",
        "Are you still there?" => "你还在吗？",
        "AWS CLI Not Installed" => "未安装 AWS CLI",
        "Beta" => "测试版",
        "Billed to API" => "按 API 计费",
        "Character limit exceeded." => "超出字符数限制。",
        "Command edited." => "命令已编辑。",
        "Comment imported from GitHub" => "从 GitHub 导入的评论",
        "Custom" => "自定义",
        "Custom endpoints" => "自定义端点",
        "Configure notifications" => "配置通知",
        "Cursor" => "光标",
        "Cycle suggestions" => "切换建议",
        "Debug output" => "调试输出",
        "Choose the language used by Warp's interface." => "选择 Warp 界面使用的语言。",
        "Display language" => "显示语言",
        "Edit in Warp" => "在 Warp 中编辑",
        "Enable" => "启用",
        "Enable Warp's Vim keybindings?" => "启用 Warp 的 Vim 快捷键？",
        "Enable alias expansion" => "启用别名展开",
        "Enter your credentials below." => "在下方输入你的凭据。",
        "From GitHub" => "来自 GitHub",
        "Full-screen Apps" => "全屏应用",
        "Grace period (seconds)" => "宽限期（秒）",
        "Icon" => "图标",
        "Increase it" => "提高上限",
        "Increase your limit" => "提高你的上限",
        "Input" => "输入",
        "Language" => "语言",
        "Learn about file support and formatting" => "了解文件支持和格式设置",
        "Learn More" => "了解更多",
        "Link to Documentation" => "查看文档",
        "Log into AWS" => "登录 AWS",
        "Loading prompt..." => "正在加载提示词...",
        "Manage AI Autonomy permissions" => "管理 AI 自主权限",
        "Manage plan" => "管理方案",
        "Manage suggested code banner settings" => "管理建议代码横幅设置",
        "Meet your new agent inbox" => "认识你的新智能体收件箱",
        "Monthly overage spending limit" => "月度超额支出上限",
        "NEW" => "新",
        "Never Warpify this host" => "永不 Warpify 此主机",
        "No URLs fetched" => "未获取到 URL",
        "No URLs found" => "未找到 URL",
        "No agents available. Create one first." => "没有可用智能体。请先创建一个。",
        "No code to be restored" => "没有可恢复的代码",
        "No matching workflows found." => "未找到匹配的工作流。",
        "Outdated" => "已过时",
        "Paste your auth token below" => "在下方粘贴你的认证令牌",
        "Project Skill" => "项目技能",
        "Provided API key is not valid" => "提供的 API 密钥无效",
        "Queued" => "已排队",
        "RECENT ACTIVITY" => "最近活动",
        "Recommended" => "推荐",
        "References" => "引用",
        "Reset" => "重置",
        "Reset links" => "重置链接",
        "Save Current Configuration" => "保存当前配置",
        "Set permissions" => "设置权限",
        "Shell process exited" => "Shell 进程已退出",
        "Shell process exited prematurely!" => "Shell 进程提前退出！",
        "Simplified Chinese" => "简体中文",
        "Skip (advanced)" => "跳过（高级）",
        "Suggestions:" => "建议：",
        "Switch back to horizontal tabs" => "切回水平标签页",
        "Themes" => "主题",
        "This is an automated agent on your team." => "这是你团队中的自动化智能体。",
        "This suggestion is being edited in another tab." => "此建议正在另一个标签页中编辑。",
        " to continue." => "以继续。",
        "Troubleshoot" => "故障排查",
        "Try " => "尝试",
        "Use AWS Bedrock?" => "使用 AWS Bedrock？",
        "Use system language" => "跟随系统语言",
        "View Context" => "查看上下文",
        "View details" => "查看详情",
        "View in Warp" => "在 Warp 中查看",
        "Warp's Terms of Service" => "Warp 服务条款",
        "Warp SSH wrapper disabled" => "Warp SSH 包装器已禁用",
        "Warp SSH wrapper enabled" => "Warp SSH 包装器已启用",
        "Warp can auto-expand aliases." => "Warp 可以自动展开别名。",
        "You're viewing a snapshot" => "你正在查看快照",
        "Your credentials are encrypted end-to-end. " => "你的凭据已进行端到端加密。",
        "You’re out of credits" => "你的点数已用完",
        "creating your own workflow" => "创建你自己的工作流",
        "this conversation" => "此对话",
        "to cycle parameters" => "切换参数",
        "troubleshooting docs" => "故障排查文档",
        "Add rule" => "添加规则",
        "Already the default" => "已是默认值",
        "Always allow" => "始终允许",
        "All" => "全部",
        "Ambient agent conversations cannot be deleted" => "无法删除环境智能体对话",
        "This conversation cannot be deleted" => "此对话无法删除",
        "This API key is tied to an agent and can make requests on behalf of the agent." => {
            "此 API 密钥绑定到智能体，可代表该智能体发起请求。"
        }
        "Anyone with the link" => "拥有链接的任何人",
        "Arguments" => "参数",
        "Apply link" => "应用链接",
        "Attach file" => "附加文件",
        "Attach to active session" => "附加到当前会话",
        "Authenticate with GitHub" => "通过 GitHub 认证",
        "Auto-approve" => "自动批准",
        "A new version of the Warp plugin is available" => "有新的 Warp 插件版本可用",
        "Ask the agent to check this command now, skipping its timer." => {
            "让智能体立即检查此命令，跳过计时器。"
        }
        "Agent" => "智能体",
        "Ask the Warp agent to assist" => "让 Warp 智能体协助",
        "Ask the Warp agent to resume" => "让 Warp 智能体恢复",
        "alias name" => "别名名称",
        "Bad response" => "不好的回复",
        "Base" => "基础",
        "By discovery" => "通过发现",
        "Buy more" => "购买更多",
        "Cancel" => "取消",
        "Cancel summarization" => "取消总结",
        "Change git branch" => "切换 Git 分支",
        "Change keybinding" => "更改快捷键",
        "Change role" => "更改角色",
        "Change your current theme." => "更改当前主题。",
        "Change working directory" => "更改工作目录",
        "Choose an AI execution profile" => "选择 AI 执行配置",
        "Choose an agent model" => "选择智能体模型",
        "Choose an environment" => "选择环境",
        "Cloud child agents using this harness require an API key before they can run." => {
            "使用此运行框架的云端子智能体需要 API Key 才能运行。"
        }
        "Clear all" => "全部清除",
        "Clear filters" => "清除筛选",
        "Clear upload" => "清除上传",
        "Close" => "关闭",
        "Close panel" => "关闭面板",
        "Close session" => "关闭会话",
        "Close Welcome Tips" => "关闭欢迎提示",
        "Cloud agent run" => "云端智能体运行",
        "Cloud agent" => "云端智能体",
        "Cloud-only" => "仅云端",
        "Collapse" => "折叠",
        "Comment" => "评论",
        "Complete!" => "已完成！",
        "Combined" => "合计",
        "Commit" => "提交",
        "Commit and create PR" => "提交并创建 PR",
        "Configure" => "配置",
        "Contact Admin to request access" => "联系管理员请求访问权限",
        "Contact support" => "联系支持",
        "Contact your team admin to resolve billing issues." => "请联系团队管理员解决账单问题。",
        "Contact your Account Executive for more add-on credits." => {
            "请联系你的客户经理获取更多附加点数。"
        }
        "Contact a team admin to enable add-on credits." => {
            "请联系团队管理员启用附加点数。"
        }
        "Context window usage" => "上下文窗口用量",
        "Confirm" => "确认",
        "Continue conversation" => "继续对话",
        "Continue locally" => "在本地继续",
        "Continue summarization" => "继续总结",
        "Continue without Warpification" => "不启用 Warpification 继续",
        "Copy error" => "复制错误",
        "Copy file path" => "复制文件路径",
        "Copy link" => "复制链接",
        "Copy plan ID" => "复制计划 ID",
        "Copy session sharing link" => "复制会话共享链接",
        "Copy transcript to clipboard" => "将转录复制到剪贴板",
        "Copy workflow text" => "复制工作流文本",
        "Code review panel" => "代码审查面板",
        "Cannot detect diffs for this folder" => "无法检测此文件夹的差异",
        "Choose a locally set up project and we’ll help you set up an environment based on it" => {
            "选择一个已在本地设置好的项目，我们会基于它帮你配置环境"
        }
        "Choose a locally set up project and we'll help you set up an environment based on it" => {
            "选择一个已在本地设置好的项目，我们会基于它帮你配置环境"
        }
        "Choose how you’d like to set up your environment:" => "选择你想如何设置环境：",
        "Choose how you'd like to set up your environment" => "选择你想如何设置环境",
        "Create" => "创建",
        "Create New Project" => "新建项目",
        "Create New Tab" => "新建标签页",
        "Create New Window" => "新建窗口",
        "Create or Edit Link" => "创建或编辑链接",
        "1 team member" => "1 位团队成员",
        "{count} team members" => "{count} 位团队成员",
        "Add-ons" => "附加额度",
        "Add-on credits are purchased in prepaid packages that roll over each billing cycle and expire after one year. The more you purchase, the better the per-credit rate. Once your base plan credits are used, add-on credits will be consumed." => {
            "附加点数以预付包购买，会在每个计费周期结转，并在一年后过期。购买越多，每点数价格越优惠。基础方案点数用完后，将消耗附加点数。"
        }
        "Purchased add-on credits are added to your personal balance." => {
            "购买的附加点数会添加到你的个人余额。"
        }
        "Auto-reload is enabled" => "自动补充已启用",
        "Auto reload is disabled due to recent failed reload. Please update your payment method and try again." => {
            "由于最近一次补充失败，自动补充已禁用。请更新付款方式后重试。"
        }
        "Auto reload is disabled due to recent failed reload. Contact your team admin to update their payment method." => {
            "由于最近一次补充失败，自动补充已禁用。请联系团队管理员更新付款方式。"
        }
        "Auto-reload is paused because the next reload would exceed your monthly spend limit. Increase your limit to continue using auto-reload." => {
            "自动补充已暂停，因为下一次补充会超出你的月度消费限额。提高限额后即可继续使用自动补充。"
        }
        "Auto-reload is paused because the next reload would exceed your team’s monthly spend limit. Contact a team admin to increase it." => {
            "自动补充已暂停，因为下一次补充会超出团队月度消费限额。请联系团队管理员提高限额。"
        }
        "Auto-reload enabled. We'll refill with {credits} credits when your balance runs low." => {
            "自动补充已启用。当余额较低时，我们会补充 {credits} 点数。"
        }
        "Auto-reload disabled." => "自动补充已禁用。",
        "Authorize" => "授权",
        "Buying…" => "购买中…",
        "Additional members are billed at your plan's per-user rate. {prorated_message}" => {
            "新增成员会按你当前方案的每用户价格计费。{prorated_message}"
        }
        "Additional members are billed at your plan's per-user rate: ${monthly_cost}/month or ${yearly_cost}/year, depending on your billing interval. {prorated_message}" => {
            "新增成员会按你当前方案的每用户价格计费：${monthly_cost}/月或 ${yearly_cost}/年，取决于你的计费周期。{prorated_message}"
        }
        "{credits} credits / {dollars}" => "{credits} 点数 / {dollars}",
        "{credits} credits" => "{credits} 点数",
        "Allow Warp users with an @{domain} email to find and join the team." => {
            "允许使用 @{domain} 邮箱的 Warp 用户查找并加入团队。"
        }
        "When any member on your team’s credit balance reaches 100 credits remaining, automatically purchase {auto_reload_credit_amount}." => {
            "当团队中任一成员的点数余额降至 100 点时，自动购买 {auto_reload_credit_amount}。"
        }
        "selected credit amount" => "所选点数数量",
        "your selected" => "你所选的",
        "Your plan ({plan_display}) has a maximum capacity of {cap} members." => {
            "你的方案（{plan_display}）最多支持 {cap} 位成员。"
        }
        "Your admin has enabled auto-reload for add-on credits. When your personal add-on credit balance runs low, Warp will automatically purchase {credits} credits for {price} and add them to your balance." => {
            "你的管理员已为附加点数启用自动补充。当你的个人附加点数余额较低时，Warp 会自动购买 {credits} 点数（{price}）并添加到你的余额。"
        }
        "Your admin has enabled auto-reload for add-on credits. When your personal add-on credit balance runs low, Warp will automatically purchase add-on credits and add them to your balance." => {
            "你的管理员已为附加点数启用自动补充。当你的个人附加点数余额较低时，Warp 会自动购买附加点数并添加到你的余额。"
        }
        "You'll be charged for a portion of the team member's usage of Warp." => {
            "你将为该团队成员使用 Warp 的部分用量付费。"
        }
        "Your admin will be charged for a portion of the team member's usage of Warp." => {
            "你的管理员将为该团队成员使用 Warp 的部分用量付费。"
        }
        "Diff stats" => "Diff 统计",
        "Create environment" => "创建环境",
        "Create team" => "创建团队",
        "Custom..." => "自定义...",
        "Current" => "当前",
        "Default" => "默认",
        "Created on" => "创建时间",
        "Delete" => "删除",
        "Delete environment" => "删除环境",
        "Delete MCP" => "删除 MCP",
        "Delete rule" => "删除规则",
        "Default value (optional)" => "默认值（可选）",
        "Description" => "描述",
        "Directory path" => "目录路径",
        "Dismiss" => "关闭",
        "Divider" => "分隔线",
        "Done" => "已完成",
        "Do not show again" => "不再显示",
        "Don't show again" => "不再显示",
        "Don't show me suggested code banners again" => "不再显示建议代码横幅",
        "Edit API Keys" => "编辑 API 密钥",
        "Edit Variables" => "编辑变量",
        "Edit config" => "编辑配置",
        "Edit" => "编辑",
        "Edit rule" => "编辑规则",
        "Editing" => "编辑中",
        "Embed" => "嵌入",
        "Enable notifications" => "启用通知",
        "Enter auth token" => "输入认证令牌",
        "Env ID" => "环境 ID",
        "Environment" => "环境",
        "Environment deleted successfully" => "环境已成功删除",
        "Environments" => "环境",
        "Environments define where your ambient agents run. Set one up in minutes via GitHub (recommended), Warp-assisted setup, or manual configuration." => {
            "环境定义你的后台智能体在哪里运行。可以通过 GitHub（推荐）、Warp 辅助设置或手动配置在几分钟内完成设置。"
        }
        "Environment variables" => "环境变量",
        "Executable path" => "可执行文件路径",
        "Expand" => "展开",
        "Failed" => "失败",
        "Failed to load agents. Please close and try again." => {
            "加载智能体失败。请关闭后重试。"
        }
        "Failed to share environment with team" => "无法与团队共享环境",
        "File" => "文件",
        "File explorer" => "文件浏览器",
        "Fill out the arguments in this workflow and copy it to run in your terminal session" => {
            "填写此工作流中的参数并复制，以便在终端会话中运行"
        }
        "Follow up with existing conversation" => "在现有对话中继续跟进",
        "Finish" => "完成",
        "Focused session" => "聚焦会话",
        "Fork conversation" => "派生对话",
        "Fork in new pane" => "在新窗格中派生",
        "Fork in new tab" => "在新标签页中派生",
        "Fork this conversation locally" => "在本地派生此对话",
        "Get started" => "开始使用",
        "Get Warping" => "开始使用 Warp",
        "Give control back to agent" => "将控制权交还给智能体",
        "Good response" => "好的回复",
        "Harness" => "运行环境",
        "Has artifact" => "包含产物",
        "Hide Rich Input" => "隐藏富输入",
        "Ignore this suggestion" => "忽略此建议",
        "Import" => "导入",
        "Image" => "镜像",
        "Initialize Project" => "初始化项目",
        "Insert block" => "插入块",
        "Install the Warp plugin to enable rich agent notifications within Warp" => {
            "安装 Warp 插件以在 Warp 中启用丰富的智能体通知"
        }
        "Install nvm" => "安装 nvm",
        "Enable Warp shell integration in this session" => {
            "在此会话中启用 Warp Shell 集成"
        }
        "Invite" => "邀请",
        "Last 24 hours" => "过去 24 小时",
        "Last edited" => "上次编辑",
        "Last used" => "上次使用",
        "Last week" => "上周",
        "Launch agent" => "启动智能体",
        "Learn more" => "了解更多",
        "Link (web or file)" => "链接（网页或文件）",
        "Link SSO" => "关联 SSO",
        "Load more" => "加载更多",
        "No environments match your search." => "没有匹配你搜索的环境。",
        "Loading..." => "正在加载...",
        "Log out" => "退出登录",
        "Looks like you're out of credits. " => "你的点数似乎已用完。",
        "Make default" => "设为默认",
        "Make editor" => "设为编辑者",
        "Make viewer" => "设为查看者",
        "Manage" => "管理",
        "Manage API keys" => "管理 API 密钥",
        "Manage billing" => "管理账单",
        "Manage defaults" => "管理默认值",
        "Manage privacy settings" => "管理隐私设置",
        "Manage profiles" => "管理配置",
        "Manage rules" => "管理规则",
        "Manage shared blocks" => "管理共享块",
        "Mark all as read" => "全部标为已读",
        "Maximize Warp" => "最大化发挥 Warp 的作用",
        "Maximize" => "最大化",
        "Missing source file" => "缺少源文件",
        "Monthly limit reached" => "已达到月度限制",
        "Monthly spend limit" => "月度消费限额",
        "New agent" => "新建智能体",
        "Navigate to a repo and initialize it for coding" => "前往仓库并初始化以进行编码",
        "Next" => "下一步",
        "Nice work on finishing the welcome tips!" => "欢迎提示已完成。",
        "Name" => "名称",
        "No matches found." => "未找到匹配项。",
        "No matching themes!" => "没有匹配的主题！",
        "No results found." => "未找到结果。",
        "No tabs open" => "没有打开的标签页",
        "None" => "无",
        "Not visible to other users" => "其他用户不可见",
        "Notifications setup instructions" => "通知设置说明",
        "Only invited teammates" => "仅受邀团队成员",
        "Only people invited" => "仅受邀用户",
        "One-time purchase" => "一次性购买",
        "Open" => "打开",
        "Open Tab" => "打开标签页",
        "Open all in code review" => "全部在代码审查中打开",
        "Open file" => "打开文件",
        "Open file explorer" => "打开文件浏览器",
        "Open in GitHub" => "在 GitHub 中打开",
        "Open in Warp" => "在 Warp 中打开",
        "Open in code review" => "在代码审查中打开",
        "Open in editor" => "在编辑器中打开",
        "Open on Desktop" => "在桌面应用中打开",
        "Open repository" => "打开仓库",
        "Open Rich Input" => "打开富输入",
        "Open the admin panel" => "打开管理面板",
        "Open coding agent settings" => "打开编码智能体设置",
        "Open this conversation in the Warp desktop app" => "在 Warp 桌面应用中打开此对话",
        "Orchestration config was disapproved" => "编排配置已被拒绝",
        "Out of credits" => "点数已用完",
        "Pay-as-you-go" => "按量付费",
        "Past 3 days" => "过去 3 天",
        "Diffs only work for git repositories." => "差异比较仅适用于 Git 仓库。",
        "Diffs only work for local workspaces." => "差异比较仅适用于本地工作区。",
        "Diffs don't currently work in WSL." => "差异比较当前不支持 WSL。",
        "Panes" => "窗格",
        "Pane title as" => "窗格标题显示为",
        "Personal" => "个人",
        "Personal credits" => "个人点数",
        "Pick a theme for when your system is in dark mode." => "为系统深色模式选择主题。",
        "Pick a theme for when your system is in light mode." => "为系统浅色模式选择主题。",
        "Plan" => "计划",
        "Plugin update instructions" => "插件更新说明",
        "Preparing..." => "正在准备...",
        "PR link" => "PR 链接",
        "Profiles" => "配置",
        "Pull Request" => "拉取请求",
        "Quick setup" => "快速设置",
        "Redact secrets (API keys, passwords, IP addresses, PII etc.)" => {
            "隐藏密钥（API 密钥、密码、IP 地址、个人信息等）"
        }
        "Re-generate AGENTS.md file" => "重新生成 AGENTS.md 文件",
        "Refresh AWS Credentials" => "刷新 AWS 凭证",
        "Refresh file" => "刷新文件",
        "Remove" => "移除",
        "Remove domain" => "移除域名",
        "Remove Member" => "移除成员",
        "Remove from team" => "从团队中移除",
        "Remove queued prompt" => "移除排队提示",
        "Repos" => "仓库",
        "Replace all" => "全部替换",
        "Request edit access" => "请求编辑权限",
        "Reset to Warp defaults" => "重置为 Warp 默认值",
        "Reset to default" => "重置为默认值",
        "Restore" => "恢复",
        "Retry" => "重试",
        "Remote host" => "远程主机",
        "Resume conversation" => "恢复对话",
        "Review changes" => "审查更改",
        "Restricted due to billing issue" => "由于账单问题受限",
        "Restricted due to billing issue. Update your payment method to purchase add-on credits." => {
            "由于账单问题受限。请更新付款方式以购买附加点数。"
        }
        "Restricted due to billing issue. Contact your team admin to update their payment method." => {
            "由于账单问题受限。请联系团队管理员更新付款方式。"
        }
        "Rewind" => "回退",
        "Rewind to before this block" => "回退到此块之前",
        "Rich Input" => "富输入",
        "Rule" => "规则",
        "Running child agents is disabled by the active execution profile." => {
            "当前执行配置已禁用运行子智能体。"
        }
        "Same line prompt" => "同行提示符",
        "Save" => "保存",
        "Save as markdown file" => "另存为 Markdown 文件",
        "Save and auto-sync this plan to your Warp Drive" => "保存并自动同步此计划到 Warp Drive",
        "Screenshot" => "截图",
        "Settings" => "设置",
        "Global search" => "全局搜索",
        "Getting Started" => "开始使用",
        "Go to settings > keyboard shortcuts to configure custom keybindings" => {
            "前往 设置 > 键盘快捷键 以配置自定义快捷键"
        }
        "Search" => "搜索",
        "Search environments..." => "搜索环境...",
        "Search in files" => "在文件中搜索",
        "Search MCP Servers" => "搜索 MCP 服务器",
        "Search sessions, agents, files..." => "搜索会话、智能体、文件...",
        "Search repos" => "搜索仓库",
        "Search tabs..." => "搜索标签页...",
        "Select all" => "全选",
        "No search results found" => "未找到搜索结果",
        "Send Feedback" => "发送反馈",
        "Send now" => "立即发送",
        "Set up" => "设置",
        "See what's new" => "查看新功能",
        "Share" => "共享",
        "Share conversation" => "共享对话",
        "Share session" => "共享会话",
        "Share session..." => "共享会话...",
        "Shared by Warp and your team" => "由 Warp 和你的团队共享",
        "Show" => "显示",
        "Show credit usage details" => "显示点数用量详情",
        "Show file navigation" => "显示文件导航",
        "Show in Warp Drive" => "在 Warp Drive 中显示",
        "Show prompt" => "显示提示符",
        "Show version history" => "显示版本历史",
        "Sign in to edit" => "登录后编辑",
        "Sign up" => "注册",
        "Skip" => "跳过",
        "Skip Welcome Tips" => "跳过欢迎提示",
        "Skip for now" => "暂时跳过",
        "Slash commands" => "斜杠命令",
        "Sort by" => "排序方式",
        "Source" => "来源",
        "Status" => "状态",
        "Stop sharing" => "停止共享",
        "Stop sharing session" => "停止共享会话",
        "Successfully created environment" => "环境已成功创建",
        "Successfully shared environment" => "环境已成功共享",
        "Successfully updated environment" => "环境已成功更新",
        "Summary" => "摘要",
        "Suggested" => "建议",
        "Select the GitHub repositories you'd like to work with and we'll suggest a base image and config" => {
            "选择你想使用的 GitHub 仓库，我们会建议基础镜像和配置"
        }
        "Select the GitHub repositories you’d like to work with and we’ll suggest a base image and config" => {
            "选择你想使用的 GitHub 仓库，我们会建议基础镜像和配置"
        }
        "Setup commands" => "设置命令",
        "Split Pane Right" => "向右拆分窗格",
        "Switch to Business" => "切换到 Business",
        "Switch to the Build plan" => "切换到 Build 方案",
        "Teammates with the link" => "拥有链接的团队成员",
        "Team credits" => "团队点数",
        "Agent conversations" => "智能体对话",
        "Tab configs" => "标签页配置",
        "Tab item" => "标签项",
        "Tabs panel" => "标签页面板",
        "Tabs" => "标签页",
        "Terminal" => "终端",
        "Text" => "文本",
        "The Agentic Development Environment" => "智能体开发环境",
        "To toggle this panel" => "用于切换此面板",
        "Toggle Case Sensitivity" => "切换大小写敏感",
        "Toggle Regex" => "切换正则表达式",
        "Transfer" => "转让",
        "Trash" => "移到废纸篓",
        "Tools panel" => "工具面板",
        "Show details on hover" => "悬停时显示详情",
        "Type your answer and press Enter" => "输入答案并按 Enter",
        "Unshare" => "取消共享",
        "This purchase would exceed your monthly limit. Increase your limit to continue." => {
            "此次购买会超出你的月度限额。提高限额后即可继续。"
        }
        "This purchase would exceed your team’s monthly spend limit. Contact a team admin to increase it." => {
            "此次购买会超出团队月度消费限额。请联系团队管理员提高限额。"
        }
        "Update" => "更新",
        "Update Agent" => "更新智能体",
        "Update Warp manually" => "手动更新 Warp",
        "Unable to create environment: not logged in." => "无法创建环境：尚未登录。",
        "Unable to save: environment no longer exists." => "无法保存：环境已不存在。",
        "Unable to share environment: environment is not yet synced." => "无法共享环境：环境尚未同步。",
        "Unable to share environment: you are not currently on a team." => {
            "无法共享环境：你当前不在团队中。"
        }
        "Update Warp plugin" => "更新 Warp 插件",
        "Upgrade" => "升级",
        "Upgrade to Build" => "升级到 Build",
        "Upgrade to Business" => "升级到 Business",
        "Upgrade to Enterprise" => "升级到 Enterprise",
        "Upgrade to Lightspeed plan" => "升级到 Lightspeed 方案",
        "Upgrade to Max" => "升级到 Max",
        "Upgrade to the Build plan" => "升级到 Build 方案",
        "Upgrade to Turbo plan" => "升级到 Turbo 方案",
        "Upgrade plan" => "升级套餐",
        "bring your own key" => "自带密钥",
        "Use latest codex model" => "使用最新 Codex 模型",
        "Use Codex models in Warp" => "在 Warp 中使用 Codex 模型",
        "Codex is OpenAI's most advanced agentic coding model for real-world engineering." => {
            "Codex 是 OpenAI 面向真实工程场景的先进智能体编码模型。"
        }
        "Use Codex directly in Oz and leverage features like in-app code review, agent session sharing and file editing." => {
            "直接在 Oz 中使用 Codex，并利用应用内代码审查、智能体会话共享和文件编辑等功能。"
        }
        "Use agent" => "使用智能体",
        "Use the agent" => "使用智能体",
        "Value (optional)" => "值（可选）",
        "View" => "查看",
        "View as" => "视图显示为",
        "View Agents" => "查看智能体",
        "View all cloud runs" => "查看所有云端运行",
        "View index status" => "查看索引状态",
        "View in Oz" => "在 Oz 中查看",
        "View instructions to install the Warp plugin" => "查看安装 Warp 插件的说明",
        "View instructions to update the Warp plugin" => "查看更新 Warp 插件的说明",
        "View logs" => "查看日志",
        "View options" => "查看选项",
        "View pull request" => "查看拉取请求",
        "View this run in the Oz web app" => "在 Oz 网页应用中查看此运行",
        "View agent tasks you created" => "查看你创建的智能体任务",
        "View my runs" => "查看我的运行",
        "Visit Oz" => "访问 Oz",
        "Visit the repo" => "访问仓库",
        "Viewing" => "查看中",
        "View your agent tasks plus all shared team tasks" => {
            "查看你的智能体任务和所有共享团队任务"
        }
        " for a more flexible pricing model." => "以使用更灵活的计费模式。",
        " for custom limits and dedicated support." => "以获得自定义限制和专属支持。",
        " for increased access to AI features." => "以获得更多 AI 功能访问权限。",
        " for more AI credits." => "以获得更多 AI 点数。",
        " for more AI usage." => "以获得更多 AI 用量。",
        " for more credits and access to more models." => "以获得更多点数并访问更多模型。",
        " for security features like SSO and automatically applied zero data retention." => {
            "以获得 SSO、自动应用零数据保留等安全功能。"
        }
        " or " => " 或 ",
        " to get more AI usage." => "以获得更多 AI 用量。",
        " to purchase add-on credits." => "以购买附加点数。",
        " to regain access to AI features." => "以恢复 AI 功能访问权限。",
        "to see fine-grained credit attribution and set per-user spend limits." => {
            "以查看细粒度点数归因并设置每用户消费限制。"
        }
        "to see per-user credit attribution." => "以查看每用户点数归因。",
        "to see team-level credit usage." => "以查看团队级点数用量。",
        "to set per-user spend limits." => "以设置每用户消费限制。",
        "Voice input" => "语音输入",
        "Warp Essentials" => "Warp 精要",
        "Warp is the intelligent terminal with AI and your dev team's knowledge built-in." => {
            "Warp 是内置 AI 和开发团队知识的智能终端。"
        }
        "Warpify subshell" => "Warpify 子 Shell",
        "Warpify without TMUX" => "不使用 TMUX 进行 Warpify",
        "Welcome to Warp" => "欢迎使用 Warp",
        "Who has access" => "谁有访问权限",
        "Window" => "窗口",
        "Working" => "运行中",
        "You haven’t set up any environments yet." => "你还没有设置任何环境。",
        "never" => "从未",
        "Working Directory" => "工作目录",
        "Branch" => "分支",
        "Command / Conversation" => "命令 / 对话",
        "Additional metadata" => "附加元数据",
        "Density" => "密度",
        "Essentials" => "核心",
        "Blocks" => "块",
        "Input Editor" => "输入编辑器",
        "Local agent" => "本地智能体",
        "Fundamentals" => "基础",
        "Repo(s)" => "仓库",
        "Failed to load GitHub repositories" => "加载 GitHub 仓库失败",
        "Auth with GitHub" => "通过 GitHub 认证",
        "Type owner/repo and press Enter to add, or select from dropdown." => {
            "输入 owner/repo 并按 Enter 添加，或从下拉列表中选择。"
        }
        "Missing a repo?" => "缺少仓库？",
        "Configure access on GitHub" => "在 GitHub 上配置访问权限",
        "No repositories found" => "未找到仓库",
        "Selected repos" => "已选仓库",
        "Available indexed repos" => "可用的已索引仓库",
        "No repos selected yet" => "尚未选择仓库",
        "All locally indexed repos are already selected." => "所有本地已索引仓库都已选择。",
        "Select repos to provide context for the environment creation agent." => {
            "选择仓库，为环境创建智能体提供上下文。"
        }
        "Select locally indexed repos to provide context for the environment creation agent." => {
            "选择本地已索引仓库，为环境创建智能体提供上下文。"
        }
        "Select repos for your environment" => "为你的环境选择仓库",
        "Open AI Command Suggestions" => "打开 AI 命令建议",
        "A little gift" => "一点小礼物",
        "Break out of your laptop with cloud agents" => "用云端智能体突破本机限制",
        "Buy credits" => "购买点数",
        "Base credits" => "基础点数",
        "Cloud agents" => "云端智能体",
        "Compare plans" => "比较方案",
        "Introducing Oz" => "Oz 介绍",
        "Infinitely scalable coding agent — run in local sessions or in the cloud." => {
            "可无限扩展的编码智能体，可以在本地会话或云端运行。"
        }
        "Launch credits" => "启动点数",
        "Next: Agent automations" => "下一步：智能体自动化",
        "Next: Agent management" => "下一步：智能体管理",
        "Next: Launch credits" => "下一步：启动点数",
        "No agent harnesses are available. Contact your team admin." => {
            "没有可用的智能体运行框架。请联系团队管理员。"
        }
        "1,000 free cloud agent credits when you upgrade to Warp Build" => {
            "升级到 Warp Build 即可获得 1,000 个免费云端智能体点数"
        }
        "Orchestrate agents, turning Skills into automations" => {
            "编排智能体，将技能转化为自动化"
        }
        "Other team members' usage across add-on, pay-as-you-go, and cloud-only credits." => {
            "其他团队成员在附加额度、按量付费和仅云端点数中的用量。"
        }
        "Sync conversations to cloud" => "将对话同步到云端",
        "Track local and cloud agents seamlessly" => "无缝跟踪本地与云端智能体",
        "Try it out" => "试试看",
        "use your own API keys" => "使用你自己的 API 密钥",
        "Agent automations" => "智能体自动化",
        "Agent management" => "智能体管理",
        "Agent conversations stored in the cloud can be shared with anyone with one click, and allow conversations to be continued across devices and on logout." => {
            "存储在云端的智能体对话可以一键分享给任何人，并支持跨设备和退出登录后继续对话。"
        }
        "Oz agents can be defined using the standard Skills format. You can use the built in scheduler to setup agents to run autonomously at set intervals, or use the Oz SDK or API to programmatically start and manage Oz agents." => {
            "Oz 智能体可以使用标准 Skills 格式定义。你可以使用内置调度器让智能体按固定间隔自主运行，也可以使用 Oz SDK 或 API 以编程方式启动和管理 Oz 智能体。"
        }
        "Use cloud agents to run many agents in parallel, keep agents working when you close your laptop, or start agents programmatically. Plus, you can check on their work through the web." => {
            "使用云端智能体并行运行多个智能体，在合上笔记本后继续工作，或以编程方式启动智能体。你还可以通过网页查看它们的工作进展。"
        }
        "Upgrade to Build this month and receive 1,000 extra credits to try using Oz. Credits are only eligible for Oz runs in Warp-hosted cloud environments." => {
            "本月升级到 Build 即可获得 1,000 个额外点数来试用 Oz。点数仅适用于 Warp 托管云环境中的 Oz 运行。"
        }
        "View all of your agents across local and cloud sessions in the Warp app or at [oz.warp.dev](https://oz.warp.dev). Join live agent sessions, continue tasks locally, and steer agents with one click." => {
            "在 Warp 应用或 [oz.warp.dev](https://oz.warp.dev) 查看你的所有本地和云端智能体。加入实时智能体会话，在本地继续任务，并一键引导智能体。"
        }
        "Accept Prompt Suggestion" => "接受提示建议",
        "Accept autosuggestion" => "接受自动建议",
        "Activate next tab" => "激活下一个标签页",
        "Activate previous tab" => "激活上一个标签页",
        "Add current folder as project" => "将当前文件夹添加为项目",
        "Add Cursor Above" => "在上方添加光标",
        "Add Cursor Below" => "在下方添加光标",
        "Add Selection for Next Occurrence" => "为下一个匹配项添加选择",
        "Alternate terminal paste" => "替代终端粘贴",
        "Ask Warp AI" => "询问 Warp AI",
        "Ask Warp AI about Selection" => "询问 Warp AI 关于所选内容",
        "Ask Warp AI about last block" => "询问 Warp AI 关于上一个块",
        "Attach Selected Block as Agent Context" => "将所选块附加为智能体上下文",
        "Attach Selected Text as Agent Context" => "将所选文本附加为智能体上下文",
        "Backward tabulation within an executing command" => "在执行中的命令内反向制表",
        "Bookmark Selected Block" => "为所选块添加书签",
        "Check for updates" => "检查更新",
        "Clear Command Editor" => "清空命令编辑器",
        "Clear Screen" => "清屏",
        "Clear Selected Lines" => "清除所选行",
        "Clear selected lines" => "清除所选行",
        "Close Window" => "关闭窗口",
        "Close focused panel" => "关闭当前聚焦面板",
        "Close other tabs" => "关闭其他标签页",
        "Close tabs to the right" => "关闭右侧标签页",
        "Close the current tab" => "关闭当前标签页",
        "Copy access token to clipboard" => "将访问令牌复制到剪贴板",
        "Copy and Clear Selected Lines" => "复制并清除所选行",
        "Copy and clear selected lines" => "复制并清除所选行",
        "Copy Command" => "复制命令",
        "Copy Command Output" => "复制命令输出",
        "Copy rich-text buffer" => "复制富文本缓冲区",
        "Copy rich-text selection" => "复制富文本选区",
        "Create a new personal folder" => "新建个人文件夹",
        "Create a new personal notebook" => "新建个人笔记本",
        "Create a new personal prompt" => "新建个人提示词",
        "Create a new personal workflow" => "新建个人工作流",
        "Create a new team folder" => "新建团队文件夹",
        "Create a new team notebook" => "新建团队笔记本",
        "Create a new team prompt" => "新建团队提示词",
        "Create a new team workflow" => "新建团队工作流",
        "Create new personal environment variables" => "新建个人环境变量",
        "Create new team environment variables" => "新建团队环境变量",
        "Create new tab" => "新建标签页",
        "Create or edit link" => "创建或编辑链接",
        "Cursor at buffer end" => "光标位于缓冲区末尾",
        "Cursor at buffer start" => "光标位于缓冲区开头",
        "Cursor at Buffer End" => "光标位于缓冲区末尾",
        "Cursor at Buffer Start" => "光标位于缓冲区开头",
        "Cut All Right" => "剪切右侧全部内容",
        "Cut Word Left" => "剪切左侧单词",
        "Cut Word Right" => "剪切右侧单词",
        "Cut all left" => "剪切左侧全部内容",
        "Cut all right" => "剪切右侧全部内容",
        "Cut" => "剪切",
        "Cut word left" => "剪切左侧单词",
        "Cut word right" => "剪切右侧单词",
        "De-Select Shell Commands" => "取消选择 Shell 命令",
        "De-select shell commands" => "取消选择 Shell 命令",
        "Decrease Font Size" => "减小字体",
        "Decrease Notebook Font Size" => "减小笔记本字体",
        "Decrease zoom level" => "缩小缩放级别",
        "Delete all left" => "删除左侧全部内容",
        "Delete all right" => "删除右侧全部内容",
        "Delete to line end within an executing command" => "在执行中的命令内删除到行尾",
        "Delete to line start within an executing command" => "在执行中的命令内删除到行首",
        "Delete word left" => "删除左侧单词",
        "Delete word left within an executing command" => "在执行中的命令内删除左侧单词",
        "Delete word right" => "删除右侧单词",
        "Dump heap profile (can only be done once)" => "导出堆分析（只能执行一次）",
        "End" => "End",
        "Exit Vim insert mode" => "退出 Vim 插入模式",
        "Expand Selected Blocks Above" => "向上扩展所选块",
        "Expand Selected Blocks Below" => "向下扩展所选块",
        "Export all Warp Drive objects" => "导出所有 Warp Drive 对象",
        "Find in Notebook" => "在笔记本中查找",
        "Find in Terminal" => "在终端中查找",
        "Find in code editor" => "在代码编辑器中查找",
        "Find the Next Occurrence of Your Search Query" => "查找搜索内容的下一个匹配项",
        "Find the Previous Occurrence of Your Search Query" => "查找搜索内容的上一个匹配项",
        "Focus next match" => "聚焦下一个匹配项",
        "Focus previous match" => "聚焦上一个匹配项",
        "Focus Terminal Input" => "聚焦终端输入框",
        "Fold" => "折叠",
        "Fold selected ranges" => "折叠所选范围",
        "Hide Others" => "隐藏其他窗口",
        "Hide Warp" => "隐藏 Warp",
        "Home" => "Home",
        "Import External Settings" => "导入外部设置",
        "Import To Personal Drive" => "导入到个人云盘",
        "Import To Team Drive" => "导入到团队云盘",
        "Increase zoom level" => "放大缩放级别",
        "Initiate project for warp" => "为 Warp 初始化项目",
        "Insert Command Correction" => "插入命令修正",
        "Insert last word of previous command" => "插入上一条命令的最后一个单词",
        "Insert newline" => "插入换行",
        "Insert non-expanding space" => "插入不换行空格",
        "Inspect Command" => "检查命令",
        "Install Oz CLI command" => "安装 Oz CLI 命令",
        "Install update and relaunch" => "安装更新并重新启动",
        "Invite People..." => "邀请成员...",
        "Join our Slack community (opens external link)" => "加入我们的 Slack 社区（打开外部链接）",
        "Jump to latest agent task" => "跳转到最新智能体任务",
        "Launch configuration palette" => "启动配置面板",
        "Launch Configuration Palette" => "启动配置面板",
        "Left Panel: Agent conversations" => "左侧面板：智能体对话",
        "Left Panel: Global search" => "左侧面板：全局搜索",
        "Left Panel: Project explorer" => "左侧面板：项目浏览器",
        "Left Panel: Warp Drive" => "左侧面板：Warp Drive",
        "Load agent mode conversation (from debug link in clipboard)" => {
            "加载智能体模式对话（来自剪贴板中的调试链接）"
        }
        "Log editor state" => "记录编辑器状态",
        "Minimize" => "最小化",
        "Move Backward One Subword" => "向后移动一个子词",
        "Move Backward One Word" => "向后移动一个单词",
        "Move Forward One Subword" => "向前移动一个子词",
        "Move Forward One Word" => "向前移动一个单词",
        "Move backward one word" => "向后移动一个单词",
        "Move cursor down" => "向下移动光标",
        "Move Cursor Down" => "向下移动光标",
        "Move cursor end within an executing command" => "在执行中的命令内将光标移到末尾",
        "Move cursor home within an executing command" => "在执行中的命令内将光标移到开头",
        "Move cursor left" => "向左移动光标",
        "Move Cursor Left" => "向左移动光标",
        "Move cursor one word to the left within an executing command" => {
            "在执行中的命令内将光标左移一个单词"
        }
        "Move cursor one word to the right within an executing command" => {
            "在执行中的命令内将光标右移一个单词"
        }
        "Move cursor right" => "向右移动光标",
        "Move Cursor Right" => "向右移动光标",
        "Move cursor to the bottom" => "将光标移到底部",
        "Move Cursor to the Bottom" => "将光标移到底部",
        "Move cursor to the top" => "将光标移到顶部",
        "Move cursor up" => "向上移动光标",
        "Move Cursor Up" => "向上移动光标",
        "Move forward one word" => "向前移动一个单词",
        "Move to end of line" => "移动到行尾",
        "Move to End of Line" => "移动到行尾",
        "Move to end of paragraph" => "移动到段落末尾",
        "Move to line end" => "移动到行尾",
        "Move to line start" => "移动到行首",
        "Move to start of line" => "移动到行首",
        "Move to start of paragraph" => "移动到段落开头",
        "Move to the end of the buffer" => "移动到缓冲区末尾",
        "Move to the end of the paragraph" => "移动到段落末尾",
        "Move to the start of the buffer" => "移动到缓冲区开头",
        "Move to the start of the paragraph" => "移动到段落开头",
        "New Agent Tab" => "新建智能体标签页",
        "New Cloud Agent Tab" => "新建云端智能体标签页",
        "New Terminal Tab" => "新建终端标签页",
        "Open Left Panel" => "打开左侧面板",
        "Open New Window" => "打开新窗口",
        "Open Settings" => "打开设置",
        "Open Settings: AI" => "打开设置：AI",
        "Open Settings: About" => "打开设置：关于",
        "Open Settings: Account" => "打开设置：账户",
        "Open Settings: Appearance" => "打开设置：外观",
        "Open Settings: Billing and usage" => "打开设置：账单与用量",
        "Open Settings: Code" => "打开设置：代码",
        "Open Settings: Environments" => "打开设置：环境",
        "Open Settings: Features" => "打开设置：功能",
        "Open Settings: Keyboard Shortcuts" => "打开设置：键盘快捷键",
        "Open Settings: MCP Servers" => "打开设置：MCP 服务器",
        "Open Settings: Privacy" => "打开设置：隐私",
        "Open Settings: Referrals" => "打开设置：邀请好友",
        "Open Settings: Shared Blocks" => "打开设置：共享块",
        "Open Settings: Teams" => "打开设置：团队",
        "Open Settings: Warpify" => "打开设置：Warpify",
        "Open Block Context Menu" => "打开块上下文菜单",
        "Open global search" => "打开全局搜索",
        "Open keybindings editor" => "打开快捷键编辑器",
        "Open Keybindings Editor" => "打开快捷键编辑器",
        "Open settings file" => "打开设置文件",
        "Open tab configs menu" => "打开标签页配置菜单",
        "Open theme picker" => "打开主题选择器",
        "Open view tree debugger" => "打开视图树调试器",
        "Paste" => "粘贴",
        "Quit Warp" => "退出 Warp",
        "Reinput selected commands" => "重新输入所选命令",
        "Reinput selected commands as root" => "以 root 身份重新输入所选命令",
        "Remove the previous character" => "删除前一个字符",
        "Rename the current tab" => "重命名当前标签页",
        "Resize Pane > Move Divider Down" => "调整窗格大小 > 向下移动分隔线",
        "Resize Pane > Move Divider Left" => "调整窗格大小 > 向左移动分隔线",
        "Resize Pane > Move Divider Right" => "调整窗格大小 > 向右移动分隔线",
        "Resize Pane > Move Divider Up" => "调整窗格大小 > 向上移动分隔线",
        "Reset zoom level to default" => "将缩放级别重置为默认值",
        "Run selected commands" => "运行所选命令",
        "Sample Process" => "采样进程",
        "Save new launch configuration" => "保存新的启动配置",
        "Scroll terminal output down one line" => "将终端输出向下滚动一行",
        "Scroll terminal output up one line" => "将终端输出向上滚动一行",
        "Scroll to Top of Selected Block" => "滚动到所选块顶部",
        "Scroll to Bottom of Selected Block" => "滚动到所选块底部",
        "Select and move to the bottom" => "选择并移动到底部",
        "Select and move to the top" => "选择并移动到顶部",
        "Select down" => "向下选择",
        "Select one character to the left" => "向左选择一个字符",
        "Select one character to the right" => "向右选择一个字符",
        "Select One Character to the Right" => "向右选择一个字符",
        "Select one subword to the left" => "向左选择一个子词",
        "Select One Subword to the Left" => "向左选择一个子词",
        "Select one subword to the right" => "向右选择一个子词",
        "Select One Subword to the Right" => "向右选择一个子词",
        "Select one word to the left" => "向左选择一个单词",
        "Select One Word to the Left" => "向左选择一个单词",
        "Select one word to the right" => "向右选择一个单词",
        "Select One Word to the Right" => "向右选择一个单词",
        "Select Next Command" => "选择下一个命令",
        "Select Previous Command" => "选择上一个命令",
        "Select Previous Block" => "选择上一个块",
        "Select Shell Command at Cursor" => "选择光标处的 Shell 命令",
        "Take Control of Running Command" => "接管正在运行的命令",
        "Terminal Session" => "终端会话",
        "Select the closest bookmark down" => "选择最近的下一个书签",
        "Select the closest bookmark up" => "选择最近的上一个书签",
        "Select to end of line" => "选择到行尾",
        "Select To Line End" => "选择到行尾",
        "Select to start of line" => "选择到行首",
        "Select To Line Start" => "选择到行首",
        "Select and Move to the Bottom" => "选择并移动到底部",
        "Select and Move to the Top" => "选择并移动到顶部",
        "Select up" => "向上选择",
        "Select to End of Paragraph" => "选择到段落末尾",
        "Select to Start of Paragraph" => "选择到段落开头",
        "Send feedback (opens external link)" => "发送反馈（打开外部链接）",
        "Set Input Mode to Agent Mode" => "将输入模式设为智能体模式",
        "Set Input Mode to Terminal Mode" => "将输入模式设为终端模式",
        "Setup Guide" => "设置指南",
        "Share current session" => "共享当前会话",
        "Stop sharing current session" => "停止共享当前会话",
        "Switch Focus to Left Panel" => "将焦点切换到左侧面板",
        "Switch Focus to Right Panel" => "将焦点切换到右侧面板",
        "Switch to 1st tab" => "切换到第 1 个标签页",
        "Switch to 2nd tab" => "切换到第 2 个标签页",
        "Switch to 3rd tab" => "切换到第 3 个标签页",
        "Switch to 4th tab" => "切换到第 4 个标签页",
        "Switch to 5th tab" => "切换到第 5 个标签页",
        "Switch to 6th tab" => "切换到第 6 个标签页",
        "Switch to 7th tab" => "切换到第 7 个标签页",
        "Switch to 8th tab" => "切换到第 8 个标签页",
        "Switch to 8th Tab" => "切换到第 8 个标签页",
        "Switch to last tab" => "切换到最后一个标签页",
        "Switch to Last Tab" => "切换到最后一个标签页",
        "Toggle Agent conversation list view" => "切换智能体对话列表视图",
        "Toggle Agent Conversation List View" => "切换智能体对话列表视图",
        "Toggle Auto-Execute Mode" => "切换自动执行模式",
        "Toggle Block Filter on Selected or Last Block" => {
            "切换所选或最后一个块的过滤器"
        }
        "Toggle code review" => "切换代码审查",
        "Toggle Code Review" => "切换代码审查",
        "Toggle command palette" => "切换命令面板",
        "Toggle keyboard shortcuts" => "切换键盘快捷键",
        "Toggle navigation palette" => "切换导航面板",
        "Toggle project explorer" => "切换项目浏览器",
        "Toggle resource center" => "切换资源中心",
        "Toggle vertical tabs panel" => "切换垂直标签页面板",
        "Toggle Vertical Tabs Panel" => "切换垂直标签页面板",
        "Copy" => "复制",
        "Copy command" => "复制命令",
        "Copy git branch" => "复制 Git 分支",
        "Expand selected blocks above" => "向上扩展所选块",
        "Expand selected blocks below" => "向下扩展所选块",
        "Open block context menu" => "打开块上下文菜单",
        "Select next command" => "选择下一个命令",
        "Select previous command" => "选择上一个命令",
        "Select shell command at cursor" => "选择光标处的 Shell 命令",
        "Select to end of paragraph" => "选择到段落末尾",
        "Select to start of paragraph" => "选择到段落开头",
        "Toggle Auto-execute Mode" => "切换自动执行模式",
        "Toggle CLI Agent Rich Input" => "切换 CLI 智能体富输入",
        "Toggle Files Palette" => "切换文件面板",
        "Toggle Hide CLI Responses" => "切换隐藏 CLI 响应",
        "Toggle Mouse Reporting" => "切换鼠标报告",
        "Toggle PTY Recording for Session" => "切换会话 PTY 录制",
        "Toggle Queue Next Prompt" => "切换下一个提示排队",
        "Toggle Sticky Command Header in Active Pane" => "切换当前窗格中的粘性命令头",
        "Toggle Warp AI" => "切换 Warp AI",
        "Toggle Warp Drive" => "切换 Warp Drive",
        "Toggle block filter on selected or last block" => "切换所选或最后一个块的过滤器",
        "Toggle case-sensitive search" => "切换大小写敏感搜索",
        "Toggle comment" => "切换注释",
        "Toggle inline code styling" => "切换内联代码样式",
        "Toggle notification mailbox" => "切换通知邮箱",
        "Toggle regular expression search" => "切换正则表达式搜索",
        "Toggle rich-text debug mode" => "切换富文本调试模式",
        "Toggle sticky command header" => "切换粘性命令头",
        "Toggle strikethrough styling" => "切换删除线样式",
        "Toggle team workflows modal" => "切换团队工作流弹窗",
        "Toggle the agent management view" => "切换智能体管理视图",
        "Toggle the Agent Management View" => "切换智能体管理视图",
        "Toggle underline styling" => "切换下划线样式",
        "Toggle Underline Styling" => "切换下划线样式",
        "Trigger a panic (for testing sentry-rust)" => "触发 panic（用于测试 sentry-rust）",
        "Trigger Auto Detection" => "触发自动检测",
        "Turn notifications off" => "关闭通知",
        "Turn notifications on" => "开启通知",
        "Unfold" => "展开",
        "Undo" => "撤销",
        "Uninstall Oz CLI command" => "卸载 Oz CLI 命令",
        "What's New?" => "新功能？",
        "Read all changelogs" => "查看所有更新日志",
        "Shared by Warp and" => "由 Warp 及",
        "Shared by Warp and from other devices" => "由 Warp 和其他设备共享",
        "Shared from Warp" => "由 Warp 共享",
        "Detected from" => "检测自",
        "My MCPs" => "我的 MCP",
        "Advanced Setup" => "高级设置",
        "Unable to fetch the latest changelog." => "无法获取最新更新日志。",
        "Learn more." => "了解更多。",
        "See supported providers." => "查看支持的提供方。",
        "Add MCP servers to extend the Warp Agent's capabilities. MCP servers expose data sources or tools to agents through a standardized interface, essentially acting like plugins. Add a custom server, or use the presets to get started with popular servers. You can also find team servers that have been shared with you here. " => {
            "添加 MCP 服务器以扩展 Warp 智能体的能力。MCP 服务器通过标准化接口向智能体暴露数据源或工具，本质上就像插件。你可以添加自定义服务器，或使用预设快速开始使用热门服务器。你也可以在这里找到共享给你的团队服务器。"
        }
        "Once you add a MCP server, it will be shown here." => "添加 MCP 服务器后，它会显示在这里。",
        "View Warp logs" => "查看 Warp 日志",
        "View latest changelog" => "查看最新更新日志",
        "View privacy policy (opens external link)" => "查看隐私政策（打开外部链接）",
        "View user docs (opens external link)" => "查看用户文档（打开外部链接）",
        "Warpify ssh session" => "对 SSH 会话启用 Warpify",
        "Warpify Ssh Session" => "对 SSH 会话启用 Warpify",
        "Warpify Subshell" => "Warpify 子 Shell",
        "Write current codebase index snapshot" => "写入当前代码库索引快照",
        "[Debug] Generate codebase index" => "[调试] 生成代码库索引",
        "[Debug] Install OpenCode Warp plugin" => "[调试] 安装 OpenCode Warp 插件",
        "[Debug] Log review comment send status for active tab" => {
            "[调试] 记录活动标签页的评论发送状态"
        }
        "[Debug] Onboarding Callout: Modality - No Project" => {
            "[调试] 新手引导提示：模式 - 无项目"
        }
        "[Debug] Onboarding Callout: Modality - Project" => {
            "[调试] 新手引导提示：模式 - 项目"
        }
        "[Debug] Onboarding Callout: Modality - Terminal" => {
            "[调试] 新手引导提示：模式 - 终端"
        }
        "[Debug] Onboarding Callout: WarpInput - No Project" => {
            "[调试] 新手引导提示：WarpInput - 无项目"
        }
        "[Debug] Onboarding Callout: WarpInput - Project" => {
            "[调试] 新手引导提示：WarpInput - 项目"
        }
        "[Debug] Onboarding Callout: WarpInput - Terminal" => {
            "[调试] 新手引导提示：WarpInput - 终端"
        }
        "[Debug] Open Build Plan Migration Modal" => "[调试] 打开构建计划迁移弹窗",
        "[Debug] Open OpenWarp Launch Modal" => "[调试] 打开 OpenWarp 启动弹窗",
        "[Debug] Open Oz Launch Modal" => "[调试] 打开 Oz 启动弹窗",
        "[Debug] Open Session Config Modal" => "[调试] 打开会话配置弹窗",
        "[Debug] Reset Build Plan Migration Modal State" => "[调试] 重置构建计划迁移弹窗状态",
        "[Debug] Reset OpenWarp Launch Modal State" => "[调试] 重置 OpenWarp 启动弹窗状态",
        "[Debug] Reset Oz Launch Modal State" => "[调试] 重置 Oz 启动弹窗状态",
        "[Debug] Start HOA Onboarding Flow" => "[调试] 启动 HOA 新手引导流程",
        "[Debug] Un-dismiss AWS login banner" => "[调试] 重新显示 AWS 登录横幅",
        "[Debug] Use local OpenCode Warp plugin (testing only)" => {
            "[调试] 使用本地 OpenCode Warp 插件（仅测试）"
        }
        "[a11y] Set concise accessibility announcements" => "[a11y] 设为简洁的辅助功能播报",
        "[a11y] Set verbose accessibility announcements" => "[a11y] 设为详细的辅助功能播报",
        "terminal:ask_ai_assistant" => "询问 Warp AI",
        "workspace:add_current_dir_as_project" => "将当前目录添加为项目",
        "workspace:close_panel" => "关闭面板",
        "workspace:close_tabs_right_active_tab" => "关闭右侧标签页",
        "workspace:close_window" => "关闭窗口",
        "workspace:crash" => "触发崩溃",
        "workspace:create_personal_ai_prompt" => "新建个人 AI 提示词",
        "workspace:create_personal_env_vars" => "新建个人环境变量",
        "workspace:create_personal_folder" => "新建个人文件夹",
        "workspace:create_personal_notebook" => "新建个人笔记本",
        "workspace:create_personal_workflow" => "新建个人工作流",
        "workspace:create_team_ai_prompt" => "新建团队 AI 提示词",
        "workspace:create_team_env_vars" => "新建团队环境变量",
        "workspace:create_team_folder" => "新建团队文件夹",
        "workspace:create_team_notebook" => "新建团队笔记本",
        "workspace:create_team_workflow" => "新建团队工作流",
        "workspace:init_project_rules" => "初始化项目规则",
        "workspace:move_tab_left" => "将标签页左移",
        "workspace:move_tab_right" => "将标签页右移",
        "workspace:open_ai_fact_collection" => "打开 AI 规则",
        "workspace:open_mcp_servers" => "打开 MCP 服务器",
        "workspace:open_repository" => "打开仓库",
        "workspace:send_feedback" => "发送反馈",
        "workspace:show_ai_settings_page" => "打开 AI 设置页",
        "workspace:show_mcp_servers_settings_page" => "打开 MCP 服务器设置页",
        "workspace:show_settings" => "打开设置",
        "workspace:show_settings_about_page" => "打开设置：关于",
        "workspace:show_settings_appearance_page" => "打开设置：外观",
        "workspace:show_settings_billing_and_usage_page" => "打开设置：账单与用量",
        "workspace:show_settings_code_page" => "打开设置：代码",
        "workspace:show_settings_environments_page" => "打开设置：环境",
        "workspace:show_settings_keyboard_shortcuts_page" => "打开设置：键盘快捷键",
        "workspace:show_settings_privacy_page" => "打开设置：隐私",
        "workspace:show_settings_referrals_page" => "打开设置：邀请好友",
        "workspace:show_settings_shared_blocks_page" => "打开设置：共享块",
        "workspace:show_settings_teams_page" => "打开设置：团队",
        "workspace:show_settings_warpify_page" => "打开设置：Warpify",
        "workspace:toggle_ai_assistant" => "切换 Warp AI",
        "workspace:toggle_command_palette" => "切换命令面板",
        "workspace:toggle_left_panel" => "切换左侧面板",
        "workspace:toggle_navigation_palette" => "切换导航面板",
        "workspace:write_codebase_index" => "写入代码库索引",
        "Select All Blocks" => "选择所有块",
        "Requires the GitHub CLI to be installed and authenticated" => {
            "需要安装并完成 GitHub CLI 认证"
        }
        "here." => "此处。",
        "Working directory" => "工作目录",
        "Your new team name" => "新的团队名称",
        " and open" => "并打开",
        "Already have an account? " => "已经有账号？",
        "Are you sure you want to disable AI features?" => "确定要停用 AI 功能吗？",
        "Are you sure you want to disable Warp Drive?" => "确定要停用 Warp Drive 吗？",
        "Are you sure you want to skip login?" => "确定要跳过登录吗？",
        "Auth Token" => "认证令牌",
        "Back" => "返回",
        "Browser auth token" => "浏览器认证令牌",
        "By continuing, you agree to Warp's " => "继续即表示你同意 Warp 的",
        "Choose files..." => "选择文件...",
        "Auto-reload" => "自动充值",
        "Click here to paste your token from the browser" => "点击此处粘贴浏览器中的令牌",
        "Connect your account to enable AI-powered planning, coding, and automation." => {
            "连接账号以启用 AI 驱动的规划、编码和自动化。"
        }
        "Connect your account to save and share notebooks, workflows, and more across devices." => {
            "连接账号以跨设备保存和共享笔记本、工作流等内容。"
        }
        "Disable AI features" => "停用 AI 功能",
        "Disable Warp Drive" => "停用 Warp Drive",
        "Don't want to sign in right now? " => "现在还不想登录？",
        "Get started with AI" => "开始使用 AI",
        "Get started with Warp Drive" => "开始使用 Warp Drive",
        "If your browser hasn't launched, " => "如果浏览器没有打开，",
        "If you'd like to opt out of analytics and AI features," => {
            "如果你想停用分析和 AI 功能，"
        }
        "If you'd like to opt out of analytics and AI features, you can adjust your " => {
            "如果你想停用分析和 AI 功能，可以调整"
        }
        "If you'd like to opt out of analytics, you can adjust your " => {
            "如果你想停用分析，可以调整"
        }
        "In order to create more objects in Warp Drive, please create an account." => {
            "要在 Warp Drive 中创建更多对象，请先创建账号。"
        }
        "In order to share, please create an account." => "要共享内容，请先创建账号。",
        "In order to use Warp’s AI features or collaborate with others, please create an account." => {
            "要使用 Warp 的 AI 功能或与他人协作，请先创建账号。"
        }
        "Privacy Settings" => "隐私设置",
        "Sign in" => "登录",
        "Sign in on your browser \nto continue" => "在浏览器中登录\n以继续",
        "Sign in on your browser to continue" => "在浏览器中登录以继续",
        "Sign up for Warp" => "注册 Warp",
        "Terms of Service" => "服务条款",
        "Using Warp Offline" => "离线使用 Warp",
        "Warp is better with AI. By continuing, you won't have access to any of the following features:" => {
            "启用 AI 后 Warp 体验更好。继续后，你将无法使用以下功能："
        }
        "Warp Drive lets you save workflows and knowledge across devices and share them with your team. By continuing, you won't have access to the following features:" => {
            "Warp Drive 可让你跨设备保存工作流和知识，并与团队共享。继续后，你将无法使用以下功能："
        }
        "Welcome to Warp!" => "欢迎使用 Warp！",
        "Yes, skip login" => "是，跳过登录",
        "You can sign up later, but some features, such as AI," => {
            "你可以稍后注册，但某些功能（例如 AI）"
        }
        "and open the page manually." => "并手动打开页面。",
        "are only available to logged-in users. " => "仅对已登录用户可用。",
        "copy the URL" => "复制 URL",
        "the page manually." => "手动打开页面。",
        "you can adjust your " => "你可以调整",
        "All of Warp’s non-cloud features work offline." => "Warp 的所有非云端功能都可离线使用。",
        "Agent conversations can be shared with others and are retained when you log in on different devices. This data is only stored for product functionality, and Warp will not use it for analytics." => {
            "智能体对话可以与他人共享，并会在你登录不同设备时保留。此数据仅用于产品功能，Warp 不会将其用于分析。"
        }
        "Agent conversations are only stored locally on your machine, are lost upon logout, and cannot be shared. Note: conversation data for ambient agents are still stored in the cloud." => {
            "智能体对话仅存储在本机，退出登录后会丢失，且无法共享。注意：环境智能体的对话数据仍会存储在云端。"
        }
        "Crash reporting helps Warp's engineering team understand stability and improve performance." => {
            "崩溃报告可帮助 Warp 工程团队了解稳定性并提升性能。"
        }
        "Codebase context" => "代码库上下文",
        "Continue" => "继续",
        "Deleting..." => "正在删除...",
        "Download" => "下载",
        "Download Warp Desktop?" => "下载 Warp 桌面版？",
        "Enable AI features" => "启用 AI 功能",
        "Enable Warp Drive" => "启用 Warp Drive",
        "Environment name" => "环境名称",
        "Agents over SSH" => "SSH 上的智能体",
        "Failed to load blocks. Please try again." => "加载块失败。请重试。",
        "Future links will automatically open on desktop." => "之后的链接会自动在桌面版中打开。",
        "Getting blocks..." => "正在获取块...",
        "Get Started" => "开始使用",
        "Help improve Warp" => "帮助改进 Warp",
        "High-level feature usage data helps Warp's product team prioritize the roadmap." => {
            "高级功能使用数据可帮助 Warp 产品团队确定路线图优先级。"
        }
        "However, we require users to be online when using Warp for the first time in order to enable Warp's AI and cloud features." => {
            "不过，为了启用 Warp 的 AI 和云端功能，首次使用 Warp 时需要联网。"
        }
        "Send crash reports" => "发送崩溃报告",
        "Next command predictions" => "下一条命令预测",
        "Open in Warp Desktop?" => "在 Warp 桌面版中打开？",
        "Open conversation" => "打开对话",
        "Oz cloud agents platform" => "Oz 云端智能体平台",
        "Prompt suggestions" => "提示建议",
        "Remote control with Claude Code, Codex, and other agents" => {
            "通过 Claude Code、Codex 和其他智能体远程控制"
        }
        "Separator" => "分隔符",
        "Session Sharing" => "会话共享",
        "Saving..." => "正在保存...",
        "Shortcut" => "快捷键",
        "Store AI conversations in the cloud" => "在云端存储 AI 对话",
        "TRASH" => "废纸篓",
        "Unshare block" => "取消共享块",
        "Use auto-reload to never miss a beat." => "启用自动充值，避免中断。",
        "View screenshot" => "查看截图",
        "Warp agents" => "Warp 智能体",
        "We offer cloud features to all users, and so we need an internet connection to meter AI usage, prevent abuse, and associate cloud objects with users. If you opt to use Warp logged-out, a unique ID will be attached to an anonymous user account in order to support these features." => {
            "我们向所有用户提供云端功能，因此需要联网来计量 AI 使用量、防止滥用，并将云端对象与用户关联。如果你选择未登录使用 Warp，系统会为匿名用户账号附加唯一 ID 以支持这些功能。"
        }
        "You are currently offline. An internet connection is required to use Warp for the first time." => {
            "你当前处于离线状态。首次使用 Warp 需要互联网连接。"
        }
        "You can change this at any time in settings." => "你可以随时在设置中更改。",
        "You don't have any shared blocks yet." => "你还没有任何共享块。",
        "Yes" => "是",
        "New tab" => "新建标签页",
        "New window" => "新建窗口",
        "No notifications" => "没有通知",
        "Notifications" => "通知",
        "Split pane" => "拆分窗格",
        "for terminal" => "返回终端",
        "Auto-reload will automatically purchase credits at your selected rate when your account balance reaches 100 credits. Your monthly spend limit is set at your legacy plan's monthly cost and can be updated in Settings > Billing & usage." => {
            "当账号余额降至 100 点数时，自动充值会按你选择的金额购买点数。你的月度消费限额会设为旧套餐的月度费用，并可在“设置 > 账单与用量”中更新。"
        }
        "Are you sure you want to unshare this block?\n\nIt will no longer be accessible by link and will be permanently deleted from Warp servers." => {
            "确定要取消共享此块吗？\n\n它将无法再通过链接访问，并会从 Warp 服务器永久删除。"
        }
        "Activate next pane" => "激活下一个窗格",
        "Activate previous pane" => "激活上一个窗格",
        "A command in this session is still running." => "此会话中仍有命令在运行。",
        "Add cursor above" => "在上方添加光标",
        "Add cursor below" => "在下方添加光标",
        "Add selection for next occurrence" => "为下一个匹配项添加选择",
        "Agent conversation list view" => "智能体对话列表视图",
        "Clear Blocks" => "清空块",
        "Clear command editor" => "清空命令编辑器",
        "Comments sent to agent" => "评论已发送给智能体",
        "Command Palette" => "命令面板",
        "Command Search" => "命令搜索",
        "Conversation" => "对话",
        "Conversation deleted" => "对话已删除",
        "Conversation forking failed." => "派生对话失败。",
        "Copied branch name" => "已复制分支名",
        "Copied to clipboard" => "已复制到剪贴板",
        "Copy command and output" => "复制命令和输出",
        "Copy command output" => "复制命令输出",
        "Could not submit comments to the agent" => "无法将评论发送给智能体",
        "Custom URI is invalid." => "自定义 URI 无效。",
        "Find" => "查找",
        "Failed to load conversation." => "加载对话失败。",
        "Failed to load file." => "加载文件失败。",
        "Failed to save file." => "保存文件失败。",
        "File saved." => "文件已保存。",
        "Finished exporting objects" => "对象导出完成",
        "Find within selected block" => "在所选块中查找",
        "Focus terminal input" => "聚焦终端输入框",
        "Global Search" => "全局搜索",
        "Go to line" => "转到行",
        "Forked" => "已派生",
        "History" => "历史记录",
        "Link copied" => "链接已复制",
        "Link copied to clipboard" => "链接已复制到剪贴板",
        "Looks like you're out of AI credits." => "看起来你的 AI 点数已用完。",
        "Move tab left" => "将标签页左移",
        "Move tab right" => "将标签页右移",
        "Navigation Palette" => "导航面板",
        "New Personal Environment Variables" => "新建个人环境变量",
        "New Personal Notebook" => "新建个人笔记本",
        "New Personal Prompt" => "新建个人提示词",
        "New Personal Workflow" => "新建个人工作流",
        "New Team Environment Variables" => "新建团队环境变量",
        "New Team Notebook" => "新建团队笔记本",
        "New Team Prompt" => "新建团队提示词",
        "New Team Workflow" => "新建团队工作流",
        "No active conversation to export" => "没有可导出的活动对话",
        "Open PR" => "打开 PR",
        "Open AI Rules" => "知识库",
        "Open MCP Servers" => "MCP 服务器",
        "Project Explorer" => "项目浏览器",
        "PR successfully created." => "PR 创建成功。",
        "Plan ID copied to clipboard" => "计划 ID 已复制到剪贴板",
        "PowerShell subshells not supported" => "不支持 PowerShell 子 Shell",
        "Redo" => "重做",
        "Download QR code" => "下载二维码",
        "QR code downloaded." => "二维码已下载。",
        "Remote control link copied." => "远程控制链接已复制。",
        "Resource not found or access denied" => "资源不存在或无访问权限",
        "Reset Zoom" => "重置缩放",
        "Reset font size to default" => "将字体大小重置为默认值",
        "Sampling process for 3 seconds..." => "正在采样进程 3 秒...",
        "Select all blocks" => "选择所有块",
        "Select next block" => "选择下一个块",
        "Select previous block" => "选择上一个块",
        "Search Drive" => "搜索云盘",
        "Share session QR code" => "共享会话二维码",
        "Share selected block" => "共享所选块",
        "Share pane contents" => "共享窗格内容",
        "Show QR code" => "显示二维码",
        "Stop Synchronizing Any Panes" => "停止同步所有窗格",
        "Scroll to top of selected block" => "滚动到所选块顶部",
        "Scroll to bottom of selected block" => "滚动到所选块底部",
        "Thank you for the feedback!" => "感谢你的反馈！",
        "Unable to create QR code for this session link." => "无法为此会话链接创建二维码。",
        "Unable to download QR code." => "无法下载二维码。",
        "View QR code" => "查看二维码",
        "This plan is already in context." => "此计划已在上下文中。",
        "Toggle Maximize Active Pane" => "切换最大化当前窗格",
        "Toggle Synchronizing All Panes in All Tabs" => "切换在所有标签页中同步所有窗格",
        "Toggle Synchronizing All Panes in Current Tab" => "切换在当前标签页中同步所有窗格",
        "Upgrade for more credits." => "升级以获取更多点数。",
        "View changelog" => "查看更新日志",
        "Workflows" => "工作流",
        "Warp updated!" => "Warp 已更新！",
        "Zoom In" => "放大",
        "Zoom Out" => "缩小",
        "Increase font size" => "增大字体",
        "Decrease font size" => "减小字体",
        "Switch to next tab" => "切换到下一个标签页",
        "Switch to previous tab" => "切换到上一个标签页",
        "Bookmark selected block" => "为所选块添加书签",
        "Attach Selection as Agent Context" => "将选中内容附加为智能体上下文",
        "Cannot open a new terminal session" => "无法打开新的终端会话",
        "Disabled all synchronized inputs." => "已禁用所有同步输入。",
        "Conversation exported to clipboard" => "对话已导出到剪贴板",
        "Failed to prepare file download." => "准备文件下载失败。",
        "Failed to start voice input (you may need to enable Microphone access)" => {
            "启动语音输入失败（你可能需要启用麦克风权限）"
        }
        "Folder has too many files to display in the file explorer." => {
            "文件夹中的文件过多，无法在文件浏览器中显示。"
        }
        "MCP server updated" => "MCP 服务器已更新",
        "No MCP Server specified." => "未指定 MCP 服务器。",
        "This MCP server contains secrets. Visit Settings > Privacy to modify your secret redaction settings." => {
            "此 MCP 服务器包含敏感信息。请前往“设置 > 隐私”修改敏感信息脱敏设置。"
        }
        "Are you sure you want to delete this team?" => "确定要删除此团队吗？",
        "Are you sure you want to leave this team?" => "确定要离开此团队吗？",
        "Are you sure you want to remove this member?" => "确定要移除此成员吗？",
        "Deleting this team will permanently delete it and all of its related content, including billing information or credits. You will not be able to restore them." => {
            "删除此团队会永久删除团队及其所有相关内容，包括账单信息或点数。删除后无法恢复。"
        }
        "You will need to be reinvited in order to rejoin." => "如需重新加入，你需要再次收到邀请。",
        "If you leave this team, you’ll lose access to any remaining reload credits tied to it. You’ll regain access to any unused, non-expired credits if you rejoin the same team later." => {
            "如果你离开此团队，将无法继续使用与该团队关联的剩余补充点数。之后重新加入同一团队时，可重新获得未使用且未过期的点数。"
        }
        "This member will lose access to any remaining reload credits tied to this team. If they rejoin later, they’ll regain access to any unused, non-expired credits." => {
            "此成员将无法继续使用与该团队关联的剩余补充点数。如果之后重新加入，可重新获得未使用且未过期的点数。"
        }
        "Yes, delete" => "是，删除",
        "Yes, leave" => "是，离开",
        "Leave Team" => "离开团队",
        "+ Add model" => "添加模型",
        "Add endpoint" => "添加端点",
        "Are you sure you want to remove this endpoint? You won't be able to use its models in your agent sessions moving forward." => {
            "确定要移除此端点吗？之后你将无法在智能体会话中使用它的模型。"
        }
        "Automatically detect and spawn MCP servers from globally-scoped third-party AI agent configuration files (e.g. in your home directory). Servers detected inside a repository are never spawned automatically and must be enabled individually in the \"Detected from\" sections below. " => {
            "自动检测并启动全局范围第三方 AI 智能体配置文件（例如主目录中）里的 MCP 服务器。在仓库内检测到的服务器永远不会自动启动，必须在下方“检测自”区域中逐个启用。"
        }
        "Balance" => "余额",
        "Copy URL" => "复制 URL",
        "Copy conversation text" => "复制对话文本",
        "Copy output as Markdown" => "复制输出为 Markdown",
        "Copy path" => "复制路径",
        "Copy prompt" => "复制提示",
        "Copy share link" => "复制共享链接",
        "Fork" => "派生",
        "Fork from here" => "从此处派生",
        "Last 30 days" => "最近 30 天",
        "New" => "新",
        "No usage history" => "暂无用量历史",
        "Please include 'https://'" => "请包含 'https://'",
        "Purchased this month" => "本月已购买",
        "Remove endpoint" => "移除端点",
        "Rewind to before here" => "回退到此处之前",
        "Save as prompt" => "另存为提示词",
        "remaining" => "剩余",
        "e.g., GLM-5" => "例如，GLM-5",
        "e.g., GLM-5-FP8" => "例如，GLM-5-FP8",
        "e.g., Zach's external models" => "例如，Zach 的外部模型",
        "e.g., sk-..." => "例如，sk-...",
        "Accept w/o orchestration" => "不编排并接受",
        "Authenticate GitHub" => "认证 GitHub",
        "Add-on credits" => "附加点数",
        " on GitHub so we can better identify the problem." => {
            " 到 GitHub，这样我们能更好地定位问题。"
        }
        " for more powerful cloud agents." => " 以使用更强大的云端智能体。",
        "Agent questions" => "智能体问题",
        "As you or the Agent make changes, you'll be able to track them here." => {
            "当你或智能体进行更改时，可以在这里跟踪这些更改。"
        }
        "Auto reload" => "自动补充",
        "auto-reload" => "自动补充",
        "Background color" => "背景颜色",
        "Bring Warp's features to your remote session. Blocks, full text editing, auto-complete, Oz, and more. " => {
            "将 Warp 功能带到远程会话中，包括块、完整文本编辑、自动补全、Oz 等。"
        }
        "Choose your experience for this remote session:" => "选择此远程会话的体验：",
        "Cloud agents require an environment that they'll run in to get their task done. Create your first environment below. You'll be able to edit the environment later, or add new environments when you need them." => {
            "云端智能体需要一个运行环境来完成任务。请在下方创建你的第一个环境。之后你可以编辑该环境，或在需要时添加新环境。"
        }
        "Code review" => "代码审查",
        "Continue this cloud conversation" => "继续此云端对话",
        "Contact sales" => "联系销售",
        "Create an account" => "创建账户",
        "Create agent" => "创建智能体",
        "Choose your agent" => "选择智能体",
        "Check out the " => "查看 ",
        " to learn more." => " 了解更多。",
        "Getting started with Oz cloud agents" => "Oz 云端智能体入门",
        "Start Oz cloud agents directly in Warp from an integration (Linear, Slack), with an event (GitHub, built-in schedule), or programmatically with the Oz SDK or CLI." => {
            "可直接在 Warp 中通过集成（Linear、Slack）、事件（GitHub、内置计划任务）或 Oz SDK/CLI 以编程方式启动 Oz 云端智能体。"
        }
        "Oz documentation" => "Oz 文档",
        "Quick start: Visit oz.warp.dev for a UI-based setup experience." => {
            "快速开始：访问 oz.warp.dev 使用图形化设置体验。"
        }
        "Manual setup: Create a Slack or Linear integration with the Oz CLI" => {
            "手动设置：使用 Oz CLI 创建 Slack 或 Linear 集成"
        }
        "Create an environment" => "创建环境",
        "First, set up an environment to create an integration." => {
            "首先，设置一个环境来创建集成。"
        }
        "Use Warp's environment setup command to have an agent help you through it. " => {
            "使用 Warp 的环境设置命令，让智能体帮助你完成。"
        }
        "Or, supply your own existing docker image." => "或者，提供你现有的 Docker 镜像。",
        "Create an integration" => "创建集成",
        "Integrate Slack or Linear to assign Warp's Agent tasks with @Warp. " => {
            "集成 Slack 或 Linear，以便使用 @Warp 分配 Warp 智能体任务。"
        }
        "Create Environment" => "创建环境",
        "Create Environment (CLI)" => "创建环境（CLI）",
        "Create Linear Integration" => "创建 Linear 集成",
        "Create Slack Integration" => "创建 Slack 集成",
        "Create your first tab config" => "创建第一个标签页配置",
        "Changes" => "更改",
        "Error loading diffs" => "加载差异失败",
        "Error" => "错误",
        "Artifacts" => "产物",
        "Environment setup commands" => "环境设置命令",
        "Environment details" => "环境详情",
        "Open in Oz" => "在 Oz 中打开",
        "Initial query" => "初始查询",
        "Credits used" => "已用点数",
        "Run time" => "运行时间",
        " Retry" => " 重试",
        "This file has unsaved changes. {shortcut} to save" => {
            "此文件有未保存的更改。按 {shortcut} 保存"
        }
        "Diff is too large to render" => "差异过大，无法渲染",
        "Binary file - no diff available" => "二进制文件，没有可用差异",
        "File renamed without changes" => "文件已重命名，无内容更改",
        "New empty file" => "新建空文件",
        "Unable to load file content" => "无法加载文件内容",
        "No file selected" => "未选择文件",
        "No files to discard" => "没有可丢弃的文件",
        "No matches" => "无匹配项",
        "Included commits" => "包含的提交",
        "Include unstaged" => "包含未暂存更改",
        "Commit message" => "提交消息",
        "Install" => "安装",
        "No updates available" => "没有可用更新",
        "No MCP server selected" => "未选择 MCP 服务器",
        "No tools available" => "没有可用工具",
        "Custom models" => "自定义模型",
        "Disabled by your administrator" => "已被管理员禁用",
        "Don't ask me this again" => "不要再次询问",
        "Docker image to use for the environment" => "环境使用的 Docker 镜像",
        "Fix with Oz" => "使用 Oz 修复",
        "filing an issue" => "提交 issue",
        "Free credits" => "免费点数",
        "GitHub link or local filepath to the repository" => "仓库的 GitHub 链接或本地文件路径",
        "Grep for " => "Grep 搜索 ",
        "Grepping for " => "正在 Grep 搜索 ",
        "Hand off to cloud (or type &)" => "移交到云端（或输入 &）",
        "ID of the environment to integrate with" => "要集成的环境 ID",
        "In order to Warpify your SSH session, a more recent version of tmux (>=3.0) must be installed. " => {
            "要对 SSH 会话启用 Warpify，需要安装较新版本的 tmux（>=3.0）。"
        }
        "In order to Warpify your SSH session, tmux must be installed. " => {
            "要对 SSH 会话启用 Warpify，需要先安装 tmux。"
        }
        "Installed" => "已安装",
        "Loading session..." => "正在加载会话...",
        "Loading…" => "加载中…",
        "Manage Warpify settings" => "管理 Warpify 设置",
        "Manage Agent permissions" => "管理智能体权限",
        "More info" => "更多信息",
        "more info" => "更多信息",
        "Name for the environment" => "环境名称",
        "New conversation" => "新对话",
        "New environment" => "新环境",
        "No conversations yet" => "暂无对话",
        "No open changes" => "没有打开的更改",
        "No secrets found" => "未找到密钥",
        "No secrets found. Save to use this value directly or click the key to add a secret." => {
            "未找到密钥。保存后可直接使用此值，或点击密钥图标添加密钥。"
        }
        "No folders have been initialized yet." => "尚未初始化任何文件夹。",
        "Or" => "或",
        "Please report any issues" => "请报告任何问题",
        "Questions unavailable" => "问题不可用",
        "Raw" => "原始",
        "Reload Credits" => "补充点数",
        "Rendered" => "渲染后",
        "Runs autonomously in a cloud environment you choose. Best for parallel or long-running work." => {
            "在你选择的云端环境中自主运行。最适合并行或长时间运行的工作。"
        }
        "Runs on your machine and requires supervision. Best for quick, interactive tasks." => {
            "在你的机器上运行，需要你监督。最适合快速、交互式任务。"
        }
        "Search secrets or create a new one" => "搜索密钥或新建密钥",
        "Sending message to " => "正在发送消息给 ",
        "Session type" => "会话类型",
        "Set up a reusable starting point for your tabs. Pick a repo, choose a session type, and optionally attach a worktree. Use it whenever you want to open a tab with this setup." => {
            "为标签页设置可复用的起点。选择仓库、会话类型，并可选择附加工作树。之后需要用这套设置打开标签页时可随时使用。"
        }
        "Share with team" => "与团队共享",
        "Show initialization block" => "显示初始化块",
        "Start a new Oz cloud agent" => "启动新的 Oz 云端智能体",
        "Tasks" => "任务",
        "Theme name" => "主题名称",
        "Total overages" => "超额总量",
        "Unable to load secrets" => "无法加载密钥",
        "Usage" => "用量",
        "Use cloud agents to run parallel agents, build agents that run autonomously, and check in on your agents from anywhere. " => {
            "使用云端智能体运行并行智能体、构建可自主运行的智能体，并随时随地查看智能体状态。"
        }
        "Use Oz cloud agents to run parallel agents, build agents that run autonomously, and check in on your agents from anywhere. " => {
            "使用 Oz 云端智能体运行并行智能体、构建可自主运行的智能体，并随时随地查看智能体状态。"
        }
        "Visit docs" => "查看文档",
        "We are actively working on improving the stability of SSH in Warp. Please consider " => {
            "我们正在积极提升 Warp 中 SSH 的稳定性。请考虑"
        }
        "When enabled, " => "启用后，",
        " will automatically purchase your selected package when you run out. " => {
            " 会在点数用完时自动购买你选择的套餐。"
        }
        "Why do I need tmux?" => "为什么需要 tmux？",
        "pricing page" => "价格页面",
        "press enter" => "按 Enter",
        "settings" => "设置",
        "these instructions" => "这些说明",
        "{display_name} is not available for free users. " => {
            "{display_name} 不对免费用户开放。"
        }
        "Run {package_manager_name} to update" => "运行 {package_manager_name} 进行更新",
        "Runs" => "运行",
        "Loading cloud agent runs" => "正在加载云端智能体运行",
        "Loading agents..." => "正在加载智能体...",
        "No results matched your filters" => "没有符合筛选条件的结果",
        "Fork current conversation" => "复刻当前对话",
        "No matching conversations" => "没有匹配的对话",
        "No tabs match your search." => "没有匹配搜索的标签页。",
        "No results found" => "未找到结果",
        "Loading results..." => "正在加载结果...",
        "Code symbols indexing..." => "正在索引代码符号...",
        "I'm looking for..." => "我想查找...",
        "Example queries" => "示例查询",
        "Failed to start environment" => "启动环境失败",
        "GitHub Authentication Required" => "需要 GitHub 认证",
        "Please authenticate with GitHub to continue" => "请通过 GitHub 认证以继续",
        "Cloud Agent Run Cancelled" => "云端智能体运行已取消",
        "No cloud environment was started" => "未启动云端环境",
        "Allow" => "允许",
        "Skip all" => "全部跳过",
        "Refine" => "优化",
        "Take over" => "接管",
        "Take control" => "取得控制权",
        "Run" => "运行",
        "Reject" => "拒绝",
        "AWS credentials expired or missing" => "AWS 凭证已过期或缺失",
        "Failed to authenticate with AWS Bedrock when using {model}. Run `{command}` to refresh credentials." => {
            "使用 {model} 时无法通过 AWS Bedrock 认证。运行 `{command}` 刷新凭证。"
        }
        "Always run automatically" => "始终自动运行",
        "New terminal session" => "新终端会话",
        "Thinking..." => "正在思考...",
        "Customizable in appearance settings." => "可在外观设置中自定义。",
        "Project explorer unavailable" => "项目资源管理器不可用",
        "Add as context" => "添加为上下文",
        "This file has saved changes that are not reflected here." => {
            "此文件有已保存但尚未反映到这里的更改。"
        }
        "Install nvm to enable version switching" => "安装 nvm 以启用版本切换",
        "This menu helps you switch between Node.js versions — but it requires nvm to be installed." => {
            "此菜单可帮助你切换 Node.js 版本，但需要先安装 nvm。"
        }
        "No node versions installed" => "未安装 Node 版本",
        "Try installing versions with nvm" => "尝试使用 nvm 安装版本",
        "New worktree" => "新建工作树",
        "Select repository" => "选择仓库",
        "Select branch" => "选择分支",
        "Autogenerate worktree branch name" => "自动生成工作树分支名称",
        "Worktree branch name" => "工作树分支名称",
        "Select directory" => "选择目录",
        "Select a git repository to enable worktree support" => {
            "选择一个 Git 仓库以启用工作树支持"
        }
        "Automatically create a worktree when opening a new tab" => {
            "打开新标签页时自动创建工作树"
        }
        "You must select that you want to automatically create a worktree in order to select this" => {
            "需要先选择自动创建工作树，才能选择此项"
        }
        "Auto-generate worktree branch name" => "自动生成工作树分支名称",
        "Warp will prevent use of premium models when this dollar limit is reached. Resets on a monthly basis." => {
            "达到此美元限额后，Warp 将阻止使用高级模型。该限额每月重置。"
        }
        "Note that AI credits made near your chosen limit may exceed it by a few dollars." => {
            "请注意，接近所选限额时产生的 AI 点数费用可能会超出几美元。"
        }
        "Kick off an agent task to view usage history here." => {
            "启动一个智能体任务后，可在此查看使用历史。"
        }
        "Enable terminal command autodetection" => "启用终端命令自动检测",
        "Disable terminal command autodetection" => "禁用终端命令自动检测",
        "Turn off auto-approve all agent actions" => "关闭所有智能体操作的自动批准",
        "Auto-approve all agent actions for this task" => "自动批准此任务的所有智能体操作",
        "Start remote control" => "启动远程控制",
        "Log in to use /remote-control" => "登录后使用 /remote-control",
        "Warp plugin installed. Please restart the session to activate." => {
            "Warp 插件已安装。请重启会话以激活。"
        }
        "Warp plugin updated. Please restart the session to activate." => {
            "Warp 插件已更新。请重启会话以激活。"
        }
        "Installing Warp plugin..." => "正在安装 Warp 插件...",
        "Failed to install Warp plugin" => "Warp 插件安装失败",
        "Updating Warp plugin..." => "正在更新 Warp 插件...",
        "Failed to update Warp plugin" => "Warp 插件更新失败",
        "See logs for details" => "查看日志了解详情",
        "Voice input limit reached" => "已达到语音输入限制",
        "Failed to transcribe voice input" => "语音输入转录失败",
        "Voice input is enabled. You can also press and hold the `{key}` key to activate voice input (configure in Settings > AI > Voice)" => {
            "语音输入已启用。你也可以按住 `{key}` 键来激活语音输入（可在 设置 > AI > 语音 中配置）"
        }
        "\nThe " => "\n该 ",
        "You have 1 free credit to use on Oz cloud agents." => {
            "你有 1 个免费点数可用于 Oz 云端智能体。"
        }
        "You have {credits} free credits to use on Oz cloud agents." => {
            "你有 {credits} 个免费点数可用于 Oz 云端智能体。"
        }
        "Your active and past conversations with local and ambient agents will appear here." => {
            "你与本地和环境智能体的当前及历史对话会显示在这里。"
        }
        "Orchestrate any agent, anywhere" => "在任意位置编排任意智能体",
        "We've made major improvements to Warp's cloud agent orchestration platform, Oz." => {
            "我们对 Warp 的云端智能体编排平台 Oz 做了重大改进。"
        }
        "Run any agent harness in the cloud" => "在云端运行任意智能体运行框架",
        "Use Oz to spin up Claude Code or Codex agents in the cloud; Oz will help you track and steer the agents." => {
            "使用 Oz 在云端启动 Claude Code 或 Codex 智能体；Oz 会帮助你跟踪和引导这些智能体。"
        }
        "Multi-agent orchestration" => "多智能体编排",
        "Warp Agents will now orchestrate swarms of subagents, allowing you to parallelize tasks." => {
            "Warp 智能体现在可以编排多个子智能体，帮助你并行处理任务。"
        }
        "Agent Memory" => "智能体记忆",
        "Agents will now store and access long-term memories, enabling self-improvement over time." => {
            "智能体现在会存储和访问长期记忆，从而随着时间自我改进。"
        }
        "Research preview" => "研究预览",
        "Warp is now open-source" => "Warp 现已开源",
        "You, our community, can participate in building Warp using an agent-first workflow." => {
            "你和我们的社区可以通过智能体优先的工作流参与构建 Warp。"
        }
        "Contribute" => "参与贡献",
        "Warp's client code is now open source. Get started by using the /feedback skill to open an issue, and follow the contribution guidelines here." => {
            "Warp 客户端代码现已开源。你可以使用 /feedback 技能提交 issue，并在此处查看贡献指南。"
        }
        "here" => "此处",
        "Open Automated Development" => "开放自动化开发",
        "The Warp repo is managed by an agent-first workflow powered by Oz, our cloud agent orchestration platform." => {
            "Warp 仓库采用由 Oz（我们的云端智能体编排平台）驱动的智能体优先工作流进行管理。"
        }
        "Introducing 'auto (open-weights)'" => "介绍“auto (open-weights)”",
        "We've added a new auto model that picks the best open weight model for a task, like Kimi or MiniMax." => {
            "我们新增了一个 auto 模型，它会为任务选择最合适的开源权重模型，例如 Kimi 或 MiniMax。"
        }
        "Introducing universal agent support: level up any coding agent with Warp" => {
            "介绍通用智能体支持：用 Warp 升级任何编码智能体"
        }
        "Vertical tabs" => "垂直标签页",
        "Rich tab titles and metadata like git branch, worktree, and PR. Fully customizable." => {
            "丰富的标签标题和元数据，例如 Git 分支、工作树和 PR。完全可自定义。"
        }
        "Tab-level schema to set your directory, startup commands, theme, and worktree with one click" => {
            "使用标签级 schema 一键设置目录、启动命令、主题和工作树。"
        }
        "Agent inbox" => "智能体收件箱",
        "Notifications when any agent needs your attention, also accessible in a central inbox" => {
            "当任何智能体需要你关注时发出通知，也可在中央收件箱中访问。"
        }
        "Native code review" => "原生代码审查",
        "Send inline comments from Warp's code review directly to Claude Code, Codex, or OpenCode" => {
            "将 Warp 代码审查中的行内评论直接发送给 Claude Code、Codex 或 OpenCode。"
        }
        "Use your custom prompt" => "使用自定义提示符",
        "Set up Warp to honor your PS1 setting" => "设置 Warp 以遵循你的 PS1 设置",
        "View documentation" => "查看文档",
        "Integrate Warp with your IDE" => "将 Warp 与 IDE 集成",
        "Configure Warp to launch from your most used development tools" => {
            "配置 Warp 从你最常用的开发工具启动"
        }
        "How Warp uses Warp" => "Warp 如何使用 Warp",
        "Learn how Warp's engineering team uses their favorite features" => {
            "了解 Warp 工程团队如何使用他们最喜欢的功能"
        }
        "Learn more on our " => "在我们的 ",
        "Read article" => "阅读文章",
        "Create your first block" => "创建第一个块",
        "Run a command to see your command and output grouped." => {
            "运行命令，查看命令和输出如何组合显示。"
        }
        "Navigate blocks" => "导航块",
        "Click to select a block and navigate with arrow keys." => {
            "点击选择块，并使用方向键导航。"
        }
        "Take an action on block" => "对块执行操作",
        "Right click on a block to copy/paste, share, more." => {
            "右键点击块以复制/粘贴、共享或查看更多操作。"
        }
        "Open command palette" => "打开命令面板",
        "Access all of Warp via the keyboard." => "通过键盘访问 Warp 的全部功能。",
        "Set your theme" => "设置主题",
        "Make Warp your own by choosing a theme." => "选择主题，让 Warp 更符合你的偏好。",
        "Command search" => "命令搜索",
        "Find and run previously executed commands, workflows, and more." => {
            "查找并运行之前执行过的命令、工作流等。"
        }
        "AI command search" => "AI 命令搜索",
        "Generate shell commands with natural language." => "使用自然语言生成 Shell 命令。",
        "Split panes" => "拆分窗格",
        "Split tabs into multiple panes to make your ideal layout." => {
            "将标签页拆分为多个窗格，构建理想布局。"
        }
        "Launch configuration" => "启动配置",
        "Save your current configuration of windows, tabs, and panes." => {
            "保存当前窗口、标签页和窗格配置。"
        }
        "Start a new conversation" => "开始新对话",
        "Start a new cloud agent conversation" => "开始新的云端智能体对话",
        "Add a new MCP server via the MCP settings page" => {
            "通过 MCP 设置页面添加新的 MCP 服务器"
        }
        "Pull GitHub PR review comments" => "拉取 GitHub PR 审查评论",
        "Create an Oz environment (Docker image + repos) via guided setup" => {
            "通过引导式设置创建 Oz 环境（Docker 镜像 + 仓库）"
        }
        "Create a new docker sandbox terminal session" => "创建新的 Docker 沙箱终端会话",
        "Have Oz walk you through creating a new coding project" => {
            "让 Oz 引导你创建新的编码项目"
        }
        "Open a skill's markdown file in Warp's built-in editor" => {
            "在 Warp 内置编辑器中打开技能的 Markdown 文件"
        }
        "Invoke a skill" => "调用技能",
        "Add new Agent prompt" => "添加新的智能体提示词",
        "Add a new global rule for the agent" => "为智能体添加新的全局规则",
        "Open a file in Warp's code editor" => "在 Warp 代码编辑器中打开文件",
        "Set the color of the current tab" => "设置当前标签页颜色",
        "Fork the current conversation in a new pane or a new tab" => {
            "在新窗格或新标签页中派生当前对话"
        }
        "Hand off this conversation to a cloud agent" => "将此对话移交给云端智能体",
        "Open code review" => "打开代码审查",
        "Index this codebase" => "索引此代码库",
        "Index this codebase and generate an AGENTS.md file" => {
            "索引此代码库并生成 AGENTS.md 文件"
        }
        "Open the project rules file (AGENTS.md)" => "打开项目规则文件（AGENTS.md）",
        "Open MCP servers" => "打开 MCP 服务器",
        "Open settings file (TOML)" => "打开设置文件（TOML）",
        "Open the latest changelog" => "打开最新更新日志",
        "Send feedback" => "发送反馈",
        "Switch to another indexed repository" => "切换到另一个已索引仓库",
        "View all of your global and project rules" => "查看所有全局和项目规则",
        "Start a new conversation (alias for /agent)" => "开始新对话（/agent 的别名）",
        "Switch the base agent model" => "切换基础智能体模型",
        "Switch the cloud agent execution host" => "切换云端智能体执行主机",
        "Switch the cloud agent harness" => "切换云端智能体运行框架",
        "Switch the cloud agent environment" => "切换云端智能体环境",
        "Switch the active execution profile" => "切换当前执行配置",
        "Prompt the agent to do some research and create a plan for a task" => {
            "提示智能体进行研究并为任务创建计划"
        }
        "Break a task into subtasks and run them in parallel with multiple agents" => {
            "将任务拆分为子任务，并用多个智能体并行运行"
        }
        "Free up context by summarizing convo history" => "通过总结对话历史释放上下文",
        "Compact conversation and then send a follow-up prompt" => {
            "压缩对话后发送后续提示"
        }
        "Queue a prompt to send after the agent finishes responding" => {
            "将提示加入队列，在智能体完成响应后发送"
        }
        "Fork current conversation and compact it in the forked copy" => {
            "派生当前对话，并在派生副本中压缩它"
        }
        "Fork conversation from a specific query" => "从指定查询派生对话",
        "Continue this cloud conversation locally" => "在本地继续此云端对话",
        "Open billing and usage settings" => "打开账单与用量设置",
        "Start remote control for this session" => "为此会话启动远程控制",
        "Toggle credit usage details" => "切换点数用量详情",
        "Open conversation history" => "打开对话历史",
        "Search saved prompts" => "搜索已保存提示词",
        "Rewind to a previous point in the conversation" => "回退到对话中的先前位置",
        "Export current conversation to clipboard in markdown format" => {
            "以 Markdown 格式将当前对话导出到剪贴板"
        }
        "Export current conversation to a markdown file" => {
            "将当前对话导出为 Markdown 文件"
        }
        _ => return None,
    })
}

fn en_us(key: I18nKey) -> &'static str {
    match key {
        I18nKey::SettingsSearchPlaceholder => "Search",
        I18nKey::SettingsSearchNoResultsTitle => "No settings match your search.",
        I18nKey::SettingsSearchNoResultsDescription => {
            "You may want to try using different keywords or checking for any possible typos."
        }
        I18nKey::SettingsNavAbout => "About",
        I18nKey::SettingsNavAccount => "Account",
        I18nKey::SettingsNavAgents => "Agents",
        I18nKey::SettingsNavBillingAndUsage => "Billing and usage",
        I18nKey::SettingsNavCode => "Code",
        I18nKey::SettingsNavCloudPlatform => "Cloud platform",
        I18nKey::SettingsNavTeams => "Teams",
        I18nKey::SettingsNavAppearance => "Appearance",
        I18nKey::SettingsNavFeatures => "Features",
        I18nKey::SettingsNavKeybindings => "Keyboard shortcuts",
        I18nKey::SettingsNavWarpify => "Warpify",
        I18nKey::SettingsNavReferrals => "Referrals",
        I18nKey::SettingsNavSharedBlocks => "Shared blocks",
        I18nKey::SettingsNavWarpDrive => "Warp Drive",
        I18nKey::SettingsNavPrivacy => "Privacy",
        I18nKey::SettingsNavMcpServers => "MCP Servers",
        I18nKey::SettingsNavWarpAgent => "Warp Agent",
        I18nKey::SettingsNavAgentProfiles => "Profiles",
        I18nKey::SettingsNavAgentMcpServers => "MCP servers",
        I18nKey::SettingsNavKnowledge => "Knowledge",
        I18nKey::SettingsNavThirdPartyCliAgents => "Third party CLI agents",
        I18nKey::SettingsNavCodeIndexing => "Indexing and projects",
        I18nKey::SettingsNavEditorAndCodeReview => "Editor and Code Review",
        I18nKey::SettingsNavCloudEnvironments => "Environments",
        I18nKey::SettingsNavOzCloudApiKeys => "Oz Cloud API Keys",
        I18nKey::TeamsCreateTeam => "Create a team",
        I18nKey::TeamsJoinExistingTeam => "Or, join an existing team within your company",
        I18nKey::TeamsTeamMembers => "Team members",
        I18nKey::TeamsInviteByLink => "Invite by Link",
        I18nKey::TeamsInviteByEmail => "Invite by Email",
        I18nKey::TeamsRestrictByDomain => "Restrict by domain",
        I18nKey::TeamsMakeTeamDiscoverable => "Make team discoverable",
        I18nKey::TeamsFreePlanUsageLimits => "Free plan usage limits",
        I18nKey::TeamsPlanUsageLimits => "Plan usage limits",
        I18nKey::TeamsSharedNotebooks => "Shared Notebooks",
        I18nKey::TeamsSharedWorkflows => "Shared Workflows",
        I18nKey::TeamsNeedMoreSeatsPrefix => "Need more seats? ",
        I18nKey::TeamsUpgradeToEnterprise => "Upgrade to Enterprise",
        I18nKey::TeamsUpgradeToBusiness => "Upgrade to Business",
        I18nKey::TeamsUnlimitedTeamMemberLimitSuffix => {
            " for an unlimited team member limit."
        }
        I18nKey::TeamsHigherTeamMemberLimitSuffix => " for a higher team member limit.",
        I18nKey::MenuWhatsNew => "What's new",
        I18nKey::MenuSettings => "Settings",
        I18nKey::MenuKeyboardShortcuts => "Keyboard shortcuts",
        I18nKey::MenuDocumentation => "Documentation",
        I18nKey::MenuFeedback => "Feedback",
        I18nKey::MenuViewWarpLogs => "View Warp logs",
        I18nKey::MenuSlack => "Slack",
        I18nKey::MenuSignUp => "Sign up",
        I18nKey::MenuBillingAndUsage => "Billing and usage",
        I18nKey::MenuUpgrade => "Upgrade",
        I18nKey::MenuInviteAFriend => "Invite a friend",
        I18nKey::MenuLogOut => "Log out",
        I18nKey::MenuUpdateAndRelaunchWarp => "Update and relaunch Warp",
        I18nKey::MenuUpdatingTo => "Updating to",
        I18nKey::MenuUpdateWarpManually => "Update Warp manually",
        I18nKey::MenuRearrangeToolbarItems => "Re-arrange toolbar items",
        I18nKey::MenuNewWindow => "New Window",
        I18nKey::MenuNewTerminalTab => "New Terminal Tab",
        I18nKey::MenuNewAgentTab => "New Agent Tab",
        I18nKey::MenuPreferences => "Preferences",
        I18nKey::MenuPrivacyPolicy => "Privacy Policy...",
        I18nKey::MenuDebug => "Debug",
        I18nKey::MenuSetDefaultTerminal => "Set Warp as Default Terminal",
        I18nKey::MenuFile => "File",
        I18nKey::MenuOpenRecent => "Open Recent",
        I18nKey::MenuLaunchConfigurations => "Launch Configurations",
        I18nKey::MenuSaveNew => "Save New...",
        I18nKey::MenuOpenRepository => "Open Repository",
        I18nKey::MenuCloseFocusedPanel => "Close focused panel",
        I18nKey::MenuCloseWindow => "Close Window",
        I18nKey::MenuEdit => "Edit",
        I18nKey::MenuUseWarpsPrompt => "Use Warp's Prompt",
        I18nKey::MenuCopyOnSelectWithinTerminal => "Copy on Select within the Terminal",
        I18nKey::MenuSynchronizeInputs => "Synchronize Inputs",
        I18nKey::MenuView => "View",
        I18nKey::MenuToggleMouseReporting => "Toggle Mouse Reporting",
        I18nKey::MenuToggleScrollReporting => "Toggle Scroll Reporting",
        I18nKey::MenuToggleFocusReporting => "Toggle Focus Reporting",
        I18nKey::MenuCompactMode => "Compact Mode",
        I18nKey::MenuTab => "Tab",
        I18nKey::MenuAi => "AI",
        I18nKey::MenuBlocks => "Blocks",
        I18nKey::MenuDrive => "Drive",
        I18nKey::MenuWindow => "Window",
        I18nKey::MenuHelp => "Help",
        I18nKey::MenuSendFeedback => "Send Feedback...",
        I18nKey::MenuWarpDocumentation => "Warp Documentation...",
        I18nKey::MenuGithubIssues => "GitHub Issues...",
        I18nKey::MenuWarpSlackCommunity => "Warp Slack Community...",
        I18nKey::CommonAgent => "Agent",
        I18nKey::CommonTerminal => "Terminal",
        I18nKey::CommonCloudOz => "Cloud Oz",
        I18nKey::CommonLocalDockerSandbox => "Local Docker Sandbox",
        I18nKey::CommonNewWorktreeConfig => "New worktree config",
        I18nKey::CommonNewTabConfig => "New tab config",
        I18nKey::CommonReopenClosedSession => "Reopen closed session",
        I18nKey::CommonSplitPaneRight => "Split pane right",
        I18nKey::CommonSplitPaneLeft => "Split pane left",
        I18nKey::CommonSplitPaneDown => "Split pane down",
        I18nKey::CommonSplitPaneUp => "Split pane up",
        I18nKey::CommonClosePane => "Close pane",
        I18nKey::CommonCopyLink => "Copy link",
        I18nKey::CommonRename => "Rename",
        I18nKey::CommonShare => "Share",
        I18nKey::CommonCollapseAll => "Collapse all",
        I18nKey::CommonRetry => "Retry",
        I18nKey::CommonRevertToServer => "Revert to server",
        I18nKey::CommonAttachToActiveSession => "Attach to active session",
        I18nKey::CommonCopyId => "Copy id",
        I18nKey::CommonCopyVariables => "Copy variables",
        I18nKey::CommonLoadInSubshell => "Load in subshell",
        I18nKey::CommonOpenOnDesktop => "Open on Desktop",
        I18nKey::CommonDuplicate => "Duplicate",
        I18nKey::CommonExport => "Export",
        I18nKey::CommonTrash => "Trash",
        I18nKey::CommonOpen => "Open",
        I18nKey::CommonEdit => "Edit",
        I18nKey::CommonRestore => "Restore",
        I18nKey::CommonDeleteForever => "Delete forever",
        I18nKey::CommonUntitled => "Untitled",
        I18nKey::CommonRefresh => "Refresh",
        I18nKey::CommonCancel => "Cancel",
        I18nKey::CommonDelete => "Delete",
        I18nKey::CommonReject => "Reject",
        I18nKey::CommonOverwrite => "Overwrite",
        I18nKey::CommonPrevious => "Previous",
        I18nKey::CommonNext => "Next",
        I18nKey::CommonUndo => "Undo",
        I18nKey::CommonRemove => "Remove",
        I18nKey::CommonSave => "Save",
        I18nKey::CommonVariables => "Variables",
        I18nKey::CommonEditing => "Editing",
        I18nKey::CommonViewing => "Viewing",
        I18nKey::CommonMoveTo => "Move to",
        I18nKey::TabStopSharing => "Stop sharing",
        I18nKey::TabShareSession => "Share session",
        I18nKey::TabStopSharingAll => "Stop sharing all",
        I18nKey::TabRenameTab => "Rename tab",
        I18nKey::TabResetTabName => "Reset tab name",
        I18nKey::TabMoveTabDown => "Move Tab Down",
        I18nKey::TabMoveTabRight => "Move Tab Right",
        I18nKey::TabMoveTabUp => "Move Tab Up",
        I18nKey::TabMoveTabLeft => "Move Tab Left",
        I18nKey::TabCloseTab => "Close tab",
        I18nKey::TabCloseOtherTabs => "Close other tabs",
        I18nKey::TabCloseTabsBelow => "Close Tabs Below",
        I18nKey::TabCloseTabsRight => "Close Tabs to the Right",
        I18nKey::TabSaveAsNewConfig => "Save as new config",
        I18nKey::CommonCopy => "Copy",
        I18nKey::CommonCut => "Cut",
        I18nKey::CommonPaste => "Paste",
        I18nKey::CommonSelectAll => "Select all",
        I18nKey::CommonCopyPath => "Copy path",
        I18nKey::CommonOpenInWarp => "Open in Warp",
        I18nKey::CommonOpenInEditor => "Open in editor",
        I18nKey::CommonShowInFinder => "Show in Finder",
        I18nKey::CommonShowContainingFolder => "Show containing folder",
        I18nKey::TerminalCopyUrl => "Copy URL",
        I18nKey::TerminalInsertIntoInput => "Insert into input",
        I18nKey::TerminalCopyCommand => "Copy command",
        I18nKey::TerminalCopyCommands => "Copy commands",
        I18nKey::TerminalFindWithinBlock => "Find within block",
        I18nKey::TerminalFindWithinBlocks => "Find within blocks",
        I18nKey::TerminalScrollToTopOfBlock => "Scroll to top of block",
        I18nKey::TerminalScrollToTopOfBlocks => "Scroll to top of blocks",
        I18nKey::TerminalScrollToBottomOfBlock => "Scroll to bottom of block",
        I18nKey::TerminalScrollToBottomOfBlocks => "Scroll to bottom of blocks",
        I18nKey::TerminalShare => "Share...",
        I18nKey::TerminalShareBlock => "Share block...",
        I18nKey::TerminalSaveAsWorkflow => "Save as workflow",
        I18nKey::TerminalAskWarpAi => "Ask Warp AI",
        I18nKey::TerminalCopyOutput => "Copy output",
        I18nKey::TerminalCopyFilteredOutput => "Copy filtered output",
        I18nKey::TerminalToggleBlockFilter => "Toggle block filter",
        I18nKey::TerminalToggleBookmark => "Toggle bookmark",
        I18nKey::TerminalForkFromHere => "Fork from here (dev only)",
        I18nKey::TerminalRewindToBeforeHere => "Rewind to before here",
        I18nKey::TerminalCopyPrompt => "Copy prompt",
        I18nKey::TerminalCopyRightPrompt => "Copy right prompt",
        I18nKey::TerminalCopyWorkingDirectory => "Copy working directory",
        I18nKey::TerminalCopyGitBranch => "Copy git branch",
        I18nKey::TerminalEditCliAgentToolbelt => "Edit CLI agent toolbelt",
        I18nKey::TerminalEditAgentToolbelt => "Edit agent toolbelt",
        I18nKey::TerminalEditPrompt => "Edit prompt",
        I18nKey::TerminalCommandSearch => "Command search",
        I18nKey::TerminalAiCommandSearch => "AI command search",
        I18nKey::TerminalCopyOutputAsMarkdown => "Copy output as Markdown",
        I18nKey::TerminalSaveAsPrompt => "Save as prompt",
        I18nKey::TerminalShareConversation => "Share conversation",
        I18nKey::TerminalCopyConversationText => "Copy conversation text",
        I18nKey::DriveFolder => "Folder",
        I18nKey::DriveNotebook => "Notebook",
        I18nKey::DriveWorkflow => "Workflow",
        I18nKey::DrivePrompt => "Prompt",
        I18nKey::DriveEnvironmentVariables => "Environment variables",
        I18nKey::DriveNewFolder => "New folder",
        I18nKey::DriveNewNotebook => "New notebook",
        I18nKey::DriveNewWorkflow => "New workflow",
        I18nKey::DriveNewPrompt => "New prompt",
        I18nKey::DriveNewEnvironmentVariables => "New environment variables",
        I18nKey::DriveImport => "Import",
        I18nKey::DriveRemove => "Remove",
        I18nKey::DriveEmptyTrash => "Empty trash",
        I18nKey::DriveTrashTitle => "Trash",
        I18nKey::DriveCreateTeamText => "Share commands & knowledge with your teammates.",
        I18nKey::DriveTeamZeroStateText => {
            "Drag or move a personal workflow or notebook here to share it with your team."
        }
        I18nKey::DriveOfflineBannerText => "You are offline. Some files will be read only.",
        I18nKey::DriveCopyPrompt => "Copy prompt",
        I18nKey::DriveCopyWorkflowText => "Copy workflow text",
        I18nKey::DriveCopyToPersonal => "Copy to Personal",
        I18nKey::DriveCopyAll => "Copy All",
        I18nKey::DriveRestoreNotebookTooltip => "Restore notebook from trash",
        I18nKey::DriveCopyNotebookToPersonalTooltip => {
            "Copy notebook contents into your personal workspace"
        }
        I18nKey::DriveCopyNotebookToClipboardTooltip => {
            "Copy notebook contents to your clipboard"
        }
        I18nKey::DriveRestoreWorkflowTooltip => "Restore workflow from trash",
        I18nKey::DriveRefreshNotebookTooltip => "Refresh notebook",
        I18nKey::WorkflowSignInToEditTooltip => "Sign in to edit",
        I18nKey::EnvVarsCommand => "Command",
        I18nKey::EnvVarsClearSecret => "Clear secret",
        I18nKey::EnvVarsNoAccess => {
            "You no longer have access to these environment variables"
        }
        I18nKey::EnvVarsMovedToTrash => "Environment variables were moved to trash",
        I18nKey::EnvVarsRestoreTooltip => "Restore environment variables from trash",
        I18nKey::CodeHunk => "Hunk:",
        I18nKey::CodeOpenInNewPane => "Open in new pane",
        I18nKey::CodeOpenInNewTab => "Open in new tab",
        I18nKey::CodeOpenFile => "Open file",
        I18nKey::CodeNewFile => "New file",
        I18nKey::CodeCdToDirectory => "cd to directory",
        I18nKey::CodeRevealInFinder => "Reveal in Finder",
        I18nKey::CodeRevealInExplorer => "Reveal in Explorer",
        I18nKey::CodeRevealInFileManager => "Reveal in file manager",
        I18nKey::CodeAttachAsContext => "Attach as context",
        I18nKey::CodeCopyRelativePath => "Copy relative path",
        I18nKey::CodeOpeningUnavailableRemoteTooltip => {
            "Opening files is unavailable for remote sessions"
        }
        I18nKey::CodeGoToDefinition => "Go to definition",
        I18nKey::CodeFindReferences => "Find references",
        I18nKey::CodeDiscardThisVersion => "Discard this version",
        I18nKey::CodeAcceptAndSave => "Accept and save",
        I18nKey::CodeCloseSaved => "Close saved",
        I18nKey::CodeCopyFilePath => "Copy file path",
        I18nKey::CodeViewMarkdownPreview => "View Markdown preview",
        I18nKey::CodeSearchDiffSetsPlaceholder => {
            "Search diff sets or branches to compare…"
        }
        I18nKey::CodeLineNumberColumnPlaceholder => "Line number:Column",
        I18nKey::CodeReviewCopyText => "Copy text",
        I18nKey::CodeReviewSendToAgent => "Send to Agent",
        I18nKey::CodeReviewViewInGithub => "View in GitHub",
        I18nKey::CodeReviewOneComment => "1 Comment",
        I18nKey::CodeReviewCommentSingular => "comment",
        I18nKey::CodeReviewCommentPlural => "comments",
        I18nKey::CodeReviewOutdatedCommentSingular => "outdated comment",
        I18nKey::CodeReviewOutdatedCommentPlural => "outdated comments",
        I18nKey::CodeReviewCommit => "Commit",
        I18nKey::CodeReviewPush => "Push",
        I18nKey::CodeReviewPublish => "Publish",
        I18nKey::CodeReviewCreatePr => "Create PR",
        I18nKey::CodeReviewAddDiffSetAsContext => "Add diff set as context",
        I18nKey::CodeReviewAddFileDiffAsContext => "Add file diff as context",
        I18nKey::CodeReviewShowSavedComment => "Show saved comment",
        I18nKey::CodeReviewAddComment => "Add comment",
        I18nKey::CodeReviewDiscardAll => "Discard all",
        I18nKey::CodeReviewStashChanges => "Stash changes",
        I18nKey::CodeReviewNoChangesToCommit => "No changes to commit",
        I18nKey::CodeReviewNoGitActionsAvailable => "No git actions available",
        I18nKey::CodeReviewMaximize => "Maximize",
        I18nKey::CodeReviewShowFileNavigation => "Show file navigation",
        I18nKey::CodeReviewHideFileNavigation => "Hide file navigation",
        I18nKey::CodeReviewInitializeCodebase => "Initialize codebase",
        I18nKey::CodeReviewInitializeCodebaseTooltip => "Enables codebase indexing and WARP.md",
        I18nKey::CodeReviewOpenRepository => "Open repository",
        I18nKey::CodeReviewOpenRepositoryTooltip => {
            "Navigate to a repo and initialize it for coding"
        }
        I18nKey::CodeReviewDiscardChanges => "Discard changes",
        I18nKey::CodeReviewFileLevelCommentsCantBeEdited => {
            "File-level comments currently can't be edited."
        }
        I18nKey::CodeReviewOutdatedCommentsCantBeEdited => {
            "Outdated comments can't be edited."
        }
        I18nKey::SettingsPageTitleAccount => "Account",
        I18nKey::SettingsPageTitlePrivacy => "Privacy",
        I18nKey::SettingsPageTitleMcpServers => "MCP Servers",
        I18nKey::SettingsPageTitleInviteFriend => "Invite a friend to Warp",
        I18nKey::SettingsCategoryBillingAndUsage => "Billing and usage",
        I18nKey::SettingsCategoryFeaturesGeneral => "General",
        I18nKey::SettingsCategoryFeaturesSession => "Session",
        I18nKey::SettingsCategoryFeaturesKeys => "Keys",
        I18nKey::SettingsCategoryFeaturesTextEditing => "Text Editing",
        I18nKey::SettingsCategoryFeaturesTerminalInput => "Terminal Input",
        I18nKey::SettingsCategoryFeaturesTerminal => "Terminal",
        I18nKey::SettingsCategoryFeaturesNotifications => "Notifications",
        I18nKey::SettingsCategoryFeaturesWorkflows => "Workflows",
        I18nKey::SettingsCategoryFeaturesSystem => "System",
        I18nKey::SettingsCategoryCodebaseIndexing => "Codebase Indexing",
        I18nKey::SettingsCategoryCodeEditorAndReview => "Code Editor and Review",
        I18nKey::SettingsCategoryWarpifySubshells => "Subshells",
        I18nKey::SettingsCategoryWarpifySubshellsDescription => {
            "Subshells supported: bash, zsh, and fish."
        }
        I18nKey::SettingsCategoryWarpifySsh => "SSH",
        I18nKey::SettingsCategoryWarpifySshDescription => {
            "Warpify your interactive SSH sessions."
        }
        I18nKey::WarpDriveSignUpPrompt => "To use Warp Drive, please create an account.",
        I18nKey::WarpDriveSignUpButton => "Sign up",
        I18nKey::WarpDriveLabel => "Warp Drive",
        I18nKey::WarpDriveDescription => {
            "Warp Drive is a workspace in your terminal where you can save Workflows, Notebooks, Prompts, and Environment Variables for personal use or to share with a team."
        }
        I18nKey::WarpifyTitle => "Warpify",
        I18nKey::WarpifyDescription => {
            "Configure whether Warp attempts to \"Warpify\" (add support for blocks, input modes, etc) certain shells. "
        }
        I18nKey::WarpifyLearnMore => "Learn more",
        I18nKey::WarpifyCommandPlaceholder => "command (supports regex)",
        I18nKey::WarpifyHostPlaceholder => "host (supports regex)",
        I18nKey::WarpifyAddedCommands => "Added commands",
        I18nKey::WarpifyDenylistedCommands => "Denylisted commands",
        I18nKey::WarpifySshSessions => "Warpify SSH Sessions",
        I18nKey::WarpifyInstallSshExtension => "Install SSH extension",
        I18nKey::WarpifyInstallSshExtensionDescription => {
            "Controls the installation behavior for Warp's SSH extension when a remote host doesn't have it installed."
        }
        I18nKey::WarpifyUseTmuxWarpification => "Use Tmux Warpification",
        I18nKey::WarpifyTmuxWarpificationDescription => {
            "The tmux ssh wrapper works in many situations where the default one does not, but may require you to hit a button to warpify. Takes effect in new tabs."
        }
        I18nKey::WarpifyDenylistedHosts => "Denylisted hosts",
        I18nKey::WarpifyInstallModeAlwaysAsk => "Always ask",
        I18nKey::WarpifyInstallModeAlwaysInstall => "Always install",
        I18nKey::WarpifyInstallModeNeverInstall => "Never install",
        I18nKey::AccountSignUp => "Sign up",
        I18nKey::AccountFreePlan => "Free",
        I18nKey::AccountComparePlans => "Compare plans",
        I18nKey::AccountContactSupport => "Contact support",
        I18nKey::AccountManageBilling => "Manage billing",
        I18nKey::AccountUpgradeTurboPlan => "Upgrade to Turbo plan",
        I18nKey::AccountUpgradeLightspeedPlan => "Upgrade to Lightspeed plan",
        I18nKey::AccountSettingsSync => "Settings sync",
        I18nKey::AccountReferralCta => {
            "Earn rewards by sharing Warp with friends & colleagues"
        }
        I18nKey::AccountReferFriend => "Refer a friend",
        I18nKey::AccountVersion => "Version",
        I18nKey::AccountLogOut => "Log out",
        I18nKey::AccountUpdateUpToDate => "Up to date",
        I18nKey::AccountUpdateCheckForUpdates => "Check for updates",
        I18nKey::AccountUpdateChecking => "checking for update...",
        I18nKey::AccountUpdateDownloading => "downloading update...",
        I18nKey::AccountUpdateAvailable => "Update available",
        I18nKey::AccountUpdateRelaunchWarp => "Relaunch Warp",
        I18nKey::AccountUpdateUpdating => "Updating...",
        I18nKey::AccountUpdateInstalled => "Installed update",
        I18nKey::AccountUpdateUnableToInstall => {
            "A new version of Warp is available but can't be installed"
        }
        I18nKey::AccountUpdateManual => "Update Warp manually",
        I18nKey::AccountUpdateUnableToLaunch => {
            "A new version of Warp is installed but can't be launched."
        }
        I18nKey::PrivacySafeModeTitle => "Secret redaction",
        I18nKey::PrivacySafeModeDescription => {
            "When this setting is enabled, Warp will scan blocks, the contents of Warp Drive objects, and Oz prompts for potential sensitive information and prevent saving or sending this data to any servers. You can customize this list via regexes."
        }
        I18nKey::PrivacyCustomRedactionTitle => "Custom secret redaction",
        I18nKey::PrivacyCustomRedactionDescription => {
            "Use regex to define additional secrets or data you'd like to redact. This will take effect when the next command runs. You can use the inline (?i) flag as a prefix to your regex to make it case-insensitive."
        }
        I18nKey::PrivacyTelemetryTitle => "Help improve Warp",
        I18nKey::PrivacyTelemetryDescriptionEnterprise => {
            "App analytics help us make the product better for you. We only collect app usage metadata, never console input or output."
        }
        I18nKey::PrivacyTelemetryDescription => {
            "App analytics help us make the product better for you. We may collect certain console interactions to improve Warp's AI capabilities."
        }
        I18nKey::PrivacyTelemetryFreeTierNote => {
            "On the free tier, analytics must be enabled to use AI features."
        }
        I18nKey::PrivacyTelemetryDocsLink => "Read more about Warp's use of data",
        I18nKey::PrivacyDataManagementTitle => "Manage your data",
        I18nKey::PrivacyDataManagementDescription => {
            "At any time, you may choose to delete your Warp account permanently. You will no longer be able to use Warp."
        }
        I18nKey::PrivacyDataManagementLink => "Visit the data management page",
        I18nKey::PrivacyPolicyTitle => "Privacy policy",
        I18nKey::PrivacyPolicyLink => "Read Warp's privacy policy",
        I18nKey::PrivacyAddRegexPatternTitle => "Add regex pattern",
        I18nKey::PrivacySecretDisplayAsterisks => "Asterisks",
        I18nKey::PrivacySecretDisplayStrikethrough => "Strikethrough",
        I18nKey::PrivacySecretDisplayAlwaysShow => "Always show secrets",
        I18nKey::PrivacyTabPersonal => "Personal",
        I18nKey::PrivacyTabEnterprise => "Enterprise",
        I18nKey::PrivacyEnterpriseReadonly => "Enterprise secret redaction cannot be modified.",
        I18nKey::PrivacyNoEnterpriseRegexes => {
            "No enterprise regexes have been configured by your organization."
        }
        I18nKey::PrivacyRecommended => "Recommended",
        I18nKey::PrivacyAddAll => "Add all",
        I18nKey::PrivacyEnabledByOrganization => "Enabled by your organization.",
        I18nKey::PrivacySecretVisualMode => "Secret visual redaction mode",
        I18nKey::PrivacySecretVisualModeDescription => {
            "Choose how secrets are visually presented in the block list while keeping them searchable. This setting only affects what you see in the block list."
        }
        I18nKey::PrivacyAddRegex => "Add regex",
        I18nKey::PrivacyZdrTooltip => {
            "Your administrator has enabled zero data retention for your team. User generated content will never be collected."
        }
        I18nKey::PrivacyManagedByOrganization => "This setting is managed by your organization.",
        I18nKey::PrivacyCrashReportsTitle => "Send crash reports",
        I18nKey::PrivacyCrashReportsDescription => {
            "Crash reports assist with debugging and stability improvements."
        }
        I18nKey::PrivacyCloudConversationStorageTitle => "Store AI conversations in the cloud",
        I18nKey::PrivacyCloudConversationStorageEnabledDescription => {
            "Agent conversations can be shared with others and are retained when you log in on different devices. This data is only stored for product functionality, and Warp will not use it for analytics."
        }
        I18nKey::PrivacyCloudConversationStorageDisabledDescription => {
            "Agent conversations are only stored locally on your machine, are lost upon logout, and cannot be shared. Note: conversation data for ambient agents are still stored in the cloud."
        }
        I18nKey::PrivacyNetworkLogTitle => "Network log console",
        I18nKey::PrivacyNetworkLogDescription => {
            "We've built a native console that allows you to view all communications from Warp to external servers to ensure you feel comfortable that your work is always kept safe."
        }
        I18nKey::PrivacyNetworkLogLink => "View network logging",
        I18nKey::PrivacyModalNameOptional => "Name (optional)",
        I18nKey::PrivacyModalNamePlaceholder => "e.g. \"Google API Key\"",
        I18nKey::PrivacyModalRegexPattern => "Regex pattern",
        I18nKey::PrivacyModalInvalidRegex => "Invalid regex",
        I18nKey::PrivacyModalCancel => "Cancel",
        I18nKey::CodeIndexNewFolder => "Index new folder",
        I18nKey::CodeFeatureName => "Code",
        I18nKey::CodeInitializationSettingsHeader => "Initialization Settings",
        I18nKey::CodebaseIndexingLabel => "Codebase indexing",
        I18nKey::CodebaseIndexingDescription => {
            "Warp can automatically index code repositories as you navigate them, helping agents quickly understand context and provide solutions. Code is never stored on the server. If a codebase is unable to be indexed, Warp can still navigate your codebase and gain insights via grep and find tool calling."
        }
        I18nKey::CodeWarpIndexingIgnoreDescription => {
            "To exclude specific files or directories from indexing, add them to the .warpindexingignore file in your repository directory. These files will still be accessible to AI features, but they won't be included in codebase embeddings."
        }
        I18nKey::CodeAutoIndexFeatureName => "Index new folders by default",
        I18nKey::CodeAutoIndexDescription => {
            "When set to true, Warp will automatically index code repositories as you navigate them - helping agents quickly understand context and provide targeted solutions."
        }
        I18nKey::CodeIndexingDisabledAdmin => "Team admins have disabled codebase indexing.",
        I18nKey::CodeIndexingWorkspaceEnabledAdmin => {
            "Team admins have enabled codebase indexing."
        }
        I18nKey::CodeIndexingDisabledGlobalAi => {
            "AI Features must be enabled to use codebase indexing."
        }
        I18nKey::CodebaseIndexLimitReached => {
            "You have reached the maximum number of codebase indices for your plan. Delete existing indices to auto-index new codebases."
        }
        I18nKey::CodeSubpageCodebaseIndexing => "Codebase Indexing",
        I18nKey::CodeSubpageEditorAndCodeReview => "Editor and Code Review",
        I18nKey::CodeInitializedFolders => "Initialized / indexed folders",
        I18nKey::CodeNoFoldersInitialized => "No folders have been initialized yet.",
        I18nKey::CodeOpenProjectRules => "Open project rules",
        I18nKey::CodeIndexingSection => "INDEXING",
        I18nKey::CodeNoIndexCreated => "No index created",
        I18nKey::CodeIndexDiscoveredChunks => "Discovered {count} chunks",
        I18nKey::CodeIndexSyncingProgress => "Syncing - {completed} / {total}",
        I18nKey::CodeIndexSyncing => "Syncing...",
        I18nKey::CodeIndexSynced => "Synced",
        I18nKey::CodeIndexTooLarge => "Codebase too large",
        I18nKey::CodeIndexStale => "Stale",
        I18nKey::CodeIndexFailed => "Failed",
        I18nKey::CodeIndexNotBuilt => "No index built",
        I18nKey::CodeLspServersSection => "LSP SERVERS",
        I18nKey::CodeLspInstalled => "Installed",
        I18nKey::CodeLspInstalling => "Installing...",
        I18nKey::CodeLspChecking => "Checking...",
        I18nKey::CodeLspAvailableForDownload => "Available for download",
        I18nKey::CodeLspRestartServer => "Restart server",
        I18nKey::CodeLspViewLogs => "View logs",
        I18nKey::CodeLspAvailable => "Available",
        I18nKey::CodeLspBusy => "Busy",
        I18nKey::CodeLspStopped => "Stopped",
        I18nKey::CodeLspNotRunning => "Not running",
        I18nKey::CodeAutoOpenCodeReviewPanel => "Auto open code review panel",
        I18nKey::CodeAutoOpenCodeReviewPanelDescription => {
            "When this setting is on, the code review panel will open on the first accepted diff of a conversation"
        }
        I18nKey::CodeShowCodeReviewButton => "Show code review button",
        I18nKey::CodeShowCodeReviewButtonDescription => {
            "Show a button in the top right of the window to toggle the code review panel."
        }
        I18nKey::CodeShowDiffStats => "Show diff stats on code review button",
        I18nKey::CodeShowDiffStatsDescription => {
            "Show lines added and removed counts on the code review button."
        }
        I18nKey::CodeProjectExplorer => "Project explorer",
        I18nKey::CodeProjectExplorerDescription => {
            "Adds an IDE-style project explorer / file tree to the left side tools panel."
        }
        I18nKey::CodeGlobalFileSearch => "Global file search",
        I18nKey::CodeGlobalFileSearchDescription => {
            "Adds global file search to the left side tools panel."
        }
        I18nKey::CodeExternalDefaultApp => "Default App",
        I18nKey::CodeExternalSplitPane => "Split Pane",
        I18nKey::CodeExternalNewTab => "New Tab",
        I18nKey::CodeExternalChooseFileLinksEditor => "Choose an editor to open file links",
        I18nKey::CodeExternalChooseCodePanelEditor => {
            "Choose an editor to open files from the code review panel, project explorer, and global search"
        }
        I18nKey::CodeExternalChooseLayout => "Choose a layout to open files in Warp",
        I18nKey::CodeExternalTabbedFileViewer => "Group files into single editor pane",
        I18nKey::CodeExternalTabbedFileViewerDescription => {
            "When this setting is on, any files opened in the same tab will be automatically grouped into a single editor pane."
        }
        I18nKey::CodeExternalMarkdownViewerDefault => {
            "Open Markdown files in Warp's Markdown Viewer by default"
        }
        I18nKey::KeybindingsSearchPlaceholder => "Search by name or by keys (ex. \"cmd d\")",
        I18nKey::KeybindingsConflictWarning => "This shortcut conflicts with other keybinds",
        I18nKey::KeybindingsDefault => "Default",
        I18nKey::KeybindingsCancel => "Cancel",
        I18nKey::KeybindingsClear => "Clear",
        I18nKey::KeybindingsSave => "Save",
        I18nKey::KeybindingsPressNewShortcut => "Press new keyboard shortcut",
        I18nKey::KeybindingsDescription => {
            "Add your own custom keybindings to existing actions below."
        }
        I18nKey::KeybindingsUse => "Use",
        I18nKey::KeybindingsReferenceInSidePane => {
            "to reference these keybindings in a side pane at anytime."
        }
        I18nKey::KeybindingsNotSyncedTooltip => "Keyboard shortcuts are not synced to the cloud",
        I18nKey::KeybindingsConfigureTitle => "Configure keyboard shortcuts",
        I18nKey::KeybindingsCommandColumn => "Command",
        I18nKey::PlatformApiKeyDeletedToast => "API key deleted",
        I18nKey::PlatformNewApiKeyTitle => "New API key",
        I18nKey::PlatformSaveYourKeyTitle => "Save your key",
        I18nKey::PlatformApiKeysTitle => "Oz Cloud API Keys",
        I18nKey::PlatformCreateApiKeyButton => "+ Create API Key",
        I18nKey::PlatformDescriptionPrefix => {
            "Create and manage API keys to allow other Oz cloud agents to access your Warp account.\nFor more information, visit the "
        }
        I18nKey::PlatformDocumentationLink => "Documentation.",
        I18nKey::PlatformHeaderName => "Name",
        I18nKey::PlatformHeaderKey => "Key",
        I18nKey::PlatformHeaderScope => "Scope",
        I18nKey::PlatformHeaderCreated => "Created",
        I18nKey::PlatformHeaderLastUsed => "Last used",
        I18nKey::PlatformHeaderExpiresAt => "Expires at",
        I18nKey::PlatformNever => "Never",
        I18nKey::PlatformScopePersonal => "Personal",
        I18nKey::PlatformScopeTeam => "Team",
        I18nKey::PlatformNoApiKeys => "No API Keys",
        I18nKey::PlatformNoApiKeysDescription => "Create a key to manage external access to Warp",
        I18nKey::PlatformApiKeyTypePersonalDescription => {
            "This API key is tied to your user and can make requests against your Warp account."
        }
        I18nKey::PlatformApiKeyTypeTeamDescription => {
            "This API key is tied to your team and can make requests on behalf of your team."
        }
        I18nKey::PlatformExpirationOneDay => "1 day",
        I18nKey::PlatformExpirationThirtyDays => "30 days",
        I18nKey::PlatformExpirationNinetyDays => "90 days",
        I18nKey::PlatformExpirationNever => "Never",
        I18nKey::PlatformTeamKeyNoCurrentTeamError => {
            "Unable to create a team API key because there is no current team."
        }
        I18nKey::PlatformCreateApiKeyFailed => "Failed to create API key. Please try again.",
        I18nKey::PlatformSecretKeyInfo => {
            "This secret key is shown only once. Copy and store it securely."
        }
        I18nKey::PlatformCopied => "Copied",
        I18nKey::PlatformCopy => "Copy",
        I18nKey::PlatformDone => "Done",
        I18nKey::PlatformNameLabel => "Name",
        I18nKey::PlatformTypeLabel => "Type",
        I18nKey::PlatformExpirationLabel => "Expiration",
        I18nKey::PlatformCreating => "Creating...",
        I18nKey::PlatformCreateKey => "Create key",
        I18nKey::PlatformSecretKeyCopiedToast => "Secret key copied.",
        I18nKey::PlatformDeleteApiKeyFailed => "Failed to delete API key. Please try again.",
        I18nKey::FeaturesPinToTop => "Pin to top",
        I18nKey::FeaturesPinToBottom => "Pin to bottom",
        I18nKey::FeaturesPinToLeft => "Pin to left",
        I18nKey::FeaturesPinToRight => "Pin to right",
        I18nKey::FeaturesActiveScreen => "Active Screen",
        I18nKey::FeaturesDefault => "Default",
        I18nKey::FeaturesNewTabAfterAllTabs => "After all tabs",
        I18nKey::FeaturesNewTabAfterCurrentTab => "After current tab",
        I18nKey::FeaturesGlobalHotkeyDisabled => "Disabled",
        I18nKey::FeaturesGlobalHotkeyDedicatedWindow => "Dedicated hotkey window",
        I18nKey::FeaturesGlobalHotkeyShowHideAllWindows => "Show/hide all windows",
        I18nKey::FeaturesCtrlTabActivatePrevNextTab => "Activate previous/next tab",
        I18nKey::FeaturesCtrlTabCycleMostRecentSession => "Cycle most recent session",
        I18nKey::FeaturesCtrlTabCycleMostRecentTab => "Cycle most recent tab",
        I18nKey::FeaturesTabBehaviorOpenCompletions => "Open completions menu",
        I18nKey::FeaturesTabBehaviorAcceptAutosuggestion => "Accept autosuggestion",
        I18nKey::FeaturesTabBehaviorUserDefined => "User defined",
        I18nKey::FeaturesDefaultSessionTerminal => "Terminal",
        I18nKey::FeaturesDefaultSessionAgent => "Agent",
        I18nKey::FeaturesDefaultSessionCloudAgent => "Cloud Oz",
        I18nKey::FeaturesDefaultSessionTabConfig => "Tab Config",
        I18nKey::FeaturesDefaultSessionDockerSandbox => "Local Docker Sandbox",
        I18nKey::FeaturesChangesApplyNewWindows => "Changes will apply to new windows.",
        I18nKey::FeaturesCurrentBackend => "Current backend: {backend}",
        I18nKey::FeaturesOpenLinksInDesktopApp => "Open links in desktop app",
        I18nKey::FeaturesOpenLinksInDesktopAppTooltip => {
            "Automatically open links in desktop app whenever possible."
        }
        I18nKey::FeaturesRestoreSession => "Restore windows, tabs, and panes on startup",
        I18nKey::FeaturesWaylandRestoreWarning => {
            "Window positions won't be restored on Wayland. "
        }
        I18nKey::FeaturesSeeDocs => "See docs.",
        I18nKey::FeaturesStickyCommandHeader => "Show sticky command header",
        I18nKey::FeaturesLinkTooltip => "Show tooltip on click on links",
        I18nKey::FeaturesQuitWarning => "Show warning before quitting/logging out",
        I18nKey::FeaturesStartWarpAtLoginMac => "Start Warp at login (requires macOS 13+)",
        I18nKey::FeaturesStartWarpAtLogin => "Start Warp at login",
        I18nKey::FeaturesQuitWhenAllWindowsClosed => "Quit when all windows are closed",
        I18nKey::FeaturesShowChangelogToast => "Show changelog toast after updates",
        I18nKey::FeaturesAllowedValuesOneToTwenty => "Allowed Values: 1-20",
        I18nKey::FeaturesMouseScrollLines => "Lines scrolled by mouse wheel interval",
        I18nKey::FeaturesMouseScrollTooltip => {
            "Supports floating point values between 1 and 20."
        }
        I18nKey::FeaturesAutoOpenCodeReviewPanel => "Auto open code review panel",
        I18nKey::FeaturesAutoOpenCodeReviewPanelDescription => {
            "When this setting is on, the code review panel will open on the first accepted diff of a conversation"
        }
        I18nKey::FeaturesWarpDefaultTerminal => "Warp is the default terminal",
        I18nKey::FeaturesMakeWarpDefaultTerminal => "Make Warp the default terminal",
        I18nKey::FeaturesMaximumRowsInBlock => "Maximum rows in a block",
        I18nKey::FeaturesBlockMaximumRowsDescription => {
            "Setting the limit above 100k lines may impact performance. Maximum rows supported is {max_rows}."
        }
        I18nKey::FeaturesSshWrapper => "Warp SSH Wrapper",
        I18nKey::FeaturesSshWrapperNewSessions => {
            "This change will take effect in new sessions"
        }
        I18nKey::FeaturesReceiveDesktopNotifications => "Receive desktop notifications from Warp",
        I18nKey::FeaturesNotifyAgentTaskCompleted => "Notify when an agent completes a task",
        I18nKey::FeaturesNotifyCommandNeedsAttention => {
            "Notify when a command or agent needs your attention to continue"
        }
        I18nKey::FeaturesPlayNotificationSounds => "Play notification sounds",
        I18nKey::FeaturesShowInAppAgentNotifications => "Show in-app agent notifications",
        I18nKey::FeaturesNotificationWhenCommandLongerThan => {
            "When a command takes longer than"
        }
        I18nKey::FeaturesNotificationSecondsToComplete => "seconds to complete",
        I18nKey::FeaturesToastNotificationsStayVisibleFor => {
            "Toast notifications stay visible for"
        }
        I18nKey::FeaturesSeconds => "seconds",
        I18nKey::FeaturesDefaultShellForNewSessions => "Default shell for new sessions",
        I18nKey::FeaturesWorkingDirectoryForNewSessions => "Working directory for new sessions",
        I18nKey::FeaturesConfirmCloseSharedSession => "Confirm before closing shared session",
        I18nKey::FeaturesLeftOptionKeyMeta => "Left Option key is Meta",
        I18nKey::FeaturesRightOptionKeyMeta => "Right Option key is Meta",
        I18nKey::FeaturesLeftAltKeyMeta => "Left Alt key is Meta",
        I18nKey::FeaturesRightAltKeyMeta => "Right Alt key is Meta",
        I18nKey::FeaturesGlobalHotkeyLabel => "Global hotkey:",
        I18nKey::FeaturesNotSupportedOnWayland => "Not supported on Wayland. ",
        I18nKey::FeaturesWidthPercent => "Width %",
        I18nKey::FeaturesHeightPercent => "Height %",
        I18nKey::FeaturesAutohideKeyboardFocus => "Autohides on loss of keyboard focus",
        I18nKey::FeaturesKeybinding => "Keybinding",
        I18nKey::FeaturesClickSetGlobalHotkey => "Click to set global hotkey",
        I18nKey::FeaturesChangeKeybinding => "Change keybinding",
        I18nKey::FeaturesAutocompleteSymbols => "Autocomplete quotes, parentheses, and brackets",
        I18nKey::FeaturesErrorUnderlining => "Error underlining for commands",
        I18nKey::FeaturesSyntaxHighlighting => "Syntax highlighting for commands",
        I18nKey::FeaturesOpenCompletionsTyping => "Open completions menu as you type",
        I18nKey::FeaturesSuggestCorrectedCommands => "Suggest corrected commands",
        I18nKey::FeaturesExpandAliasesTyping => "Expand aliases as you type",
        I18nKey::FeaturesMiddleClickPaste => "Middle-click to paste",
        I18nKey::FeaturesVimMode => "Edit code and commands with Vim keybindings",
        I18nKey::FeaturesVimUnnamedRegisterClipboard => {
            "Set unnamed register as system clipboard"
        }
        I18nKey::FeaturesVimStatusBar => "Show Vim status bar",
        I18nKey::FeaturesAtContextMenuTerminalMode => {
            "Enable '@' context menu in terminal mode"
        }
        I18nKey::FeaturesSlashCommandsTerminalMode => "Enable slash commands in terminal mode",
        I18nKey::FeaturesOutlineCodebaseSymbolsContextMenu => {
            "Outline codebase symbols for '@' context menu"
        }
        I18nKey::FeaturesShowTerminalInputMessageLine => "Show terminal input message line",
        I18nKey::FeaturesShowAutosuggestionKeybindingHint => {
            "Show autosuggestion keybinding hint"
        }
        I18nKey::FeaturesShowAutosuggestionIgnoreButton => "Show autosuggestion ignore button",
        I18nKey::FeaturesArrowAcceptsAutosuggestions => "-> accepts autosuggestions.",
        I18nKey::FeaturesKeyAcceptsAutosuggestions => "{key} accepts autosuggestions.",
        I18nKey::FeaturesCompletionsOpenTyping => "Completions open as you type.",
        I18nKey::FeaturesCompletionsOpenTypingOrKey => {
            "Completions open as you type (or {key})."
        }
        I18nKey::FeaturesCompletionMenuUnbound => "Opening the completion menu is unbound.",
        I18nKey::FeaturesKeyOpensCompletionMenu => "{key} opens completion menu.",
        I18nKey::FeaturesTabKeyBehavior => "Tab key behavior",
        I18nKey::FeaturesCtrlTabBehaviorLabel => "Ctrl+Tab behavior:",
        I18nKey::FeaturesEnableMouseReporting => "Enable Mouse Reporting",
        I18nKey::FeaturesEnableScrollReporting => "Enable Scroll Reporting",
        I18nKey::FeaturesEnableFocusReporting => "Enable Focus Reporting",
        I18nKey::FeaturesUseAudibleBell => "Use Audible Bell",
        I18nKey::FeaturesWordCharactersLabel => "Characters considered part of a word",
        I18nKey::FeaturesDoubleClickSmartSelection => "Double-click smart selection",
        I18nKey::FeaturesShowHelpBlockNewSessions => "Show help block in new sessions",
        I18nKey::FeaturesCopyOnSelect => "Copy on select",
        I18nKey::FeaturesNewTabPlacement => "New tab placement",
        I18nKey::FeaturesDefaultModeForNewSessions => "Default mode for new sessions",
        I18nKey::FeaturesShowGlobalWorkflowsCommandSearch => {
            "Show Global Workflows in Command Search (ctrl-r)"
        }
        I18nKey::FeaturesHonorLinuxSelectionClipboard => "Honor linux selection clipboard",
        I18nKey::FeaturesLinuxSelectionClipboardTooltip => {
            "Whether the Linux primary clipboard should be supported."
        }
        I18nKey::FeaturesPreferIntegratedGpu => {
            "Prefer rendering new windows with integrated GPU (low power)"
        }
        I18nKey::FeaturesUseWaylandWindowManagement => "Use Wayland for window management",
        I18nKey::FeaturesWaylandTooltip => "Enables the use of Wayland",
        I18nKey::FeaturesWaylandSecondaryText => {
            "Enabling this setting disables global hotkey support. When disabled, text may be blurry if your Wayland compositor is using fraction scaling (ex: 125%)."
        }
        I18nKey::FeaturesWaylandRestart => "Restart Warp for changes to take effect.",
        I18nKey::FeaturesPreferredGraphicsBackend => "Preferred graphics backend",
        I18nKey::AiWarpAgentTitle => "Warp Agent",
        I18nKey::AiRemoteSessionOrgPolicy => {
            "Your organization disallows AI when the active pane contains content from a remote session"
        }
        I18nKey::AiSignUpPrompt => "To use AI features, please create an account.",
        I18nKey::AiUsage => "Usage",
        I18nKey::AiResetsDate => "Resets {date}",
        I18nKey::AiRestrictedBilling => "Restricted due to billing issue",
        I18nKey::AiUnlimited => "Unlimited",
        I18nKey::AiUsageLimitDescription => {
            "This is the {duration} limit of AI credits for your account."
        }
        I18nKey::AiCredits => "Credits",
        I18nKey::AiUpgrade => "Upgrade",
        I18nKey::AiComparePlans => "Compare plans",
        I18nKey::AiContactSupport => "Contact support",
        I18nKey::AiUpgradeMoreUsage => " to get more AI usage.",
        I18nKey::AiForMoreUsage => " for more AI usage.",
        I18nKey::AiActiveAi => "Active AI",
        I18nKey::AiNextCommand => "Next Command",
        I18nKey::AiNextCommandDescription => {
            "Let AI suggest the next command to run based on your command history, outputs, and common workflows."
        }
        I18nKey::AiPromptSuggestions => "Prompt Suggestions",
        I18nKey::AiPromptSuggestionsDescription => {
            "Let AI suggest natural language prompts, as inline banners in the input, based on recent commands and their outputs."
        }
        I18nKey::AiSuggestedCodeBanners => "Suggested Code Banners",
        I18nKey::AiSuggestedCodeBannersDescription => {
            "Let AI suggest code diffs and queries as inline banners in the blocklist, based on recent commands and their outputs."
        }
        I18nKey::AiNaturalLanguageAutosuggestions => "Natural Language Autosuggestions",
        I18nKey::AiNaturalLanguageAutosuggestionsDescription => {
            "Let AI suggest natural language autosuggestions, based on recent commands and their outputs."
        }
        I18nKey::AiSharedBlockTitleGeneration => "Shared Block Title Generation",
        I18nKey::AiSharedBlockTitleGenerationDescription => {
            "Let AI generate a title for your shared block based on the command and output."
        }
        I18nKey::AiCommitPrGeneration => "Commit & Pull Request Generation",
        I18nKey::AiCommitPrGenerationDescription => {
            "Let AI generate commit messages and pull request titles and descriptions."
        }
        I18nKey::AiAgents => "Agents",
        I18nKey::AiAgentsDescription => {
            "Set the boundaries for how your Agent operates. Choose what it can access, how much autonomy it has, and when it must ask for your approval. You can also fine-tune behavior around natural language input, codebase awareness, and more."
        }
        I18nKey::AiProfiles => "Profiles",
        I18nKey::AiProfilesDescription => {
            "Profiles let you define how your Agent operates - from the actions it can take and when it needs approval, to the models it uses for tasks like coding and planning. You can also scope them to individual projects."
        }
        I18nKey::AiModels => "Models",
        I18nKey::AiContextWindowTokens => "Context window (tokens)",
        I18nKey::AiPermissions => "Permissions",
        I18nKey::AiApplyCodeDiffs => "Apply code diffs",
        I18nKey::AiReadFiles => "Read files",
        I18nKey::AiExecuteCommands => "Execute commands",
        I18nKey::AiPermissionsManagedByWorkspace => {
            "Some of your permissions are managed by your workspace."
        }
        I18nKey::AiInteractWithRunningCommands => "Interact with running commands",
        I18nKey::AiCommandDenylist => "Command denylist",
        I18nKey::AiCommandDenylistDescription => {
            "Regular expressions to match commands that the Warp Agent should always ask permission to execute."
        }
        I18nKey::AiCommandAllowlist => "Command allowlist",
        I18nKey::AiCommandAllowlistDescription => {
            "Regular expressions to match commands that can be automatically executed by the Warp Agent."
        }
        I18nKey::AiDirectoryAllowlist => "Directory allowlist",
        I18nKey::AiDirectoryAllowlistDescription => {
            "Give the agent file access to certain directories."
        }
        I18nKey::AiShowModelPickerInPrompt => "Show model picker in prompt",
        I18nKey::AiBaseModel => "Base model",
        I18nKey::AiBaseModelDescription => {
            "This model serves as the primary engine behind the Warp Agent. It powers most interactions and invokes other models for tasks like planning or code generation when necessary. Warp may automatically switch to alternate models based on model availability or for auxiliary tasks such as conversation summarization."
        }
        I18nKey::AiCodebaseContext => "Codebase Context",
        I18nKey::AiCodebaseContextDescription => {
            "Allow the Warp Agent to generate an outline of your codebase that can be used for context. No code is ever stored on our servers. "
        }
        I18nKey::AiLearnMore => "Learn more",
        I18nKey::AiCallMcpServers => "Call MCP servers",
        I18nKey::AiMcpZeroStatePrefix => {
            "You haven't added any MCP servers yet. Once you do, you'll be able to control how much autonomy the Warp Agent has when interacting with them. "
        }
        I18nKey::AiAddServer => "Add a server",
        I18nKey::AiMcpZeroStateMiddle => " or ",
        I18nKey::AiMcpZeroStateLearnMore => "learn more about MCPs.",
        I18nKey::AiMcpAllowlist => "MCP allowlist",
        I18nKey::AiMcpAllowlistDescription => "Allow the Warp Agent to call these MCP servers.",
        I18nKey::AiMcpDenylist => "MCP denylist",
        I18nKey::AiMcpDenylistDescription => {
            "The Warp Agent will always ask for permission before calling any MCP servers on this list."
        }
        I18nKey::AiInput => "Input",
        I18nKey::AiShowInputHintText => "Show input hint text",
        I18nKey::AiShowAgentTips => "Show agent tips",
        I18nKey::AiIncludeAgentCommandsHistory => {
            "Include agent-executed commands in history"
        }
        I18nKey::AiIncorrectDetectionPrefix => "Encountered an incorrect detection? ",
        I18nKey::AiLetUsKnow => "Let us know",
        I18nKey::AiAutodetectAgentPrompts => "Autodetect agent prompts in terminal input",
        I18nKey::AiAutodetectTerminalCommands => "Autodetect terminal commands in agent input",
        I18nKey::AiNaturalLanguageDetectionDescription => {
            "Enabling natural language detection will detect when natural language is written in the terminal input, and then automatically switch to Agent Mode for AI queries."
        }
        I18nKey::AiIncorrectInputDetectionPrefix => {
            " Encountered an incorrect input detection? "
        }
        I18nKey::AiNaturalLanguageDetection => "Natural language detection",
        I18nKey::AiNaturalLanguageDenylist => "Natural language denylist",
        I18nKey::AiNaturalLanguageDenylistDescription => {
            "Commands listed here will never trigger natural language detection."
        }
        I18nKey::AiMcpServers => "MCP Servers",
        I18nKey::AiMcpServersDescription => {
            "Add MCP servers to extend the Warp Agent's capabilities. MCP servers expose data sources or tools to agents through a standardized interface, essentially acting like plugins. "
        }
        I18nKey::AiAutoSpawnThirdPartyServers => {
            "Auto-spawn servers from third-party agents"
        }
        I18nKey::AiFileBasedMcpDescription => {
            "Automatically detect and spawn MCP servers from globally-scoped third-party AI agent configuration files (e.g. in your home directory). Servers detected inside a repository are never spawned automatically and must be enabled individually from the MCP settings page. "
        }
        I18nKey::AiSeeSupportedProviders => "See supported providers.",
        I18nKey::AiManageMcpServers => "Manage MCP servers",
        I18nKey::AiRules => "Rules",
        I18nKey::AiRulesDescription => {
            "Rules help the Warp Agent follow your conventions, whether for codebases or specific workflows. "
        }
        I18nKey::AiSuggestedRules => "Suggested Rules",
        I18nKey::AiSuggestedRulesDescription => {
            "Let AI suggest rules to save based on your interactions."
        }
        I18nKey::AiWarpDriveAgentContext => "Warp Drive as agent context",
        I18nKey::AiWarpDriveAgentContextDescription => {
            "The Warp Agent can leverage your Warp Drive Contents to tailor responses to your personal and team developer workflows and environments. This includes any Workflows, Notebooks, and Environment Variables."
        }
        I18nKey::AiKnowledge => "Knowledge",
        I18nKey::AiManageRules => "Manage rules",
        I18nKey::AiVoiceInput => "Voice Input",
        I18nKey::AiVoiceInputDescriptionPrefix => {
            "Voice input allows you to control Warp by speaking directly to your terminal (powered by "
        }
        I18nKey::AiVoiceInputDescriptionSuffix => ").",
        I18nKey::AiKeyForActivatingVoiceInput => "Key for Activating Voice Input",
        I18nKey::AiPressAndHoldToActivate => "Press and hold to activate.",
        I18nKey::AiVoice => "Voice",
        I18nKey::AiOther => "Other",
        I18nKey::AiShowOzChangelog => "Show Oz changelog in new conversation view",
        I18nKey::AiShowUseAgentFooter => "Show \"Use Agent\" footer",
        I18nKey::AiUseAgentFooterDescription => {
            "Shows hint to use the \"Full Terminal Use\"-enabled agent in long running commands."
        }
        I18nKey::AiShowConversationHistory => "Show conversation history in tools panel",
        I18nKey::AiAgentThinkingDisplay => "Agent thinking display",
        I18nKey::AiAgentThinkingDisplayDescription => {
            "Controls how reasoning/thinking traces are displayed."
        }
        I18nKey::AiPreferredConversationLayout => {
            "Preferred layout when opening existing agent conversations"
        }
        I18nKey::AiThirdPartyCliAgents => "Third party CLI agents",
        I18nKey::AiShowCodingAgentToolbar => "Show coding agent toolbar",
        I18nKey::AiCodingAgentToolbarDescriptionPrefix => {
            "Show a toolbar with quick actions when running coding agents like "
        }
        I18nKey::AiCodingAgentToolbarDescriptionMiddle => ", or ",
        I18nKey::AiCodingAgentToolbarDescriptionSuffix => ".",
        I18nKey::AiAutoShowHideRichInput => "Auto show/hide Rich Input based on agent status",
        I18nKey::AiRequiresWarpPlugin => "Requires the Warp plugin for your coding agent",
        I18nKey::AiAutoOpenRichInput => {
            "Auto open Rich Input when a coding agent session starts"
        }
        I18nKey::AiAutoDismissRichInput => "Auto dismiss Rich Input after prompt submission",
        I18nKey::AiCommandsEnableToolbar => "Commands that enable the toolbar",
        I18nKey::AiToolbarCommandDescription => {
            "Add regex patterns to show the coding agent toolbar for matching commands."
        }
        I18nKey::AiOrgEnforcedTooltip => {
            "This option is enforced by your organization's settings and cannot be customized."
        }
        I18nKey::AiEnableAgentAttribution => "Enable agent attribution",
        I18nKey::AiAgentAttribution => "Agent Attribution",
        I18nKey::AiAgentAttributionDescription => {
            "Oz can add attribution to commit messages and pull requests it creates"
        }
        I18nKey::AiComputerUseCloudAgents => "Computer use in Cloud Agents",
        I18nKey::AiExperimental => "Experimental",
        I18nKey::AiCloudComputerUseDescription => {
            "Enable computer use in cloud agent conversations started from the Warp app."
        }
        I18nKey::AiOrchestration => "Orchestration",
        I18nKey::AiOrchestrationDescription => {
            "Enable multi-agent orchestration, allowing the agent to spawn and coordinate parallel sub-agents."
        }
        I18nKey::AiApiKeys => "API Keys",
        I18nKey::AiByokDescription => {
            "Use your own API keys from model providers for the Warp Agent to use. API keys are stored locally and never synced to the cloud. Using auto models or models from providers you have not provided API keys for will consume Warp credits."
        }
        I18nKey::AiOpenAiApiKey => "OpenAI API Key",
        I18nKey::AiAnthropicApiKey => "Anthropic API Key",
        I18nKey::AiGoogleApiKey => "Google API Key",
        I18nKey::AiContactSales => "Contact sales",
        I18nKey::AiContactSalesByok => {
            " to enable bringing your own API keys on your Enterprise plan."
        }
        I18nKey::AiUpgradeBuildPlan => "Upgrade to the Build plan",
        I18nKey::AiUseOwnApiKeys => " to use your own API keys.",
        I18nKey::AiAskAdminUpgradeBuild => {
            "Ask your team's admin to upgrade to the Build plan to use your own API keys."
        }
        I18nKey::AiWarpCreditFallback => "Warp credit fallback",
        I18nKey::AiWarpCreditFallbackDescription => {
            "When enabled, agent requests may be routed to one of Warp's provided models in the event of an error. Warp will prioritize using your API keys over your Warp credits."
        }
        I18nKey::AiAwsBedrock => "AWS Bedrock",
        I18nKey::AiAwsManagedDescription => {
            "Warp loads and sends local AWS CLI credentials for Bedrock-supported models. This setting is managed by your organization."
        }
        I18nKey::AiAwsDescription => {
            "Warp loads and sends local AWS CLI credentials for Bedrock-supported models."
        }
        I18nKey::AiUseAwsBedrockCredentials => "Use AWS Bedrock credentials",
        I18nKey::AiLoginCommand => "Login Command",
        I18nKey::AiAwsProfile => "AWS Profile",
        I18nKey::AiAutoRunLoginCommand => "Automatically run login command",
        I18nKey::AiAutoRunLoginDescription => {
            "When enabled, the login command will run automatically when AWS Bedrock credentials expire."
        }
        I18nKey::AiRefresh => "Refresh",
        I18nKey::AiAddProfile => "Add Profile",
        I18nKey::AiEdit => "Edit",
        I18nKey::AiAuto => "Auto",
        I18nKey::AiModelsUpper => "MODELS",
        I18nKey::AiPermissionsUpper => "PERMISSIONS",
        I18nKey::AiBaseModelColon => "Base model:",
        I18nKey::AiFullTerminalUseColon => "Full terminal use:",
        I18nKey::AiComputerUseColon => "Computer use:",
        I18nKey::AiApplyCodeDiffsColon => "Apply code diffs:",
        I18nKey::AiReadFilesColon => "Read files:",
        I18nKey::AiExecuteCommandsColon => "Execute commands:",
        I18nKey::AiInteractWithRunningCommandsColon => "Interact with running commands:",
        I18nKey::AiAskQuestionsColon => "Ask questions:",
        I18nKey::AiRunAgentsColon => "Run agents:",
        I18nKey::AiCallMcpServersColon => "Call MCP servers:",
        I18nKey::AiCallWebToolsColon => "Call web tools:",
        I18nKey::AiAutoSyncPlansColon => "Auto-sync plans to Warp Drive:",
        I18nKey::AiNone => "None",
        I18nKey::AiAgentDecides => "Agent decides",
        I18nKey::AiAlwaysAllow => "Always allow",
        I18nKey::AiAlwaysAsk => "Always ask",
        I18nKey::AiUnknown => "Unknown",
        I18nKey::AiAskOnFirstWrite => "Ask on first write",
        I18nKey::AiNever => "Never",
        I18nKey::AiNeverAsk => "Never ask",
        I18nKey::AiAskUnlessAutoApprove => "Ask unless auto-approve",
        I18nKey::AiOn => "On",
        I18nKey::AiOff => "Off",
        I18nKey::AiDirectoryAllowlistColon => "Directory allowlist:",
        I18nKey::AiCommandAllowlistColon => "Command allowlist:",
        I18nKey::AiCommandDenylistColon => "Command denylist:",
        I18nKey::AiMcpAllowlistColon => "MCP allowlist:",
        I18nKey::AiMcpDenylistColon => "MCP denylist:",
        I18nKey::AiSelectMcpServers => "Select MCP servers",
        I18nKey::AiReadOnly => "Read only",
        I18nKey::AiSupervised => "Supervised",
        I18nKey::AiAllowSpecificDirectories => "Allow in specific directories",
        I18nKey::AiDefault => "Default",
        I18nKey::AiDisabled => "disabled",
        I18nKey::AiProfileDefault => "Profile default",
        I18nKey::AiFreePlanFrontierModelsUnavailable => {
            "Frontier models are unavailable on free plans. "
        }
        I18nKey::AiProfileEditorTitle => "Edit Profile",
        I18nKey::AiProfileEditorName => "Name",
        I18nKey::AiDefaultProfileNameReadonly => "Default profile name cannot be changed.",
        I18nKey::AiDeleteProfile => "Delete profile",
        I18nKey::AiDirectoryPathPlaceholder => "e.g. ~/code-repos/repo",
        I18nKey::AiCommandAllowPlaceholder => "e.g. ls .*",
        I18nKey::AiCommandDenyPlaceholder => "e.g. rm .*",
        I18nKey::AiCommandsCommaSeparatedPlaceholder => "Commands, comma separated",
        I18nKey::AiCommandRegexPlaceholder => "command (supports regex)",
        I18nKey::AiProfileNamePlaceholder => "e.g. \"YOLO code\"",
        I18nKey::AiFullTerminalUseModel => "Full terminal use model",
        I18nKey::AiFullTerminalUseModelDescription => {
            "The model used when the agent operates inside interactive terminal applications like database shells, debuggers, REPLs, or dev servers - reading live output and writing commands to the PTY."
        }
        I18nKey::AiComputerUse => "Computer use",
        I18nKey::AiComputerUseModel => "Computer use model",
        I18nKey::AiComputerUseModelDescription => {
            "The model used when the agent takes control of your computer to interact with graphical applications through mouse movements, clicks, and keyboard input."
        }
        I18nKey::AiContextWindow => "Context window",
        I18nKey::AiContextWindowDescription => {
            "The base model's working memory - how many tokens of your conversation, code, and documents it can consider at once. Larger windows enable longer conversations and more coherent responses over bigger codebases, at the cost of higher latency and compute usage."
        }
        I18nKey::AiPermissionAgentDecidesDescription => {
            "The Agent chooses the safest path: acting on its own when confident, and asking for approval when uncertain."
        }
        I18nKey::AiPermissionAlwaysAllowDescription => {
            "Give the Agent full autonomy - no manual approval ever required."
        }
        I18nKey::AiPermissionAlwaysAskDescription => {
            "Require explicit approval before the Agent takes any action."
        }
        I18nKey::AiWriteToPtyAskOnFirstWriteDescription => {
            "The agent will ask for permission the first time it needs to interact with a running command. After that, it will continue automatically for the rest of that command."
        }
        I18nKey::AiWriteToPtyAlwaysAskDescription => {
            "The agent will always ask for permission to interact with a running command."
        }
        I18nKey::AiComputerUseNeverDescription => {
            "Computer use tools are disabled and will not be available to the Agent."
        }
        I18nKey::AiComputerUseAlwaysAskDescription => {
            "Require explicit approval before the Agent uses computer use tools."
        }
        I18nKey::AiComputerUseAlwaysAllowDescription => {
            "Give the Agent full autonomy to use computer use tools without approval."
        }
        I18nKey::AiUnknownSettingDescription => "Unknown setting.",
        I18nKey::AiAskQuestions => "Ask questions",
        I18nKey::AiRunOrchestratedAgents => "Run orchestrated agents",
        I18nKey::AiRunAgentsNeverDescription => {
            "The Agent cannot run child agents and the run_agents tool will not be available."
        }
        I18nKey::AiRunAgentsAlwaysAllowDescription => {
            "Give the Agent full autonomy to run child agents without approval."
        }
        I18nKey::AiRunAgentsAlwaysAskDescription => {
            "Require explicit approval before the Agent runs child agents."
        }
        I18nKey::AiAskQuestionAskUnlessAutoApproveDescription => {
            "The Agent may ask a question and pause for your response, but will continue automatically when auto-approve is on."
        }
        I18nKey::AiAskQuestionNeverDescription => {
            "The Agent will not ask questions and will continue with its best judgment."
        }
        I18nKey::AiAskQuestionAlwaysAskDescription => {
            "The Agent may ask a question and will pause for your response even when auto-approve is on."
        }
        I18nKey::AiMcpAllowlistProfileDescription => {
            "MCP servers that are allowed to be called by Oz."
        }
        I18nKey::AiMcpDenylistProfileDescription => {
            "MCP servers that are not allowed to be called by Oz."
        }
        I18nKey::AiPlanAutoSync => "Plan auto-sync",
        I18nKey::AiPlanAutoSyncDescription => {
            "The plans this agent creates will be automatically added and synced to Warp Drive."
        }
        I18nKey::AiCallWebTools => "Call web tools",
        I18nKey::AiCallWebToolsDescription => {
            "The agent may use web search when helpful for completing tasks."
        }
        I18nKey::AiToolbarLayout => "Toolbar layout",
        I18nKey::AiSelectCodingAgent => "Select coding agent",
        I18nKey::AiShowAndCollapse => "Show & collapse",
        I18nKey::AiAlwaysShow => "Always show",
        I18nKey::AiNeverShow => "Never show",
        I18nKey::AiLeft => "Left",
        I18nKey::AiRight => "Right",
        I18nKey::AppearanceCategoryLanguage => "Language",
        I18nKey::AppearanceCategoryThemes => "Themes",
        I18nKey::AppearanceCategoryIcon => "Icon",
        I18nKey::AppearanceCategoryWindow => "Window",
        I18nKey::AppearanceCategoryInput => "Input",
        I18nKey::AppearanceCategoryPanes => "Panes",
        I18nKey::AppearanceCategoryBlocks => "Blocks",
        I18nKey::AppearanceCategoryText => "Text",
        I18nKey::AppearanceCategoryCursor => "Cursor",
        I18nKey::AppearanceCategoryTabs => "Tabs",
        I18nKey::AppearanceCategoryFullscreenApps => "Full-screen Apps",
        I18nKey::AppearanceLanguageLabel => "Display language",
        I18nKey::AppearanceLanguageDescription => "Choose the language used by Warp's interface.",
        I18nKey::AppearanceThemeCreateCustom => "Create your own custom theme",
        I18nKey::AppearanceThemeLight => "Light",
        I18nKey::AppearanceThemeDark => "Dark",
        I18nKey::AppearanceThemeCurrent => "Current theme",
        I18nKey::AppearanceThemeSyncWithOs => "Sync with OS",
        I18nKey::AppearanceThemeSyncWithOsDescription => {
            "Automatically switch between light and dark themes when your system does."
        }
        I18nKey::AppearanceAppIconLabel => "Customize your app icon",
        I18nKey::AppearanceAppIconDefault => "Default",
        I18nKey::AppearanceAppIconBundleWarning => {
            "Changing the app icon requires the app to be bundled."
        }
        I18nKey::AppearanceAppIconRestartWarning => {
            "You may need to restart Warp for MacOS to apply the preferred icon style."
        }
        I18nKey::AppearanceWindowCustomSizeLabel => "Open new windows with custom size",
        I18nKey::AppearanceWindowColumns => "Columns",
        I18nKey::AppearanceWindowRows => "Rows",
        I18nKey::AppearanceWindowOpacityLabel => "Window Opacity",
        I18nKey::AppearanceWindowTransparencyUnsupported => {
            "Transparency is not supported with your graphics drivers."
        }
        I18nKey::AppearanceWindowTransparencyWarning => {
            "The selected graphics settings may not support rendering transparent windows."
        }
        I18nKey::AppearanceWindowTransparencySettingsSuggestion => {
            " Try changing the settings for the graphics backend or integrated GPU in Features > System."
        }
        I18nKey::AppearanceWindowBlurRadiusLabel => "Window Blur Radius",
        I18nKey::AppearanceWindowBlurTextureLabel => "Use Window Blur (Acrylic texture)",
        I18nKey::AppearanceWindowHardwareTransparencyWarning => {
            "The selected hardware may not support rendering transparent windows."
        }
        I18nKey::AppearanceToolsPanelConsistentAcrossTabs => {
            "Tools panel visibility is consistent across tabs"
        }
        I18nKey::AppearanceInputTypeLabel => "Input type",
        I18nKey::AppearanceInputRadioWarp => "Warp",
        I18nKey::AppearanceInputRadioShellPs1 => "Shell (PS1)",
        I18nKey::AppearanceInputPositionLabel => "Input position",
        I18nKey::AppearancePanesDimInactive => "Dim inactive panes",
        I18nKey::AppearancePanesFocusFollowsMouse => "Focus follows mouse",
        I18nKey::AppearanceBlocksCompactMode => "Compact mode",
        I18nKey::AppearanceBlocksJumpToBottomButton => "Show Jump to Bottom of Block button",
        I18nKey::AppearanceBlocksShowDividers => "Show block dividers",
        I18nKey::AppearanceTextAgentFont => "Agent font",
        I18nKey::AppearanceTextDefaultSuffix => "default",
        I18nKey::AppearanceTextMatchTerminal => "Match terminal",
        I18nKey::AppearanceTextLineHeight => "Line height",
        I18nKey::AppearanceTextResetToDefault => "Reset to default",
        I18nKey::AppearanceTextTerminalFont => "Terminal font",
        I18nKey::AppearanceTextViewAllFonts => "View all available system fonts",
        I18nKey::AppearanceTextFontWeight => "Font weight",
        I18nKey::AppearanceTextFontSizePx => "Font size (px)",
        I18nKey::AppearanceTextNotebookFontSize => "Notebook font size",
        I18nKey::AppearanceTextUseThinStrokes => "Use thin strokes",
        I18nKey::AppearanceTextMinimumContrast => "Enforce minimum contrast",
        I18nKey::AppearanceTextShowLigatures => "Show ligatures in terminal",
        I18nKey::AppearanceTextLigaturesTooltip => "Ligatures may reduce performance",
        I18nKey::AppearanceCursorType => "Cursor type",
        I18nKey::AppearanceCursorTypeDisabledVim => "Cursor type is disabled in Vim mode",
        I18nKey::AppearanceCursorBlinking => "Blinking cursor",
        I18nKey::AppearanceCursorBar => "Bar",
        I18nKey::AppearanceCursorBlock => "Block",
        I18nKey::AppearanceCursorUnderline => "Underline",
        I18nKey::AppearanceTabsCloseButtonPosition => "Tab close button position",
        I18nKey::AppearanceTabsShowIndicators => "Show tab indicators",
        I18nKey::AppearanceTabsShowCodeReviewButton => "Show code review button",
        I18nKey::AppearanceTabsPreserveActiveColor => "Preserve active tab color for new tabs",
        I18nKey::AppearanceTabsUseVerticalLayout => "Use vertical tab layout",
        I18nKey::AppearanceTabsShowVerticalPanelRestored => {
            "Show vertical tabs panel in restored windows"
        }
        I18nKey::AppearanceTabsShowVerticalPanelRestoredDescription => {
            "When enabled, reopening or restoring a window opens the vertical tabs panel even if it was closed when the window was last saved."
        }
        I18nKey::AppearanceTabsUseLatestPromptTitle => {
            "Use latest user prompt as conversation title in tab names"
        }
        I18nKey::AppearanceTabsUseLatestPromptTitleDescription => {
            "Show the latest user prompt instead of the generated conversation title for Oz and third-party agent sessions in vertical tabs."
        }
        I18nKey::AppearanceTabsHeaderToolbarLayout => "Header toolbar layout",
        I18nKey::AppearanceTabsDirectoryColors => "Directory tab colors",
        I18nKey::AppearanceTabsDirectoryColorsDescription => {
            "Automatically color tabs based on the directory or repo you're working in."
        }
        I18nKey::AppearanceTabsDefaultNoColor => "Default (no color)",
        I18nKey::AppearanceTabsShowTabBar => "Show the tab bar",
        I18nKey::AppearanceFullscreenUseCustomPadding => "Use custom padding in alt-screen",
        I18nKey::AppearanceFullscreenUniformPaddingPx => "Uniform padding (px)",
        I18nKey::AppearanceZoomLabel => "Zoom",
        I18nKey::AppearanceZoomDescription => "Adjusts the default zoom level across all windows",
        I18nKey::AppearanceInputModePinnedBottom => "Pin to the bottom (Warp mode)",
        I18nKey::AppearanceInputModePinnedTop => "Pin to the top (Reverse mode)",
        I18nKey::AppearanceInputModeWaterfall => "Start at the top (Classic mode)",
        I18nKey::AppearanceOptionNever => "Never",
        I18nKey::AppearanceOptionLowDpiDisplays => "On low-DPI displays",
        I18nKey::AppearanceOptionHighDpiDisplays => "On high-DPI displays",
        I18nKey::AppearanceOptionAlways => "Always",
        I18nKey::AppearanceContrastOnlyNamedColors => "Only for named colors",
        I18nKey::AppearanceWorkspaceDecorationsAlways => "Always",
        I18nKey::AppearanceWorkspaceDecorationsWindowed => "When windowed",
        I18nKey::AppearanceWorkspaceDecorationsHover => "Only on hover",
        I18nKey::AppearanceOptionRight => "Right",
        I18nKey::AppearanceOptionLeft => "Left",
        I18nKey::LanguagePreferenceSystem => "Use system language",
        I18nKey::LanguagePreferenceEnglish => "English",
        I18nKey::LanguagePreferenceChineseSimplified => "Simplified Chinese",
    }
}

fn zh_cn(key: I18nKey) -> &'static str {
    match key {
        I18nKey::SettingsSearchPlaceholder => "搜索",
        I18nKey::SettingsSearchNoResultsTitle => "没有匹配的设置。",
        I18nKey::SettingsSearchNoResultsDescription => "可以尝试其他关键词，或检查是否有拼写错误。",
        I18nKey::SettingsNavAbout => "关于",
        I18nKey::SettingsNavAccount => "账户",
        I18nKey::SettingsNavAgents => "智能体",
        I18nKey::SettingsNavBillingAndUsage => "账单与用量",
        I18nKey::SettingsNavCode => "代码",
        I18nKey::SettingsNavCloudPlatform => "云平台",
        I18nKey::SettingsNavTeams => "团队",
        I18nKey::SettingsNavAppearance => "外观",
        I18nKey::SettingsNavFeatures => "功能",
        I18nKey::SettingsNavKeybindings => "键盘快捷键",
        I18nKey::SettingsNavWarpify => "Warpify",
        I18nKey::SettingsNavReferrals => "推荐",
        I18nKey::SettingsNavSharedBlocks => "共享块",
        I18nKey::SettingsNavWarpDrive => "Warp Drive",
        I18nKey::SettingsNavPrivacy => "隐私",
        I18nKey::SettingsNavMcpServers => "MCP 服务器",
        I18nKey::SettingsNavWarpAgent => "Warp 智能体",
        I18nKey::SettingsNavAgentProfiles => "配置档案",
        I18nKey::SettingsNavAgentMcpServers => "MCP 服务器",
        I18nKey::SettingsNavKnowledge => "知识库",
        I18nKey::SettingsNavThirdPartyCliAgents => "第三方 CLI 智能体",
        I18nKey::SettingsNavCodeIndexing => "索引与项目",
        I18nKey::SettingsNavEditorAndCodeReview => "编辑器与代码审查",
        I18nKey::SettingsNavCloudEnvironments => "环境",
        I18nKey::SettingsNavOzCloudApiKeys => "Oz Cloud API 密钥",
        I18nKey::TeamsCreateTeam => "创建团队",
        I18nKey::TeamsJoinExistingTeam => "或者，加入公司内已有的团队",
        I18nKey::TeamsTeamMembers => "团队成员",
        I18nKey::TeamsInviteByLink => "通过链接邀请",
        I18nKey::TeamsInviteByEmail => "通过邮箱邀请",
        I18nKey::TeamsRestrictByDomain => "按域名限制",
        I18nKey::TeamsMakeTeamDiscoverable => "允许发现团队",
        I18nKey::TeamsFreePlanUsageLimits => "免费计划用量限制",
        I18nKey::TeamsPlanUsageLimits => "计划用量限制",
        I18nKey::TeamsSharedNotebooks => "共享笔记本",
        I18nKey::TeamsSharedWorkflows => "共享工作流",
        I18nKey::TeamsNeedMoreSeatsPrefix => "需要更多席位？",
        I18nKey::TeamsUpgradeToEnterprise => "升级到 Enterprise",
        I18nKey::TeamsUpgradeToBusiness => "升级到 Business",
        I18nKey::TeamsUnlimitedTeamMemberLimitSuffix => "以获得无限团队成员上限。",
        I18nKey::TeamsHigherTeamMemberLimitSuffix => "以获得更高的团队成员上限。",
        I18nKey::MenuWhatsNew => "新功能",
        I18nKey::MenuSettings => "设置",
        I18nKey::MenuKeyboardShortcuts => "键盘快捷键",
        I18nKey::MenuDocumentation => "文档",
        I18nKey::MenuFeedback => "反馈",
        I18nKey::MenuViewWarpLogs => "查看 Warp 日志",
        I18nKey::MenuSlack => "Slack",
        I18nKey::MenuSignUp => "注册",
        I18nKey::MenuBillingAndUsage => "账单与用量",
        I18nKey::MenuUpgrade => "升级",
        I18nKey::MenuInviteAFriend => "邀请好友",
        I18nKey::MenuLogOut => "退出登录",
        I18nKey::MenuUpdateAndRelaunchWarp => "更新并重新启动 Warp",
        I18nKey::MenuUpdatingTo => "正在更新到",
        I18nKey::MenuUpdateWarpManually => "手动更新 Warp",
        I18nKey::MenuRearrangeToolbarItems => "重新排列工具栏项目",
        I18nKey::MenuNewWindow => "新建窗口",
        I18nKey::MenuNewTerminalTab => "新建终端标签页",
        I18nKey::MenuNewAgentTab => "新建智能体标签页",
        I18nKey::MenuPreferences => "偏好设置",
        I18nKey::MenuPrivacyPolicy => "隐私政策...",
        I18nKey::MenuDebug => "调试",
        I18nKey::MenuSetDefaultTerminal => "将 Warp 设为默认终端",
        I18nKey::MenuFile => "文件",
        I18nKey::MenuOpenRecent => "打开最近项目",
        I18nKey::MenuLaunchConfigurations => "启动配置",
        I18nKey::MenuSaveNew => "另存为新配置...",
        I18nKey::MenuOpenRepository => "打开仓库",
        I18nKey::MenuCloseFocusedPanel => "关闭当前面板",
        I18nKey::MenuCloseWindow => "关闭窗口",
        I18nKey::MenuEdit => "编辑",
        I18nKey::MenuUseWarpsPrompt => "使用 Warp 提示符",
        I18nKey::MenuCopyOnSelectWithinTerminal => "在终端中选中即复制",
        I18nKey::MenuSynchronizeInputs => "同步输入",
        I18nKey::MenuView => "视图",
        I18nKey::MenuToggleMouseReporting => "切换鼠标上报",
        I18nKey::MenuToggleScrollReporting => "切换滚动上报",
        I18nKey::MenuToggleFocusReporting => "切换焦点上报",
        I18nKey::MenuCompactMode => "紧凑模式",
        I18nKey::MenuTab => "标签页",
        I18nKey::MenuAi => "智能体",
        I18nKey::MenuBlocks => "块",
        I18nKey::MenuDrive => "云盘",
        I18nKey::MenuWindow => "窗口",
        I18nKey::MenuHelp => "帮助",
        I18nKey::MenuSendFeedback => "发送反馈...",
        I18nKey::MenuWarpDocumentation => "Warp 文档...",
        I18nKey::MenuGithubIssues => "GitHub Issues...",
        I18nKey::MenuWarpSlackCommunity => "Warp Slack 社区...",
        I18nKey::CommonAgent => "智能体",
        I18nKey::CommonTerminal => "终端",
        I18nKey::CommonCloudOz => "云端 Oz",
        I18nKey::CommonLocalDockerSandbox => "本地 Docker 沙盒",
        I18nKey::CommonNewWorktreeConfig => "新建 worktree 配置",
        I18nKey::CommonNewTabConfig => "新建标签页配置",
        I18nKey::CommonReopenClosedSession => "重新打开已关闭会话",
        I18nKey::CommonSplitPaneRight => "向右拆分窗格",
        I18nKey::CommonSplitPaneLeft => "向左拆分窗格",
        I18nKey::CommonSplitPaneDown => "向下拆分窗格",
        I18nKey::CommonSplitPaneUp => "向上拆分窗格",
        I18nKey::CommonClosePane => "关闭窗格",
        I18nKey::CommonCopyLink => "复制链接",
        I18nKey::CommonRename => "重命名",
        I18nKey::CommonShare => "共享",
        I18nKey::CommonCollapseAll => "全部折叠",
        I18nKey::CommonRetry => "重试",
        I18nKey::CommonRevertToServer => "还原为服务器版本",
        I18nKey::CommonAttachToActiveSession => "附加到当前会话",
        I18nKey::CommonCopyId => "复制 ID",
        I18nKey::CommonCopyVariables => "复制变量",
        I18nKey::CommonLoadInSubshell => "在子 Shell 中加载",
        I18nKey::CommonOpenOnDesktop => "在桌面应用中打开",
        I18nKey::CommonDuplicate => "复制副本",
        I18nKey::CommonExport => "导出",
        I18nKey::CommonTrash => "移到废纸篓",
        I18nKey::CommonOpen => "打开",
        I18nKey::CommonEdit => "编辑",
        I18nKey::CommonRestore => "恢复",
        I18nKey::CommonDeleteForever => "永久删除",
        I18nKey::CommonUntitled => "未命名",
        I18nKey::CommonRefresh => "刷新",
        I18nKey::CommonCancel => "取消",
        I18nKey::CommonDelete => "删除",
        I18nKey::CommonReject => "拒绝",
        I18nKey::CommonOverwrite => "覆盖",
        I18nKey::CommonPrevious => "上一个",
        I18nKey::CommonNext => "下一个",
        I18nKey::CommonUndo => "撤销",
        I18nKey::CommonRemove => "移除",
        I18nKey::CommonSave => "保存",
        I18nKey::CommonVariables => "变量",
        I18nKey::CommonEditing => "编辑中",
        I18nKey::CommonViewing => "查看中",
        I18nKey::CommonMoveTo => "移动到",
        I18nKey::TabStopSharing => "停止共享",
        I18nKey::TabShareSession => "共享会话",
        I18nKey::TabStopSharingAll => "停止所有共享",
        I18nKey::TabRenameTab => "重命名标签页",
        I18nKey::TabResetTabName => "重置标签页名称",
        I18nKey::TabMoveTabDown => "下移标签页",
        I18nKey::TabMoveTabRight => "右移标签页",
        I18nKey::TabMoveTabUp => "上移标签页",
        I18nKey::TabMoveTabLeft => "左移标签页",
        I18nKey::TabCloseTab => "关闭标签页",
        I18nKey::TabCloseOtherTabs => "关闭其他标签页",
        I18nKey::TabCloseTabsBelow => "关闭下方标签页",
        I18nKey::TabCloseTabsRight => "关闭右侧标签页",
        I18nKey::TabSaveAsNewConfig => "另存为新配置",
        I18nKey::CommonCopy => "复制",
        I18nKey::CommonCut => "剪切",
        I18nKey::CommonPaste => "粘贴",
        I18nKey::CommonSelectAll => "全选",
        I18nKey::CommonCopyPath => "复制路径",
        I18nKey::CommonOpenInWarp => "在 Warp 中打开",
        I18nKey::CommonOpenInEditor => "在编辑器中打开",
        I18nKey::CommonShowInFinder => "在 Finder 中显示",
        I18nKey::CommonShowContainingFolder => "显示所在文件夹",
        I18nKey::TerminalCopyUrl => "复制 URL",
        I18nKey::TerminalInsertIntoInput => "插入到输入框",
        I18nKey::TerminalCopyCommand => "复制命令",
        I18nKey::TerminalCopyCommands => "复制命令",
        I18nKey::TerminalFindWithinBlock => "在块内查找",
        I18nKey::TerminalFindWithinBlocks => "在块内查找",
        I18nKey::TerminalScrollToTopOfBlock => "滚动到块顶部",
        I18nKey::TerminalScrollToTopOfBlocks => "滚动到块顶部",
        I18nKey::TerminalScrollToBottomOfBlock => "滚动到块底部",
        I18nKey::TerminalScrollToBottomOfBlocks => "滚动到块底部",
        I18nKey::TerminalShare => "共享...",
        I18nKey::TerminalShareBlock => "共享块...",
        I18nKey::TerminalSaveAsWorkflow => "另存为工作流",
        I18nKey::TerminalAskWarpAi => "询问 Warp AI",
        I18nKey::TerminalCopyOutput => "复制输出",
        I18nKey::TerminalCopyFilteredOutput => "复制过滤后的输出",
        I18nKey::TerminalToggleBlockFilter => "切换块过滤器",
        I18nKey::TerminalToggleBookmark => "切换书签",
        I18nKey::TerminalForkFromHere => "从这里派生（仅开发版）",
        I18nKey::TerminalRewindToBeforeHere => "回退到这里之前",
        I18nKey::TerminalCopyPrompt => "复制提示符",
        I18nKey::TerminalCopyRightPrompt => "复制右侧提示符",
        I18nKey::TerminalCopyWorkingDirectory => "复制工作目录",
        I18nKey::TerminalCopyGitBranch => "复制 Git 分支",
        I18nKey::TerminalEditCliAgentToolbelt => "编辑 CLI 智能体工具栏",
        I18nKey::TerminalEditAgentToolbelt => "编辑智能体工具栏",
        I18nKey::TerminalEditPrompt => "编辑提示符",
        I18nKey::TerminalCommandSearch => "命令搜索",
        I18nKey::TerminalAiCommandSearch => "AI 命令搜索",
        I18nKey::TerminalCopyOutputAsMarkdown => "复制输出为 Markdown",
        I18nKey::TerminalSaveAsPrompt => "另存为提示词",
        I18nKey::TerminalShareConversation => "共享对话",
        I18nKey::TerminalCopyConversationText => "复制对话文本",
        I18nKey::DriveFolder => "文件夹",
        I18nKey::DriveNotebook => "笔记本",
        I18nKey::DriveWorkflow => "工作流",
        I18nKey::DrivePrompt => "提示词",
        I18nKey::DriveEnvironmentVariables => "环境变量",
        I18nKey::DriveNewFolder => "新建文件夹",
        I18nKey::DriveNewNotebook => "新建笔记本",
        I18nKey::DriveNewWorkflow => "新建工作流",
        I18nKey::DriveNewPrompt => "新建提示词",
        I18nKey::DriveNewEnvironmentVariables => "新建环境变量",
        I18nKey::DriveImport => "导入",
        I18nKey::DriveRemove => "移除",
        I18nKey::DriveEmptyTrash => "清空废纸篓",
        I18nKey::DriveTrashTitle => "废纸篓",
        I18nKey::DriveCreateTeamText => "与团队成员共享命令和知识。",
        I18nKey::DriveTeamZeroStateText => {
            "将个人工作流或笔记本拖放或移动到这里，即可与团队共享。"
        }
        I18nKey::DriveOfflineBannerText => "你已离线。部分文件将以只读方式显示。",
        I18nKey::DriveCopyPrompt => "复制提示词",
        I18nKey::DriveCopyWorkflowText => "复制工作流文本",
        I18nKey::DriveCopyToPersonal => "复制到个人空间",
        I18nKey::DriveCopyAll => "全部复制",
        I18nKey::DriveRestoreNotebookTooltip => "从废纸篓恢复笔记本",
        I18nKey::DriveCopyNotebookToPersonalTooltip => {
            "将笔记本内容复制到你的个人工作区"
        }
        I18nKey::DriveCopyNotebookToClipboardTooltip => "将笔记本内容复制到剪贴板",
        I18nKey::DriveRestoreWorkflowTooltip => "从废纸篓恢复工作流",
        I18nKey::DriveRefreshNotebookTooltip => "刷新笔记本",
        I18nKey::WorkflowSignInToEditTooltip => "登录后编辑",
        I18nKey::EnvVarsCommand => "命令",
        I18nKey::EnvVarsClearSecret => "清除密钥",
        I18nKey::EnvVarsNoAccess => "你已无法访问这些环境变量",
        I18nKey::EnvVarsMovedToTrash => "环境变量已移到废纸篓",
        I18nKey::EnvVarsRestoreTooltip => "从废纸篓恢复环境变量",
        I18nKey::CodeHunk => "变更块：",
        I18nKey::CodeOpenInNewPane => "在新窗格中打开",
        I18nKey::CodeOpenInNewTab => "在新标签页中打开",
        I18nKey::CodeOpenFile => "打开文件",
        I18nKey::CodeNewFile => "新建文件",
        I18nKey::CodeCdToDirectory => "cd 到目录",
        I18nKey::CodeRevealInFinder => "在 Finder 中显示",
        I18nKey::CodeRevealInExplorer => "在资源管理器中显示",
        I18nKey::CodeRevealInFileManager => "在文件管理器中显示",
        I18nKey::CodeAttachAsContext => "作为上下文附加",
        I18nKey::CodeCopyRelativePath => "复制相对路径",
        I18nKey::CodeOpeningUnavailableRemoteTooltip => "远程会话中无法打开文件",
        I18nKey::CodeGoToDefinition => "转到定义",
        I18nKey::CodeFindReferences => "查找引用",
        I18nKey::CodeDiscardThisVersion => "丢弃此版本",
        I18nKey::CodeAcceptAndSave => "接受并保存",
        I18nKey::CodeCloseSaved => "关闭已保存",
        I18nKey::CodeCopyFilePath => "复制文件路径",
        I18nKey::CodeViewMarkdownPreview => "查看 Markdown 预览",
        I18nKey::CodeSearchDiffSetsPlaceholder => "搜索 diff 集或要比较的分支…",
        I18nKey::CodeLineNumberColumnPlaceholder => "行号:列号",
        I18nKey::CodeReviewCopyText => "复制文本",
        I18nKey::CodeReviewSendToAgent => "发送给智能体",
        I18nKey::CodeReviewViewInGithub => "在 GitHub 中查看",
        I18nKey::CodeReviewOneComment => "1 条评论",
        I18nKey::CodeReviewCommentSingular => "条评论",
        I18nKey::CodeReviewCommentPlural => "条评论",
        I18nKey::CodeReviewOutdatedCommentSingular => "条过时评论",
        I18nKey::CodeReviewOutdatedCommentPlural => "条过时评论",
        I18nKey::CodeReviewCommit => "提交",
        I18nKey::CodeReviewPush => "推送",
        I18nKey::CodeReviewPublish => "发布",
        I18nKey::CodeReviewCreatePr => "创建 PR",
        I18nKey::CodeReviewAddDiffSetAsContext => "将 diff 集添加为上下文",
        I18nKey::CodeReviewAddFileDiffAsContext => "将文件 diff 添加为上下文",
        I18nKey::CodeReviewShowSavedComment => "显示已保存评论",
        I18nKey::CodeReviewAddComment => "添加评论",
        I18nKey::CodeReviewDiscardAll => "全部丢弃",
        I18nKey::CodeReviewStashChanges => "暂存更改",
        I18nKey::CodeReviewNoChangesToCommit => "没有可提交的更改",
        I18nKey::CodeReviewNoGitActionsAvailable => "没有可用的 Git 操作",
        I18nKey::CodeReviewMaximize => "最大化",
        I18nKey::CodeReviewShowFileNavigation => "显示文件导航",
        I18nKey::CodeReviewHideFileNavigation => "隐藏文件导航",
        I18nKey::CodeReviewInitializeCodebase => "初始化代码库",
        I18nKey::CodeReviewInitializeCodebaseTooltip => "启用代码库索引和 WARP.md",
        I18nKey::CodeReviewOpenRepository => "打开仓库",
        I18nKey::CodeReviewOpenRepositoryTooltip => "导航到仓库并初始化编码功能",
        I18nKey::CodeReviewDiscardChanges => "丢弃更改",
        I18nKey::CodeReviewFileLevelCommentsCantBeEdited => "目前无法编辑文件级评论。",
        I18nKey::CodeReviewOutdatedCommentsCantBeEdited => "无法编辑已过时评论。",
        I18nKey::SettingsPageTitleAccount => "账户",
        I18nKey::SettingsPageTitlePrivacy => "隐私",
        I18nKey::SettingsPageTitleMcpServers => "MCP 服务器",
        I18nKey::SettingsPageTitleInviteFriend => "邀请好友使用 Warp",
        I18nKey::SettingsCategoryBillingAndUsage => "账单与用量",
        I18nKey::SettingsCategoryFeaturesGeneral => "通用",
        I18nKey::SettingsCategoryFeaturesSession => "会话",
        I18nKey::SettingsCategoryFeaturesKeys => "按键",
        I18nKey::SettingsCategoryFeaturesTextEditing => "文本编辑",
        I18nKey::SettingsCategoryFeaturesTerminalInput => "终端输入",
        I18nKey::SettingsCategoryFeaturesTerminal => "终端",
        I18nKey::SettingsCategoryFeaturesNotifications => "通知",
        I18nKey::SettingsCategoryFeaturesWorkflows => "工作流",
        I18nKey::SettingsCategoryFeaturesSystem => "系统",
        I18nKey::SettingsCategoryCodebaseIndexing => "代码库索引",
        I18nKey::SettingsCategoryCodeEditorAndReview => "代码编辑器与代码审查",
        I18nKey::SettingsCategoryWarpifySubshells => "子 Shell",
        I18nKey::SettingsCategoryWarpifySubshellsDescription => {
            "支持的子 Shell：bash、zsh 和 fish。"
        }
        I18nKey::SettingsCategoryWarpifySsh => "SSH",
        I18nKey::SettingsCategoryWarpifySshDescription => "为交互式 SSH 会话启用 Warpify。",
        I18nKey::WarpDriveSignUpPrompt => "要使用 Warp Drive，请先创建账户。",
        I18nKey::WarpDriveSignUpButton => "注册",
        I18nKey::WarpDriveLabel => "Warp Drive",
        I18nKey::WarpDriveDescription => {
            "Warp Drive 是终端中的工作区，可保存工作流、笔记本、提示词和环境变量，供个人使用或与团队共享。"
        }
        I18nKey::WarpifyTitle => "Warpify",
        I18nKey::WarpifyDescription => {
            "配置 Warp 是否尝试对特定 shell 进行 Warpify（添加块、输入模式等支持）。"
        }
        I18nKey::WarpifyLearnMore => "了解更多",
        I18nKey::WarpifyCommandPlaceholder => "命令（支持正则表达式）",
        I18nKey::WarpifyHostPlaceholder => "主机（支持正则表达式）",
        I18nKey::WarpifyAddedCommands => "已添加命令",
        I18nKey::WarpifyDenylistedCommands => "拒绝列表命令",
        I18nKey::WarpifySshSessions => "对 SSH 会话启用 Warpify",
        I18nKey::WarpifyInstallSshExtension => "安装 SSH 扩展",
        I18nKey::WarpifyInstallSshExtensionDescription => {
            "控制远程主机未安装 Warp SSH 扩展时的安装行为。"
        }
        I18nKey::WarpifyUseTmuxWarpification => "使用 Tmux Warpification",
        I18nKey::WarpifyTmuxWarpificationDescription => {
            "tmux ssh 包装器在许多默认方式无法工作的场景中可用，但可能需要你点击按钮来完成 Warpify。对新标签页生效。"
        }
        I18nKey::WarpifyDenylistedHosts => "拒绝列表主机",
        I18nKey::WarpifyInstallModeAlwaysAsk => "始终询问",
        I18nKey::WarpifyInstallModeAlwaysInstall => "始终安装",
        I18nKey::WarpifyInstallModeNeverInstall => "从不安装",
        I18nKey::AccountSignUp => "注册",
        I18nKey::AccountFreePlan => "免费",
        I18nKey::AccountComparePlans => "比较套餐",
        I18nKey::AccountContactSupport => "联系支持",
        I18nKey::AccountManageBilling => "管理账单",
        I18nKey::AccountUpgradeTurboPlan => "升级到 Turbo 套餐",
        I18nKey::AccountUpgradeLightspeedPlan => "升级到 Lightspeed 套餐",
        I18nKey::AccountSettingsSync => "设置同步",
        I18nKey::AccountReferralCta => "与朋友和同事分享 Warp 来获得奖励",
        I18nKey::AccountReferFriend => "推荐好友",
        I18nKey::AccountVersion => "版本",
        I18nKey::AccountLogOut => "退出登录",
        I18nKey::AccountUpdateUpToDate => "已是最新版本",
        I18nKey::AccountUpdateCheckForUpdates => "检查更新",
        I18nKey::AccountUpdateChecking => "正在检查更新...",
        I18nKey::AccountUpdateDownloading => "正在下载更新...",
        I18nKey::AccountUpdateAvailable => "有可用更新",
        I18nKey::AccountUpdateRelaunchWarp => "重新启动 Warp",
        I18nKey::AccountUpdateUpdating => "正在更新...",
        I18nKey::AccountUpdateInstalled => "更新已安装",
        I18nKey::AccountUpdateUnableToInstall => "Warp 有新版本，但无法安装",
        I18nKey::AccountUpdateManual => "手动更新 Warp",
        I18nKey::AccountUpdateUnableToLaunch => "Warp 新版本已安装，但无法启动。",
        I18nKey::PrivacySafeModeTitle => "密钥脱敏",
        I18nKey::PrivacySafeModeDescription => {
            "启用后，Warp 会扫描块、Warp Drive 对象内容和 Oz 提示词中的潜在敏感信息，并阻止将这些数据保存或发送到任何服务器。你可以通过正则表达式自定义此列表。"
        }
        I18nKey::PrivacyCustomRedactionTitle => "自定义密钥脱敏",
        I18nKey::PrivacyCustomRedactionDescription => {
            "使用正则表达式定义其他需要脱敏的密钥或数据。下次命令运行时生效。你可以在正则表达式前添加 inline (?i) 标志以忽略大小写。"
        }
        I18nKey::PrivacyTelemetryTitle => "帮助改进 Warp",
        I18nKey::PrivacyTelemetryDescriptionEnterprise => {
            "应用分析能帮助我们为你改进产品。我们只收集应用使用元数据，绝不会收集控制台输入或输出。"
        }
        I18nKey::PrivacyTelemetryDescription => {
            "应用分析能帮助我们为你改进产品。我们可能会收集某些控制台交互，以改进 Warp 的 AI 能力。"
        }
        I18nKey::PrivacyTelemetryFreeTierNote => {
            "免费套餐需要启用分析才能使用 AI 功能。"
        }
        I18nKey::PrivacyTelemetryDocsLink => "了解 Warp 如何使用数据",
        I18nKey::PrivacyDataManagementTitle => "管理你的数据",
        I18nKey::PrivacyDataManagementDescription => {
            "你可以随时选择永久删除自己的 Warp 账户。删除后你将无法再使用 Warp。"
        }
        I18nKey::PrivacyDataManagementLink => "访问数据管理页面",
        I18nKey::PrivacyPolicyTitle => "隐私政策",
        I18nKey::PrivacyPolicyLink => "阅读 Warp 隐私政策",
        I18nKey::PrivacyAddRegexPatternTitle => "添加正则表达式模式",
        I18nKey::PrivacySecretDisplayAsterisks => "星号",
        I18nKey::PrivacySecretDisplayStrikethrough => "删除线",
        I18nKey::PrivacySecretDisplayAlwaysShow => "始终显示密钥",
        I18nKey::PrivacyTabPersonal => "个人",
        I18nKey::PrivacyTabEnterprise => "企业",
        I18nKey::PrivacyEnterpriseReadonly => "企业密钥脱敏配置无法修改。",
        I18nKey::PrivacyNoEnterpriseRegexes => "你的组织尚未配置企业正则表达式。",
        I18nKey::PrivacyRecommended => "推荐",
        I18nKey::PrivacyAddAll => "全部添加",
        I18nKey::PrivacyEnabledByOrganization => "已由你的组织启用。",
        I18nKey::PrivacySecretVisualMode => "密钥视觉脱敏模式",
        I18nKey::PrivacySecretVisualModeDescription => {
            "选择密钥在块列表中的视觉呈现方式，同时保持可搜索。此设置只影响你在块列表中看到的内容。"
        }
        I18nKey::PrivacyAddRegex => "添加正则",
        I18nKey::PrivacyZdrTooltip => {
            "你的管理员已为团队启用零数据保留。用户生成内容永远不会被收集。"
        }
        I18nKey::PrivacyManagedByOrganization => "此设置由你的组织管理。",
        I18nKey::PrivacyCrashReportsTitle => "发送崩溃报告",
        I18nKey::PrivacyCrashReportsDescription => "崩溃报告有助于调试和稳定性改进。",
        I18nKey::PrivacyCloudConversationStorageTitle => "在云端存储 AI 对话",
        I18nKey::PrivacyCloudConversationStorageEnabledDescription => {
            "Agent 对话可以分享给他人，并会在你登录不同设备时保留。这些数据仅用于产品功能，Warp 不会将其用于分析。"
        }
        I18nKey::PrivacyCloudConversationStorageDisabledDescription => {
            "Agent 对话仅存储在你的本机，退出登录后会丢失，也无法分享。注意：环境 Agent 的对话数据仍会存储在云端。"
        }
        I18nKey::PrivacyNetworkLogTitle => "网络日志控制台",
        I18nKey::PrivacyNetworkLogDescription => {
            "我们构建了一个原生控制台，让你查看 Warp 与外部服务器之间的所有通信，确保你的工作始终安全。"
        }
        I18nKey::PrivacyNetworkLogLink => "查看网络日志",
        I18nKey::PrivacyModalNameOptional => "名称（可选）",
        I18nKey::PrivacyModalNamePlaceholder => "例如 \"Google API Key\"",
        I18nKey::PrivacyModalRegexPattern => "正则表达式模式",
        I18nKey::PrivacyModalInvalidRegex => "正则表达式无效",
        I18nKey::PrivacyModalCancel => "取消",
        I18nKey::CodeIndexNewFolder => "索引新文件夹",
        I18nKey::CodeFeatureName => "代码",
        I18nKey::CodeInitializationSettingsHeader => "初始化设置",
        I18nKey::CodebaseIndexingLabel => "代码库索引",
        I18nKey::CodebaseIndexingDescription => {
            "Warp 可以在你浏览代码仓库时自动建立索引，帮助 Agent 快速理解上下文并提供解决方案。代码绝不会存储在服务器上。如果某个代码库无法建立索引，Warp 仍可通过 grep 和 find 工具调用来浏览代码库并获取洞察。"
        }
        I18nKey::CodeWarpIndexingIgnoreDescription => {
            "如需从索引中排除特定文件或目录，请将它们添加到仓库目录中的 .warpindexingignore 文件。这些文件仍可供 AI 功能访问，但不会包含在代码库嵌入中。"
        }
        I18nKey::CodeAutoIndexFeatureName => "默认索引新文件夹",
        I18nKey::CodeAutoIndexDescription => {
            "设为 true 后，Warp 会在你浏览代码仓库时自动建立索引，帮助 Agent 快速理解上下文并提供更有针对性的解决方案。"
        }
        I18nKey::CodeIndexingDisabledAdmin => "团队管理员已禁用代码库索引。",
        I18nKey::CodeIndexingWorkspaceEnabledAdmin => "团队管理员已启用代码库索引。",
        I18nKey::CodeIndexingDisabledGlobalAi => "必须启用 AI 功能才能使用代码库索引。",
        I18nKey::CodebaseIndexLimitReached => {
            "你已达到当前套餐的代码库索引数量上限。请删除现有索引后再自动索引新的代码库。"
        }
        I18nKey::CodeSubpageCodebaseIndexing => "代码库索引",
        I18nKey::CodeSubpageEditorAndCodeReview => "编辑器与代码审查",
        I18nKey::CodeInitializedFolders => "已初始化 / 已索引文件夹",
        I18nKey::CodeNoFoldersInitialized => "尚未初始化任何文件夹。",
        I18nKey::CodeOpenProjectRules => "打开项目规则",
        I18nKey::CodeIndexingSection => "索引",
        I18nKey::CodeNoIndexCreated => "未创建索引",
        I18nKey::CodeIndexDiscoveredChunks => "已发现 {count} 个分块",
        I18nKey::CodeIndexSyncingProgress => "正在同步 - {completed} / {total}",
        I18nKey::CodeIndexSyncing => "正在同步...",
        I18nKey::CodeIndexSynced => "已同步",
        I18nKey::CodeIndexTooLarge => "代码库过大",
        I18nKey::CodeIndexStale => "已过期",
        I18nKey::CodeIndexFailed => "失败",
        I18nKey::CodeIndexNotBuilt => "未构建索引",
        I18nKey::CodeLspServersSection => "LSP 服务器",
        I18nKey::CodeLspInstalled => "已安装",
        I18nKey::CodeLspInstalling => "正在安装...",
        I18nKey::CodeLspChecking => "正在检查...",
        I18nKey::CodeLspAvailableForDownload => "可下载",
        I18nKey::CodeLspRestartServer => "重启服务器",
        I18nKey::CodeLspViewLogs => "查看日志",
        I18nKey::CodeLspAvailable => "可用",
        I18nKey::CodeLspBusy => "忙碌",
        I18nKey::CodeLspStopped => "已停止",
        I18nKey::CodeLspNotRunning => "未运行",
        I18nKey::CodeAutoOpenCodeReviewPanel => "自动打开代码审查面板",
        I18nKey::CodeAutoOpenCodeReviewPanelDescription => {
            "启用后，代码审查面板会在一次对话中首次接受 diff 时打开。"
        }
        I18nKey::CodeShowCodeReviewButton => "显示代码审查按钮",
        I18nKey::CodeShowCodeReviewButtonDescription => {
            "在窗口右上角显示一个用于切换代码审查面板的按钮。"
        }
        I18nKey::CodeShowDiffStats => "在代码审查按钮上显示 diff 统计",
        I18nKey::CodeShowDiffStatsDescription => {
            "在代码审查按钮上显示新增和删除行数。"
        }
        I18nKey::CodeProjectExplorer => "项目浏览器",
        I18nKey::CodeProjectExplorerDescription => {
            "在左侧工具面板中添加类似 IDE 的项目浏览器 / 文件树。"
        }
        I18nKey::CodeGlobalFileSearch => "全局文件搜索",
        I18nKey::CodeGlobalFileSearchDescription => "在左侧工具面板中添加全局文件搜索。",
        I18nKey::CodeExternalDefaultApp => "默认应用",
        I18nKey::CodeExternalSplitPane => "拆分窗格",
        I18nKey::CodeExternalNewTab => "新标签页",
        I18nKey::CodeExternalChooseFileLinksEditor => "选择用于打开文件链接的编辑器",
        I18nKey::CodeExternalChooseCodePanelEditor => {
            "选择用于打开代码审查面板、项目浏览器和全局搜索中文件的编辑器"
        }
        I18nKey::CodeExternalChooseLayout => "选择在 Warp 中打开文件的布局",
        I18nKey::CodeExternalTabbedFileViewer => "将文件分组到单个编辑器窗格",
        I18nKey::CodeExternalTabbedFileViewerDescription => {
            "启用后，同一标签页中打开的任何文件都会自动分组到单个编辑器窗格中。"
        }
        I18nKey::CodeExternalMarkdownViewerDefault => {
            "默认使用 Warp 的 Markdown 查看器打开 Markdown 文件"
        }
        I18nKey::KeybindingsSearchPlaceholder => "按名称或按键搜索（例如 \"cmd d\"）",
        I18nKey::KeybindingsConflictWarning => "此快捷键与其他快捷键冲突",
        I18nKey::KeybindingsDefault => "默认",
        I18nKey::KeybindingsCancel => "取消",
        I18nKey::KeybindingsClear => "清除",
        I18nKey::KeybindingsSave => "保存",
        I18nKey::KeybindingsPressNewShortcut => "按下新的键盘快捷键",
        I18nKey::KeybindingsDescription => "在下方为现有操作添加你自己的自定义快捷键。",
        I18nKey::KeybindingsUse => "使用",
        I18nKey::KeybindingsReferenceInSidePane => "可随时在侧边窗格中查看这些快捷键。",
        I18nKey::KeybindingsNotSyncedTooltip => "键盘快捷键不会同步到云端",
        I18nKey::KeybindingsConfigureTitle => "配置键盘快捷键",
        I18nKey::KeybindingsCommandColumn => "命令",
        I18nKey::PlatformApiKeyDeletedToast => "API Key 已删除",
        I18nKey::PlatformNewApiKeyTitle => "新建 API Key",
        I18nKey::PlatformSaveYourKeyTitle => "保存你的 Key",
        I18nKey::PlatformApiKeysTitle => "Oz Cloud API Keys",
        I18nKey::PlatformCreateApiKeyButton => "+ 创建 API Key",
        I18nKey::PlatformDescriptionPrefix => {
            "创建和管理 API Key，让其他 Oz Cloud Agent 能够访问你的 Warp 账户。\n如需更多信息，请访问"
        }
        I18nKey::PlatformDocumentationLink => "文档。",
        I18nKey::PlatformHeaderName => "名称",
        I18nKey::PlatformHeaderKey => "Key",
        I18nKey::PlatformHeaderScope => "范围",
        I18nKey::PlatformHeaderCreated => "创建时间",
        I18nKey::PlatformHeaderLastUsed => "上次使用",
        I18nKey::PlatformHeaderExpiresAt => "过期时间",
        I18nKey::PlatformNever => "从不",
        I18nKey::PlatformScopePersonal => "个人",
        I18nKey::PlatformScopeTeam => "团队",
        I18nKey::PlatformNoApiKeys => "没有 API Key",
        I18nKey::PlatformNoApiKeysDescription => "创建一个 Key 来管理对 Warp 的外部访问",
        I18nKey::PlatformApiKeyTypePersonalDescription => {
            "此 API Key 绑定到你的用户，可向你的 Warp 账户发起请求。"
        }
        I18nKey::PlatformApiKeyTypeTeamDescription => {
            "此 API Key 绑定到你的团队，可代表你的团队发起请求。"
        }
        I18nKey::PlatformExpirationOneDay => "1 天",
        I18nKey::PlatformExpirationThirtyDays => "30 天",
        I18nKey::PlatformExpirationNinetyDays => "90 天",
        I18nKey::PlatformExpirationNever => "从不",
        I18nKey::PlatformTeamKeyNoCurrentTeamError => {
            "无法创建团队 API Key，因为当前没有团队。"
        }
        I18nKey::PlatformCreateApiKeyFailed => "创建 API Key 失败。请重试。",
        I18nKey::PlatformSecretKeyInfo => "此密钥只会显示一次。请复制并安全保存。",
        I18nKey::PlatformCopied => "已复制",
        I18nKey::PlatformCopy => "复制",
        I18nKey::PlatformDone => "完成",
        I18nKey::PlatformNameLabel => "名称",
        I18nKey::PlatformTypeLabel => "类型",
        I18nKey::PlatformExpirationLabel => "过期时间",
        I18nKey::PlatformCreating => "正在创建...",
        I18nKey::PlatformCreateKey => "创建 Key",
        I18nKey::PlatformSecretKeyCopiedToast => "密钥已复制。",
        I18nKey::PlatformDeleteApiKeyFailed => "删除 API Key 失败。请重试。",
        I18nKey::FeaturesPinToTop => "固定到顶部",
        I18nKey::FeaturesPinToBottom => "固定到底部",
        I18nKey::FeaturesPinToLeft => "固定到左侧",
        I18nKey::FeaturesPinToRight => "固定到右侧",
        I18nKey::FeaturesActiveScreen => "当前屏幕",
        I18nKey::FeaturesDefault => "默认",
        I18nKey::FeaturesNewTabAfterAllTabs => "所有标签页之后",
        I18nKey::FeaturesNewTabAfterCurrentTab => "当前标签页之后",
        I18nKey::FeaturesGlobalHotkeyDisabled => "已禁用",
        I18nKey::FeaturesGlobalHotkeyDedicatedWindow => "专用快捷键窗口",
        I18nKey::FeaturesGlobalHotkeyShowHideAllWindows => "显示/隐藏所有窗口",
        I18nKey::FeaturesCtrlTabActivatePrevNextTab => "激活上一个/下一个标签页",
        I18nKey::FeaturesCtrlTabCycleMostRecentSession => "循环最近使用的会话",
        I18nKey::FeaturesCtrlTabCycleMostRecentTab => "循环最近使用的标签页",
        I18nKey::FeaturesTabBehaviorOpenCompletions => "打开补全菜单",
        I18nKey::FeaturesTabBehaviorAcceptAutosuggestion => "接受自动建议",
        I18nKey::FeaturesTabBehaviorUserDefined => "用户自定义",
        I18nKey::FeaturesDefaultSessionTerminal => "终端",
        I18nKey::FeaturesDefaultSessionAgent => "智能体",
        I18nKey::FeaturesDefaultSessionCloudAgent => "Cloud Oz",
        I18nKey::FeaturesDefaultSessionTabConfig => "标签页配置",
        I18nKey::FeaturesDefaultSessionDockerSandbox => "本地 Docker 沙箱",
        I18nKey::FeaturesChangesApplyNewWindows => "更改将应用于新窗口。",
        I18nKey::FeaturesCurrentBackend => "当前后端：{backend}",
        I18nKey::FeaturesOpenLinksInDesktopApp => "在桌面应用中打开链接",
        I18nKey::FeaturesOpenLinksInDesktopAppTooltip => {
            "尽可能自动在桌面应用中打开链接。"
        }
        I18nKey::FeaturesRestoreSession => "启动时恢复窗口、标签页和窗格",
        I18nKey::FeaturesWaylandRestoreWarning => "Wayland 上不会恢复窗口位置。",
        I18nKey::FeaturesSeeDocs => "查看文档。",
        I18nKey::FeaturesStickyCommandHeader => "显示粘性命令标题",
        I18nKey::FeaturesLinkTooltip => "点击链接时显示提示",
        I18nKey::FeaturesQuitWarning => "退出或注销前显示警告",
        I18nKey::FeaturesStartWarpAtLoginMac => "登录时启动 Warp（需要 macOS 13+）",
        I18nKey::FeaturesStartWarpAtLogin => "登录时启动 Warp",
        I18nKey::FeaturesQuitWhenAllWindowsClosed => "关闭所有窗口时退出",
        I18nKey::FeaturesShowChangelogToast => "更新后显示变更日志 toast",
        I18nKey::FeaturesAllowedValuesOneToTwenty => "允许值：1-20",
        I18nKey::FeaturesMouseScrollLines => "鼠标滚轮每次滚动的行数",
        I18nKey::FeaturesMouseScrollTooltip => "支持 1 到 20 之间的小数值。",
        I18nKey::FeaturesAutoOpenCodeReviewPanel => "自动打开代码审查面板",
        I18nKey::FeaturesAutoOpenCodeReviewPanelDescription => {
            "启用后，代码审查面板会在一次对话中首次接受 diff 时打开"
        }
        I18nKey::FeaturesWarpDefaultTerminal => "Warp 是默认终端",
        I18nKey::FeaturesMakeWarpDefaultTerminal => "将 Warp 设为默认终端",
        I18nKey::FeaturesMaximumRowsInBlock => "块中的最大行数",
        I18nKey::FeaturesBlockMaximumRowsDescription => {
            "将限制设置为超过 100k 行可能影响性能。支持的最大行数为 {max_rows}。"
        }
        I18nKey::FeaturesSshWrapper => "Warp SSH Wrapper",
        I18nKey::FeaturesSshWrapperNewSessions => "此更改将在新会话中生效",
        I18nKey::FeaturesReceiveDesktopNotifications => "接收来自 Warp 的桌面通知",
        I18nKey::FeaturesNotifyAgentTaskCompleted => "智能体完成任务时通知",
        I18nKey::FeaturesNotifyCommandNeedsAttention => {
            "命令或智能体需要你继续处理时通知"
        }
        I18nKey::FeaturesPlayNotificationSounds => "播放通知声音",
        I18nKey::FeaturesShowInAppAgentNotifications => "显示应用内智能体通知",
        I18nKey::FeaturesNotificationWhenCommandLongerThan => "当命令运行超过",
        I18nKey::FeaturesNotificationSecondsToComplete => "秒仍未完成",
        I18nKey::FeaturesToastNotificationsStayVisibleFor => "Toast 通知保持可见",
        I18nKey::FeaturesSeconds => "秒",
        I18nKey::FeaturesDefaultShellForNewSessions => "新会话的默认 shell",
        I18nKey::FeaturesWorkingDirectoryForNewSessions => "新会话的工作目录",
        I18nKey::FeaturesConfirmCloseSharedSession => "关闭共享会话前确认",
        I18nKey::FeaturesLeftOptionKeyMeta => "左 Option 键作为 Meta",
        I18nKey::FeaturesRightOptionKeyMeta => "右 Option 键作为 Meta",
        I18nKey::FeaturesLeftAltKeyMeta => "左 Alt 键作为 Meta",
        I18nKey::FeaturesRightAltKeyMeta => "右 Alt 键作为 Meta",
        I18nKey::FeaturesGlobalHotkeyLabel => "全局快捷键：",
        I18nKey::FeaturesNotSupportedOnWayland => "Wayland 上不支持。",
        I18nKey::FeaturesWidthPercent => "宽度 %",
        I18nKey::FeaturesHeightPercent => "高度 %",
        I18nKey::FeaturesAutohideKeyboardFocus => "失去键盘焦点时自动隐藏",
        I18nKey::FeaturesKeybinding => "快捷键",
        I18nKey::FeaturesClickSetGlobalHotkey => "点击设置全局快捷键",
        I18nKey::FeaturesChangeKeybinding => "更改快捷键",
        I18nKey::FeaturesAutocompleteSymbols => "自动补全引号、括号和方括号",
        I18nKey::FeaturesErrorUnderlining => "命令错误下划线",
        I18nKey::FeaturesSyntaxHighlighting => "命令语法高亮",
        I18nKey::FeaturesOpenCompletionsTyping => "输入时打开补全菜单",
        I18nKey::FeaturesSuggestCorrectedCommands => "建议修正命令",
        I18nKey::FeaturesExpandAliasesTyping => "输入时展开别名",
        I18nKey::FeaturesMiddleClickPaste => "中键点击粘贴",
        I18nKey::FeaturesVimMode => "使用 Vim 键位编辑代码和命令",
        I18nKey::FeaturesVimUnnamedRegisterClipboard => {
            "将未命名寄存器设为系统剪贴板"
        }
        I18nKey::FeaturesVimStatusBar => "显示 Vim 状态栏",
        I18nKey::FeaturesAtContextMenuTerminalMode => "在终端模式中启用 '@' 上下文菜单",
        I18nKey::FeaturesSlashCommandsTerminalMode => "在终端模式中启用斜杠菜单",
        I18nKey::FeaturesOutlineCodebaseSymbolsContextMenu => {
            "为 '@' 上下文菜单提供代码库符号大纲"
        }
        I18nKey::FeaturesShowTerminalInputMessageLine => "显示终端输入消息行",
        I18nKey::FeaturesShowAutosuggestionKeybindingHint => "显示自动建议快捷键提示",
        I18nKey::FeaturesShowAutosuggestionIgnoreButton => "显示忽略自动建议按钮",
        I18nKey::FeaturesArrowAcceptsAutosuggestions => "-> 接受自动建议。",
        I18nKey::FeaturesKeyAcceptsAutosuggestions => "{key} 接受自动建议。",
        I18nKey::FeaturesCompletionsOpenTyping => "输入时会打开补全。",
        I18nKey::FeaturesCompletionsOpenTypingOrKey => "输入时会打开补全（或按 {key}）。",
        I18nKey::FeaturesCompletionMenuUnbound => "打开补全菜单未绑定快捷键。",
        I18nKey::FeaturesKeyOpensCompletionMenu => "{key} 打开补全菜单。",
        I18nKey::FeaturesTabKeyBehavior => "Tab 键行为",
        I18nKey::FeaturesCtrlTabBehaviorLabel => "Ctrl+Tab 行为：",
        I18nKey::FeaturesEnableMouseReporting => "启用鼠标报告",
        I18nKey::FeaturesEnableScrollReporting => "启用滚动报告",
        I18nKey::FeaturesEnableFocusReporting => "启用焦点报告",
        I18nKey::FeaturesUseAudibleBell => "使用响铃提示",
        I18nKey::FeaturesWordCharactersLabel => "视为单词一部分的字符",
        I18nKey::FeaturesDoubleClickSmartSelection => "双击智能选择",
        I18nKey::FeaturesShowHelpBlockNewSessions => "在新会话中显示帮助块",
        I18nKey::FeaturesCopyOnSelect => "选中即复制",
        I18nKey::FeaturesNewTabPlacement => "新标签页位置",
        I18nKey::FeaturesDefaultModeForNewSessions => "新会话默认模式",
        I18nKey::FeaturesShowGlobalWorkflowsCommandSearch => {
            "在命令搜索中显示全局工作流（ctrl-r）"
        }
        I18nKey::FeaturesHonorLinuxSelectionClipboard => "遵循 Linux 选择剪贴板",
        I18nKey::FeaturesLinuxSelectionClipboardTooltip => "是否支持 Linux primary 剪贴板。",
        I18nKey::FeaturesPreferIntegratedGpu => "优先使用集成 GPU 渲染新窗口（低功耗）",
        I18nKey::FeaturesUseWaylandWindowManagement => "使用 Wayland 进行窗口管理",
        I18nKey::FeaturesWaylandTooltip => "启用 Wayland",
        I18nKey::FeaturesWaylandSecondaryText => {
            "启用此设置会禁用全局快捷键支持。禁用时，如果你的 Wayland compositor 使用分数缩放（例如 125%），文本可能会模糊。"
        }
        I18nKey::FeaturesWaylandRestart => "重启 Warp 以应用更改。",
        I18nKey::FeaturesPreferredGraphicsBackend => "首选图形后端",
        I18nKey::AiWarpAgentTitle => "Warp 智能体",
        I18nKey::AiRemoteSessionOrgPolicy => {
            "当活动窗格包含远程会话内容时，你的组织不允许使用 AI"
        }
        I18nKey::AiSignUpPrompt => "要使用 AI 功能，请先创建账户。",
        I18nKey::AiUsage => "用量",
        I18nKey::AiResetsDate => "{date} 重置",
        I18nKey::AiRestrictedBilling => "因账单问题受限",
        I18nKey::AiUnlimited => "无限制",
        I18nKey::AiUsageLimitDescription => "这是你账户 AI 点数的 {duration} 限额。",
        I18nKey::AiCredits => "点数",
        I18nKey::AiUpgrade => "升级",
        I18nKey::AiComparePlans => "比较套餐",
        I18nKey::AiContactSupport => "联系支持",
        I18nKey::AiUpgradeMoreUsage => "以获得更多 AI 用量。",
        I18nKey::AiForMoreUsage => "以获得更多 AI 用量。",
        I18nKey::AiActiveAi => "主动 AI",
        I18nKey::AiNextCommand => "下一条命令",
        I18nKey::AiNextCommandDescription => {
            "让 AI 根据你的命令历史、输出和常见工作流建议下一条要运行的命令。"
        }
        I18nKey::AiPromptSuggestions => "提示词建议",
        I18nKey::AiPromptSuggestionsDescription => {
            "让 AI 根据最近的命令及其输出，在输入框中以内联横幅形式建议自然语言提示词。"
        }
        I18nKey::AiSuggestedCodeBanners => "代码建议横幅",
        I18nKey::AiSuggestedCodeBannersDescription => {
            "让 AI 根据最近的命令及其输出，在块列表中以内联横幅形式建议代码 diff 和查询。"
        }
        I18nKey::AiNaturalLanguageAutosuggestions => "自然语言自动建议",
        I18nKey::AiNaturalLanguageAutosuggestionsDescription => {
            "让 AI 根据最近的命令及其输出建议自然语言自动补全。"
        }
        I18nKey::AiSharedBlockTitleGeneration => "共享块标题生成",
        I18nKey::AiSharedBlockTitleGenerationDescription => {
            "让 AI 根据命令和输出为你的共享块生成标题。"
        }
        I18nKey::AiCommitPrGeneration => "提交与 Pull Request 生成",
        I18nKey::AiCommitPrGenerationDescription => {
            "让 AI 生成提交消息、Pull Request 标题和描述。"
        }
        I18nKey::AiAgents => "智能体",
        I18nKey::AiAgentsDescription => {
            "设置智能体的运行边界。选择它可以访问什么、有多少自主权，以及何时必须请求你的批准。你也可以微调自然语言输入、代码库感知等行为。"
        }
        I18nKey::AiProfiles => "配置档案",
        I18nKey::AiProfilesDescription => {
            "配置档案用于定义智能体如何运行，包括可执行的操作、何时需要批准，以及用于编码和规划等任务的模型。你也可以将它们限定到单个项目。"
        }
        I18nKey::AiModels => "模型",
        I18nKey::AiContextWindowTokens => "上下文窗口（tokens）",
        I18nKey::AiPermissions => "权限",
        I18nKey::AiApplyCodeDiffs => "应用代码 diff",
        I18nKey::AiReadFiles => "读取文件",
        I18nKey::AiExecuteCommands => "执行命令",
        I18nKey::AiPermissionsManagedByWorkspace => "你的部分权限由工作区管理。",
        I18nKey::AiInteractWithRunningCommands => "与正在运行的命令交互",
        I18nKey::AiCommandDenylist => "命令拒绝列表",
        I18nKey::AiCommandDenylistDescription => {
            "匹配智能体始终应请求执行权限的命令正则表达式。"
        }
        I18nKey::AiCommandAllowlist => "命令允许列表",
        I18nKey::AiCommandAllowlistDescription => {
            "匹配智能体可自动执行的命令正则表达式。"
        }
        I18nKey::AiDirectoryAllowlist => "目录允许列表",
        I18nKey::AiDirectoryAllowlistDescription => "授予智能体访问特定目录中文件的权限。",
        I18nKey::AiShowModelPickerInPrompt => "在提示词中显示模型选择器",
        I18nKey::AiBaseModel => "基础模型",
        I18nKey::AiBaseModelDescription => {
            "此模型是 Warp 智能体背后的主要引擎。它负责大多数交互，并在需要时调用其他模型执行规划或代码生成等任务。Warp 可能会根据模型可用性，或为对话摘要等辅助任务自动切换到其他模型。"
        }
        I18nKey::AiCodebaseContext => "代码库上下文",
        I18nKey::AiCodebaseContextDescription => {
            "允许 Warp 智能体生成可用作上下文的代码库大纲。代码绝不会存储在我们的服务器上。"
        }
        I18nKey::AiLearnMore => "了解更多",
        I18nKey::AiCallMcpServers => "调用 MCP 服务器",
        I18nKey::AiMcpZeroStatePrefix => {
            "你还没有添加任何 MCP 服务器。添加后，你可以控制 Warp 智能体与它们交互时的自主程度。"
        }
        I18nKey::AiAddServer => "添加服务器",
        I18nKey::AiMcpZeroStateMiddle => " 或 ",
        I18nKey::AiMcpZeroStateLearnMore => "了解 MCP。",
        I18nKey::AiMcpAllowlist => "MCP 允许列表",
        I18nKey::AiMcpAllowlistDescription => "允许 Warp 智能体调用这些 MCP 服务器。",
        I18nKey::AiMcpDenylist => "MCP 拒绝列表",
        I18nKey::AiMcpDenylistDescription => {
            "调用此列表中的任何 MCP 服务器前，Warp 智能体都会请求权限。"
        }
        I18nKey::AiInput => "输入",
        I18nKey::AiShowInputHintText => "显示输入提示文本",
        I18nKey::AiShowAgentTips => "显示智能体提示",
        I18nKey::AiIncludeAgentCommandsHistory => "将智能体执行的命令加入历史记录",
        I18nKey::AiIncorrectDetectionPrefix => "遇到错误检测？",
        I18nKey::AiLetUsKnow => "告诉我们",
        I18nKey::AiAutodetectAgentPrompts => "在终端输入中自动检测智能体提示词",
        I18nKey::AiAutodetectTerminalCommands => "在智能体输入中自动检测终端命令",
        I18nKey::AiNaturalLanguageDetectionDescription => {
            "启用自然语言检测后，当终端输入中写入自然语言时会自动检测，并切换到智能体模式处理 AI 查询。"
        }
        I18nKey::AiIncorrectInputDetectionPrefix => " 遇到错误输入检测？",
        I18nKey::AiNaturalLanguageDetection => "自然语言检测",
        I18nKey::AiNaturalLanguageDenylist => "自然语言拒绝列表",
        I18nKey::AiNaturalLanguageDenylistDescription => {
            "此处列出的命令永远不会触发自然语言检测。"
        }
        I18nKey::AiMcpServers => "MCP 服务器",
        I18nKey::AiMcpServersDescription => {
            "添加 MCP 服务器来扩展 Warp 智能体的能力。MCP 服务器通过标准化接口向智能体暴露数据源或工具，本质上类似插件。"
        }
        I18nKey::AiAutoSpawnThirdPartyServers => "从第三方智能体自动启动服务器",
        I18nKey::AiFileBasedMcpDescription => {
            "自动检测并启动全局范围第三方 AI 智能体配置文件中的 MCP 服务器（例如主目录中的配置）。仓库内部检测到的服务器绝不会自动启动，必须在 MCP 设置页中单独启用。"
        }
        I18nKey::AiSeeSupportedProviders => "查看支持的提供方。",
        I18nKey::AiManageMcpServers => "管理 MCP 服务器",
        I18nKey::AiRules => "规则",
        I18nKey::AiRulesDescription => {
            "规则可帮助 Warp 智能体遵循你的约定，无论是针对代码库还是特定工作流。"
        }
        I18nKey::AiSuggestedRules => "建议规则",
        I18nKey::AiSuggestedRulesDescription => "让 AI 根据你的交互建议可保存的规则。",
        I18nKey::AiWarpDriveAgentContext => "将 Warp Drive 用作智能体上下文",
        I18nKey::AiWarpDriveAgentContextDescription => {
            "Warp 智能体可以利用你的 Warp Drive 内容，为个人和团队开发工作流与环境定制回答。这包括任何工作流、笔记本和环境变量。"
        }
        I18nKey::AiKnowledge => "知识库",
        I18nKey::AiManageRules => "管理规则",
        I18nKey::AiVoiceInput => "语音输入",
        I18nKey::AiVoiceInputDescriptionPrefix => "语音输入允许你直接对终端说话来控制 Warp（由 ",
        I18nKey::AiVoiceInputDescriptionSuffix => " 提供支持）。",
        I18nKey::AiKeyForActivatingVoiceInput => "激活语音输入的按键",
        I18nKey::AiPressAndHoldToActivate => "按住以激活。",
        I18nKey::AiVoice => "语音",
        I18nKey::AiOther => "其他",
        I18nKey::AiShowOzChangelog => "在新对话视图中显示 Oz 变更日志",
        I18nKey::AiShowUseAgentFooter => "显示 \"Use Agent\" 页脚",
        I18nKey::AiUseAgentFooterDescription => {
            "在长时间运行的命令中提示使用已启用 \"Full Terminal Use\" 的智能体。"
        }
        I18nKey::AiShowConversationHistory => "在工具面板中显示对话历史",
        I18nKey::AiAgentThinkingDisplay => "智能体思考显示",
        I18nKey::AiAgentThinkingDisplayDescription => "控制推理/思考轨迹的显示方式。",
        I18nKey::AiPreferredConversationLayout => "打开现有智能体对话时的首选布局",
        I18nKey::AiThirdPartyCliAgents => "第三方 CLI 智能体",
        I18nKey::AiShowCodingAgentToolbar => "显示编码智能体工具栏",
        I18nKey::AiCodingAgentToolbarDescriptionPrefix => "运行编码智能体时显示带快捷操作的工具栏，例如 ",
        I18nKey::AiCodingAgentToolbarDescriptionMiddle => " 或 ",
        I18nKey::AiCodingAgentToolbarDescriptionSuffix => "。",
        I18nKey::AiAutoShowHideRichInput => "根据智能体状态自动显示/隐藏 Rich Input",
        I18nKey::AiRequiresWarpPlugin => "需要你的编码智能体安装 Warp 插件",
        I18nKey::AiAutoOpenRichInput => "编码智能体会话开始时自动打开 Rich Input",
        I18nKey::AiAutoDismissRichInput => "提交提示词后自动关闭 Rich Input",
        I18nKey::AiCommandsEnableToolbar => "启用工具栏的命令",
        I18nKey::AiToolbarCommandDescription => {
            "添加正则表达式模式，以便对匹配命令显示编码智能体工具栏。"
        }
        I18nKey::AiOrgEnforcedTooltip => "此选项由你的组织设置强制执行，无法自定义。",
        I18nKey::AiEnableAgentAttribution => "启用智能体署名",
        I18nKey::AiAgentAttribution => "智能体署名",
        I18nKey::AiAgentAttributionDescription => {
            "Oz 可以在它创建的提交消息和 Pull Request 中添加署名"
        }
        I18nKey::AiComputerUseCloudAgents => "云智能体中的计算机使用",
        I18nKey::AiExperimental => "实验性",
        I18nKey::AiCloudComputerUseDescription => {
            "为从 Warp 应用启动的云智能体对话启用计算机使用。"
        }
        I18nKey::AiOrchestration => "编排",
        I18nKey::AiOrchestrationDescription => {
            "启用多智能体编排，允许智能体生成并协调并行子智能体。"
        }
        I18nKey::AiApiKeys => "API Key",
        I18nKey::AiByokDescription => {
            "使用模型提供商的自有 API Key 供 Warp 智能体调用。API Key 仅存储在本地，绝不会同步到云端。使用自动模型，或使用未提供 API Key 的提供商模型时，将消耗 Warp 点数。"
        }
        I18nKey::AiOpenAiApiKey => "OpenAI API Key",
        I18nKey::AiAnthropicApiKey => "Anthropic API Key",
        I18nKey::AiGoogleApiKey => "Google API Key",
        I18nKey::AiContactSales => "联系销售",
        I18nKey::AiContactSalesByok => "以在你的 Enterprise 套餐中启用自带 API Key。",
        I18nKey::AiUpgradeBuildPlan => "升级到 Build 套餐",
        I18nKey::AiUseOwnApiKeys => "以使用你自己的 API Key。",
        I18nKey::AiAskAdminUpgradeBuild => {
            "请让团队管理员升级到 Build 套餐，以使用你自己的 API Key。"
        }
        I18nKey::AiWarpCreditFallback => "Warp 点数兜底",
        I18nKey::AiWarpCreditFallbackDescription => {
            "启用后，当发生错误时，智能体请求可能会路由到 Warp 提供的模型之一。Warp 会优先使用你的 API Key，而不是 Warp 点数。"
        }
        I18nKey::AiAwsBedrock => "AWS Bedrock",
        I18nKey::AiAwsManagedDescription => {
            "Warp 会加载并发送本地 AWS CLI 凭证，用于 Bedrock 支持的模型。此设置由你的组织管理。"
        }
        I18nKey::AiAwsDescription => {
            "Warp 会加载并发送本地 AWS CLI 凭证，用于 Bedrock 支持的模型。"
        }
        I18nKey::AiUseAwsBedrockCredentials => "使用 AWS Bedrock 凭证",
        I18nKey::AiLoginCommand => "登录命令",
        I18nKey::AiAwsProfile => "AWS Profile",
        I18nKey::AiAutoRunLoginCommand => "自动运行登录命令",
        I18nKey::AiAutoRunLoginDescription => {
            "启用后，当 AWS Bedrock 凭证过期时会自动运行登录命令。"
        }
        I18nKey::AiRefresh => "刷新",
        I18nKey::AiAddProfile => "添加配置档案",
        I18nKey::AiEdit => "编辑",
        I18nKey::AiAuto => "自动",
        I18nKey::AiModelsUpper => "模型",
        I18nKey::AiPermissionsUpper => "权限",
        I18nKey::AiBaseModelColon => "基础模型：",
        I18nKey::AiFullTerminalUseColon => "完整终端使用：",
        I18nKey::AiComputerUseColon => "计算机使用：",
        I18nKey::AiApplyCodeDiffsColon => "应用代码 diff：",
        I18nKey::AiReadFilesColon => "读取文件：",
        I18nKey::AiExecuteCommandsColon => "执行命令：",
        I18nKey::AiInteractWithRunningCommandsColon => "与正在运行的命令交互：",
        I18nKey::AiAskQuestionsColon => "提问：",
        I18nKey::AiRunAgentsColon => "运行智能体：",
        I18nKey::AiCallMcpServersColon => "调用 MCP 服务器：",
        I18nKey::AiCallWebToolsColon => "调用 Web 工具：",
        I18nKey::AiAutoSyncPlansColon => "自动同步计划到 Warp Drive：",
        I18nKey::AiNone => "无",
        I18nKey::AiAgentDecides => "智能体决定",
        I18nKey::AiAlwaysAllow => "始终允许",
        I18nKey::AiAlwaysAsk => "始终询问",
        I18nKey::AiUnknown => "未知",
        I18nKey::AiAskOnFirstWrite => "首次写入时询问",
        I18nKey::AiNever => "从不",
        I18nKey::AiNeverAsk => "从不询问",
        I18nKey::AiAskUnlessAutoApprove => "除非自动批准，否则询问",
        I18nKey::AiOn => "开",
        I18nKey::AiOff => "关",
        I18nKey::AiDirectoryAllowlistColon => "目录允许列表：",
        I18nKey::AiCommandAllowlistColon => "命令允许列表：",
        I18nKey::AiCommandDenylistColon => "命令拒绝列表：",
        I18nKey::AiMcpAllowlistColon => "MCP 允许列表：",
        I18nKey::AiMcpDenylistColon => "MCP 拒绝列表：",
        I18nKey::AiSelectMcpServers => "选择 MCP 服务器",
        I18nKey::AiReadOnly => "只读",
        I18nKey::AiSupervised => "受监督",
        I18nKey::AiAllowSpecificDirectories => "允许特定目录",
        I18nKey::AiDefault => "默认",
        I18nKey::AiDisabled => "已禁用",
        I18nKey::AiProfileDefault => "配置档案默认值",
        I18nKey::AiFreePlanFrontierModelsUnavailable => "免费套餐不可用 Frontier 模型。",
        I18nKey::AiProfileEditorTitle => "编辑配置档案",
        I18nKey::AiProfileEditorName => "名称",
        I18nKey::AiDefaultProfileNameReadonly => "默认配置档案名称无法更改。",
        I18nKey::AiDeleteProfile => "删除配置档案",
        I18nKey::AiDirectoryPathPlaceholder => "例如 ~/code-repos/repo",
        I18nKey::AiCommandAllowPlaceholder => "例如 ls .*",
        I18nKey::AiCommandDenyPlaceholder => "例如 rm .*",
        I18nKey::AiCommandsCommaSeparatedPlaceholder => "命令，用逗号分隔",
        I18nKey::AiCommandRegexPlaceholder => "命令（支持正则表达式）",
        I18nKey::AiProfileNamePlaceholder => "例如 \"YOLO code\"",
        I18nKey::AiFullTerminalUseModel => "完整终端使用模型",
        I18nKey::AiFullTerminalUseModelDescription => {
            "智能体在数据库 shell、调试器、REPL 或开发服务器等交互式终端应用中运行时使用的模型，用于读取实时输出并向 PTY 写入命令。"
        }
        I18nKey::AiComputerUse => "计算机使用",
        I18nKey::AiComputerUseModel => "计算机使用模型",
        I18nKey::AiComputerUseModelDescription => {
            "智能体控制你的计算机，通过鼠标移动、点击和键盘输入与图形应用交互时使用的模型。"
        }
        I18nKey::AiContextWindow => "上下文窗口",
        I18nKey::AiContextWindowDescription => {
            "基础模型的工作记忆，即一次可考虑的对话、代码和文档 token 数。更大的窗口支持更长对话，并能在更大的代码库上保持更连贯的回答，但会增加延迟和计算成本。"
        }
        I18nKey::AiPermissionAgentDecidesDescription => {
            "智能体会选择最安全的路径：有把握时自主执行，不确定时请求批准。"
        }
        I18nKey::AiPermissionAlwaysAllowDescription => "给予智能体完全自主权，无需手动批准。",
        I18nKey::AiPermissionAlwaysAskDescription => "智能体执行任何操作前都需要明确批准。",
        I18nKey::AiWriteToPtyAskOnFirstWriteDescription => {
            "智能体首次需要与正在运行的命令交互时会请求权限。之后在该命令剩余期间会自动继续。"
        }
        I18nKey::AiWriteToPtyAlwaysAskDescription => {
            "智能体每次与正在运行的命令交互前都会请求权限。"
        }
        I18nKey::AiComputerUseNeverDescription => "计算机使用工具已禁用，智能体无法使用。",
        I18nKey::AiComputerUseAlwaysAskDescription => {
            "智能体使用计算机使用工具前需要明确批准。"
        }
        I18nKey::AiComputerUseAlwaysAllowDescription => {
            "允许智能体无需批准即可自主使用计算机使用工具。"
        }
        I18nKey::AiUnknownSettingDescription => "未知设置。",
        I18nKey::AiAskQuestions => "提问",
        I18nKey::AiRunOrchestratedAgents => "运行编排智能体",
        I18nKey::AiRunAgentsNeverDescription => {
            "智能体无法运行子智能体，run_agents 工具也不可用。"
        }
        I18nKey::AiRunAgentsAlwaysAllowDescription => {
            "允许智能体无需批准即可自主运行子智能体。"
        }
        I18nKey::AiRunAgentsAlwaysAskDescription => {
            "智能体运行子智能体前需要明确批准。"
        }
        I18nKey::AiAskQuestionAskUnlessAutoApproveDescription => {
            "智能体可以提问并等待你的回答；但在自动批准开启时会自动继续。"
        }
        I18nKey::AiAskQuestionNeverDescription => {
            "智能体不会提问，会根据自己的最佳判断继续。"
        }
        I18nKey::AiAskQuestionAlwaysAskDescription => {
            "智能体可以提问并等待你的回答，即使自动批准开启也会暂停。"
        }
        I18nKey::AiMcpAllowlistProfileDescription => "允许 Oz 调用的 MCP 服务器。",
        I18nKey::AiMcpDenylistProfileDescription => "不允许 Oz 调用的 MCP 服务器。",
        I18nKey::AiPlanAutoSync => "计划自动同步",
        I18nKey::AiPlanAutoSyncDescription => {
            "此智能体创建的计划会自动添加并同步到 Warp Drive。"
        }
        I18nKey::AiCallWebTools => "调用 Web 工具",
        I18nKey::AiCallWebToolsDescription => "智能体可在有助于完成任务时使用 Web 搜索。",
        I18nKey::AiToolbarLayout => "工具栏布局",
        I18nKey::AiSelectCodingAgent => "选择编码智能体",
        I18nKey::AiShowAndCollapse => "显示并折叠",
        I18nKey::AiAlwaysShow => "始终显示",
        I18nKey::AiNeverShow => "从不显示",
        I18nKey::AiLeft => "左",
        I18nKey::AiRight => "右",
        I18nKey::AppearanceCategoryLanguage => "语言",
        I18nKey::AppearanceCategoryThemes => "主题",
        I18nKey::AppearanceCategoryIcon => "图标",
        I18nKey::AppearanceCategoryWindow => "窗口",
        I18nKey::AppearanceCategoryInput => "输入",
        I18nKey::AppearanceCategoryPanes => "窗格",
        I18nKey::AppearanceCategoryBlocks => "块",
        I18nKey::AppearanceCategoryText => "文本",
        I18nKey::AppearanceCategoryCursor => "光标",
        I18nKey::AppearanceCategoryTabs => "标签页",
        I18nKey::AppearanceCategoryFullscreenApps => "全屏应用",
        I18nKey::AppearanceLanguageLabel => "显示语言",
        I18nKey::AppearanceLanguageDescription => "选择 Warp 界面使用的语言。",
        I18nKey::AppearanceThemeCreateCustom => "创建自定义主题",
        I18nKey::AppearanceThemeLight => "浅色",
        I18nKey::AppearanceThemeDark => "深色",
        I18nKey::AppearanceThemeCurrent => "当前主题",
        I18nKey::AppearanceThemeSyncWithOs => "跟随系统",
        I18nKey::AppearanceThemeSyncWithOsDescription => {
            "系统浅色/深色外观切换时自动切换主题。"
        }
        I18nKey::AppearanceAppIconLabel => "自定义应用图标",
        I18nKey::AppearanceAppIconDefault => "默认",
        I18nKey::AppearanceAppIconBundleWarning => "更改应用图标需要应用以 bundle 形式运行。",
        I18nKey::AppearanceAppIconRestartWarning => {
            "可能需要重启 Warp，macOS 才会应用首选图标样式。"
        }
        I18nKey::AppearanceWindowCustomSizeLabel => "以自定义尺寸打开新窗口",
        I18nKey::AppearanceWindowColumns => "列",
        I18nKey::AppearanceWindowRows => "行",
        I18nKey::AppearanceWindowOpacityLabel => "窗口不透明度",
        I18nKey::AppearanceWindowTransparencyUnsupported => "当前图形驱动不支持透明效果。",
        I18nKey::AppearanceWindowTransparencyWarning => "当前图形设置可能不支持透明窗口渲染。",
        I18nKey::AppearanceWindowTransparencySettingsSuggestion => {
            " 请在 功能 > 系统 中尝试更改图形后端或集成 GPU 设置。"
        }
        I18nKey::AppearanceWindowBlurRadiusLabel => "窗口模糊半径",
        I18nKey::AppearanceWindowBlurTextureLabel => "使用窗口模糊（亚克力纹理）",
        I18nKey::AppearanceWindowHardwareTransparencyWarning => {
            "当前硬件可能不支持透明窗口渲染。"
        }
        I18nKey::AppearanceToolsPanelConsistentAcrossTabs => {
            "工具面板在不同标签页中保持一致"
        }
        I18nKey::AppearanceInputTypeLabel => "输入类型",
        I18nKey::AppearanceInputRadioWarp => "Warp",
        I18nKey::AppearanceInputRadioShellPs1 => "Shell (PS1)",
        I18nKey::AppearanceInputPositionLabel => "输入位置",
        I18nKey::AppearancePanesDimInactive => "淡化非活动窗格",
        I18nKey::AppearancePanesFocusFollowsMouse => "焦点跟随鼠标",
        I18nKey::AppearanceBlocksCompactMode => "紧凑模式",
        I18nKey::AppearanceBlocksJumpToBottomButton => "显示跳转到块底部按钮",
        I18nKey::AppearanceBlocksShowDividers => "显示块分隔线",
        I18nKey::AppearanceTextAgentFont => "智能体字体",
        I18nKey::AppearanceTextDefaultSuffix => "默认",
        I18nKey::AppearanceTextMatchTerminal => "匹配终端",
        I18nKey::AppearanceTextLineHeight => "行高",
        I18nKey::AppearanceTextResetToDefault => "重置为默认值",
        I18nKey::AppearanceTextTerminalFont => "终端字体",
        I18nKey::AppearanceTextViewAllFonts => "查看所有可用系统字体",
        I18nKey::AppearanceTextFontWeight => "字体粗细",
        I18nKey::AppearanceTextFontSizePx => "字号（px）",
        I18nKey::AppearanceTextNotebookFontSize => "笔记本字号",
        I18nKey::AppearanceTextUseThinStrokes => "使用细笔画",
        I18nKey::AppearanceTextMinimumContrast => "强制最小对比度",
        I18nKey::AppearanceTextShowLigatures => "在终端中显示连字",
        I18nKey::AppearanceTextLigaturesTooltip => "连字可能降低性能",
        I18nKey::AppearanceCursorType => "光标类型",
        I18nKey::AppearanceCursorTypeDisabledVim => "Vim 模式下光标类型已禁用",
        I18nKey::AppearanceCursorBlinking => "闪烁光标",
        I18nKey::AppearanceCursorBar => "竖线",
        I18nKey::AppearanceCursorBlock => "方块",
        I18nKey::AppearanceCursorUnderline => "下划线",
        I18nKey::AppearanceTabsCloseButtonPosition => "标签页关闭按钮位置",
        I18nKey::AppearanceTabsShowIndicators => "显示标签页指示器",
        I18nKey::AppearanceTabsShowCodeReviewButton => "显示代码审查按钮",
        I18nKey::AppearanceTabsPreserveActiveColor => "新标签页保留当前活动标签页颜色",
        I18nKey::AppearanceTabsUseVerticalLayout => "使用垂直标签页布局",
        I18nKey::AppearanceTabsShowVerticalPanelRestored => "恢复窗口时显示垂直标签页面板",
        I18nKey::AppearanceTabsShowVerticalPanelRestoredDescription => {
            "启用后，重新打开或恢复窗口时会显示垂直标签页面板，即使上次保存窗口时该面板处于关闭状态。"
        }
        I18nKey::AppearanceTabsUseLatestPromptTitle => {
            "使用最新用户提示作为标签页会话标题"
        }
        I18nKey::AppearanceTabsUseLatestPromptTitleDescription => {
            "在垂直标签页中，对 Oz 和第三方智能体会话显示最新用户提示，而不是生成的会话标题。"
        }
        I18nKey::AppearanceTabsHeaderToolbarLayout => "顶部工具栏布局",
        I18nKey::AppearanceTabsDirectoryColors => "目录标签页颜色",
        I18nKey::AppearanceTabsDirectoryColorsDescription => {
            "根据当前所在目录或仓库自动为标签页着色。"
        }
        I18nKey::AppearanceTabsDefaultNoColor => "默认（无颜色）",
        I18nKey::AppearanceTabsShowTabBar => "显示标签栏",
        I18nKey::AppearanceFullscreenUseCustomPadding => "在全屏应用中使用自定义内边距",
        I18nKey::AppearanceFullscreenUniformPaddingPx => "统一内边距（px）",
        I18nKey::AppearanceZoomLabel => "缩放",
        I18nKey::AppearanceZoomDescription => "调整所有窗口的默认缩放级别",
        I18nKey::AppearanceInputModePinnedBottom => "固定在底部（Warp 模式）",
        I18nKey::AppearanceInputModePinnedTop => "固定在顶部（反向模式）",
        I18nKey::AppearanceInputModeWaterfall => "从顶部开始（经典模式）",
        I18nKey::AppearanceOptionNever => "从不",
        I18nKey::AppearanceOptionLowDpiDisplays => "低 DPI 显示器",
        I18nKey::AppearanceOptionHighDpiDisplays => "高 DPI 显示器",
        I18nKey::AppearanceOptionAlways => "始终",
        I18nKey::AppearanceContrastOnlyNamedColors => "仅命名颜色",
        I18nKey::AppearanceWorkspaceDecorationsAlways => "始终",
        I18nKey::AppearanceWorkspaceDecorationsWindowed => "窗口模式时",
        I18nKey::AppearanceWorkspaceDecorationsHover => "仅悬停时",
        I18nKey::AppearanceOptionRight => "右侧",
        I18nKey::AppearanceOptionLeft => "左侧",
        I18nKey::LanguagePreferenceSystem => "跟随系统语言",
        I18nKey::LanguagePreferenceEnglish => "English",
        I18nKey::LanguagePreferenceChineseSimplified => "简体中文",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn all_i18n_keys_have_non_empty_catalog_entries() {
        for key in I18nKey::iter() {
            assert!(
                !en_us(key).trim().is_empty(),
                "missing English entry for {key:?}"
            );
            assert!(
                !zh_cn(key).trim().is_empty(),
                "missing Simplified Chinese entry for {key:?}"
            );
        }
    }

    #[test]
    fn locale_from_identifier_detects_chinese_prefixes() {
        assert_eq!(locale_from_identifier("zh_CN.UTF-8"), Locale::ZhCn);
        assert_eq!(locale_from_identifier("zh-Hans"), Locale::ZhCn);
        assert_eq!(locale_from_identifier("en_US.UTF-8"), Locale::EnUs);
    }
}
