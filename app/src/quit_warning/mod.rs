use itertools::Itertools;
use settings::ToggleableSetting as _;
use std::fmt::Write;
use warpui::{
    modals::{AlertDialogWithCallbacks, AppModalCallback, ModalButton},
    AppContext, EntityId, SingletonEntity, ViewContext, WeakViewHandle, WindowId,
};

use crate::{
    code::editor_management::{CodeEditorStatus, CodeEditorSummary},
    pane_group::{CodePane, PaneGroup, PaneId, TerminalPane},
    report_if_error, send_telemetry_from_app_ctx,
    server::telemetry::CloseTarget,
    session_management::{RunningSessionSummary, SessionNavigationData},
    terminal::general_settings::GeneralSettings,
    workspace::Workspace,
    TelemetryEvent,
};

/// Scope of what's being quit/closed.
#[derive(Clone)]
enum QuitScope<'a> {
    Pane {
        pane_group: &'a PaneGroup,
        pane_group_id: EntityId,
        window_id: WindowId,
        pane_id: PaneId,
    },
    Tabs(Vec<WeakViewHandle<PaneGroup>>),
    Window(WindowId),
    App,
    #[allow(dead_code)]
    EditorTab {
        file_name: Option<String>,
        editor_status: Vec<CodeEditorStatus>,
    }, // TODO: Include the "log out" confirmation modal too.
}

/// Summary of unsaved data and running processes to show the user before they quit.
pub struct UnsavedStateSummary<'a> {
    scope: QuitScope<'a>,

    /// Total number of long-running commands.
    pub total_long_running_commands: usize,
    /// Number of windows with long-running commands.
    windows_with_long_running_commands: usize,
    /// Number of tabs with long-running commands.
    tabs_with_long_running_commands: usize,

    /// All terminal sessions in this scope.
    terminal_sessions: Vec<SessionNavigationData>,

    /// The number of live shared sessions.
    pub shared_sessions: usize,
    /// Whether or not there are unsaved code changes.
    unsaved_code_changes: bool,
}

/// Builder for a warning dialog that displays unsaved state.
pub struct QuitWarningDialog<'a> {
    state: &'a UnsavedStateSummary<'a>,

    on_confirm: Option<AppModalCallback>,
    on_cancel: Option<AppModalCallback>,
    on_show_processes: Option<AppModalCallback>,
    on_save_changes: Option<AppModalCallback>,
    on_discard_changes: Option<AppModalCallback>,
}

