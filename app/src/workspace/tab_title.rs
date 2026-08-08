//! Shared agent-aware tab-title resolution.
//!
//! The vertical tabs list has always named a tab after the agent session
//! running in it (a Claude Code `/rename`, or the Oz conversation title), while
//! the horizontal tab bar only ever saw `PaneGroup::display_title` — the
//! terminal/shell title — so a renamed session showed up in one surface and not
//! the other. This module is the single place that answers "what is the agent
//! in this tab called", so both surfaces can agree.

use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity as _};

use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::pane_group::PaneGroup;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::cli_agent_sessions::handle_store::AgentSessionHandlesModel;
use crate::terminal::{CLIAgent, TerminalView};
use crate::workspace::tab_settings::{
    RailTaskInfo, TabLineCount, TabPrimaryInfo, TabSecondaryInfo, TabSettings,
};

/// The agent session name for a tab, if it has one.
///
/// Prefers a plugin-backed CLI agent's own title (what `/rename` updates) over
/// the Oz conversation title. Within each, the
/// `use_latest_user_prompt_as_conversation_title_in_tab_names` setting decides
/// whether the session's title or its latest user prompt wins — the same
/// preference the vertical tabs already honour.
///
/// Returns `None` when the tab hosts no agent, or when the CLI agent is not
/// plugin-backed (its title would be stale), letting the caller fall back to
/// the terminal title.
pub(crate) fn agent_session_title(pane_group: &PaneGroup, app: &AppContext) -> Option<String> {
    let terminal_view = pane_group.focused_session_view(app)?;
    let terminal_view = terminal_view.as_ref(app);
    // Resolved by exactly the same helpers the vertical tabs use, so the two
    // surfaces cannot drift apart on what an agent is called.
    let agent_text = terminal_agent_text(terminal_view, app);
    let (conversation_title, cli_agent_title) =
        preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));
    cli_agent_title.or(conversation_title)
}

/// The name to show on a tab: an explicit rename wins, then whatever
/// `TabPrimaryInfo` selects, then the terminal/shell title.
///
/// Gated on `FeatureFlag::Projects` for now because the horizontal tab bar is
/// where the Projects × Tasks layout renders tasks, and naming a task after its
/// agent is only meaningful there.
pub(crate) fn tab_title(pane_group: &PaneGroup, app: &AppContext) -> String {
    // A blank custom title is not a rename, it is a tab with nothing to say:
    // let the resolution below name it rather than honouring the blank.
    if let Some(custom_title) = pane_group.custom_title(app).and_then(non_blank) {
        return custom_title;
    }
    if FeatureFlag::Projects.is_enabled() {
        let primary = TabSettings::as_ref(app).tab_primary_info;
        if let Some(text) = tab_info_text(pane_group, primary.into(), app) {
            return text;
        }
    }
    pane_group.display_title(app)
}

/// The smaller second line of a two-line tab, or `None` when the tab is
/// single-line or there is nothing useful to show.
///
/// The secondary choice is resolved against the primary first, so a tab never
/// shows the same information twice.
pub(crate) fn tab_secondary_line(pane_group: &PaneGroup, app: &AppContext) -> Option<String> {
    let settings = TabSettings::as_ref(app);
    if !FeatureFlag::Projects.is_enabled()
        || !matches!(settings.tab_line_count, TabLineCount::TwoLine)
    {
        return None;
    }
    let secondary = settings
        .tab_secondary_info
        .resolved_for(settings.tab_primary_info);
    tab_info_text(pane_group, secondary.into(), app)
}

/// The distinct kinds of text a tab line can show. Both lines resolve through
/// here so the two settings share one implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabInfoKind {
    AgentSession,
    UserInstruction,
    Command,
    WorkingDirectory,
    Branch,
}

