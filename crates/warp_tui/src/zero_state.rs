//! The pre-first-interaction "zero state" filling the transcript area: the
//! Warp Agent CLI title and version, a "What's new" changelog section, and the
//! session's project context (rules and skills discovered).
//!
//! The session view owns visibility: the zero state fills the transcript
//! slot while the transcript has no visible content, so it dismisses once
//! the first accepted submission produces a block and returns whenever the
//! transcript empties out again.
//!
//! Which sections the zero state draws is user-configurable: every section
//! except the title and version has an `appearance.zero_state.show_*` toggle
//! (see [`ZeroStateSectionVisibility`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ai::project_context::model::{
    ProjectContextModel, ProjectContextModelEvent, ProjectRulesResult,
};
use warp::settings::{TuiZeroStateSettings, TuiZeroStateSettingsChangedEvent};
use warp::tui_export::{
    ActiveSession, ActiveSessionEvent, ChangelogModel, ChangelogModelEvent, ChangelogState,
    SkillManager, SkillManagerEvent, TuiMcpConfigState, TuiMcpManager, TuiMcpServerStatus,
    TuiUserInfoManager, TuiUserInfoManagerEvent,
};
use warp_core::channel::ChannelState;
use warp_core::settings::Setting;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Color, Modifier, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiStack, TuiText,
};
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, ViewContext};

use crate::autoupdate::{TuiAutoupdateStatus, TuiAutoupdater, TuiAutoupdaterEvent};
use crate::tui_builder::TuiUiBuilder;
use crate::ui::abbreviate_home_prefix;
use crate::zero_state_animation::{
    WarpLogoStyles, ZeroStateAnimationConfig, ZeroStateAnimationConfigEvent,
    ZeroStateAnimationElement, ZeroStateStarfieldElement,
};

/// Cap on "What's new" bullets, mirroring the compact zero-state mock.
const MAX_CHANGELOG_BULLETS: usize = 3;

/// Fixed width for the two constrained sub-sections of the overlay column (top: title +
/// version + changelog bullets; bottom: project context body + MCP). Pinning both axes
/// to the same value keeps wrapping stable while those sections load asynchronously.
///
/// The project path *header* is rendered outside these constrained boxes so it can expand
/// to the full available terminal width without being capped by this constant.
const LEFT_COLUMN_COLS: u16 = 48;

/// Width of the right-aligned animation region. This keeps the logo secondary
/// to the copy and input while leaving enough cells for its wireframe detail.
const ANIMATION_PANEL_COLS: u16 = 32;

// ---------------------------------------------------------------------------
// ZeroStateSectionVisibility
// ---------------------------------------------------------------------------

/// Which optional zero-state sections are rendered, resolved from the
/// `appearance.zero_state.show_*` settings.
///
/// Every field defaults to `true`, so an unset (or unreadable) setting keeps
/// the section visible and the zero state looks exactly as it did before the
/// toggles existed. The title and version lines are always drawn and are
/// deliberately not represented here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ZeroStateSectionVisibility {
    /// The `Signed in as …` account line.
    signed_in_user: bool,
    /// The "What's new" changelog header and its bullets.
    changelog: bool,
    /// The project path header together with its rules/skills body.
    project_info: bool,
    /// The MCP header and status line.
    mcp: bool,
    /// The rotating object together with its starfield.
    animation: bool,
}

impl Default for ZeroStateSectionVisibility {
    fn default() -> Self {
        Self {
            signed_in_user: true,
            changelog: true,
            project_info: true,
            mcp: true,
            animation: true,
        }
    }
}