impl QuitScope<'_> {
    /// All sessions in this scope.
    fn sessions(&self, ctx: &AppContext) -> Vec<SessionNavigationData> {
        match self {
            Self::Pane {
                pane_group,
                pane_id,
                pane_group_id,
                window_id,
            } => pane_group
                .downcast_pane_by_id::<TerminalPane>(*pane_id)
                .map(|pane| pane.session_navigation_data(*pane_group_id, *window_id, ctx))
                .into_iter()
                .collect_vec(),
            Self::Tabs(ref tabs) => {
                // We can't use SessionNavigationData::all_sessions here, as the caller is likely
                // updating the tab's Workspace. This temporarily removes it from the app context,
                // so it's not visible to all_sessions.
                tabs.iter()
                    .filter_map(|tab| tab.upgrade(ctx))
                    .flat_map(|pane_group| {
                        pane_group.as_ref(ctx).pane_sessions(
                            pane_group.id(),
                            pane_group.window_id(ctx),
                            ctx,
                        )
                    })
                    .collect_vec()
            }
            Self::Window(window_id) => SessionNavigationData::all_sessions(ctx)
                .filter(|session| session.window_id() == *window_id)
                .collect_vec(),
            Self::App => SessionNavigationData::all_sessions(ctx).collect_vec(),
            Self::EditorTab { .. } => Vec::new(),
        }
    }

    /// All code editors in this scope.
    fn code_editors(&self, ctx: &AppContext) -> Vec<CodeEditorStatus> {
        match self {
            Self::Pane {
                pane_group,
                pane_id,
                ..
            } => pane_group
                .downcast_pane_by_id::<CodePane>(*pane_id)
                .map(|code_pane| code_pane.editor_status(ctx))
                .into_iter()
                .collect(),
            Self::Tabs(ref tabs) => tabs
                .iter()
                .filter_map(|tab| tab.upgrade(ctx))
                .flat_map(|pane_group| CodeEditorStatus::editors_in_tab(&pane_group, ctx))
                .collect_vec(),
            Self::Window(window_id) => {
                CodeEditorStatus::editors_in_window(*window_id, ctx).collect_vec()
            }
            Self::App => CodeEditorStatus::all_editors(ctx).collect_vec(),
            Self::EditorTab { editor_status, .. } => editor_status.clone(),
        }
    }

    /// All code review views in this scope (from the panel, not panes).
    fn code_review_views(&self, ctx: &AppContext) -> Vec<CodeEditorStatus> {
        match self {
            Self::Pane { .. } => {
                vec![] // There cannot be a code review view in a pane.
            }
            Self::Tabs(ref tabs) => {
                let window_ids: Vec<_> = tabs
                    .iter()
                    .filter_map(|tab| tab.upgrade(ctx))
                    .map(|pane_group| pane_group.window_id(ctx))
                    .unique()
                    .collect();
                window_ids
                    .into_iter()
                    .flat_map(|window_id| {
                        CodeEditorStatus::code_review_views_in_window(window_id, ctx)
                    })
                    .collect_vec()
            }
            Self::Window(window_id) => {
                CodeEditorStatus::code_review_views_in_window(*window_id, ctx).collect_vec()
            }
            Self::App => ctx
                .window_ids()
                .flat_map(|window_id| CodeEditorStatus::code_review_views_in_window(window_id, ctx))
                .collect_vec(),
            Self::EditorTab { .. } => vec![],
        }
    }

    /// Count of shared sessions in this scope.
    fn shared_sessions(&self, ctx: &AppContext) -> usize {
        match self {
            Self::Pane {
                pane_group,
                pane_id,
                ..
            } => pane_group
                .terminal_view_from_pane_id(*pane_id, ctx)
                .filter(|view| view.as_ref(ctx).is_sharing_session())
                .into_iter()
                .count(),
            Self::Tabs(ref tabs) => tabs
                .iter()
                .filter_map(|tab| tab.upgrade(ctx))
                .map(|tab| tab.as_ref(ctx).number_of_shared_sessions(ctx))
                .sum(),
            Self::Window(window_id) => ctx
                .views_of_type::<PaneGroup>(*window_id)
                .map(|views| {
                    views
                        .into_iter()
                        .map(|view| view.as_ref(ctx).number_of_shared_sessions(ctx))
                        .sum()
                })
                .unwrap_or_default(),
            Self::App => crate::session_management::num_shared_sessions(ctx),
            Self::EditorTab { .. } => 0,
        }
    }

    fn close_target(&self) -> CloseTarget {
        match self {
            Self::Pane { .. } => CloseTarget::Pane,
            Self::Tabs(_) => CloseTarget::Tab,
            Self::Window(_) => CloseTarget::Window,
            Self::App => CloseTarget::App,
            Self::EditorTab { .. } => CloseTarget::EditorTab,
        }
    }
}

impl UnsavedStateSummary<'static> {
    pub fn for_app(ctx: &mut AppContext) -> Self {
        Self::for_scope(QuitScope::App, ctx)
    }

    pub fn for_window(window_id: WindowId, ctx: &mut AppContext) -> Self {
        Self::for_scope(QuitScope::Window(window_id), ctx)
    }

    pub fn for_tabs(tabs: Vec<WeakViewHandle<PaneGroup>>, ctx: &mut AppContext) -> Self {
        Self::for_scope(QuitScope::Tabs(tabs), ctx)
    }

    #[allow(dead_code)]
    pub fn for_editor_tab(
        file_name: Option<String>,
        editor_status: Vec<CodeEditorStatus>,
        ctx: &mut AppContext,
    ) -> Self {
        Self::for_scope(
            QuitScope::EditorTab {
                file_name,
                editor_status,
            },
            ctx,
        )
    }
}