impl From<TabPrimaryInfo> for TabInfoKind {
    fn from(value: TabPrimaryInfo) -> Self {
        match value {
            TabPrimaryInfo::AgentSession => Self::AgentSession,
            TabPrimaryInfo::UserInstruction => Self::UserInstruction,
            TabPrimaryInfo::Command => Self::Command,
            TabPrimaryInfo::WorkingDirectory => Self::WorkingDirectory,
            TabPrimaryInfo::Branch => Self::Branch,
        }
    }
}

impl From<TabSecondaryInfo> for TabInfoKind {
    fn from(value: TabSecondaryInfo) -> Self {
        match value {
            TabSecondaryInfo::AgentSession => Self::AgentSession,
            TabSecondaryInfo::UserInstruction => Self::UserInstruction,
            TabSecondaryInfo::Command => Self::Command,
            TabSecondaryInfo::WorkingDirectory => Self::WorkingDirectory,
            TabSecondaryInfo::Branch => Self::Branch,
        }
    }
}

impl From<RailTaskInfo> for TabInfoKind {
    fn from(value: RailTaskInfo) -> Self {
        match value {
            RailTaskInfo::AgentSession => Self::AgentSession,
            RailTaskInfo::UserInstruction => Self::UserInstruction,
            RailTaskInfo::Command => Self::Command,
            RailTaskInfo::WorkingDirectory => Self::WorkingDirectory,
            RailTaskInfo::Branch => Self::Branch,
        }
    }
}

/// The last-resort name for a live tab, when even the shell has nothing to say.
pub(crate) const SHELL_LABEL_FLOOR: &str = "Shell";

/// The label for one task row in the project rail.
///
/// Gathers the row's four possible names and hands them to
/// [`resolve_rail_task_label`], which owns the order and the never-blank
/// guarantee.
///
/// The shell name is read last and lazily: it takes a short terminal-model
/// lock, which must not happen for every row of every frame. (No caller on the
/// rail's render path holds that lock, so taking it here is safe.)
pub(crate) fn rail_task_label(pane_group: &PaneGroup, app: &AppContext) -> String {
    let kind = TabSettings::as_ref(app).rail_task_info;
    resolve_rail_task_label(
        tab_info_text(pane_group, kind.into(), app),
        || stored_handle_title(pane_group, app),
        || tab_title(pane_group, app),
        || {
            pane_group
                .focused_session_view(app)
                .map(|view| view.as_ref(app).terminal_title_from_shell())
        },
    )
}

/// The rail's label-resolution order, as a pure function of its four sources.
///
/// Every source can legitimately answer with a blank string — an agent that
/// reported an empty title, a cached handle title stored before the name
/// arrived, a [`PaneGroup::display_title`] whose `title` is
/// `unwrap_or_default()` over a focused pane content the tab does not have yet
/// (a file or code pane is constructed with an empty title; a lazily-started
/// shell has no content at all) — and "the source had a value" must never be
/// allowed to mean "the row draws blank". So every step passes through
/// [`non_blank`], and the chain bottoms out in a constant rather than in
/// whatever the last source happened to hold. That is what makes
/// never-blank structural: no combination of inputs can return an empty label.
///
/// Sources after the first are `FnOnce` so nothing below the winner is ever
/// computed.
pub(crate) fn resolve_rail_task_label(
    configured: Option<String>,
    stored_title: impl FnOnce() -> Option<String>,
    tab_title: impl FnOnce() -> String,
    shell_name: impl FnOnce() -> Option<String>,
) -> String {
    configured
        .and_then(non_blank)
        .or_else(|| stored_title().and_then(non_blank))
        .or_else(|| non_blank(tab_title()))
        .or_else(|| shell_name().and_then(non_blank))
        .unwrap_or_else(|| SHELL_LABEL_FLOOR.to_owned())
}

