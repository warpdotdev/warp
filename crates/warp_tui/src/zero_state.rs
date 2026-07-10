//! The pre-first-interaction "zero state" filling the transcript area: the
//! Warp Agent title and version, a "What's new" changelog section, and the
//! session's project context (rules and skills discovered).
//!
//! The session view owns visibility: the zero state fills the transcript
//! slot while the transcript has no visible content, so it dismisses once
//! the first accepted submission produces a block and returns whenever the
//! transcript empties out again.

use std::path::PathBuf;

use ai::project_context::model::ProjectContextModel;
use warp::tui_export::{ChangelogModel, ChangelogState, SkillManager};
use warp_core::channel::ChannelState;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{Modifier, TuiConstrainedBox, TuiElement, TuiFlex, TuiText};
use warpui_core::AppContext;

use crate::autoupdate::{TuiAutoupdateStatus, TuiAutoupdater};
use crate::tui_builder::TuiUiBuilder;
use crate::ui::abbreviate_home_prefix;

/// Cap on "What's new" bullets, mirroring the compact zero-state mock.
const MAX_CHANGELOG_BULLETS: usize = 3;

/// Width cap on the text column so bullets wrap like the mock.
const LEFT_COLUMN_MAX_COLS: u16 = 48;

/// Renders the zero state for the transcript area. `cwd` is the session's
/// working directory for the project section.
pub(crate) fn render_zero_state(cwd: Option<&str>, app: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    TuiConstrainedBox::new(render_left_column(cwd, &builder, app).finish())
        .with_max_cols(LEFT_COLUMN_MAX_COLS)
        .finish()
}

/// The left text column: title, version, "What's new", and project context.
fn render_left_column(cwd: Option<&str>, builder: &TuiUiBuilder, app: &AppContext) -> TuiFlex {
    let title_style = builder.accent_border_style().add_modifier(Modifier::BOLD);
    let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
    let muted = builder.muted_text_style();

    let mut column = TuiFlex::column()
        .child(
            TuiText::new("Warp Agent")
                .with_style(title_style)
                .truncate()
                .finish(),
        )
        .child(render_version_line(builder, app));

    let bullets = changelog_bullets(app);
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

    if let Some(cwd) = cwd {
        column = render_project_section(cwd, column, builder, app);
    }
    column
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

/// Appends the project section: the project root (or cwd) as a header, then
/// one line per discovered rule file and a discovered-skill count. Discovery
/// is asynchronous, so a placeholder shows until results land.
fn render_project_section(
    cwd: &str,
    mut column: TuiFlex,
    builder: &TuiUiBuilder,
    app: &AppContext,
) -> TuiFlex {
    let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
    let muted = builder.muted_text_style();
    let check = builder.success_glyph_style();

    let cwd_path = LocalOrRemotePath::Local(PathBuf::from(cwd));
    let rules = ProjectContextModel::as_ref(app).find_applicable_project_rules(&cwd_path);

    // Rule files that actively apply to the cwd, deduplicated by file name
    // (nested roots can contribute rules with the same name).
    let mut rule_files: Vec<String> = Vec::new();
    if let Some(rules) = &rules {
        for rule in &rules.active_rules {
            if let Some(name) = rule.path.file_name() {
                if !rule_files.iter().any(|file| file == name) {
                    rule_files.push(name.to_owned());
                }
            }
        }
    }

    let project_skill_count = SkillManager::as_ref(app)
        .get_skills_for_working_directory(Some(&cwd_path), app)
        .iter()
        .filter(|skill| skill.is_project_skill())
        .count();

    let header = rules
        .as_ref()
        .map(|rules| rules.root_path.display_path())
        .unwrap_or_else(|| cwd.to_owned());
    column = column.child(blank_row()).child(
        TuiText::new(abbreviate_home_prefix(&header))
            .with_style(header_style)
            .truncate()
            .finish(),
    );

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
    let from_sections = changelog
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
        .take(MAX_CHANGELOG_BULLETS)
        .cloned()
        .collect::<Vec<_>>();
    if !from_sections.is_empty() {
        return from_sections;
    }
    // Newer payloads may only populate the markdown sections; fall back to
    // their top-level bullet lines.
    changelog
        .markdown_sections
        .iter()
        .flat_map(|section| section.markdown.lines())
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("* ").or_else(|| line.strip_prefix("- "))
        })
        .take(MAX_CHANGELOG_BULLETS)
        .map(ToOwned::to_owned)
        .collect()
}

/// A one-row spacer between sections.
fn blank_row() -> Box<dyn TuiElement> {
    TuiText::new(" ").truncate().finish()
}
