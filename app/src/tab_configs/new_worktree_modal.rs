use std::path::PathBuf;

use warpui::{
    elements::{
        Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
        Fill as ElementFill, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, Padding,
        ParentElement, Radius, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    keymap::FixedBinding,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        checkbox::Checkbox,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

/// Registers keybindings for the new-worktree modal (ESC to close).
pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_fixed_bindings(vec![FixedBinding::new(
        "escape",
        NewWorktreeModalAction::Escape,
        id!("NewWorktreeModal"),
    )]);
}

use warp_core::ui::theme::color::internal_colors;

use crate::{
    ai::persisted_workspace::PersistedWorkspace,
    appearance::Appearance,
    editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions},
    modal::ModalAction,
    tab_configs::{
        branch_picker::BranchPicker,
        repo_picker::{RepoPicker, RepoPickerEvent},
    },
};

/// Gap between sections in the modal body (repo picker, branch picker, checkbox).
const SECTION_GAP: f32 = 16.;
/// Gap between a section label and its picker below.
const LABEL_BOTTOM_MARGIN: f32 = 4.;
/// Horizontal padding for the modal body and footer (matches Figma px-24).
const CONTENT_HORIZONTAL_PADDING: f32 = 24.;
/// Header top padding (Figma: pt-24).
const HEADER_PADDING_TOP: f32 = 24.;
/// Header bottom padding (Figma: pb-12).
const HEADER_PADDING_BOTTOM: f32 = 12.;
/// Header title font size (Figma: 16px bold).
const HEADER_TITLE_FONT_SIZE: f32 = 16.;
/// Bottom padding of the form area above the footer.
const BODY_BOTTOM_PADDING: f32 = 16.;
/// Vertical padding of the footer bar.
const FOOTER_VERTICAL_PADDING: f32 = 12.;
/// Checkbox outer size (Figma: 16px with 12px inner container).
const CHECKBOX_SIZE: f32 = 16.;
/// Height of footer buttons (Figma: h-32).
const FOOTER_BUTTON_HEIGHT: f32 = 32.;
/// Horizontal padding inside footer buttons (Figma: px-12).
const FOOTER_BUTTON_HORIZONTAL_PADDING: f32 = 12.;
/// Gap between Cancel and Open buttons (Figma: gap-8).
const FOOTER_BUTTON_GAP: f32 = 8.;
/// Corner radius for footer buttons (Figma: rounded-4).
const FOOTER_BUTTON_RADIUS: Radius = Radius::Pixels(4.);
/// Size of the ESC keyboard shortcut badge (Figma: 14px tall, 10px font).
const ESC_BADGE_HEIGHT: f32 = 14.;
const ESC_BADGE_FONT_SIZE: f32 = 10.;
const ESC_BADGE_CORNER_RADIUS: Radius = Radius::Pixels(3.);
/// Size of the close (X) icon in the header.
const CLOSE_ICON_SIZE: f32 = 14.;
/// Font size for inline validation error messages.
const ERROR_FONT_SIZE: f32 = 12.;
/// Error shown when the user-entered worktree branch name contains invalid characters.
const INVALID_BRANCH_NAME_ERROR: &str =
    "Name can only contain letters, numbers, hyphens, and underscores";

/// Returns `true` if `name` is a valid worktree branch name.
///
/// Valid names contain only ASCII letters, digits, hyphens, and underscores.
/// The name must also be non-empty after trimming whitespace.
fn is_valid_worktree_branch_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Body view for the "New worktree" modal.
///
/// Renders a repo picker, branch picker, auto-generate checkbox, and
/// Cancel / Open footer. The workspace wraps this in a `Modal<NewWorktreeModal>`.
pub struct NewWorktreeModal {
    repo_picker: ViewHandle<RepoPicker>,
    branch_picker: ViewHandle<BranchPicker>,
    worktree_name_editor: ViewHandle<EditorView>,
    autogenerate_branch_name: bool,
    selected_repo: Option<String>,
    selected_branch: Option<String>,
    cancel_button_mouse_state: MouseStateHandle,
    open_button_mouse_state: MouseStateHandle,
    checkbox_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
}

pub enum NewWorktreeModalEvent {
    Close,
    Submit {
        repo: String,
        /// The base branch to create the worktree from.
        branch: String,
        /// `None` when autogenerate is enabled (the workspace handler
        /// will generate a name); `Some(name)` when the user typed a
        /// name manually.
        worktree_branch_name: Option<String>,
    },
    /// The user clicked "+ Add new repo..." in the repo picker; the workspace
    /// should open a folder picker and call [`NewWorktreeModal::on_new_repo_selected`].
    PickNewRepo,
}

#[derive(Clone, Copy, Debug)]
pub enum NewWorktreeModalAction {
    Cancel,
    Open,
    ToggleAutogenerate,
    Escape,
}