impl<'a> UnsavedStateSummary<'a> {
    pub fn for_pane(
        pane_group: &'a PaneGroup,
        pane_id: PaneId,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Self {
        Self::for_scope(
            QuitScope::Pane {
                pane_group,
                pane_id,
                pane_group_id: ctx.view_id(),
                window_id: ctx.window_id(),
            },
            ctx,
        )
    }

    fn for_scope(scope: QuitScope<'a>, ctx: &mut AppContext) -> Self {
        let sessions = scope.sessions(ctx);
        let sessions_summary = RunningSessionSummary::new(&sessions);

        let code_editors = scope.code_editors(ctx);
        let code_editor_summary = CodeEditorSummary::new(&code_editors);

        let code_review_views = scope.code_review_views(ctx);
        let code_review_summary = CodeEditorSummary::new(&code_review_views);

        let num_shared_sessions = scope.shared_sessions(ctx);

        UnsavedStateSummary {
            scope,
            total_long_running_commands: sessions_summary.long_running_cmds.len(),
            windows_with_long_running_commands: sessions_summary.windows_running().len(),
            tabs_with_long_running_commands: sessions_summary.tabs_running().len(),
            terminal_sessions: sessions,
            shared_sessions: num_shared_sessions,
            unsaved_code_changes: !code_editor_summary.unsaved_changes.is_empty()
                || !code_review_summary.unsaved_changes.is_empty(),
        }
    }

    pub fn should_display_warning(&self, ctx: &AppContext) -> bool {
        *GeneralSettings::as_ref(ctx).show_warning_before_quitting
            && (self.total_long_running_commands > 0
                || self.shared_sessions > 0
                || self.unsaved_code_changes)
    }

    pub fn running_sessions(&self) -> RunningSessionSummary<'_> {
        RunningSessionSummary::new(&self.terminal_sessions)
    }

    /// Initializes a [`QuitWarningDialog`] with this summary of unsaved state.
    pub fn dialog(&self) -> QuitWarningDialog<'_> {
        QuitWarningDialog::new(self)
    }

    /// Builds warning text describing what unsaved data there is.
    pub fn warning_text(&self) -> String {
        let mut info_text_lines = Vec::<String>::new();

        let scope_suffix = match self.scope {
            QuitScope::Tabs(ref tabs) if tabs.len() == 1 => " in this tab.",
            QuitScope::Window(_) => " in this window.",
            QuitScope::Pane { .. } => " in this pane.",
            QuitScope::App | QuitScope::Tabs(_) | QuitScope::EditorTab { .. } => ".",
        };

        if self.total_long_running_commands > 0 {
            let mut process_info_text = format!(
                "You have {} {} running",
                self.total_long_running_commands,
                pluralize(self.total_long_running_commands, "process", "processes")
            );
            if self.windows_with_long_running_commands > 1 {
                let _ = write!(
                    &mut process_info_text,
                    " in {} windows",
                    self.windows_with_long_running_commands
                );
            } else if self.tabs_with_long_running_commands > 1 {
                let _ = write!(
                    &mut process_info_text,
                    " in {} tabs",
                    self.tabs_with_long_running_commands
                );
            }
            process_info_text.push_str(scope_suffix);
            info_text_lines.push(process_info_text);
        }

        if self.shared_sessions > 0 {
            info_text_lines.push(format!(
                "You are sharing {} {}{scope_suffix}",
                self.shared_sessions,
                pluralize(self.shared_sessions, "session", "sessions")
            ));
        }

        if self.unsaved_code_changes {
            if let QuitScope::EditorTab { ref file_name, .. } = self.scope {
                info_text_lines.push(format!("Do you want to save the changes you made to {}? Your changes will be discarded if you don't save them.", file_name.clone().unwrap_or("this file".to_string())));
            } else {
                info_text_lines.push(format!("You have unsaved file changes{scope_suffix}"));
            }
        }

        info_text_lines.join("\n")
    }
}

impl<'a> QuitWarningDialog<'a> {
    pub fn new(state: &'a UnsavedStateSummary) -> Self {
        Self {
            state,
            on_confirm: None,
            on_cancel: None,
            on_show_processes: None,
            on_save_changes: None,
            on_discard_changes: None,
        }
    }