impl ZeroStateSectionVisibility {
    /// Reads the current toggles from [`TuiZeroStateSettings`].
    ///
    /// Falls back to [`Self::default`] (everything visible) when the settings
    /// group is not registered, which is the case in focused unit tests that
    /// render zero-state elements without the full app settings stack.
    fn from_settings(ctx: &AppContext) -> Self {
        if !ctx.has_singleton_model::<TuiZeroStateSettings>() {
            return Self::default();
        }
        let settings = TuiZeroStateSettings::as_ref(ctx);
        Self {
            signed_in_user: *settings.show_signed_in_user.value(),
            changelog: *settings.show_changelog.value(),
            project_info: *settings.show_project_info.value(),
            mcp: *settings.show_mcp.value(),
            animation: *settings.show_animation.value(),
        }
    }
}

// ---------------------------------------------------------------------------
// TuiZeroStateView
// ---------------------------------------------------------------------------

/// The zero-state view: displayed when the transcript is empty.
///
/// Owns the animation clock so the logo's rotation remains continuous across
/// view re-renders (e.g. when MCP connects or a changelog loads).
pub(crate) struct TuiZeroStateView {
    clock: AnimationClock,
    animation_config: Arc<ZeroStateAnimationConfig>,
    active_session: ModelHandle<ActiveSession>,
    section_visibility: ZeroStateSectionVisibility,
}

impl TuiZeroStateView {
    pub(crate) fn new(
        active_session: ModelHandle<ActiveSession>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Subscribe to events that change what the zero state displays so
        // this view re-renders independently of its parent.
        ctx.subscribe_to_model(
            &ChangelogModel::handle(ctx),
            |_, _, event: &ChangelogModelEvent, ctx| {
                if let ChangelogModelEvent::ChangelogRequestComplete { .. } = event {
                    ctx.notify();
                }
            },
        );
        ctx.subscribe_to_model(
            &TuiAutoupdater::handle(ctx),
            |_, _, event: &TuiAutoupdaterEvent, ctx| {
                let TuiAutoupdaterEvent::StatusChanged = event;
                ctx.notify();
            },
        );
        ctx.subscribe_to_model(
            &ProjectContextModel::handle(ctx),
            |_, _, event: &ProjectContextModelEvent, ctx| {
                if let ProjectContextModelEvent::PathIndexed = event {
                    ctx.notify();
                }
            },
        );
        ctx.subscribe_to_model(
            &SkillManager::handle(ctx),
            |_, _, SkillManagerEvent::SkillsChanged { .. }, ctx| ctx.notify(),
        );
        ctx.subscribe_to_model(&TuiMcpManager::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&TuiUserInfoManager::handle(ctx), |_, _, event, ctx| {
            let TuiUserInfoManagerEvent::Updated = event;
            ctx.notify();
        });
        ctx.subscribe_to_model(&active_session, |_, _, event, ctx| {
            let ActiveSessionEvent::UpdatedPwd = event else {
                return;
            };
            ctx.notify();
        });
        let animation_config = ZeroStateAnimationConfig::handle(ctx);
        let animation_config_snapshot = Arc::new(animation_config.as_ref(ctx).clone());
        ctx.subscribe_to_model(
            &animation_config,
            |view, animation_config, event, ctx| match event {
                ZeroStateAnimationConfigEvent::Updated => {
                    view.animation_config = Arc::new(animation_config.as_ref(ctx).clone());
                    ctx.notify();
                }
                ZeroStateAnimationConfigEvent::LoadFailed(_) => {}
            },
        );
        // Re-render as soon as a section is toggled so the zero state follows
        // the settings file without restarting. The object/rotation/extrusion
        // settings are owned by `ZeroStateAnimationConfig`, which emits its own
        // `Updated` event, so they are ignored here.
        if ctx.has_singleton_model::<TuiZeroStateSettings>() {
            ctx.subscribe_to_model(&TuiZeroStateSettings::handle(ctx), |view, _, event, ctx| {
                match event {
                    TuiZeroStateSettingsChangedEvent::TuiZeroStateShowSignedInUserSetting {
                        ..
                    }
                    | TuiZeroStateSettingsChangedEvent::TuiZeroStateShowChangelogSetting {
                        ..
                    }
                    | TuiZeroStateSettingsChangedEvent::TuiZeroStateShowProjectInfoSetting {
                        ..
                    }
                    | TuiZeroStateSettingsChangedEvent::TuiZeroStateShowMcpSetting { .. }
                    | TuiZeroStateSettingsChangedEvent::TuiZeroStateShowAnimationSetting {
                        ..
                    } => {
                        view.section_visibility = ZeroStateSectionVisibility::from_settings(ctx);
                        ctx.notify();
                    }
                    TuiZeroStateSettingsChangedEvent::TuiZeroStateObjectSetting { .. }
                    | TuiZeroStateSettingsChangedEvent::TuiZeroStateRotationPeriodSecondsSetting {
                        ..
                    }
                    | TuiZeroStateSettingsChangedEvent::TuiZeroStateExtrusionDepthSetting {
                        ..
                    } => {}
                }
            });
        }

        Self {
            clock: AnimationClock::starting_at(Duration::ZERO),
            animation_config: animation_config_snapshot,
            active_session,
            section_visibility: ZeroStateSectionVisibility::from_settings(ctx),
        }
    }
}