impl NewWorktreeModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let repo_picker = Self::build_repo_picker(None, ctx);
        let branch_picker = Self::build_branch_picker(None, ctx);
        let worktree_name_editor = Self::build_worktree_name_editor(ctx);

        Self {
            repo_picker,
            branch_picker,
            worktree_name_editor,
            autogenerate_branch_name: true,
            selected_repo: None,
            selected_branch: None,
            cancel_button_mouse_state: Default::default(),
            open_button_mouse_state: Default::default(),
            checkbox_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
        }
    }

    fn build_repo_picker(
        default: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<RepoPicker> {
        let picker = ctx.add_typed_action_view(|ctx| RepoPicker::new(default, ctx));
        ctx.subscribe_to_view(&picker, |me, _, event, ctx| match event {
            RepoPickerEvent::Selected(value) => {
                me.selected_repo = Some(value.clone());
                me.selected_branch = None;
                me.branch_picker.update(ctx, |picker, ctx| {
                    picker.refetch_branches(PathBuf::from(value.as_str()), ctx);
                });
                ctx.notify();
            }
            RepoPickerEvent::RequestAddRepo => {
                ctx.emit(NewWorktreeModalEvent::PickNewRepo);
            }
        });
        picker
    }

    fn build_branch_picker(
        cwd: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<BranchPicker> {
        let picker = ctx.add_typed_action_view(move |ctx| BranchPicker::new(cwd, None, ctx));
        ctx.subscribe_to_view(&picker, |me, _, value, ctx| {
            me.selected_branch = Some(value.clone());
            ctx.notify();
        });
        picker
    }

    fn build_worktree_name_editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions::default();
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("my-feature-branch", ctx);
            editor
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| match event {
            EditorEvent::Enter => me.try_submit(ctx),
            EditorEvent::Escape => ctx.emit(NewWorktreeModalEvent::Close),
            EditorEvent::Edited(_) => ctx.notify(),
            _ => {}
        });
        editor
    }

    /// Called by the workspace before making the modal visible.
    pub fn on_open(&mut self, cwd: Option<PathBuf>, ctx: &mut ViewContext<Self>) {
        self.autogenerate_branch_name = true;
        self.selected_repo = None;
        self.selected_branch = None;
        self.worktree_name_editor.update(ctx, |e, ctx| {
            e.clear_buffer_and_reset_undo_stack(ctx);
        });

        // Prefer the active session's cwd; fall back to the first known
        // workspace so that both pickers start populated even when no
        // terminal session is active yet.
        let effective_cwd = cwd.or_else(|| {
            PersistedWorkspace::as_ref(ctx)
                .workspaces()
                .next()
                .map(|ws| ws.path.clone())
        });

        let default_repo = effective_cwd
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        self.repo_picker = Self::build_repo_picker(default_repo, ctx);
        self.branch_picker = Self::build_branch_picker(effective_cwd, ctx);

        ctx.focus(&self.repo_picker);
        ctx.notify();
    }

    /// Called by the workspace when the modal is dismissed.
    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected_repo = None;
        self.selected_branch = None;
        ctx.notify();
    }

    /// Called by the workspace after the user adds a new repo via the folder picker.
    pub fn on_new_repo_selected(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let path_str = path.to_string_lossy().to_string();
        self.selected_repo = Some(path_str);
        self.repo_picker.update(ctx, |repo_picker, ctx| {
            repo_picker.refresh_and_select(path.clone(), ctx);
        });
        // Clear stale branch; refetch will auto-select the new repo's main
        // branch and emit it back via the subscription.
        self.selected_branch = None;
        self.branch_picker.update(ctx, |picker, ctx| {
            picker.refetch_branches(path, ctx);
        });
        ctx.notify();
    }

    fn try_submit(&mut self, ctx: &mut ViewContext<Self>) {
        let repo = self
            .selected_repo
            .clone()
            .or_else(|| self.repo_picker.as_ref(ctx).selected_value(ctx));

        let Some(repo) = repo else {
            return;
        };

        let branch = self
            .selected_branch
            .clone()
            .or_else(|| self.branch_picker.as_ref(ctx).selected_value(ctx));

        let Some(branch) = branch else {
            return;
        };

        let worktree_branch_name = if self.autogenerate_branch_name {
            None
        } else {
            let text = self.worktree_name_editor.as_ref(ctx).buffer_text(ctx);
            if !is_valid_worktree_branch_name(&text) {
                return;
            }
            Some(text.trim().to_string())
        };

        ctx.emit(NewWorktreeModalEvent::Submit {
            repo,
            branch,
            worktree_branch_name,
        });
    }

    fn render_section_label(text: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new_inline(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
        .with_margin_bottom(LABEL_BOTTOM_MARGIN)
        .finish()
    }
}

