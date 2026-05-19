//! Picker for the cloud-agent worker host slug.
//!
//! In list mode it shows a dropdown styled to match the other orchestration
//! pickers; in custom mode it swaps the top bar for an inline editor that
//! accepts a self-hosted worker slug. The layout mirrors the Oz webapp's
//! host selector: workspace default first (badged "Default"), then warp,
//! then the user's most recent custom slug, then a "Custom host…" entry.

use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    PositionedElementAnchor, Radius,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use warp_core::ui::theme::Fill;

use crate::ai::blocklist::inline_action::orchestration_controls::{
    self as oc, ORCHESTRATION_PICKER_BORDER_WIDTH, ORCHESTRATION_PICKER_FONT_SIZE,
    ORCHESTRATION_PICKER_HEIGHT, ORCHESTRATION_PICKER_RADIUS, ORCHESTRATION_WARP_WORKER_HOST,
};
use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::menu::{MenuItem, MenuItemFields};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::dropdown::{
    Dropdown, DropdownAction, DropdownEvent, DropdownStyle, DROPDOWN_PADDING,
};

// ── Public API types ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum HostPickerEvent {
    /// Emitted with a non-empty, trimmed slug whenever the user picks a
    /// known host or commits a custom entry.
    HostChanged { slug: String },
    /// Emitted when the menu closes or the inline editor blurs, so the
    /// parent can refocus its own input.
    Closed,
}

const CUSTOM_HOST_LABEL: &str = "Custom host…";
const DEFAULT_BADGE: &str = "Default";
const EDITOR_PLACEHOLDER: &str = "my-worker-host";

// ── Internal action plumbing ────────────────────────────────────────

/// Dispatched by the inner dropdown items and the inline cancel button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalAction {
    /// Pick a known host (warp, workspace default, or a recent slug).
    SelectKnown(String),
    /// Switch to custom-mode text input.
    EnterCustomMode,
    /// Exit custom mode without committing the editor contents.
    CancelCustom,
}

// ── View ────────────────────────────────────────────────────────────

pub struct HostPicker {
    /// The slug that would be sent to the server if dispatched now.
    current_slug: String,
    /// Admin-configured workspace default, when set.
    default_host: Option<String>,
    /// User's most-recent custom host, deduped against warp / default.
    recent_host: Option<String>,
    dropdown: ViewHandle<Dropdown<InternalAction>>,
    editor: ViewHandle<EditorView>,
    clear_mouse_state: MouseStateHandle,
    is_custom_mode: bool,
    /// Snapshot taken when the editor was opened so cancel can revert.
    slug_before_edit: Option<String>,
}

impl HostPicker {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let (_styles, colors) = oc::picker_styles(Appearance::as_ref(ctx));

        // Inner dropdown — styled to match the other orchestration pickers.
        let dropdown = ctx.add_typed_action_view(|ctx_dropdown| {
            let mut dropdown = Dropdown::<InternalAction>::new(ctx_dropdown);
            dropdown.set_use_overlay_layer(false, ctx_dropdown);
            dropdown.set_main_axis_size(MainAxisSize::Max, ctx_dropdown);
            dropdown.set_style(DropdownStyle::ActionButtonSecondary, ctx_dropdown);
            dropdown.set_top_bar_height(ORCHESTRATION_PICKER_HEIGHT, ctx_dropdown);
            dropdown.set_top_bar_max_width(f32::INFINITY);
            dropdown.set_padding(colors.padding, ctx_dropdown);
            dropdown.set_border_radius(colors.corner_radius, ctx_dropdown);
            dropdown.set_background(colors.background, ctx_dropdown);
            dropdown.set_border_width(ORCHESTRATION_PICKER_BORDER_WIDTH, ctx_dropdown);
            dropdown.set_font_size(ORCHESTRATION_PICKER_FONT_SIZE, ctx_dropdown);
            dropdown
        });
        ctx.subscribe_to_view(&dropdown, |me, _, event, ctx| {
            if let DropdownEvent::Close = event {
                // Don't propagate Closed while transitioning into custom
                // mode — the parent would refocus itself, blur the editor
                // we just focused, and the resulting commit-on-blur would
                // immediately revert us back out of custom mode.
                if me.is_custom_mode {
                    return;
                }
                ctx.emit(HostPickerEvent::Closed);
                ctx.notify();
            }
        });