impl Entity for TuiZeroStateView {
    type Event = ();
}

impl TuiView for TuiZeroStateView {
    fn ui_name() -> &'static str {
        "TuiZeroStateView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        let session = self.active_session.as_ref(ctx);
        let cwd = session.current_working_directory().cloned().or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned())
        });
        let overlay =
            build_zero_state_overlay(cwd.as_deref(), self.section_visibility, &builder, ctx);
        if !self.section_visibility.animation {
            // No animation means no starfield and no reserved animation panel:
            // the copy is the whole zero state.
            return build_zero_state_copy_only_layout(overlay);
        }
        let animation = ZeroStateAnimationElement::new(
            self.clock,
            self.animation_config.clone(),
            WarpLogoStyles {
                front: builder.accent_text_style(),
                back: builder.primary_text_style(),
                side: builder.dim_text_style(),
                background: builder.muted_text_style(),
            },
        )
        .without_background_stars()
        .finish();
        let starfield = ZeroStateStarfieldElement::new(
            self.clock,
            builder.muted_text_style(),
            LEFT_COLUMN_COLS,
            ANIMATION_PANEL_COLS,
        )
        .finish();
        build_zero_state_layout(starfield, animation, overlay)
    }
}

/// Centers the animation within the space beside the copy and centers the
/// opaque copy block vertically. Reserving the copy column before measuring
/// the animation also hides the artwork when a narrow terminal cannot display
/// both regions.
fn build_zero_state_layout(
    starfield: Box<dyn TuiElement>,
    animation: Box<dyn TuiElement>,
    overlay: Box<dyn TuiElement>,
) -> Box<dyn TuiElement> {
    let copy_column_reservation = TuiConstrainedBox::new(TuiText::new("").finish())
        .with_min_cols(LEFT_COLUMN_COLS)
        .with_max_cols(LEFT_COLUMN_COLS)
        .finish();
    let animation = TuiConstrainedBox::new(animation)
        .with_max_cols(ANIMATION_PANEL_COLS)
        .finish();
    let animation_region = TuiFlex::row()
        .flex_child(TuiText::new("").finish())
        .child(animation)
        .flex_child(TuiText::new("").finish())
        .finish();
    let animation_layer = TuiFlex::row()
        .child(copy_column_reservation)
        .flex_child(animation_region)
        .finish();

    TuiStack::new()
        .child(starfield)
        .child(animation_layer)
        .child(centered_overlay_layer(overlay))
        .finish()
}

/// The layout used when the animation section is hidden: just the vertically
/// centered copy, with no starfield behind it and no animation panel reserved
/// beside it.
fn build_zero_state_copy_only_layout(overlay: Box<dyn TuiElement>) -> Box<dyn TuiElement> {
    centered_overlay_layer(overlay)
}

