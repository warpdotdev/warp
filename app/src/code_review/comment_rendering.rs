//! Shared rendering helpers for displaying code review comments.
//!
//! These functions are used by both the `CommentListView` (in the code review panel)
//! and the blocklist's imported comments rendering.

use std::rc::Rc;

use chrono::{Duration, Local};
use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors::{neutral_1, neutral_2, text_sub};
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::elements::{
    Border, ChildView, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, ScrollbarWidth,
    Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::units::Pixels;
use warpui::{AppContext, Element, EventContext, SingletonEntity, View, ViewContext, ViewHandle};

use crate::appearance::Appearance;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::comment_editor::create_readonly_comment_markdown_editor;
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, LineDiffContent,
};
use crate::editor::InteractionState;
use crate::notebooks::editor::view::RichTextEditorView;
use crate::util::time_format::human_readable_approx_duration;

/// Configuration for making the comment header clickable.
pub(crate) struct HeaderClickHandler {
    pub mouse_state: MouseStateHandle,
    pub on_click: Rc<dyn Fn(&mut EventContext) + 'static>,
}

/// Wraps the given content element in the standard comment card chrome
/// (rounded corners, neutral background, outline border).
fn comment_card_container(
    content: Box<dyn Element>,
    theme: &warp_core::ui::theme::WarpTheme,
) -> Box<dyn Element> {
    Container::new(content)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_background(Fill::Solid(neutral_1(theme)))
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .finish()
}

/// Renders a collapsed comment card showing only the file-path header and an
/// optional trailing element (e.g. action buttons).
fn render_collapsed_comment_card(
    title: &str,
    is_outdated: bool,
    header_trailing_element: Option<Box<dyn Element>>,
    on_header_click: Option<&HeaderClickHandler>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let header = render_comment_file_path_header(
        title,
        is_outdated,
        header_trailing_element,
        CornerRadius::with_all(Radius::Pixels(8.)),
        on_header_click,
        appearance,
    );

    comment_card_container(header, theme)
}