impl Entity for NewWorktreeModal {
    type Event = NewWorktreeModalEvent;
}

impl View for NewWorktreeModal {
    fn ui_name() -> &'static str {
        "NewWorktreeModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let has_repo = self.selected_repo.is_some()
            || self.repo_picker.as_ref(app).selected_value(app).is_some();
        let has_branch = self.selected_branch.is_some()
            || self.branch_picker.as_ref(app).selected_value(app).is_some();
        let worktree_name_text = self.worktree_name_editor.as_ref(app).buffer_text(app);
        let worktree_name_valid =
            self.autogenerate_branch_name || is_valid_worktree_branch_name(&worktree_name_text);
        let worktree_name_has_error = !self.autogenerate_branch_name
            && !worktree_name_text.trim().is_empty()
            && !is_valid_worktree_branch_name(&worktree_name_text);
        let can_submit = has_repo && has_branch && worktree_name_valid;

        // ── Header (custom — Modal wrapper has no title) ────────────────
        let header = {
            let title = Text::new_inline(
                "New worktree".to_string(),
                appearance.ui_font_family(),
                HEADER_TITLE_FONT_SIZE,
            )
            .with_color(theme.active_ui_text_color().into())
            .with_style(Properties::default().weight(Weight::Bold))
            .finish();

            // ESC keyboard shortcut badge (matches Figma keyboardBase component)
            let esc_badge = {
                let badge_bg = internal_colors::neutral_2(theme);
                let badge_text = Text::new_inline(
                    "ESC".to_string(),
                    appearance.ui_font_family(),
                    ESC_BADGE_FONT_SIZE,
                )
                .with_color(theme.foreground().into())
                .finish();

                Container::new(
                    ConstrainedBox::new(badge_text)
                        .with_height(ESC_BADGE_HEIGHT)
                        .finish(),
                )
                .with_horizontal_padding(2.)
                .with_background(badge_bg)
                .with_corner_radius(CornerRadius::with_all(ESC_BADGE_CORNER_RADIUS))
                .finish()
            };

            // X close icon
            let close_icon = ConstrainedBox::new(
                warp_core::ui::Icon::X
                    .to_warpui_icon(theme.sub_text_color(theme.background()))
                    .finish(),
            )
            .with_width(CLOSE_ICON_SIZE)
            .with_height(CLOSE_ICON_SIZE)
            .finish();

            let close_button = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(2.)
                .with_child(close_icon)
                .with_child(esc_badge)
                .finish();

            let close_hoverable = warpui::elements::Hoverable::new(
                self.close_button_mouse_state.clone(),
                move |_state| close_button,
            )
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ModalAction::Close);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Shrinkable::new(1., title).finish())
                    .with_child(close_hoverable)
                    .finish(),
            )
            .with_padding(
                Padding::uniform(0.)
                    .with_top(HEADER_PADDING_TOP)
                    .with_bottom(HEADER_PADDING_BOTTOM)
                    .with_left(CONTENT_HORIZONTAL_PADDING)
                    .with_right(CONTENT_HORIZONTAL_PADDING),
            )
            .finish()
        };

        // ── Form body ───────────────────────────────────────────────────
        let mut body = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Repo picker
        body.add_child(Self::render_section_label("Select repository", appearance));
        body.add_child(ChildView::new(&self.repo_picker).finish());

        // Branch picker (with gap)
        body.add_child(
            Container::new(Self::render_section_label("Select branch", appearance))
                .with_margin_top(SECTION_GAP)
                .finish(),
        );
        body.add_child(ChildView::new(&self.branch_picker).finish());

        // Checkbox
        let checkbox_label_color = theme.sub_text_color(theme.background());
        // Figma: 16px outer, 12px inner container, rounded 1.333px,
        // checked state: accent background, white checkmark.
        // Checkbox default has margin = font_size/2 on all sides; override
        // to zero so the checkbox aligns with the left edge of the labels.
        let zero_margin = Coords::uniform(0.);
        let checkbox_default = UiComponentStyles {
            font_size: Some(CHECKBOX_SIZE),
            border_width: Some(1.),
            border_color: Some(checkbox_label_color.into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(1.))),
            margin: Some(zero_margin),
            ..Default::default()
        };
        let checkbox_checked = UiComponentStyles {
            font_size: Some(CHECKBOX_SIZE),
            background: Some(theme.accent_button_color().into()),
            font_color: Some(theme.main_text_color(theme.accent_button_color()).into()),
            border_width: Some(1.),
            border_color: Some(theme.accent_button_color().into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(1.))),
            margin: Some(zero_margin),
            ..Default::default()
        };
        let checkbox_element = Checkbox::new(
            self.checkbox_mouse_state.clone(),
            checkbox_default,
            None,
            Some(checkbox_checked),
            None,
        )
        .check(self.autogenerate_branch_name)
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(NewWorktreeModalAction::ToggleAutogenerate);
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let checkbox_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(checkbox_element)
            .with_child(
                Text::new_inline(
                    "Autogenerate worktree branch name".to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(checkbox_label_color.into())
                .finish(),
            )
            .finish();

        body.add_child(
            Container::new(checkbox_row)
                .with_margin_top(SECTION_GAP)
                .finish(),
        );

        // Worktree branch name text field — shown when autogenerate is unchecked.
        if !self.autogenerate_branch_name {
            body.add_child(
                Container::new(Self::render_section_label(
                    "Worktree branch name",
                    appearance,
                ))
                .with_margin_top(SECTION_GAP)
                .finish(),
            );
            body.add_child(ChildView::new(&self.worktree_name_editor).finish());

            if worktree_name_has_error {
                body.add_child(
                    Container::new(
                        Text::new_inline(
                            INVALID_BRANCH_NAME_ERROR.to_string(),
                            appearance.ui_font_family(),
                            ERROR_FONT_SIZE,
                        )
                        .with_color(theme.ui_error_color())
                        .finish(),
                    )
                    .with_margin_top(LABEL_BOTTOM_MARGIN)
                    .finish(),
                );
            }
        }

        let body_container = Container::new(body.finish())
            .with_padding(
                Padding::uniform(0.)
                    .with_left(CONTENT_HORIZONTAL_PADDING)
                    .with_right(CONTENT_HORIZONTAL_PADDING)
                    .with_bottom(BODY_BOTTOM_PADDING),
            )
            .finish();

        // ── Footer ──────────────────────────────────────────────────────
        // Figma: text-only buttons, semibold 14px, h-32, px-12, no background.
        // Cancel uses main text color; Open uses disabled text color when no repo.
        let text_button_base = UiComponentStyles {
            font_size: Some(appearance.ui_font_size() + 2.),
            font_weight: Some(Weight::Semibold),
            height: Some(FOOTER_BUTTON_HEIGHT),
            padding: Some(
                Coords::uniform(0.)
                    .left(FOOTER_BUTTON_HORIZONTAL_PADDING)
                    .right(FOOTER_BUTTON_HORIZONTAL_PADDING),
            ),
            background: Some(ElementFill::None),
            border_width: Some(0.),
            border_radius: Some(CornerRadius::with_all(FOOTER_BUTTON_RADIUS)),
            ..Default::default()
        };

        let main_text = theme.main_text_color(theme.background());

        let cancel_button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.cancel_button_mouse_state.clone())
            .with_text_label(crate::i18n::tr_static(app, "Cancel").to_string())
            .with_style(text_button_base)
            .with_style(UiComponentStyles {
                font_color: Some(main_text.into()),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(NewWorktreeModalAction::Cancel);
            })
            .finish();

        let open_button = {
            let font_color = if can_submit {
                main_text
            } else {
                theme.disabled_text_color(theme.background())
            };

            let mut builder = appearance
                .ui_builder()
                .button(ButtonVariant::Text, self.open_button_mouse_state.clone())
                .with_text_label(crate::i18n::tr_static(app, "Open").to_string())
                .with_style(text_button_base)
                .with_style(UiComponentStyles {
                    font_color: Some(font_color.into()),
                    ..Default::default()
                });

            if !can_submit {
                builder = builder.with_cursor(None);
            }

            if can_submit {
                builder
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(NewWorktreeModalAction::Open);
                    })
                    .finish()
            } else {
                builder.build().disable().finish()
            }
        };

        // The border-top spans the full modal width; horizontal padding
        // is only on the button row inside.
        let button_row = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(FOOTER_BUTTON_GAP)
                .with_child(cancel_button)
                .with_child(open_button)
                .finish(),
        )
        .with_padding(
            Padding::uniform(FOOTER_VERTICAL_PADDING)
                .with_left(CONTENT_HORIZONTAL_PADDING)
                .with_right(CONTENT_HORIZONTAL_PADDING),
        )
        .finish();

        let footer = Container::new(button_row)
            .with_border(Border::top(1.).with_border_fill(theme.outline()))
            .finish();

        // ── Assemble ────────────────────────────────────────────────────
        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(body_container)
            .with_child(footer)
            .finish()
    }
}

impl TypedActionView for NewWorktreeModal {
    type Action = NewWorktreeModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            NewWorktreeModalAction::Cancel | NewWorktreeModalAction::Escape => {
                ctx.emit(NewWorktreeModalEvent::Close);
            }
            NewWorktreeModalAction::Open => self.try_submit(ctx),
            NewWorktreeModalAction::ToggleAutogenerate => {
                self.autogenerate_branch_name = !self.autogenerate_branch_name;
                ctx.notify();
            }
        }
    }
}
