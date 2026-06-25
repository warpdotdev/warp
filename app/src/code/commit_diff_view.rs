//! View for the main-area read-only diff pane: shows what a single commit changed
//! in a single file (`commit~1..commit`, i.e. what this one commit itself changed).
//! Triggered by clicking a changed file in the Git Graph commit detail.
//!
//! Reuses the editor's diff overlay mechanism: first feeds the file's full content
//! at its parent commit into the editor as the diff base, then overlays this commit's
//! deltas for the file (converted from unified diff hunks). It does NOT register a
//! FileModel (does not call [`InlineDiffView::register_file`]) → the editor has no
//! file backend, staying read-only and unsavable, which avoids accidentally writing
//! a historical revision back to the working-tree file.

use std::path::Path;

use ai::diff_validation::DiffType;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::{Align, ChildView, Text};
use warpui::text_layout::ClipConfig;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::code::diff_viewer::{DiffViewer, DisplayMode};
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::code::inline_diff::InlineDiffView;
use crate::code_review::diff_state::{convert_hunks_to_diff_deltas, DiffHunk};
use crate::editor::InteractionState;
use crate::menu::{MenuItem, MenuItemFields};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};

/// Action for the commit diff pane's header overflow menu.
#[derive(Debug, Clone)]
pub enum CommitDiffMenuAction {
    /// Maximize / restore this pane.
    ToggleMaximized,
}

/// How the read-only diff pane should present a file. Most files render their
/// textual diff; binary files and symlinks have no meaningful text diff, so the
/// pane shows a single centered placeholder line instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffPreview {
    /// A normal textual diff — render `base_content` overlaid with `hunks`.
    Text,
    /// git reports a binary file (no parsable hunks); feeding its bytes to the
    /// text editor would just render garbage.
    Binary,
    /// A symbolic link. Its only content is the target path it points to, so a
    /// textual diff is a lone one-line entry that looks broken — show the target
    /// in the placeholder instead.
    Symlink { target: String },
}

/// Read-only view of what a single commit changed in a single file.
pub struct CommitDiffView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// The inline diff view that actually renders the diff (editor + overlaid deltas).
    diff_view: ViewHandle<InlineDiffView>,
    /// Header / tab title: `file_name @ short_hash`.
    header_title: String,
    /// How to present this file. For `Binary`/`Symlink` the diff editor is empty
    /// and a single centered placeholder line is rendered instead.
    preview: DiffPreview,
}

impl CommitDiffView {
    /// `repo_relative_path` repo-relative path; `short_hash` short commit hash (title only);
    /// `base_content` the file's full content at the parent commit (empty string for an
    /// added file / the root commit); `hunks` this commit's unified diff hunks for the file;
    /// `preview` how to present the file (`Text` renders the diff; `Binary`/`Symlink` leave
    /// `base_content`/`hunks` empty and show a centered placeholder instead).
    pub fn new(
        repo_relative_path: String,
        short_hash: String,
        base_content: String,
        hunks: Vec<DiffHunk>,
        preview: DiffPreview,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let header_title = Self::compute_title(&repo_relative_path, &short_hash);

        let pane_configuration = ctx.add_model({
            let title = header_title.clone();
            // Set the title via set_title (rather than passing it to ::new) so the tab
            // renders immediately, following CodeDiffPane.
            move |ctx| {
                let mut cfg = PaneConfiguration::new("");
                cfg.set_title(title, ctx);
                cfg
            }
        });

        let diff_view = Self::build_diff_view(&repo_relative_path, &base_content, &hunks, ctx);

        Self {
            pane_configuration,
            focus_handle: None,
            diff_view,
            header_title,
            preview,
        }
    }