fn render_comment_file_path_header(
    title: &str,
    is_outdated: bool,
    trailing_element: Option<Box<dyn Element>>,
    corner_radius: CornerRadius,
    on_header_click: Option<&HeaderClickHandler>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let file_path_text = Text::new(
        title.to_owned(),
        appearance.ui_font_family(),
        appearance.ui_font_size() + 2.,
    )
    .soft_wrap(false)
    .with_clip(ClipConfig::start())
    .with_color(
        theme
            .main_text_color(Fill::Solid(neutral_2(theme)))
            .into_solid(),
    )
    .finish();

    let mut header_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(Shrinkable::new(1., file_path_text).finish());

    if is_outdated {
        let yellow_border: ColorU = theme.terminal_colors().normal.yellow.into();
        let yellow_text: ColorU = theme.terminal_colors().bright.yellow.into();

        let outdated_chip = Container::new(
            Text::new(
                "Outdated",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(yellow_text)
            .finish(),
        )
        .with_margin_left(8.)
        .with_horizontal_padding(8.)
        .with_vertical_padding(4.)
        .with_border(Border::all(1.).with_border_fill(Fill::Solid(yellow_border)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
        .with_background(Fill::Solid(neutral_2(theme)))
        .finish();

        header_row.add_child(outdated_chip);
    }

    if let Some(trailing) = trailing_element {
        header_row = header_row
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);
        header_row.add_child(Container::new(trailing).with_margin_left(8.).finish());
    }

    let container = Container::new(header_row.finish())
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .with_background(Fill::Solid(neutral_2(theme)))
        .with_corner_radius(corner_radius)
        .finish();

    if let Some(click_handler) = on_header_click {
        let callback = Rc::clone(&click_handler.on_click);
        Hoverable::new(click_handler.mouse_state.clone(), |_| container)
            .on_click(move |ctx, _, _| {
                callback(ctx);
            })
            .with_cursor(Cursor::PointingHand)
            .with_defer_events_to_children()
            .finish()
    } else {
        container
    }
}

fn render_comment_text_section(
    comment_editor: &ViewHandle<RichTextEditorView>,
    last_updated_duration: Duration,
    is_imported_from_github: bool,
    metadata_trailing_element: Option<Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = Fill::Solid(neutral_1(theme));

    let mut left_section = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(8.);

    if is_imported_from_github {
        left_section.add_child(
            Text::new(
                "From GitHub".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(text_sub(theme, background))
            .finish(),
        );
    }

    left_section.add_child(
        Text::new(
            human_readable_approx_duration(last_updated_duration, true /* sentence_case */),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(
            appearance
                .theme()
                .disabled_text_color(background)
                .into_solid(),
        )
        .finish(),
    );

    let mut metadata_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(left_section.finish());

    if let Some(trailing) = metadata_trailing_element {
        metadata_row = metadata_row
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(trailing);
    } else {
        metadata_row = metadata_row.with_main_axis_alignment(MainAxisAlignment::Start);
    }

    let comment_content_child = ChildView::new(comment_editor).finish();

    let column = Flex::column()
        .with_children([metadata_row.finish(), comment_content_child])
        .finish();

    Container::new(column)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_uniform_padding(8.)
        .finish()
}

/// Creates a read-only, syntax-highlighted code editor for displaying static diff content.
///
/// The editor is configured with infinite height, no diff UI, no line numbers, and selectable
/// interaction state. The buffer is populated with the diff's original text and syntax
/// highlighting is set based on the file path.
fn create_static_diff_content_editor<V: View>(
    content: &LineDiffContent,
    file_path: Option<&LocalOrRemotePath>,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<CodeEditorView> {
    let editor = ctx.add_typed_action_view(|ctx| {
        CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight),
            ctx,
        )
        .with_can_show_diff_ui(false)
        .with_show_line_numbers(false)
        .with_horizontal_scrollbar_appearance(ScrollableAppearance::new(
            ScrollbarWidth::Auto,
            false,
        ))
    });
    editor.update(ctx, |view, ctx| {
        view.set_show_current_line_highlights(false, ctx);
        view.set_interaction_state(InteractionState::Selectable, ctx);
        let original_text = content.original_text();
        let state = InitialBufferState::plain_text(original_text.trim());
        view.reset(state, ctx);
        if let Some(file_path) = file_path {
            let language_path = file_path.path_component();
            view.set_language_with_path(&language_path, ctx);
        }
    });
    editor
}

fn render_static_diff_content_element(
    editor: &ViewHandle<CodeEditorView>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    Container::new(ChildView::new(editor).finish())
        .with_background(theme.background())
        .with_horizontal_padding(8.)
        .with_vertical_padding(4.)
        .finish()
}

/// How to display the diff content section of a comment card.
enum CommentDiffContent {
    /// Diff content rendered via a live editor lens. The element is provided at render time.
    EditorLens,
    /// Diff content rendered via a static, read-only `CodeEditorView`.
    StaticEditor(ViewHandle<CodeEditorView>),
}

/// A shared UI component for a single code review comment card.
///
/// Used by both `CommentListView` (code review panel) and the blocklist's
/// imported comments. Owns the view handles for the comment body editor and
/// (optionally) a static diff editor, plus the underlying comment data.
pub(crate) struct CommentViewCard {
    comment_editor: ViewHandle<RichTextEditorView>,
    diff_content: Option<CommentDiffContent>,
    source: AttachedReviewComment,
    title: String,
    last_updated_duration: Duration,
    is_collapsed: bool,
}

impl CommentViewCard {
    pub(crate) fn new<V: View>(
        source: AttachedReviewComment,
        always_use_static_diff: bool,
        disable_scrolling: bool,
        max_width: Option<Pixels>,
        repo_path: Option<&LocalOrRemotePath>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let comment_editor = create_readonly_comment_markdown_editor(
            &source.content,
            disable_scrolling,
            max_width,
            ctx,
        );
        let diff_content = Self::diff_content_for_comment(&source, always_use_static_diff, ctx);
        let title = Self::compute_title(&source, repo_path);
        let last_updated_duration = Local::now() - source.last_update_time;
        Self {
            comment_editor,
            diff_content,
            source,
            title,
            last_updated_duration,
            is_collapsed: false,
        }
    }

    fn diff_content_for_comment<V: View>(
        comment: &AttachedReviewComment,
        always_use_static_diff: bool,
        ctx: &mut ViewContext<V>,
    ) -> Option<CommentDiffContent> {
        if let AttachedReviewCommentTarget::Line {
            absolute_file_path,
            content,
            ..
        } = &comment.target
        {
            if always_use_static_diff || comment.outdated {
                Some(CommentDiffContent::StaticEditor(
                    // Language detection only needs the path component (extension).
                    create_static_diff_content_editor(content, Some(absolute_file_path), ctx),
                ))
            } else {
                Some(CommentDiffContent::EditorLens)
            }
        } else {
            None
        }
    }

    pub(crate) fn toggle_collapsed(&mut self) {
        self.is_collapsed = !self.is_collapsed;
    }

    pub(crate) fn is_collapsed(&self) -> bool {
        self.is_collapsed
    }

    /// Updates the comment data and resets the body editor with the new content.
    pub(crate) fn update_source<V: View>(
        &mut self,
        new_source: AttachedReviewComment,
        repo_path: Option<&LocalOrRemotePath>,
        ctx: &mut ViewContext<V>,
    ) {
        self.comment_editor.update(ctx, |editor, ctx| {
            editor.model().update(ctx, |model, ctx| {
                model.reset_with_markdown(&new_source.content, ctx);
            });
        });
        self.source = new_source;
        self.title = Self::compute_title(&self.source, repo_path);
    }

    /// Renders the comment card. When collapsed, only the header and trailing
    /// element are shown. When expanded, the full card with diff content and
    /// comment text is rendered.
    ///
    /// When `diff_content` is `EditorLens`, the caller must supply the live element via
    /// `editor_lens_element`. For `StaticEditor` or `None` it is ignored.
    ///
    /// When `on_header_click` is provided, the entire header area becomes clickable.
    pub(crate) fn render(
        &self,
        editor_lens_element: Option<Box<dyn Element>>,
        header_trailing_element: Option<Box<dyn Element>>,
        metadata_trailing_element: Option<Box<dyn Element>>,
        on_header_click: Option<&HeaderClickHandler>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if self.is_collapsed {
            return render_collapsed_comment_card(
                &self.title,
                self.source.outdated,
                header_trailing_element,
                on_header_click,
                app,
            );
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut card = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        card.add_child(render_comment_file_path_header(
            &self.title,
            self.source.outdated,
            header_trailing_element,
            CornerRadius::with_top(Radius::Pixels(8.)),
            on_header_click,
            appearance,
        ));

        match &self.diff_content {
            Some(CommentDiffContent::EditorLens) => {
                if let Some(lens) = editor_lens_element {
                    card.add_child(lens);
                }
            }
            Some(CommentDiffContent::StaticEditor(editor)) => {
                card.add_child(render_static_diff_content_element(editor, app));
            }
            None => {}
        }

        card.add_child(render_comment_text_section(
            &self.comment_editor,
            self.last_updated_duration,
            self.source.origin.is_imported_from_github(),
            metadata_trailing_element,
            appearance,
        ));
        comment_card_container(card.finish(), theme)
    }

    pub(crate) fn source(&self) -> &AttachedReviewComment {
        &self.source
    }

    pub(crate) fn comment_editor(&self) -> &ViewHandle<RichTextEditorView> {
        &self.comment_editor
    }

    pub(crate) fn static_diff_editor(&self) -> Option<&ViewHandle<CodeEditorView>> {
        match &self.diff_content {
            Some(CommentDiffContent::StaticEditor(editor)) => Some(editor),
            _ => None,
        }
    }

    pub(crate) fn uses_editor_lens(&self) -> bool {
        matches!(self.diff_content, Some(CommentDiffContent::EditorLens))
    }

    /// Refreshes the cached `last_updated_duration` to the current time.
    pub(crate) fn refresh_last_updated_duration(&mut self) {
        self.last_updated_duration = Local::now() - self.source.last_update_time;
    }

    fn compute_title(
        source: &AttachedReviewComment,
        repo_path: Option<&LocalOrRemotePath>,
    ) -> String {
        let file_path = source.target.absolute_file_path().map(|p| {
            repo_path
                .and_then(|rp| rp.strip_repo_prefix(p))
                .unwrap_or_else(|| p.display_path())
        });
        let line_number = source.target.line_number().map(|lc| lc.as_u32() + 1);

        match (file_path, line_number) {
            (Some(path), Some(line)) => format!("{path}:{line}"),
            (Some(path), None) => path,
            _ => source
                .head()
                .map(|head| head.title())
                .unwrap_or_else(|| "Review Comment".to_string()),
        }
    }
}