/// Vertically centers the opaque copy block within the available rows.
fn centered_overlay_layer(overlay: Box<dyn TuiElement>) -> Box<dyn TuiElement> {
    let overlay = TuiContainer::new(overlay)
        .with_background(Color::Reset)
        .finish();
    TuiFlex::column()
        .flex_child(TuiText::new("").finish())
        .child(overlay)
        .flex_child(TuiText::new("").finish())
        .finish()
}
/// Assembles the text-overlay column placed on top of the animation layer.
///
/// Both [`TuiZeroStateView::render`] and the regression tests call this function so
/// that a change to how `render` composes the overlay (e.g. moving the path header
/// back inside the `LEFT_COLUMN_COLS` constrained box) is caught by the test suite.
///
/// `visibility` decides which optional sections are drawn; a hidden section
/// contributes no rows at all (no header, no spacer).
fn build_zero_state_overlay(
    cwd: Option<&str>,
    visibility: ZeroStateSectionVisibility,
    builder: &TuiUiBuilder,
    ctx: &AppContext,
) -> Box<dyn TuiElement> {
    // Compute project context once — find_applicable_project_rules walks the
    // directory tree and clones rule file contents, so resolving it once
    // avoids a redundant allocation on every zero-state re-render (pwd change,
    // changelog load, MCP update, PathIndexed). Skipped entirely when the
    // project section is hidden.
    let project_cwd = visibility.project_info.then_some(cwd).flatten();
    let (path_header_text, project_rules) = match project_cwd {
        Some(cwd) => {
            let cwd_path = LocalOrRemotePath::Local(PathBuf::from(cwd));
            let rules = ProjectContextModel::as_ref(ctx).find_applicable_project_rules(&cwd_path);
            let header_text = project_section_header_text(cwd, rules.as_ref());
            (Some(header_text), Some(rules))
        }
        None => (None, None),
    };

    // Title, version, and (when visible) the account line and changelog —
    // constrained to LEFT_COLUMN_COLS so changelog bullets (which lack
    // `.truncate()`) do not wrap against the full terminal width.
    let constrained_top =
        TuiConstrainedBox::new(render_top_section(visibility, builder, ctx).finish())
            .with_min_cols(LEFT_COLUMN_COLS)
            .with_max_cols(LEFT_COLUMN_COLS)
            .finish();

    let mut column = TuiFlex::column().child(constrained_top);

    // The project path header lives *outside* the 48-column constrained boxes so it
    // can expand to the full available terminal width. Give it a blank-row separator
    // from the top section and place it directly above the constrained bottom section.
    // Keep the full displayed path: it stays on one row when it fits and wraps only
    // when the terminal is genuinely too narrow.
    if let Some(path_header_text) = path_header_text {
        let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
        let path_header = TuiText::new(path_header_text)
            .with_style(header_style)
            .finish();
        column = column.child(blank_row()).child(path_header);
    }

    // Project context body (rules / skills / placeholder) and MCP — also constrained
    // to LEFT_COLUMN_COLS, keeping those rows stable. Omitted altogether when both
    // sections are hidden so no empty box is measured.
    // Pass the pre-computed rules so find_applicable_project_rules is not called twice.
    if project_cwd.is_some() || visibility.mcp {
        let rules_ref = project_rules.flatten();
        let constrained_bottom = TuiConstrainedBox::new(
            render_bottom_section(project_cwd, rules_ref.as_ref(), visibility, builder, ctx)
                .finish(),
        )
        .with_min_cols(LEFT_COLUMN_COLS)
        .with_max_cols(LEFT_COLUMN_COLS)
        .finish();
        column = column.child(constrained_bottom);
    }

    column.finish()
}