    /// Reuse the same pane to open another file's diff: replace the content and update
    /// the title while keeping the pane alive (header / close button / focus state all
    /// unchanged). Used for clicking through multiple files into the same diff pane.
    pub fn load(
        &mut self,
        repo_relative_path: String,
        short_hash: String,
        base_content: String,
        hunks: Vec<DiffHunk>,
        preview: DiffPreview,
        ctx: &mut ViewContext<Self>,
    ) {
        self.header_title = Self::compute_title(&repo_relative_path, &short_hash);
        self.pane_configuration.update(ctx, {
            let title = self.header_title.clone();
            move |cfg, ctx| cfg.set_title(title, ctx)
        });
        self.diff_view = Self::build_diff_view(&repo_relative_path, &base_content, &hunks, ctx);
        self.preview = preview;
        ctx.notify();
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// Header / tab title: `file_name @ short_hash`.
    fn compute_title(repo_relative_path: &str, short_hash: &str) -> String {
        let file_name = Path::new(repo_relative_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo_relative_path.to_string());
        format!("{file_name} @ {short_hash}")
    }

    /// Build the read-only diff view: feed the parent revision's full content into the
    /// editor as the base, overlay this commit's deltas; do not register a FileModel
    /// (no file backend → unsavable), and force Selectable (FullPane is editable by default).
    fn build_diff_view(
        repo_relative_path: &str,
        base_content: &str,
        hunks: &[DiffHunk],
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<InlineDiffView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            // Disable the change bar widening on hover/active (3→8px) so every hunk marker
            // stays a uniform 3px wide.
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight)
                    .lazy_layout(),
                ctx,
            )
            .disable_diff_indicator_expansion_on_hover()
        });
        editor.update(ctx, |editor_view, ctx| {
            editor_view.set_language_with_local_path(Path::new(repo_relative_path), ctx);
            editor_view.reset(InitialBufferState::plain_text(base_content), ctx);
        });

        let diff_type = DiffType::update(convert_hunks_to_diff_deltas(hunks), None);
        let standardized_path = StandardizedPath::try_new(repo_relative_path).ok();
        let diff_view = ctx.add_typed_action_view(|ctx| {
            InlineDiffView::new(
                editor.clone(),
                Some(diff_type),
                Some(DisplayMode::FullPane),
                standardized_path,
                ctx,
            )
        });

        diff_view.update(ctx, |view, ctx| {
            view.editor().clone().update(ctx, |editor_view, ctx| {
                editor_view.set_interaction_state(InteractionState::Selectable, ctx);
            });
        });

        // When clicking the editor content takes focus, bubble it up as PaneEvent::FocusSelf
        // so the pane group activates this pane (otherwise clicking the diff content area
        // doesn't activate it and there's no active-pane indicator in the top-left — the
        // click is consumed by the editor selection and never propagates to the pane).
        let editor = diff_view.as_ref(ctx).editor().clone();
        ctx.subscribe_to_view(&editor, |_me, _editor, event, ctx| {
            if matches!(event, CodeEditorEvent::Focused) {
                ctx.emit(PaneEvent::FocusSelf);
            }
        });

        diff_view
    }

    /// Refresh the top-left active-pane indicator (`show_active_pane_indicator`) based on
    /// the current focus state: shown only when this pane is in a split and is the focused
    /// pane. Terminal panes drive this flag themselves via `is_active_session`; non-terminal
    /// panes have no such driver by default, so we sync it explicitly off `is_focused_pane`
    /// here — otherwise the active-pane indicator never appears when this read-only diff pane
    /// is focused.
    fn refresh_active_pane_indicator(&mut self, ctx: &mut ViewContext<Self>) {
        let is_focused_pane = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.split_pane_state(ctx).is_focused_pane());
        self.pane_configuration.update(ctx, move |cfg, ctx| {
            cfg.set_show_active_pane_indicator(is_focused_pane, ctx);
        });
    }

    /// Placeholder shown for files that have no meaningful text diff (binary /
    /// symlink): a single dimmed line centered both horizontally and vertically.
    /// `Align` fills the pane's bounds and centers its child, so the line stays
    /// in the middle regardless of pane size.
    fn render_placeholder(text: String, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let line = Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();
        Align::new(line).finish()
    }
}

impl Entity for CommitDiffView {
    type Event = PaneEvent;
}

impl TypedActionView for CommitDiffView {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl View for CommitDiffView {
    fn ui_name() -> &'static str {
        "CommitDiffView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        match &self.preview {
            DiffPreview::Text => ChildView::new(&self.diff_view).finish(),
            DiffPreview::Binary => Self::render_placeholder(
                "This is a binary file and won't be previewed.".to_string(),
                app,
            ),
            DiffPreview::Symlink { target } => {
                Self::render_placeholder(format!("Symbolic link → {target}"), app)
            }
        }
    }
}

impl BackingView for CommitDiffView {
    type PaneHeaderOverflowMenuAction = CommitDiffMenuAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            CommitDiffMenuAction::ToggleMaximized => ctx.emit(PaneEvent::ToggleMaximized),
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        let editor = self.diff_view.as_ref(ctx).editor().clone();
        ctx.focus(&editor);
    }

    fn pane_header_overflow_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<Self::PaneHeaderOverflowMenuAction>> {
        let is_maximized = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_maximized(ctx));
        vec![MenuItemFields::toggle_pane_action(is_maximized)
            .with_on_select_action(CommitDiffMenuAction::ToggleMaximized)
            .into_item()]
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: self.header_title.clone(),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            // Always show the close button and overflow menu (not just on hover), making it
            // easy to close/maximize the read-only diff pane.
            options: StandardHeaderOptions {
                always_show_icons: true,
                ..StandardHeaderOptions::default()
            },
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, ctx: &mut ViewContext<Self>) {
        // Subscribe to pane group focus changes: sync the top-left active-pane indicator
        // whenever this pane becomes / stops being the focused pane in the split.
        ctx.subscribe_to_model(focus_handle.focus_state_handle(), |me, _, event, ctx| {
            let affected = me
                .focus_handle
                .as_ref()
                .is_some_and(|h| h.is_affected(event));
            if affected {
                me.refresh_active_pane_indicator(ctx);
            }
        });
        self.focus_handle = Some(focus_handle);
        self.refresh_active_pane_indicator(ctx);
    }
}