    /// Set the callback to invoke if the user cancels quitting.
    pub fn on_cancel<F: FnOnce(&mut AppContext) + 'static>(mut self, f: F) -> Self {
        self.on_cancel = Some(Box::new(f));
        self
    }

    /// Set the callback to invoke if the user confirms quitting with unsaved state.
    pub fn on_confirm<F: FnOnce(&mut AppContext) + 'static>(mut self, f: F) -> Self {
        self.on_confirm = Some(Box::new(f));
        self
    }

    /// Set the callback to invoke if the user requests to see running processes.
    pub fn on_show_processes<F: FnOnce(&mut AppContext) + 'static>(mut self, f: F) -> Self {
        self.on_show_processes = Some(Box::new(f));
        self
    }

    #[allow(dead_code)]
    /// Set the callback to invoke if the user wants to save changes before exiting an editor tab.
    pub fn on_save_changes<F: FnOnce(&mut AppContext) + 'static>(mut self, f: F) -> Self {
        self.on_save_changes = Some(Box::new(f));
        self
    }

    #[allow(dead_code)]
    /// Set the callback to invoke if the user wants to discard changes before exiting an editor tab.
    pub fn on_discard_changes<F: FnOnce(&mut AppContext) + 'static>(mut self, f: F) -> Self {
        self.on_discard_changes = Some(Box::new(f));
        self
    }

    pub fn build(self) -> AlertDialogWithCallbacks<AppModalCallback> {
        let QuitWarningDialog {
            state,
            on_confirm,
            on_cancel,
            on_show_processes,
            on_save_changes,
            on_discard_changes,
        } = self;

        let mut buttons = Vec::new();

        if let Some(callback) = on_confirm {
            let confirm_title = match state.scope {
                QuitScope::Window(_) | QuitScope::Tabs(_) | QuitScope::Pane { .. } => "Yes, close",
                QuitScope::App => "Yes, quit",
                _ => "",
            };
            buttons.push(ModalButton::for_app(confirm_title.to_string(), callback));
        }

        if let Some(callback) = on_save_changes {
            buttons.push(ModalButton::for_app("Save".to_string(), callback));
        }

        if let Some(callback) = on_discard_changes {
            buttons.push(ModalButton::for_app("Don't Save".to_string(), callback));
        }

        if let Some(callback) = on_show_processes {
            if state.total_long_running_commands > 0 {
                buttons.push(ModalButton::for_app(
                    "Show running processes".to_string(),
                    move |app| {
                        callback(app);
                    },
                ))
            }
        }

        if let Some(callback) = on_cancel {
            buttons.push(ModalButton::for_app("Cancel".to_string(), callback));
        }

        let title = match &state.scope {
            QuitScope::Pane { .. } => "Close pane?",
            QuitScope::Tabs(tabs) if tabs.len() == 1 => "Close tab?",
            QuitScope::Tabs(_) => "Close tabs?",
            QuitScope::Window(_) => "Close window?",
            QuitScope::App => "Quit Warp?",
            QuitScope::EditorTab { .. } => "Save changes?",
        };

        AlertDialogWithCallbacks::for_app(
            title,
            state.warning_text(),
            buttons,
            on_disable_warning_modal,
        )
    }

    /// Show the quit warning dialog. This returns `true` if the dialog was shown, and `false` if
    /// the current platform doesn't support showing a modal.
    pub fn show(self, ctx: &mut AppContext) -> bool {
        send_telemetry_from_app_ctx!(
            TelemetryEvent::QuitModalShown {
                running_processes: self.state.total_long_running_commands as u32,
                shared_sessions: self.state.shared_sessions as u32,
                modal_for: self.state.scope.close_target()
            },
            ctx
        );

        let session_summary = self.state.running_sessions();
        let dialog = self.build();
        // We don't support showing a modal on all platforms.
        let mut shown = false;
        if cfg!(all(not(target_family = "wasm"), target_os = "macos")) {
            ctx.show_native_platform_modal(dialog);
            shown = true;
        } else if cfg!(all(
            not(target_family = "wasm"),
            any(target_os = "linux", target_os = "freebsd", windows)
        )) {
            // Find a window to show the Warp-native modal in. If there is no active window, use
            // one of the windows with a running process.
            let window_id_to_focus = ctx
                .windows()
                .active_window()
                .or_else(|| session_summary.windows_running().iter().next().copied());
            if let Some(window_id_to_focus) = window_id_to_focus {
                ctx.windows().show_window_and_focus_app(window_id_to_focus);
                if let Some(workspace) = ctx
                    .views_of_type::<Workspace>(window_id_to_focus)
                    .and_then(|workspaces| workspaces.first().cloned())
                {
                    workspace.update(ctx, |view, ctx| {
                        view.show_native_modal(dialog, ctx);
                    });
                    shown = true;
                }
            }
        }
        shown
    }
}

fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count > 1 {
        plural
    } else {
        singular
    }
}

/// Callback to disable the quit warning modal.
fn on_disable_warning_modal(ctx: &mut AppContext) {
    GeneralSettings::handle(ctx).update(ctx, |general_settings, ctx| {
        report_if_error!(general_settings
            .show_warning_before_quitting
            .toggle_and_save_value(ctx));
    });
    send_telemetry_from_app_ctx!(TelemetryEvent::QuitModalDisabled, ctx);
}