        let editor = ctx.add_typed_action_view(|ctx_editor| {
            let appearance = Appearance::as_ref(ctx_editor);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(appearance.ui_font_size()), appearance),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    select_all_on_focus: true,
                    ..Default::default()
                },
                ctx_editor,
            );
            editor.set_placeholder_text(EDITOR_PLACEHOLDER, ctx_editor);
            editor
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let mut me = Self {
            current_slug: ORCHESTRATION_WARP_WORKER_HOST.to_string(),
            default_host: None,
            recent_host: None,
            dropdown,
            editor,
            clear_mouse_state: MouseStateHandle::default(),
            is_custom_mode: false,
            slug_before_edit: None,
        };
        me.repopulate_menu(ctx);
        me.sync_dropdown_selection(ctx);
        me
    }

    // ── Public API ──────────────────────────────────────────────────

    /// Replaces the default and recent menu rows. Pass `None` to omit one.
    pub fn set_options(
        &mut self,
        default_host: Option<String>,
        recent_host: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.default_host = default_host.filter(|s| !s.trim().is_empty());
        self.recent_host = recent_host.filter(|s| !s.trim().is_empty());
        self.repopulate_menu(ctx);
        self.sync_dropdown_selection(ctx);
        ctx.notify();
    }

    /// Pass `true` to paint the open menu in the overlay layer (avoids
    /// being visually covered by sibling pickers below the host picker).
    pub fn set_use_overlay_layer(&mut self, use_overlay_layer: bool, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |dropdown, ctx_dropdown| {
            dropdown.set_use_overlay_layer(use_overlay_layer, ctx_dropdown);
        });
    }

    /// Anchors the open menu (e.g. flip upward to avoid covering siblings).
    pub fn set_menu_position(
        &mut self,
        element_anchor: PositionedElementAnchor,
        child_anchor: ChildAnchor,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dropdown.update(ctx, |dropdown, ctx_dropdown| {
            dropdown.set_menu_position(element_anchor, child_anchor, ctx_dropdown);
        });
    }

    /// Sets the displayed slug. Unknown slugs switch the picker into custom
    /// mode pre-filled with the slug. Empty input falls back to `"warp"`.
    pub fn set_selected(&mut self, slug: &str, ctx: &mut ViewContext<Self>) {
        let effective = normalize_slug(slug);
        let is_known = self.is_known_option(&effective);
        self.current_slug = effective.clone();
        if is_known {
            self.is_custom_mode = false;
            self.sync_dropdown_selection(ctx);
        } else {
            self.enter_custom_mode_with_slug(&effective, ctx);
        }
        ctx.notify();
    }

    // ── Internals ───────────────────────────────────────────────────

    fn is_known_option(&self, slug: &str) -> bool {
        if slug.eq_ignore_ascii_case(ORCHESTRATION_WARP_WORKER_HOST) {
            return true;
        }
        if self.default_host.as_deref() == Some(slug) {
            return true;
        }
        if self.recent_host.as_deref() == Some(slug) {
            return true;
        }
        false
    }

    fn repopulate_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let items = build_menu_items(self.default_host.as_deref(), self.recent_host.as_deref());
        self.dropdown.update(ctx, |dropdown, ctx_dropdown| {
            dropdown.set_rich_items(items, ctx_dropdown);
        });
    }

    fn sync_dropdown_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let label = menu_label_for(&self.current_slug, self.default_host.as_deref());
        self.dropdown.update(ctx, |dropdown, ctx_dropdown| {
            dropdown.set_selected_by_name(&label, ctx_dropdown);
        });
    }

    fn enter_custom_mode_with_slug(&mut self, slug: &str, ctx: &mut ViewContext<Self>) {
        self.is_custom_mode = true;
        self.slug_before_edit = Some(self.current_slug.clone());
        let initial = if slug.eq_ignore_ascii_case(ORCHESTRATION_WARP_WORKER_HOST) {
            String::new()
        } else {
            slug.to_string()
        };
        self.editor.update(ctx, |editor, editor_ctx| {
            editor.set_buffer_text_ignoring_undo(&initial, editor_ctx);
        });
        ctx.focus(&self.editor);
    }

    /// Commits the editor contents. Empty input reverts to the previous slug.
    fn commit_custom(&mut self, ctx: &mut ViewContext<Self>) {
        // Trim manually here — we must distinguish empty input (revert) from
        // a literal "warp" entry (commit). `normalize_slug` collapses both
        // into `"warp"`, so it's not safe to use on the commit path.
        let raw = self.editor.as_ref(ctx).buffer_text(ctx).trim().to_string();
        if raw.is_empty() {
            self.cancel_custom(ctx);
            return;
        }
        if raw.eq_ignore_ascii_case(ORCHESTRATION_WARP_WORKER_HOST) {
            // Treat "warp" typed in custom mode as a normal warp selection.
            self.current_slug = ORCHESTRATION_WARP_WORKER_HOST.to_string();
            self.is_custom_mode = false;
            self.slug_before_edit = None;
            self.sync_dropdown_selection(ctx);
            ctx.emit(HostPickerEvent::HostChanged {
                slug: self.current_slug.clone(),
            });
            ctx.emit(HostPickerEvent::Closed);
            ctx.notify();
            return;
        }
        self.current_slug = raw.clone();
        self.is_custom_mode = false;
        self.slug_before_edit = None;
        // Promote an unknown slug to the "recent" row so it stays visible
        // in the list on the next paint.
        if !self.is_known_option(&raw) {
            self.recent_host = Some(raw.clone());
            self.repopulate_menu(ctx);
        }
        self.sync_dropdown_selection(ctx);
        ctx.emit(HostPickerEvent::HostChanged { slug: raw });
        ctx.emit(HostPickerEvent::Closed);
        ctx.notify();
    }

    fn cancel_custom(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_custom_mode {
            return;
        }
        if let Some(prev) = self.slug_before_edit.take() {
            self.current_slug = prev;
        }
        self.is_custom_mode = false;
        self.sync_dropdown_selection(ctx);
        ctx.emit(HostPickerEvent::Closed);
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => self.commit_custom(ctx),
            EditorEvent::Escape => self.cancel_custom(ctx),
            EditorEvent::Blurred => {
                if self.is_custom_mode {
                    self.commit_custom(ctx);
                }
            }
            _ => {}
        }
    }

    fn render_custom_mode(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background: Fill = theme.surface_overlay_1();
        let border_color = theme.outline();

        // Center the editor vertically — without this, the row's tight
        // cross-axis constraint stretches it to fill the content height and
        // the glyphs render flush to the top.
        let centered_editor = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(ChildView::new(&self.editor).finish())
            .finish();
        let cancel_button = self.render_cancel_button(appearance);

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Expanded::new(1.0, centered_editor).finish())
            .with_child(cancel_button)
            .finish();

        // Mirror the dropdown's outer vertical margin so custom mode
        // lines up with the other pickers instead of riding higher.
        Container::new(
            ConstrainedBox::new(
                Container::new(row)
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(6.)
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        ORCHESTRATION_PICKER_RADIUS,
                    )))
                    .with_border(
                        Border::all(ORCHESTRATION_PICKER_BORDER_WIDTH)
                            .with_border_fill(border_color),
                    )
                    .finish(),
            )
            .with_height(ORCHESTRATION_PICKER_HEIGHT)
            .finish(),
        )
        .with_margin_top(DROPDOWN_PADDING)
        .with_margin_bottom(DROPDOWN_PADDING)
        .finish()
    }

    fn render_cancel_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_fill = Fill::Solid(blended_colors::text_disabled(theme, theme.surface_1()));
        let mouse_state = self.clear_mouse_state.clone();
        Hoverable::new(mouse_state, move |_| {
            ConstrainedBox::new(
                Container::new(Icon::X.to_warpui_icon(icon_fill).finish())
                    .with_uniform_padding(2.)
                    .finish(),
            )
            .with_width(16.)
            .with_height(16.)
            .finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(InternalAction::CancelCustom);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

// ── Pure helpers (also exercised by unit tests) ─────────────────────

/// Trims `slug` and falls back to `"warp"` when empty.
fn normalize_slug(slug: &str) -> String {
    let trimmed = slug.trim();
    if trimmed.is_empty() {
        ORCHESTRATION_WARP_WORKER_HOST.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Builds the menu items shown in list mode, in the order: workspace default
/// (badged "Default" if set), warp, recent custom slug (if any and not a
/// duplicate), then a "Custom host…" entry.
pub(crate) fn build_menu_items(
    default_host: Option<&str>,
    recent_host: Option<&str>,
) -> Vec<MenuItem<DropdownAction<InternalAction>>> {
    let mut items: Vec<MenuItem<DropdownAction<InternalAction>>> = Vec::new();

    if let Some(slug) = default_host {
        items.push(menu_item_for_known(
            slug,
            Some(DEFAULT_BADGE),
            InternalAction::SelectKnown(slug.to_string()),
        ));
    }
    items.push(menu_item_for_known(
        ORCHESTRATION_WARP_WORKER_HOST,
        None,
        InternalAction::SelectKnown(ORCHESTRATION_WARP_WORKER_HOST.to_string()),
    ));
    if let Some(slug) = recent_host {
        if default_host != Some(slug) && !slug.eq_ignore_ascii_case(ORCHESTRATION_WARP_WORKER_HOST)
        {
            // Recent hosts render as plain slugs; only the workspace
            // default carries a badge.
            items.push(menu_item_for_known(
                slug,
                None,
                InternalAction::SelectKnown(slug.to_string()),
            ));
        }
    }
    items.push(MenuItem::Item(
        MenuItemFields::new(CUSTOM_HOST_LABEL).with_on_select_action(
            DropdownAction::SelectActionAndClose(InternalAction::EnterCustomMode),
        ),
    ));

    items
}

/// Returns the menu label corresponding to `slug`, including the "Default"
/// badge when it matches the workspace default.
pub(crate) fn menu_label_for(slug: &str, default_host: Option<&str>) -> String {
    if default_host == Some(slug) {
        format_known_label(slug, Some(DEFAULT_BADGE))
    } else {
        format_known_label(slug, None)
    }
}

fn format_known_label(slug: &str, badge: Option<&str>) -> String {
    match badge {
        Some(badge) => format!("{slug}  ({badge})"),
        None => slug.to_string(),
    }
}

fn menu_item_for_known(
    slug: &str,
    badge: Option<&str>,
    action: InternalAction,
) -> MenuItem<DropdownAction<InternalAction>> {
    MenuItem::Item(
        MenuItemFields::new(format_known_label(slug, badge))
            .with_on_select_action(DropdownAction::SelectActionAndClose(action)),
    )
}

// ── Entity / View impls ─────────────────────────────────────────────

impl Entity for HostPicker {
    type Event = HostPickerEvent;
}

impl TypedActionView for HostPicker {
    type Action = InternalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            InternalAction::SelectKnown(slug) => {
                let slug = slug.clone();
                self.current_slug = slug.clone();
                self.is_custom_mode = false;
                self.slug_before_edit = None;
                // The inner dropdown already updated its own selection;
                // re-entering it here would trigger a circular update.
                ctx.emit(HostPickerEvent::HostChanged { slug });
                ctx.notify();
            }
            InternalAction::EnterCustomMode => {
                let current = self.current_slug.clone();
                self.enter_custom_mode_with_slug(&current, ctx);
                ctx.notify();
            }
            InternalAction::CancelCustom => {
                self.cancel_custom(ctx);
            }
        }
    }
}

impl View for HostPicker {
    fn ui_name() -> &'static str {
        "HostPicker"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        if self.is_custom_mode {
            self.render_custom_mode(appearance)
        } else {
            ChildView::new(&self.dropdown).finish()
        }
    }
}

#[cfg(test)]
#[path = "host_picker_tests.rs"]
mod tests;
