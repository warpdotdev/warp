use std::{collections::HashMap, path::PathBuf};

use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warp_editor::editor::NavigationKey;
use warpui::{
    elements::{
        Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, Padding, ParentElement, Radius, SavePosition, ScrollTarget,
        ScrollToPositionMode, ScrollbarWidth, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    keymap::{macros::*, FixedBinding, Keystroke},
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    appearance::Appearance,
    editor::{
        EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
        TextOptions,
    },
    modal::ModalAction,
    tab_configs::{
        branch_picker::BranchPicker,
        repo_picker::{RepoPicker, RepoPickerEvent},
        PickerStyle, TabConfig, TabConfigParam, TabConfigParamType,
    },
    view_components::action_button::{
        ActionButton, DisabledTheme, KeystrokeSource, NakedTheme, PrimaryTheme,
    },
};

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings(vec![
        FixedBinding::new(
            "escape",
            TabConfigParamsModalAction::Escape,
            id!("TabConfigParamsModal"),
        ),
        // Enter and Space only fire when no EditorView descendant is focused.
        // When a text field or picker filter editor has focus, the editor
        // consumes these keys and the modal handles them via event
        // subscriptions instead (see handle_editor_event).
        FixedBinding::new(
            "enter",
            TabConfigParamsModalAction::Submit,
            id!("TabConfigParamsModal") & !id!("EditorView"),
        ),
        FixedBinding::new(
            "space",
            TabConfigParamsModalAction::ToggleDropdown,
            id!("TabConfigParamsModal") & !id!("EditorView"),
        ),
    ]);
}

fn param_field_position_id(index: usize) -> String {
    format!("tab_config_param_field_{index}")
}

/// Resolves the effective value to submit for a parameter.
///
/// Returns `None` when the value is blank and the param has no default (i.e. it
/// is required and unsatisfied), causing submit to be blocked.
fn resolve_param_value(raw_value: String, param: &TabConfigParam) -> Option<String> {
    if raw_value.trim().is_empty() {
        param.default.clone()
    } else {
        Some(raw_value)
    }
}

/// A single editable field in the modal — either a text editor or a smart picker.
enum ParamField {
    /// Plain single-line text input.
    Text(ViewHandle<EditorView>),
    /// Git branch picker; stores the last selection so submit can read it.
    Branch {
        picker: ViewHandle<BranchPicker>,
        selected: Option<String>,
    },
    /// Known-repo picker; stores the last selection.
    Repo {
        picker: ViewHandle<RepoPicker>,
        selected: Option<String>,
    },
}

impl ParamField {
    fn current_value(&self, app: &AppContext) -> String {
        match self {
            ParamField::Text(editor) => editor.as_ref(app).buffer_text(app),
            ParamField::Branch { picker, selected } => picker
                .as_ref(app)
                .selected_value(app)
                .or_else(|| selected.clone())
                .unwrap_or_default(),
            // Check the stored `selected` first: it is always kept up-to-date by the
            // `RepoPickerEvent::Selected` subscription and by `on_new_repo_selected`.
            // `picker.selected_value()` is a fallback for the initial default-value
            // case before any explicit selection has been made.
            ParamField::Repo { picker, selected } => selected
                .clone()
                .or_else(|| picker.as_ref(app).selected_value(app))
                .unwrap_or_default(),
        }
    }
}

/// Body view for the tab-config parameter fill modal.
///
/// Renders one labeled input row per parameter defined in the tab config's `[params]` section.
/// The workspace creates this as the inner body of a [`crate::modal::Modal`] and calls
/// [`Self::on_open`] / [`Self::on_close`] around the modal's visibility.
pub struct TabConfigParamsModal {
    /// Ordered list of `(param_name, param_definition, field)`.
    /// Rebuilt each time [`Self::on_open`] is called.
    param_fields: Vec<(String, TabConfigParam, ParamField)>,
    /// The config being launched; kept so [`Self::try_submit`] can include it in the event.
    pending_config: Option<TabConfig>,
    title: String,
    cancel_button: ViewHandle<ActionButton>,
    submit_button: ViewHandle<ActionButton>,
    submit_button_disabled: ViewHandle<ActionButton>,
    close_button_mouse_state: MouseStateHandle,
    scroll_state: ClippedScrollStateHandle,
}