/// A label that is safe to render: trimmed, and `None` when nothing is left.
///
/// The single gate every tab and rail label passes through, so "not blank" is
/// enforced in one place instead of at each of the half-dozen sites that can
/// produce one.
pub(crate) fn non_blank(text: impl Into<String>) -> Option<String> {
    let text = text.into();
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The cached conversation title from the durable session-handle store.
///
/// This is the only name a CLI-agent task has in the rail. It covers both
/// states: while the agent runs (Warp has no live channel for a CLI agent's
/// conversation name) and after it exits (`CLIAgentSessionsModel` drops the
/// session, so [`agent_session_title`] goes `None`). Without it the row falls
/// back to the truncated cwd — the original "six rows all reading
/// `..repos/poa-agent`" problem. The handle outlives the session, so the name
/// does too, including a Claude Code `/rename`, which lands in the transcript
/// the resolver reads.
///
/// Filtered through [`non_blank`]: the store keeps whatever title a plugin
/// event carried (`AgentSessionHandleOp::SetTitle` writes it verbatim), so an
/// agent that reported an empty name would otherwise name the row nothing at
/// all.
pub(crate) fn stored_handle_title(pane_group: &PaneGroup, app: &AppContext) -> Option<String> {
    // Deliberately NOT gated on the agent having exited. Warp has no live
    // channel carrying a CLI agent's conversation name — `cli_agent_title`
    // resolves to `session_context.summary`, which is a permission blurb, not
    // a name — so a *running* session has no name to show either, and gating
    // this on "no live agent" sent running sessions back to the truncated cwd.
    // The stored title is the only name either state has.
    stored_handle_lookup(pane_group, app)
        .and_then(|handle| handle.2)
        .and_then(non_blank)
}

/// The stored session handle belonging to this tab's pane, as
/// `(agent, session_id, cached title)`, **only when the tab has no running
/// agent** — a live session must never be offered as resumable.
///
/// Naming uses [`stored_handle_lookup`] instead, which has no such guard: see
/// the note on [`stored_handle_title`] for why a running session still needs
/// the stored name.
///
/// This is what makes a tab whose agent has exited resumable *in place* — the
/// pane is already sitting in the right directory, so the rail prefills the
/// resume command there rather than opening a second tab for the same work.
pub(crate) fn stored_handle_for_tab(
    pane_group: &PaneGroup,
    app: &AppContext,
) -> Option<(CLIAgent, String, Option<String>)> {
    let has_live_agent = pane_group.focused_session_view(app).is_some_and(|view| {
        CLIAgentSessionsModel::as_ref(app)
            .session(view.id())
            .is_some()
    });
    if has_live_agent {
        return None;
    }
    stored_handle_lookup(pane_group, app)
}

/// Whether anything agent-related is, or has been, attached to this tab.
///
/// Answers the rail's "is this just a shell?" question, and deliberately
/// answers it from the same three sources this module already resolves names
/// from, so a row can never be named after an agent it is then filtered out for
/// not having:
///
/// 1. a live CLI agent session in the focused pane,
/// 2. a Warp Agent Mode conversation on it (empty and entirely-passive
///    conversations do not count — nobody has asked it anything), and
/// 3. a stored session handle bound to the pane, which is what an agent that
///    has since exited leaves behind.
///
/// Case 2 is checked against the same conversation the rail's triage reads
/// (`Workspace::tab_conversation_status`), with the same empty/passive filter,
/// rather than only the naming path's chrome title. That equivalence is what
/// makes the filter safe: **a row that shows an agent status can never be
/// hidden**, so no agent waiting on the user can be filtered out of the rail.
///
/// Scanned sessions need no case of their own: by the authority split on
/// [`session_scan`](crate::terminal::cli_agent_sessions::session_scan) they
/// have no pane at all, so they can never belong to a live tab — they render as
/// dormant rows, which the filter never sees.
pub(crate) fn pane_has_agent(pane_group: &PaneGroup, app: &AppContext) -> bool {
    let Some(terminal_view) = pane_group.focused_session_view(app) else {
        // No terminal session in the focused pane (a file, notebook or code
        // pane): not an agent, and not a shell to be hidden either.
        return false;
    };
    let agent_text = terminal_agent_text(terminal_view.as_ref(app), app);
    agent_text.cli_agent.is_some()
        || agent_text.is_oz_agent
        || BlocklistAIHistoryModel::as_ref(app)
            .active_conversation(terminal_view.id())
            .is_some_and(|conversation| {
                !conversation.is_empty() && !conversation.is_entirely_passive()
            })
        || stored_handle_lookup(pane_group, app).is_some()
}

/// The stored session handle this tab's pane hosts, regardless of whether an
/// agent is currently running in it.
///
/// Matched on the pane's persistent uuid **and** its current directory. The
/// uuid survives a restart (it is what `terminal_panes.uuid` stores), so a
/// restored tab is still recognised as the one that ran the session; the
/// directory check stops a pane that has since `cd`-ed away — or restored to a
/// different startup directory — from claiming a session it no longer hosts,
/// which would file the task under the wrong project and resume in the wrong
/// place.
fn stored_handle_lookup(
    pane_group: &PaneGroup,
    app: &AppContext,
) -> Option<(CLIAgent, String, Option<String>)> {
    if !FeatureFlag::ResumeProjectTasks.is_enabled() {
        return None;
    }
    // `active_session_path` resolves through the group's focus state, which a
    // restored tab that has never been opened does not have — and under
    // `LazyShellStartup` that is most of them after a restart. Falling back to
    // the retained startup directory is what lets such a tab still be
    // recognised as holding an agent session.
    //
    // This matters beyond naming: `pane_has_agent` consults this lookup, and
    // "Clear shells without agents" closes panes it reports as agent-less. A
    // missing cwd must not be allowed to read as "no agent ever ran here".
    let pane_cwd = pane_group
        .active_session_path(app)
        .or_else(|| pane_group.restored_terminal_startup_directory())
        .and_then(|path| path.to_str().map(str::to_owned))?;
    AgentSessionHandlesModel::as_ref(app)
        .find_by_pane_and_cwd(&pane_cwd, |pane_uuid| {
            pane_group
                .find_terminal_pane_by_session_uuid(pane_uuid)
                .is_some()
        })
        .map(|handle| {
            (
                handle.agent,
                handle.session_id.clone(),
                handle.title.clone(),
            )
        })
}

/// Resolves one kind of tab text for the tab's focused session. Returns `None`
/// when that information isn't available (no agent, no repo, no command yet),
/// letting the caller fall back.
fn tab_info_text(pane_group: &PaneGroup, kind: TabInfoKind, app: &AppContext) -> Option<String> {
    if matches!(kind, TabInfoKind::AgentSession) {
        // Filtered like every other kind below. This branch used to return
        // before reaching that filter, and an agent title of `Some("")` — which
        // a conversation title resolves to whenever its root task, initial
        // query and fallback title are all empty strings — became a rail row
        // with no text in it.
        return agent_session_title(pane_group, app).and_then(non_blank);
    }
    let terminal_view = pane_group.focused_session_view(app)?;
    let terminal_view = terminal_view.as_ref(app);
    let text = match kind {
        // Handled above; listed for exhaustiveness.
        TabInfoKind::AgentSession => None,
        // Deliberately independent of the session-title preference: this line
        // is *always* the latest instruction, so it can sit beneath a renamed
        // session rather than competing with it for the same slot.
        TabInfoKind::UserInstruction => {
            let agent_text = terminal_agent_text(terminal_view, app);
            agent_text
                .cli_agent_latest_user_prompt
                .or(agent_text.conversation_latest_user_prompt)
        }
        TabInfoKind::Command => terminal_view.last_completed_command_text(),
        TabInfoKind::WorkingDirectory => terminal_view
            .display_working_directory(app)
            .or_else(|| restored_working_directory(pane_group)),
        TabInfoKind::Branch => terminal_view.current_git_branch(app),
    };
    text.and_then(non_blank)
}

/// The directory a pane was restored into, formatted the way a live one is.
///
/// `display_working_directory` reads a prompt chip or `pwd()`, and both only
/// exist once the shell has started and reported in. A tab whose shell is
/// deferred until it is opened therefore has no live answer at all, and without
/// this it would fall all the way back to the shell name — ~50 tabs reading
/// "zsh" instead of their folders. The directory is known from restoration, so
/// use it. This mirrors `PaneGroup::session_path`, which does the same for
/// project attribution.
///
/// Deliberately reads `restored_terminal_startup_directory` rather than
/// `active_session_path`: the latter resolves through the pane group's
/// focused pane id, which a background tab restored under
/// `FeatureFlag::LazyShellStartup` may never have had, leaving nothing to
/// resolve there even though the startup directory is known.
fn restored_working_directory(pane_group: &PaneGroup) -> Option<String> {
    let path = pane_group.restored_terminal_startup_directory()?;
    let home_dir = dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
    Some(
        warp_util::path::user_friendly_path(&path.to_string_lossy(), home_dir.as_deref())
            .to_string(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentTabTextPreference {
    ConversationTitle,
    LatestUserPrompt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TerminalAgentText {
    pub(crate) conversation_display_title: Option<String>,
    pub(crate) conversation_latest_user_prompt: Option<String>,
    pub(crate) cli_agent_title: Option<String>,
    pub(crate) cli_agent_latest_user_prompt: Option<String>,
    pub(crate) is_oz_agent: bool,
    pub(crate) cli_agent: Option<CLIAgent>,
}

pub(crate) fn agent_tab_text_preference(app: &AppContext) -> AgentTabTextPreference {
    if *TabSettings::as_ref(app).use_latest_user_prompt_as_conversation_title_in_tab_names {
        AgentTabTextPreference::LatestUserPrompt
    } else {
        AgentTabTextPreference::ConversationTitle
    }
}

pub(crate) fn preferred_agent_tab_titles(
    agent_text: &TerminalAgentText,
    preference: AgentTabTextPreference,
) -> (Option<String>, Option<String>) {
    let conversation_title = match preference {
        AgentTabTextPreference::ConversationTitle => agent_text
            .conversation_display_title
            .clone()
            .or_else(|| agent_text.conversation_latest_user_prompt.clone()),
        AgentTabTextPreference::LatestUserPrompt => agent_text
            .conversation_latest_user_prompt
            .clone()
            .or_else(|| agent_text.conversation_display_title.clone()),
    };
    let cli_agent_title = match preference {
        AgentTabTextPreference::ConversationTitle => agent_text.cli_agent_title.clone(),
        AgentTabTextPreference::LatestUserPrompt => agent_text
            .cli_agent_latest_user_prompt
            .clone()
            .or_else(|| agent_text.cli_agent_title.clone()),
    };

    (conversation_title, cli_agent_title)
}

pub(crate) fn terminal_agent_text(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> TerminalAgentText {
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    let is_plugin_backed = cli_agent_session.is_some_and(|session| session.listener.is_some());
    let is_ambient_agent = terminal_view.is_ambient_agent_session(app);

    let mut agent_text = TerminalAgentText {
        is_oz_agent: is_ambient_agent,
        cli_agent: cli_agent_session.map(|session| session.agent),
        ..Default::default()
    };

    if cli_agent_session.is_some() && !is_plugin_backed {
        return agent_text;
    }

    agent_text.conversation_display_title = terminal_view.selected_conversation_display_title(app);
    agent_text.conversation_latest_user_prompt =
        terminal_view.selected_conversation_latest_user_prompt_for_tab_name(app);
    agent_text.is_oz_agent =
        agent_text.conversation_display_title.is_some() || agent_text.is_oz_agent;

    if let Some(session) = cli_agent_session {
        agent_text.cli_agent_title = session.session_context.title_like_text();
        agent_text.cli_agent_latest_user_prompt = session.session_context.latest_user_prompt();
    }

    agent_text
}

#[cfg(test)]
#[path = "tab_title_tests.rs"]
mod tests;