/// Top section of the overlay column: title, version, the signed-in account
/// line, and changelog bullets. The account line and changelog are each drawn
/// only when `visibility` enables them; the title and version are always drawn.
///
/// This is wrapped in a [`TuiConstrainedBox`] with `min = max = LEFT_COLUMN_COLS` by the
/// caller so that changelog bullets (which lack `.truncate()`) do not word-wrap against
/// the full terminal width while still rendering stably during async content loads.
fn render_top_section(
    visibility: ZeroStateSectionVisibility,
    builder: &TuiUiBuilder,
    app: &AppContext,
) -> TuiFlex {
    let title_style = builder.accent_text_style().add_modifier(Modifier::BOLD);
    let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
    let muted = builder.muted_text_style();

    let mut column = TuiFlex::column()
        .child(
            TuiText::new("Warp Agent CLI")
                .with_style(title_style)
                .truncate()
                .finish(),
        )
        .child(render_version_line(builder, app));
    if visibility.signed_in_user {
        column = column.child(render_login_line(builder, app));
    }

    let bullets = if visibility.changelog {
        changelog_bullets(app)
    } else {
        Vec::new()
    };
    if !bullets.is_empty() {
        column = column.child(blank_row()).child(
            TuiText::new("What's new")
                .with_style(header_style)
                .truncate()
                .finish(),
        );
        for bullet in bullets {
            // A fixed (non-flex) text child still wraps against the remaining
            // width while only reporting its natural width.
            column = column.child(
                TuiFlex::row()
                    .child(TuiText::new("• ").with_style(muted).truncate().finish())
                    .child(TuiText::new(bullet).with_style(muted).finish())
                    .finish(),
            );
        }
    }
    column
}

/// Bottom section of the overlay column: project context body (rules / skills / placeholder)
/// when a `cwd` is present, followed by the MCP section when it is visible.
///
/// `cwd` is already `None` when the project section is hidden, so this function only has to
/// gate the MCP rows on `visibility`.
///
/// The project path *header* is intentionally omitted here — it lives outside the constrained
/// box so it can expand to the full available terminal width (see [`TuiZeroStateView::render`]).
///
/// `rules` must be the pre-computed [`ProjectRulesResult`] for `cwd`, resolved once in the
/// caller to avoid a duplicate upward directory walk.
fn render_bottom_section(
    cwd: Option<&str>,
    rules: Option<&ProjectRulesResult>,
    visibility: ZeroStateSectionVisibility,
    builder: &TuiUiBuilder,
    app: &AppContext,
) -> TuiFlex {
    let column = TuiFlex::column();
    let column = if let Some(cwd) = cwd {
        render_project_context_body(cwd, rules, column, builder, app)
    } else {
        column
    };
    if visibility.mcp {
        render_mcp_section(column, builder, app)
    } else {
        column
    }
}

/// Returns the abbreviated path text displayed as the project section header.
///
/// Uses the project root from `rules` when available, falling back to the raw `cwd` string.
/// This is the same text previously embedded inside the 48-column constrained box; it is now
/// computed separately so the caller can render it outside that box.
///
/// `rules` must already be resolved by the caller (via [`ProjectContextModel`]) so the
/// upward directory walk is not repeated for the project context body.
fn project_section_header_text(cwd: &str, rules: Option<&ProjectRulesResult>) -> String {
    let header = rules
        .map(|rules| rules.root_path.display_path())
        .unwrap_or_else(|| cwd.to_owned());
    abbreviate_home_prefix(&header)
}

fn render_mcp_section(mut column: TuiFlex, builder: &TuiUiBuilder, app: &AppContext) -> TuiFlex {
    let snapshot = TuiMcpManager::as_ref(app).snapshot();
    let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
    let muted = builder.muted_text_style();
    column = column.child(blank_row()).child(
        TuiText::new("MCP")
            .with_style(header_style)
            .truncate()
            .finish(),
    );
    if matches!(snapshot.config_state, TuiMcpConfigState::Missing) {
        column = column.child(
            TuiText::new(abbreviate_home_prefix(
                &snapshot.config_path.display().to_string(),
            ))
            .with_style(builder.dim_text_style())
            .truncate()
            .finish(),
        );
    }

    let (label, is_error) = mcp_status_label(snapshot);
    let style = if is_error {
        builder.error_text_style()
    } else {
        muted
    };
    column.child(TuiText::new(label).with_style(style).truncate().finish())
}