pub enum TabConfigParamsModalEvent {
    Close,
    Submit {
        config: Box<TabConfig>,
        params: HashMap<String, String>,
    },
    /// The user clicked "Add new repo..." in a repo picker; the workspace should
    /// open a folder picker and call [`TabConfigParamsModal::on_new_repo_selected`].
    PickNewRepo {
        param_index: usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum TabConfigParamsModalAction {
    Cancel,
    Submit,
    Escape,
    ToggleDropdown,
}

impl TabConfigParamsModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Cancel"), NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(TabConfigParamsModalAction::Cancel);
            })
        });
        let submit_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Open Tab"), PrimaryTheme)
                .with_keybinding(
                    KeystrokeSource::Fixed(Keystroke::parse("enter").unwrap_or_default()),
                    ctx,
                )
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(TabConfigParamsModalAction::Submit);
                })
        });
        let submit_button_disabled = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Open Tab"), DisabledTheme)
        });
        Self {
            param_fields: Vec::new(),
            pending_config: None,
            title: String::new(),
            cancel_button,
            submit_button,
            submit_button_disabled,
            close_button_mouse_state: Default::default(),
            scroll_state: Default::default(),
        }
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    /// Called by the workspace before making the modal visible.
    ///
    /// Builds one field per param in `config`. `cwd` is the active terminal's
    /// working directory, used to seed the branch picker's git lookup.
    pub fn on_open(
        &mut self,
        config: TabConfig,
        cwd: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.param_fields.clear();

        // Sort params by type priority (Repo first, Branch second, Text last),
        // then alphabetically by name within each type for stable ordering.
        let mut params: Vec<(String, TabConfigParam)> = config
            .params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let type_priority = |t: &TabConfigParamType| match t {
            TabConfigParamType::Repo => 0,
            TabConfigParamType::Branch => 1,
            TabConfigParamType::Text => 2,
        };
        params.sort_by(|a, b| {
            type_priority(&a.1.param_type)
                .cmp(&type_priority(&b.1.param_type))
                .then(a.0.cmp(&b.0))
        });

        // If there's a Repo param with a default value, seed branch pickers with that repo
        // path so branches are populated on initial open. Without this, branch pickers would
        // use the terminal cwd, which may differ from the configured repo.
        let branch_initial_cwd = params
            .iter()
            .find(|(_, p)| matches!(p.param_type, TabConfigParamType::Repo))
            .and_then(|(_, p)| p.default.as_deref())
            .map(PathBuf::from)
            .or_else(|| cwd.clone());

        let picker_style = PickerStyle {
            width: 412.,
            background: Some(Appearance::as_ref(ctx).theme().background()),
        };

        for (i, (name, param)) in params.iter().enumerate() {
            let field = match param.param_type {
                TabConfigParamType::Branch => {
                    let default_value = param.default.clone();
                    let branch_cwd = branch_initial_cwd.clone();
                    let style = PickerStyle {
                        width: picker_style.width,
                        background: picker_style.background,
                    };
                    let picker = ctx.add_typed_action_view(move |ctx| {
                        BranchPicker::new_with_style(branch_cwd, default_value, Some(style), ctx)
                    });
                    ctx.subscribe_to_view(&picker, move |me, _, value, ctx| {
                        if let Some((_, _, ParamField::Branch { selected, .. })) =
                            me.param_fields.get_mut(i)
                        {
                            *selected = Some(value.clone());
                        }
                        me.reclaim_focus(ctx);
                    });
                    ParamField::Branch {
                        picker,
                        selected: param.default.clone(),
                    }
                }
                TabConfigParamType::Repo => {
                    let default_value = param.default.clone();
                    let style = PickerStyle {
                        width: picker_style.width,
                        background: picker_style.background,
                    };
                    let picker = ctx.add_typed_action_view(move |ctx| {
                        RepoPicker::new_with_style(default_value, Some(style), ctx)
                    });
                    ctx.subscribe_to_view(&picker, move |me, _, event, ctx| match event {
                        RepoPickerEvent::Selected(value) => {
                            if let Some((_, _, ParamField::Repo { selected, .. })) =
                                me.param_fields.get_mut(i)
                            {
                                *selected = Some(value.clone());
                            }
                            me.sync_branch_pickers_for_repo(PathBuf::from(value.as_str()), ctx);
                            me.reclaim_focus(ctx);
                        }
                        RepoPickerEvent::RequestAddRepo => {
                            ctx.emit(TabConfigParamsModalEvent::PickNewRepo { param_index: i });
                        }
                    });
                    ParamField::Repo {
                        picker,
                        selected: param.default.clone(),
                    }
                }
                TabConfigParamType::Text => {
                    let default_text = param.default.clone().unwrap_or_default();
                    let placeholder = if default_text.is_empty() {
                        format!("Enter {name}")
                    } else {
                        default_text.clone()
                    };
                    let text_options = TextOptions::ui_font_size(Appearance::as_ref(ctx));
                    let editor = ctx.add_typed_action_view(|ctx| {
                        let options = SingleLineEditorOptions {
                            text: text_options,
                            propagate_and_no_op_vertical_navigation_keys:
                                PropagateAndNoOpNavigationKeys::Always,
                            ..Default::default()
                        };
                        let mut editor = EditorView::single_line(options, ctx);
                        editor.set_placeholder_text(placeholder.as_str(), ctx);
                        editor
                    });

                    if !default_text.is_empty() {
                        editor.update(ctx, |e, ctx| {
                            e.system_reset_buffer_text(&default_text, ctx);
                        });
                    }

                    ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
                        me.handle_editor_event(i, event, ctx);
                    });

                    ParamField::Text(editor)
                }
            };

            self.param_fields.push((name.clone(), param.clone(), field));
        }

        self.pending_config = Some(config);

        // When the only fields are dropdowns, focus the modal itself so
        // Enter (submit) and Space (toggle dropdown) fixed bindings fire.
        // When there are text fields, focus the first one so the user can
        // start typing immediately.
        if self.has_text_fields() {
            self.focus_field(0, ctx);
        } else {
            ctx.focus_self();
        }
        ctx.notify();
    }

    /// Called by the workspace when the modal is dismissed. Clears all dynamic state.
    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.param_fields.clear();
        self.pending_config = None;
        ctx.notify();
    }

    /// Called by the workspace after the user adds a new repo via the folder picker.
    /// Refreshes the repo picker at `param_index` and pre-selects the new path.
    pub fn on_new_repo_selected(
        &mut self,
        path: PathBuf,
        param_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some((_, _, ParamField::Repo { picker, selected })) =
            self.param_fields.get_mut(param_index)
        {
            let path_str = path.to_string_lossy().to_string();
            *selected = Some(path_str);
            picker.update(ctx, |repo_picker, ctx| {
                repo_picker.refresh_and_select(path.clone(), ctx);
            });
        }
        self.sync_branch_pickers_for_repo(path, ctx);
    }

    fn sync_branch_pickers_for_repo(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        // Clear stale branch selections and collect picker handles.
        // Collecting handles first avoids borrow conflicts when calling
        // picker.update() below.
        let branch_pickers: Vec<_> = self
            .param_fields
            .iter_mut()
            .filter_map(|(_, _, field)| {
                if let ParamField::Branch { picker, selected } = field {
                    *selected = None;
                    Some(picker.clone())
                } else {
                    None
                }
            })
            .collect();

        for branch_picker in branch_pickers {
            branch_picker.update(ctx, |picker, ctx| {
                picker.refetch_branches(path.clone(), ctx);
            });
        }
        ctx.notify();
    }

    /// Restores focus to the modal itself (dropdown-only) or the first text
    /// field after a picker interaction closes its dropdown.
    fn reclaim_focus(&self, ctx: &mut ViewContext<Self>) {
        if self.has_text_fields() {
            self.focus_field(0, ctx);
        } else {
            ctx.focus_self();
        }
        ctx.notify();
    }

    fn has_text_fields(&self) -> bool {
        self.param_fields
            .iter()
            .any(|(_, _, field)| matches!(field, ParamField::Text(_)))
    }

    fn dropdown_count(&self) -> usize {
        self.param_fields
            .iter()
            .filter(|(_, _, field)| !matches!(field, ParamField::Text(_)))
            .count()
    }

    fn toggle_single_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let mut opened = false;
        for (_, _, field) in &self.param_fields {
            match field {
                ParamField::Branch { picker, .. } => {
                    opened = picker.update(ctx, |p, ctx| p.toggle_dropdown(ctx));
                    break;
                }
                ParamField::Repo { picker, .. } => {
                    opened = picker.update(ctx, |p, ctx| p.toggle_dropdown(ctx));
                    break;
                }
                ParamField::Text(_) => {}
            }
        }
        // When the dropdown just closed, reclaim focus so Enter/Space
        // fixed bindings continue to work.
        if !opened && !self.has_text_fields() {
            ctx.focus_self();
        }
    }

    fn focus_field(&self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some((_, _, field)) = self.param_fields.get(index) {
            match field {
                ParamField::Text(editor) => ctx.focus(editor),
                ParamField::Branch { picker, .. } => ctx.focus(picker),
                ParamField::Repo { picker, .. } => ctx.focus(picker),
            }
            self.scroll_state.scroll_to_position(ScrollTarget {
                position_id: param_field_position_id(index),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        }
    }

    fn handle_editor_event(
        &mut self,
        index: usize,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let count = self.param_fields.len();
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) if count > 0 => {
                self.focus_field((index + 1) % count, ctx);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) if count > 0 => {
                let prev = if index == 0 { count - 1 } else { index - 1 };
                self.focus_field(prev, ctx);
            }
            EditorEvent::Enter => self.try_submit(ctx),
            EditorEvent::Escape => ctx.emit(TabConfigParamsModalEvent::Close),
            EditorEvent::Edited(_) => ctx.notify(),
            _ => {}
        }
    }

    fn try_submit(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(config) = self.pending_config.clone() {
            let params: Option<HashMap<String, String>> = self
                .param_fields
                .iter()
                .map(|(name, param, field)| {
                    let value = field.current_value(ctx);
                    resolve_param_value(value, param).map(|v| (name.clone(), v))
                })
                .collect();
            if let Some(params) = params {
                ctx.emit(TabConfigParamsModalEvent::Submit {
                    config: Box::new(config),
                    params,
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "params_modal_tests.rs"]
mod tests;

impl Entity for TabConfigParamsModal {
    type Event = TabConfigParamsModalEvent;
}

impl View for TabConfigParamsModal {
    fn ui_name() -> &'static str {
        "TabConfigParamsModal"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // When focus arrives directly at this view (not at a child), keep
        // self-focus so the Enter/Space fixed bindings fire. This happens
        // on initial open (via focus_self in on_open) and when the Modal
        // wrapper re-focuses the body after a child (like a dropdown)
        // releases focus.
        if focus_ctx.is_self_focused() && !self.has_text_fields() {
            ctx.focus_self();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub_text = theme.sub_text_color(theme.background());

        let is_submit_enabled = self.param_fields.iter().all(|(_, param, field)| {
            let value = field.current_value(app);
            resolve_param_value(value, param).is_some()
        });

        // ── Header ───────────────────────────────────────────────────────
        let header = {
            let title = Text::new_inline(self.title.clone(), appearance.ui_font_family(), 16.)
                .with_color(theme.active_ui_text_color().into())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish();

            let esc_badge = Container::new(
                ConstrainedBox::new(
                    Text::new_inline("ESC".to_string(), appearance.ui_font_family(), 10.)
                        .with_color(theme.foreground().into())
                        .finish(),
                )
                .with_height(14.)
                .finish(),
            )
            .with_horizontal_padding(2.)
            .with_background(internal_colors::neutral_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
            .finish();

            let close_icon = ConstrainedBox::new(Icon::X.to_warpui_icon(sub_text).finish())
                .with_width(14.)
                .with_height(14.)
                .finish();

            let close_button = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(2.)
                .with_child(close_icon)
                .with_child(esc_badge)
                .finish();

            let close_hoverable =
                Hoverable::new(self.close_button_mouse_state.clone(), move |_state| {
                    close_button
                })
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
                    .with_top(24.)
                    .with_bottom(12.)
                    .with_left(24.)
                    .with_right(24.),
            )
            .finish()
        };

        // ── Form body ────────────────────────────────────────────────────
        let mut form = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        let active_text = theme.active_ui_text_color();

        for (i, (name, param, field)) in self.param_fields.iter().enumerate() {
            let mut label = Container::new(
                Text::new_inline(
                    name.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(active_text.into())
                .finish(),
            )
            .with_margin_bottom(4.);
            if i > 0 {
                label = label.with_margin_top(16.);
            }
            form.add_child(label.finish());

            // Optional description sub-label.
            if let Some(description) = &param.description {
                form.add_child(
                    Container::new(
                        Text::new_inline(
                            description.clone(),
                            appearance.ui_font_family(),
                            appearance.ui_font_size() - 1.,
                        )
                        .with_color(sub_text.into())
                        .finish(),
                    )
                    .with_margin_bottom(4.)
                    .finish(),
                );
            }

            // Default value hint (text params only — pickers show the value in their top bar).
            if matches!(param.param_type, TabConfigParamType::Text) {
                if let Some(default_value) = &param.default {
                    form.add_child(
                        Container::new(
                            Text::new_inline(
                                format!("Default: {default_value}"),
                                appearance.ui_font_family(),
                                appearance.ui_font_size() - 1.,
                            )
                            .with_color(sub_text.into())
                            .finish(),
                        )
                        .with_margin_bottom(4.)
                        .finish(),
                    );
                }
            }

            // The input field itself.
            // Text editors get the standard text_input border treatment;
            // pickers (Dropdown-based) already render their own chrome.
            let field_element: Box<dyn Element> = match field {
                ParamField::Text(editor) => appearance
                    .ui_builder()
                    .text_input(editor.clone())
                    .build()
                    .finish(),
                ParamField::Branch { picker, .. } => ChildView::new(picker).finish(),
                ParamField::Repo { picker, .. } => ChildView::new(picker).finish(),
            };

            form.add_child(SavePosition::new(field_element, &param_field_position_id(i)).finish());
        }

        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            form.finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .with_padding_start(0.)
        .with_padding_end(0.)
        .finish();

        let body_container = Container::new(
            ConstrainedBox::new(scrollable)
                .with_max_height(340.)
                .finish(),
        )
        .with_padding(
            Padding::uniform(0.)
                .with_left(24.)
                .with_right(24.)
                .with_bottom(16.),
        )
        .finish();

        // ── Footer ───────────────────────────────────────────────────────
        let button_row = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(ChildView::new(&self.cancel_button).finish())
                .with_child(if is_submit_enabled {
                    ChildView::new(&self.submit_button).finish()
                } else {
                    ChildView::new(&self.submit_button_disabled).finish()
                })
                .finish(),
        )
        .with_padding(Padding::uniform(12.).with_left(24.).with_right(24.))
        .finish();

        let footer = Container::new(button_row)
            .with_border(Border::top(1.).with_border_fill(theme.outline()))
            .finish();

        // ── Assemble ─────────────────────────────────────────────────────
        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(body_container)
            .with_child(footer)
            .finish()
    }
}

impl TypedActionView for TabConfigParamsModal {
    type Action = TabConfigParamsModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TabConfigParamsModalAction::Cancel | TabConfigParamsModalAction::Escape => {
                ctx.emit(TabConfigParamsModalEvent::Close);
            }
            TabConfigParamsModalAction::Submit => self.try_submit(ctx),
            TabConfigParamsModalAction::ToggleDropdown => {
                if self.dropdown_count() <= 1 {
                    self.toggle_single_dropdown(ctx);
                }
            }
        }
    }
}