fn mcp_status_label(snapshot: &warp::tui_export::TuiMcpSnapshot) -> (String, bool) {
    match &snapshot.config_state {
        TuiMcpConfigState::Invalid { .. } => ("Config error · run /mcp".to_string(), true),
        TuiMcpConfigState::Missing => ("Not configured · /mcp".to_string(), false),
        TuiMcpConfigState::Ready if snapshot.servers.is_empty() => {
            ("No servers configured · run /mcp".to_string(), false)
        }
        TuiMcpConfigState::Ready => {
            let mut running = 0;
            let mut starting = 0;
            let mut authenticating = 0;
            let mut stopping = 0;
            let mut failed = 0;
            let mut offline = 0;
            for server in &snapshot.servers {
                match &server.status {
                    TuiMcpServerStatus::Offline => offline += 1,
                    TuiMcpServerStatus::Starting => starting += 1,
                    TuiMcpServerStatus::Authenticating => authenticating += 1,
                    TuiMcpServerStatus::Running => running += 1,
                    TuiMcpServerStatus::Stopping => stopping += 1,
                    TuiMcpServerStatus::Failed { .. } => failed += 1,
                }
            }
            let mut parts = Vec::new();
            if running > 0 {
                parts.push(format!("{running} connected"));
            }
            if starting > 0 {
                parts.push(format!("{starting} starting"));
            }
            if authenticating > 0 {
                parts.push(format!("{authenticating} needs auth"));
            }
            if stopping > 0 {
                parts.push(format!("{stopping} stopping"));
            }
            if failed > 0 {
                parts.push(format!("{failed} failed"));
            }
            if offline > 0 {
                parts.push(format!("{offline} offline"));
            }
            (format!("{} · /mcp", parts.join(" · ")), false)
        }
    }
}

/// The login-info line: the signed-in account (email, falling back to the
/// display name) when authenticated, or a graceful "Not signed in" state when
/// not. The zero state is normally only shown after login, but the unauthenticated
/// branch keeps the surface honest if it is ever rendered before auth completes.
fn render_login_line(builder: &TuiUiBuilder, app: &AppContext) -> Box<dyn TuiElement> {
    let muted = builder.muted_text_style();
    let dim = builder.dim_text_style();
    let user_info = TuiUserInfoManager::as_ref(app).snapshot(app);
    let display = user_info
        .email
        .filter(|email| !email.is_empty())
        .or(user_info.username.filter(|username| !username.is_empty()));
    let (label, style) = if let Some(display) = display {
        (format!("Signed in as {display}"), muted)
    } else if user_info.is_logged_in {
        ("Signed in".to_owned(), muted)
    } else {
        ("Not signed in".to_owned(), dim)
    };
    TuiText::new(label).with_style(style).truncate().finish()
}

/// The version line: the release version (or "dev build"), with the
/// background auto-updater's status appended in parentheses. Dev builds
/// never run the updater (and have no version), so they render plain; the
/// `Idle` status (updater ineligible, or no stable check result yet) renders
/// no suffix either.
fn render_version_line(builder: &TuiUiBuilder, app: &AppContext) -> Box<dyn TuiElement> {
    let muted = builder.muted_text_style();
    let Some(version) = ChannelState::app_version() else {
        return TuiText::new("dev build")
            .with_style(muted)
            .truncate()
            .finish();
    };
    let suffix = match TuiAutoupdater::as_ref(app).status() {
        TuiAutoupdateStatus::Idle => None,
        TuiAutoupdateStatus::Checking => Some(("checking for updates…", muted)),
        TuiAutoupdateStatus::Updating => Some(("updating…", muted)),
        TuiAutoupdateStatus::UpToDate => Some(("up to date", muted)),
        // The one state worth drawing attention to: an update is staged and
        // a restart picks it up.
        TuiAutoupdateStatus::PendingRestart => Some((
            "update installed, restart to apply",
            builder.success_glyph_style(),
        )),
    };
    let Some((label, style)) = suffix else {
        return TuiText::new(version).with_style(muted).truncate().finish();
    };
    // Like the bullet rows below: the version reports its natural width and
    // the suffix wraps against the remaining column width.
    TuiFlex::row()
        .child(
            TuiText::new(format!("{version} "))
                .with_style(muted)
                .truncate()
                .finish(),
        )
        .child(
            TuiText::new(format!("({label})"))
                .with_style(style)
                .finish(),
        )
        .finish()
}

/// Appends the project context body rows to `column`: the discovered rule files and
/// skill count (or a placeholder while discovery is still in progress).
///
/// The project path *header* is intentionally omitted — it is rendered at the outer
/// level outside the constrained box so it can use the full terminal width (see
/// [`TuiZeroStateView::render`] and [`project_section_header_text`]).
///
/// `rules` must be the pre-computed [`ProjectRulesResult`] for `cwd`, resolved once in the
/// caller to avoid a duplicate upward directory walk.
fn render_project_context_body(
    cwd: &str,
    rules: Option<&ProjectRulesResult>,
    mut column: TuiFlex,
    builder: &TuiUiBuilder,
    app: &AppContext,
) -> TuiFlex {
    let muted = builder.muted_text_style();
    let check = builder.success_glyph_style();

    // Use the pre-computed rules from the render() call — find_applicable_project_rules
    // is not called again here.
    let mut rule_files: Vec<String> = Vec::new();
    if let Some(rules) = rules {
        for rule in &rules.active_rules {
            if let Some(name) = rule.path.file_name()
                && !rule_files.iter().any(|file| file == name)
            {
                rule_files.push(name.to_owned());
            }
        }
    }

    let cwd_path = LocalOrRemotePath::Local(PathBuf::from(cwd));
    let project_skill_count = SkillManager::as_ref(app)
        .get_skills_for_working_directory(Some(&cwd_path), app)
        .iter()
        .filter(|skill| skill.is_project_skill())
        .count();

    if rule_files.is_empty() && project_skill_count == 0 {
        // Repo detection, metadata indexing, and skill scans are async, so
        // nothing may be known yet; this also covers projects with no
        // context at all.
        return column.child(
            TuiText::new("Discovering project context…")
                .with_style(builder.dim_text_style())
                .truncate()
                .finish(),
        );
    }

    let status_row = |column: TuiFlex, text: String| {
        column.child(
            TuiFlex::row()
                .child(TuiText::new("✓ ").with_style(check).truncate().finish())
                .child(TuiText::new(text).with_style(muted).truncate().finish())
                .finish(),
        )
    };
    for file in rule_files {
        column = status_row(column, format!("{file} loaded"));
    }
    if project_skill_count > 0 {
        let plural = if project_skill_count == 1 { "" } else { "s" };
        column = status_row(
            column,
            format!("{project_skill_count} skill{plural} discovered"),
        );
    }
    column
}

/// Up to [`MAX_CHANGELOG_BULLETS`] plain-text bullets for the current
/// version's changelog, or empty when no changelog is available (request
/// failed, still pending, or a channel without release changelogs).
fn changelog_bullets(app: &AppContext) -> Vec<String> {
    let ChangelogState::Some(changelog) = &ChangelogModel::as_ref(app).changelog else {
        return Vec::new();
    };
    changelog_bullets_from_changelog(changelog)
}

fn changelog_bullets_from_changelog(changelog: &channel_versions::Changelog) -> Vec<String> {
    changelog
        .tui_updates
        .iter()
        .take(MAX_CHANGELOG_BULLETS)
        .cloned()
        .collect()
}

/// A one-row spacer between sections.
fn blank_row() -> Box<dyn TuiElement> {
    TuiText::new(" ").truncate().finish()
}

#[cfg(test)]
#[path = "zero_state_tests.rs"]
mod tests;
