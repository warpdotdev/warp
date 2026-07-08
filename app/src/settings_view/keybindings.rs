use std::collections::HashMap;

use itertools::Itertools;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, DispatchEventResult, Empty, EventHandler, Fill, Flex,
    Hoverable, MouseState, MouseStateHandle, ParentElement, Radius, SavePosition, ScrollbarWidth,
    Shrinkable, Text, Wrap,
};
use warpui::fonts::Weight;
use warpui::keymap::{DescriptionContext, Keystroke, Trigger};
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::units::Pixels;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::settings_page::{
    render_sub_header, LocalOnlyIconState, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::SettingsSection;
use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::keyboard::{write_custom_keybinding, UserDefinedKeybinding};
use crate::search_bar::SearchBar;
use crate::settings::CloudPreferencesSettings;
use crate::util::bindings::{
    filter_bindings_including_keystroke, reset_keybinding_to_default, set_custom_keybinding,
    CommandBinding,
};
use crate::{report_error, send_telemetry_from_ctx, themes, TelemetryEvent};

const FONT_DELTA: f32 = 2.;
const CANCEL_SAVE_BUTTONS_SPACING: f32 = 4.0;
const CLEAR_CANCEL_BUTTONS_SPACING: f32 = 8.0;
const ROW_INTERNAL_VERTICAL_PADDING: f32 = 8.0;
const ROW_LEFT_MARGIN: f32 = 20.0;
const ROW_HEIGHT: f32 = 28.;
const EDIT_BUTTONS_BORDER_RADIUS: f32 = 4.0;

pub const SEARCH_PLACEHOLDER: &str = "Search by name or by keys (ex. \"cmd d\")";
const SHORTCUT_CONFLICT_WARNING_TEXT: &str = "This shortcut conflicts with other keybinds";
const KEYBINDINGS_PAGE_SHORTCUT: &str = "workspace:toggle_keybindings_page";
const RESET_BUTTON_TEXT: &str = "Default";
const CANCEL_BUTTON_TEXT: &str = "Cancel";
const CLEAR_BUTTON_TEXT: &str = "Clear";
const SAVE_BUTTON_TEXT: &str = "Save";

/// Notifier for custom keybinding changed. Views could subscribe to this for
/// KeybindingChangedEvent.
#[derive(Default)]
pub struct KeybindingChangedNotifier {}

impl KeybindingChangedNotifier {
    pub fn new() -> Self {
        Default::default()
    }

    #[cfg(test)]
    pub fn mock() -> Self {
        Self::new()
    }
}

pub enum KeybindingChangedEvent {
    BindingChanged {
        /// Name of the keybinding that is being changed.
        binding_name: String,
        new_trigger: Option<Keystroke>,
    },
}

impl Entity for KeybindingChangedNotifier {
    type Event = KeybindingChangedEvent;
}

impl SingletonEntity for KeybindingChangedNotifier {}

#[derive(Clone, Debug)]
pub struct KeyBindingModifyingState {
    pub current_binding: Option<Keystroke>,
    pub unsaved_binding: Option<Keystroke>,
}

impl KeyBindingModifyingState {
    pub fn new(state: Option<Keystroke>) -> KeyBindingModifyingState {
        Self {
            current_binding: state.clone(),
            unsaved_binding: state,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.current_binding != self.unsaved_binding
    }
}

#[derive(Debug, Clone, Default)]
struct ConflictMap {
    map: HashMap<Keystroke, usize>,
}

impl ConflictMap {
    fn update(&mut self, old: &Option<Keystroke>, new: Option<Keystroke>) {
        if let Some(old) = old {
            if let Some(old_conflict_count) = self.map.get_mut(old) {
                *old_conflict_count = old_conflict_count.saturating_sub(1);
            }
        }

        if let Some(new) = new {
            let new_conflict_count = self.map.entry(new).or_default();
            *new_conflict_count += 1;
        }
    }

    fn has_conflict(&self, key: &Option<Keystroke>) -> bool {
        match key {
            Some(key) => self
                .map
                .get(key)
                .map(|count| *count > 1)
                .unwrap_or_default(),
            None => false,
        }
    }
}

impl FromIterator<Option<Keystroke>> for ConflictMap {
    fn from_iter<I: IntoIterator<Item = Option<Keystroke>>>(iter: I) -> Self {
        let mut map = HashMap::new();

        for binding in iter.into_iter().flatten() {
            let counter = map.entry(binding).or_default();
            *counter += 1;
        }

        ConflictMap { map }
    }
}

pub struct KeybindingsView {
    page: PageType<Self>,
    search_editor: ViewHandle<EditorView>,
    search_bar: ViewHandle<SearchBar>,
    clipped_scroll_state: ClippedScrollStateHandle,
    bindings: Option<Vec<CommandBinding>>,
    modifying_row: Option<KeyBindingModifyingState>,
    pub rows: Option<Vec<KeybindingRow>>,
    // Map between the keystroke and the number of conflicting bindings associated with the keystroke.
    // The bindings could be unsaved.
    conflict_map: ConflictMap,
}

#[derive(Debug)]
pub enum KeybindingsViewAction {
    KeybindingRowClicked(usize),
    KeystrokeDefined(usize, Keystroke),
    ResetToDefaultKeyStroke(usize),
    CancelKeyStrokeEditing(usize),
    ConfirmKeyStroke(usize),
    RemoveKeyStroke(usize),
}

#[derive(Default, Clone)]
struct RowMouseStates {
    keystroke_row_mouse_state: MouseStateHandle,
    reset_to_default_mouse_state: MouseStateHandle,
    remove_mouse_state: MouseStateHandle,
    cancel_mouse_state: MouseStateHandle,
    save_mouse_state: MouseStateHandle,
}

/// Wrapper around the CommandBinding structure that includes the styling/render-specific
/// attributes (such as MouseStateHandles)
#[derive(Clone)]
pub struct KeybindingRow {
    pub binding: CommandBinding,
    mouse_state_handles: RowMouseStates,
    editor_open: bool,
}

impl From<(Option<Vec<usize>>, &CommandBinding)> for KeybindingRow {
    fn from(orig: (Option<Vec<usize>>, &CommandBinding)) -> Self {
        Self {
            binding: orig.1.clone(),
            mouse_state_handles: Default::default(),
            editor_open: false,
        }
    }
}

impl KeybindingRow {
    fn render(
        &self,
        index: usize,
        is_disabled: bool,
        has_conflicting_binding: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let inner = if !is_disabled {
            let mut row = Hoverable::new(
                self.mouse_state_handles.keystroke_row_mouse_state.clone(),
                |state| {
                    let background = if state.is_hovered() {
                        Some(appearance.theme().accent().with_opacity(40).into())
                    } else if index.is_multiple_of(2) {
                        Some(internal_colors::fg_overlay_1(appearance.theme()).into())
                    } else {
                        None
                    };
                    if self.editor_open {
                        self.render_clicked(index, has_conflicting_binding, appearance)
                    } else {
                        self.render_summary(None, background, has_conflicting_binding, appearance)
                    }
                },
            );

            if !self.editor_open {
                row = row.on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(KeybindingsViewAction::KeybindingRowClicked(index));
                });
            }

            row.finish()
        } else {
            let background = if index.is_multiple_of(2) {
                Some(internal_colors::fg_overlay_1(appearance.theme()).into())
            } else {
                None
            };

            Container::new(self.render_summary(
                None,
                background,
                has_conflicting_binding,
                appearance,
            ))
            .with_foreground_overlay(appearance.theme().keybinding_row_overlay())
            .finish()
        };

        if index == 0 {
            SavePosition::new(inner, "first_keybinding_setting").finish()
        } else {
            inner
        }
    }

    fn render_summary(
        &self,
        index: Option<usize>,
        background: Option<Fill>,
        has_conflicting_binding: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let binding = &self.binding;
        let keystroke = match binding.trigger.clone() {
            None => Empty::new().finish(),
            Some(keystroke) => {
                let mut keyshortcut = appearance.ui_builder().keyboard_shortcut(&keystroke);

                if has_conflicting_binding {
                    keyshortcut = keyshortcut.with_style(UiComponentStyles {
                        border_width: Some(2.),
                        border_color: Some(themes::theme::Fill::warn().into()),
                        ..Default::default()
                    });
                }

                keyshortcut.build().finish()
            }
        };
        let element = render_columns(
            render_text(
                binding.description.in_context(DescriptionContext::Default),
                None,
                appearance,
            ),
            keystroke,
            0.7,
            background,
            None,
        );
        if let Some(index) = index {
            EventHandler::new(element)
                .on_keydown(move |ctx, _, keystroke| {
                    ctx.dispatch_typed_action(KeybindingsViewAction::KeystrokeDefined(
                        index,
                        keystroke.clone(),
                    ));
                    DispatchEventResult::StopPropagation
                })
                .finish()
        } else {
            element
        }
    }

    fn render_clicked(
        &self,
        index: usize,
        has_conflicting_binding: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let conflict_warning = if has_conflicting_binding {
            render_text(
                SHORTCUT_CONFLICT_WARNING_TEXT,
                Some(UiComponentStyles {
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                }),
                appearance,
            )
        } else {
            Empty::new().finish()
        };

        let press_new_shortcut_text = render_text("Press new keyboard shortcut", None, appearance);

        let new_shortcut_element = Container::new(press_new_shortcut_text)
            .with_margin_left(ROW_LEFT_MARGIN)
            .with_margin_top(8.0)
            .finish();

        Container::new(
            Flex::column()
                .with_child(self.render_summary(
                    Some(index),
                    Some(appearance.theme().accent().into()),
                    has_conflicting_binding,
                    appearance,
                ))
                .with_child(
                    Container::new(new_shortcut_element)
                        .with_margin_bottom(ROW_INTERNAL_VERTICAL_PADDING)
                        .finish(),
                )
                .with_child(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                1.,
                                Align::new(
                                    Container::new(conflict_warning)
                                        .with_margin_left(ROW_LEFT_MARGIN)
                                        .finish(),
                                )
                                .left()
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(
                            Container::new(self.get_edit_button_row(appearance, index))
                                .with_margin_right(CLEAR_CANCEL_BUTTONS_SPACING)
                                .finish(),
                        )
                        .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
                        .finish(),
                )
                .finish(),
        )
        .with_padding_bottom(ROW_INTERNAL_VERTICAL_PADDING)
        .with_background(appearance.theme().accent().with_opacity(40))
        .finish()
    }

    fn get_button_text_color(
        &self,
        appearance: &Appearance,
        state: &MouseState,
    ) -> themes::theme::Fill {
        let main_text_color: themes::theme::Fill = appearance
            .theme()
            .main_text_color(appearance.theme().surface_2());

        if state.is_hovered() {
            main_text_color
        } else if state.is_clicked() {
            main_text_color.with_opacity(50)
        } else {
            main_text_color.with_opacity(90)
        }
    }

    fn get_edit_button_row(&self, appearance: &Appearance, index: usize) -> Box<dyn Element> {
        let mut edit_buttons_based_on_state = Vec::new();

        if self.binding.trigger.is_some() {
            let clear = Hoverable::new(
                self.mouse_state_handles.remove_mouse_state.clone(),
                |state| {
                    render_button(
                        CLEAR_BUTTON_TEXT,
                        appearance,
                        self.get_button_text_color(appearance, state),
                    )
                },
            )
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(KeybindingsViewAction::RemoveKeyStroke(index));
            })
            .finish();

            edit_buttons_based_on_state.push(clear);
        }

        let clear = Container::new(
            Hoverable::new(
                self.mouse_state_handles
                    .reset_to_default_mouse_state
                    .clone(),
                |state| {
                    render_button(
                        RESET_BUTTON_TEXT,
                        appearance,
                        self.get_button_text_color(appearance, state),
                    )
                },
            )
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(KeybindingsViewAction::ResetToDefaultKeyStroke(index));
            })
            .finish(),
        )
        .with_padding_left(CLEAR_CANCEL_BUTTONS_SPACING)
        .finish();
        edit_buttons_based_on_state.push(clear);

        let cancel = Container::new(
            Hoverable::new(
                self.mouse_state_handles.cancel_mouse_state.clone(),
                |state| {
                    let cancel_button_color = self.get_button_text_color(appearance, state);
                    if index == 0 {
                        SavePosition::new(
                            render_button(CANCEL_BUTTON_TEXT, appearance, cancel_button_color),
                            "first_keybinding_cancel",
                        )
                        .finish()
                    } else {
                        render_button("Cancel", appearance, cancel_button_color)
                    }
                },
            )
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(KeybindingsViewAction::CancelKeyStrokeEditing(index));
            })
            .finish(),
        )
        .with_padding_left(CLEAR_CANCEL_BUTTONS_SPACING)
        .finish();

        edit_buttons_based_on_state.push(cancel);

        let save = Container::new(
            Hoverable::new(self.mouse_state_handles.save_mouse_state.clone(), |state| {
                render_button(
                    SAVE_BUTTON_TEXT,
                    appearance,
                    self.get_button_text_color(appearance, state),
                )
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(KeybindingsViewAction::ConfirmKeyStroke(index));
            })
            .finish(),
        )
        .with_padding_left(CANCEL_SAVE_BUTTONS_SPACING)
        .finish();
        edit_buttons_based_on_state.push(save);

        Flex::row()
            .with_children(edit_buttons_based_on_state)
            .finish()
    }
}

impl KeybindingsView {
    pub fn new(ctx: &mut ViewContext<KeybindingsView>) -> Self {
        let search_editor = {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            ctx.add_typed_action_view(|ctx| EditorView::single_line(options, ctx))
        };
        ctx.subscribe_to_view(&search_editor, move |me, _, event, ctx| {
            me.handle_search_editor_event(event, ctx);
        });

        search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text(SEARCH_PLACEHOLDER, ctx);
        });

        let search_bar = ctx.add_typed_action_view(|_| SearchBar::new(search_editor.clone()));

        let page = PageType::new_monolith(KeybindingsWidget::default(), None, false);
        Self {
            page,
            clipped_scroll_state: Default::default(),
            bindings: None,
            rows: Default::default(),
            modifying_row: None,
            search_bar,
            search_editor,
            conflict_map: Default::default(),
        }
    }

    /// Searches for a keybinding as if the user had typed the query into the search
    /// box. Will filter the keybinding list by the query.
    pub fn search_for_binding(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.search_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(query, ctx);
        });
        self.filter_bindings(query, ctx);
    }

    /// Filter the list of visible bindings by the given query.
    fn filter_bindings(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.rows = Some(
            filter_bindings_including_keystroke(
                self.bindings.iter().flatten(),
                query,
                DescriptionContext::Default,
            )
            .map(KeybindingRow::from)
            .collect(),
        );

        self.clipped_scroll_state.scroll_to(Pixels::zero());
        ctx.notify();
    }

    fn handle_search_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let search_term = self.search_editor.as_ref(ctx).buffer_text(ctx);
                self.filter_bindings(&search_term, ctx);
            }
            EditorEvent::Enter => ctx.notify(),
            EditorEvent::Escape => ctx.focus_self(),
            _ => {}
        }
    }

    fn binding_row_clicked(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        // Unfocus the search bar.
        ctx.focus_self();

        // Only enable editing when none of the other keystrokes are being edited.
        if self.modifying_row.is_none() {
            let maybe_row = self
                .rows
                .as_mut()
                .into_iter()
                .flatten()
                .enumerate()
                .find(|(idx, _)| *idx == index);

            if let Some((_, row)) = maybe_row {
                ctx.disable_key_bindings_dispatching();
                self.modifying_row =
                    Some(KeyBindingModifyingState::new(row.binding.trigger.clone()));
                row.editor_open = true;

                // This is entering the edit mode, and we'll want to capture the keydown events.
                // For that all actions are being suppressed for the given window.
                ctx.notify();
            }
        }
    }

    fn remove_keystroke(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
            ctx.set_custom_trigger(row.binding.name.clone(), Trigger::Empty);

            trigger_keybinding_notifier(row.binding.name.clone(), None, ctx);

            self.conflict_map.update(&row.binding.trigger, None);

            // Persist the keybinding into the `.warp` directory so that it will last beyond
            // this session
            write_custom_keybinding(row.binding.name.clone(), UserDefinedKeybinding::Removed);
            update_binding_list(&row.binding.name, None, &mut self.bindings);
            row.binding.trigger = None;

            send_telemetry_from_ctx!(
                TelemetryEvent::KeybindingRemoved {
                    action: row.binding.name.clone(),
                },
                ctx
            );
            self.modifying_row = None;
            row.editor_open = false;
            ctx.enable_key_bindings_dispatching();
            ctx.notify();
        }
    }

    fn reset_to_default_keystroke(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
            let default_trigger = reset_keybinding_to_default(&row.binding.name, ctx);
            self.conflict_map
                .update(&row.binding.trigger, default_trigger.clone());
            update_binding_list(
                &row.binding.name,
                default_trigger.clone(),
                &mut self.bindings,
            );
            row.binding.trigger = default_trigger;

            send_telemetry_from_ctx!(
                TelemetryEvent::KeybindingResetToDefault {
                    action: row.binding.name.clone(),
                },
                ctx
            );

            self.modifying_row = None;
            row.editor_open = false;
            ctx.enable_key_bindings_dispatching();
            ctx.notify();
        }
    }

    fn cancel_keystroke_editing(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
            match self.modifying_row.take() {
                Some(keybinding_state) => {
                    self.conflict_map.update(
                        &row.binding.trigger,
                        keybinding_state.current_binding.clone(),
                    );
                    update_binding_list(
                        &row.binding.name,
                        keybinding_state.current_binding.clone(),
                        &mut self.bindings,
                    );
                    row.binding.trigger = keybinding_state.current_binding;

                    row.editor_open = false;
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
                None => {
                    report_error!("Modifying row should exist");
                }
            }
        }
    }

    fn confirm_keystroke_editing(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
            match self.modifying_row.take() {
                Some(keybinding_state) => {
                    if let Some(key) = keybinding_state.unsaved_binding {
                        set_custom_keybinding(&row.binding.name, &key, ctx);
                        update_binding_list(
                            &row.binding.name,
                            Some(key.clone()),
                            &mut self.bindings,
                        );
                        row.binding.trigger = Some(key.clone());
                        send_telemetry_from_ctx!(
                            TelemetryEvent::KeybindingChanged {
                                action: row.binding.name.clone(),
                                keystroke: key,
                            },
                            ctx
                        );
                    }

                    row.editor_open = false;
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
                None => {
                    report_error!("Modifying row should exist");
                }
            }
        }
    }

    fn set_temporary_keystroke_state(
        &mut self,
        index: usize,
        key: Keystroke,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
            match &mut self.modifying_row {
                Some(keybinding_state) => {
                    keybinding_state.unsaved_binding = Some(key.clone());
                }
                None => {
                    report_error!("Modifying row does not exist when it should");
                }
            }

            self.conflict_map
                .update(&row.binding.trigger, Some(key.clone()));
            row.binding.trigger = Some(key);
            ctx.notify();
        }
    }
}

impl Entity for KeybindingsView {
    type Event = ();
}

impl View for KeybindingsView {
    fn ui_name() -> &'static str {
        "KeybindingsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for KeybindingsView {
    fn section() -> SettingsSection {
        SettingsSection::Keybindings
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_page_selected(&mut self, allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        // Reset previous modifying_row state.
        self.modifying_row = None;
        // `from_editable_lens` materializes any dynamic description resolver
        // before caching, so the dedup below (which compares descriptions)
        // sees concrete strings.
        let lenses: Vec<_> = ctx.editable_bindings().collect();
        self.bindings = Some(
            lenses
                .into_iter()
                .map(|lens| CommandBinding::from_editable_lens(lens, ctx))
                .sorted_by(|a, b| {
                    // Sort by description then name so that we can deduplicate bindings by name.
                    a.description
                        .in_context(DescriptionContext::Default)
                        .cmp(b.description.in_context(DescriptionContext::Default))
                        .then(a.name.cmp(&b.name))
                })
                // Effectively, editable bindings can only be used by one view, because the
                // corresponding context predicate and typed action are view-specific.
                //
                // If multiple views need equivalent bindings, we handle this by declaring
                // duplicates with the same name and description, but different actions and
                // predicates. Because bindings are saved/loaded by name, changes to one binding
                // will affect the others. To reduce clutter, only show one binding for a given name
                // and description.
                //
                // There are some bindings with the same name, but different descriptions. Because
                // we sort by description first, those bindings won't be deduplicated. This is
                // alright for now, since those bindings have slightly different semantics despite
                // being linked (e.g. find in block vs. find in terminal).
                //
                // TODO: Long-term, we should instead refactor TypedActionView so that common
                // bindings can be declared once and handled by multiple views.
                .dedup_by(|a, b| a.name == b.name && a.description == b.description)
                .collect(),
        );
        self.rows = Some(
            self.bindings
                .iter()
                .flatten()
                .map(|b| (None, b))
                .map(KeybindingRow::from)
                .collect(),
        );

        // Populate the conflict map at startup.
        self.conflict_map = self
            .bindings
            .iter()
            .flatten()
            .map(|binding| binding.trigger.clone())
            .collect();

        self.search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text(SEARCH_PLACEHOLDER, ctx);
        });

        if allow_steal_focus {
            ctx.focus(&self.search_editor);
        }
        ctx.notify();
    }

    fn on_tab_pressed(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.search_editor);
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<KeybindingsView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<KeybindingsView>) -> Self {
        SettingsPageViewHandle::Keybindings(view_handle)
    }
}

impl TypedActionView for KeybindingsView {
    type Action = KeybindingsViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use KeybindingsViewAction::*;

        match action {
            RemoveKeyStroke(index) => self.remove_keystroke(*index, ctx),
            ResetToDefaultKeyStroke(index) => self.reset_to_default_keystroke(*index, ctx),
            CancelKeyStrokeEditing(index) => self.cancel_keystroke_editing(*index, ctx),
            ConfirmKeyStroke(index) => self.confirm_keystroke_editing(*index, ctx),
            KeybindingRowClicked(index) => self.binding_row_clicked(*index, ctx),
            KeystrokeDefined(index, key) => {
                self.set_temporary_keystroke_state(*index, key.clone(), ctx)
            }
        }
    }
}

// TODO maybe this should be turned into a table ui component?
fn render_columns(
    left: Box<dyn Element>,
    right: Box<dyn Element>,
    left_column_flex: f32,
    background: Option<Fill>,
    padding: Option<Coords>,
) -> Box<dyn Element> {
    let columns = Flex::row()
        .with_child(Shrinkable::new(left_column_flex, Align::new(left).left().finish()).finish())
        .with_child(
            Shrinkable::new(1. - left_column_flex, Align::new(right).left().finish()).finish(),
        )
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .finish();

    let mut container = Container::new(
        ConstrainedBox::new(columns)
            .with_min_height(ROW_HEIGHT)
            .finish(),
    );
    if let Some(padding) = padding {
        container = container
            .with_padding_top(padding.top)
            .with_padding_bottom(padding.bottom)
            .with_padding_right(padding.right)
            .with_padding_left(padding.left);
    } else {
        container = container
            .with_padding_top(10.)
            .with_padding_bottom(10.)
            .with_padding_right(20.)
            .with_padding_left(20.);
    };
    if let Some(background) = background {
        container.with_background(background).finish()
    } else {
        container.finish()
    }
}

fn render_button(
    text: &'static str,
    appearance: &Appearance,
    line_color: themes::theme::Fill,
) -> Box<dyn Element> {
    Container::new(
        Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(line_color.into())
            .finish(),
    )
    .with_uniform_padding(4.0)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
        EDIT_BUTTONS_BORDER_RADIUS,
    )))
    .with_border(Border::all(1.).with_border_fill(line_color))
    .finish()
}

fn render_text(
    text: &str,
    styles: Option<UiComponentStyles>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut text = appearance
        .ui_builder()
        .wrappable_text(text.to_string(), true);

    if let Some(styles) = styles {
        text = text.with_style(styles);
    }

    text.build().finish()
}

/// Update the provided binding list by changing the binding with the given name to use a new
/// trigger.
fn update_binding_list(
    name: &str,
    trigger: Option<Keystroke>,
    list: &mut Option<Vec<CommandBinding>>,
) {
    let found_binding = list.as_mut().and_then(|vec| {
        vec.iter_mut()
            .find(|binding| !name.is_empty() && binding.name == name)
    });

    if let Some(binding) = found_binding {
        binding.trigger = trigger;
    }
}

fn trigger_keybinding_notifier(
    name: String,
    trigger: Option<Keystroke>,
    ctx: &mut ViewContext<KeybindingsView>,
) {
    KeybindingChangedNotifier::handle(ctx).update(ctx, move |_me, ctx| {
        ctx.emit(KeybindingChangedEvent::BindingChanged {
            binding_name: name,
            new_trigger: trigger,
        });
    })
}

#[derive(Default)]
struct KeybindingsWidget {
    local_only_icon_mouse_state: MouseStateHandle,
}

impl KeybindingsWidget {
    fn render_description(
        &self,
        bindings: Option<&Vec<CommandBinding>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let font_size = appearance.ui_font_size() + FONT_DELTA;
        let mut description = Flex::column().with_child(render_text(
            "Add your own custom keybindings to existing actions below.",
            Some(UiComponentStyles {
                font_size: Some(font_size),
                font_color: Some(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into_solid(),
                ),
                ..Default::default()
            }),
            appearance,
        ));

        if let Some(keystroke) = bindings
            .and_then(|bindings| {
                bindings
                    .iter()
                    .find(|&binding| binding.name == KEYBINDINGS_PAGE_SHORTCUT)
            })
            .and_then(|shortcut| shortcut.trigger.as_ref())
        {
            description = description.with_child(
                Wrap::row()
                    .with_child(
                        Container::new(render_text(
                            "Use",
                            Some(UiComponentStyles {
                                font_size: Some(font_size),
                                font_color: Some(
                                    appearance
                                        .theme()
                                        .sub_text_color(appearance.theme().background())
                                        .into_solid(),
                                ),
                                ..Default::default()
                            }),
                            appearance,
                        ))
                        .with_padding_right(10.)
                        .finish(),
                    )
                    .with_child(
                        appearance
                            .ui_builder()
                            .keyboard_shortcut(keystroke)
                            .with_style(UiComponentStyles {
                                margin: Some(Coords::default().right(5.)),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_child(
                        Container::new(render_text(
                            "to reference these keybindings in a side pane at anytime.",
                            Some(UiComponentStyles {
                                font_size: Some(font_size),
                                font_color: Some(
                                    appearance
                                        .theme()
                                        .sub_text_color(appearance.theme().background())
                                        .into_solid(),
                                ),
                                ..Default::default()
                            }),
                            appearance,
                        ))
                        .with_padding_left(5.)
                        .finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
        }
        description.finish()
    }

    fn render_binding_list(
        &self,
        view: &KeybindingsView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if let Some(rows) = view.rows.as_ref() {
            let rows = Flex::column().with_children(
                rows.iter()
                    .enumerate()
                    .map(|(idx, row)| {
                        row.render(
                            idx,
                            view.modifying_row.is_some() && !row.editor_open,
                            view.conflict_map.has_conflict(&row.binding.trigger),
                            appearance,
                        )
                    })
                    .collect::<Vec<_>>(),
            );

            return ClippedScrollable::vertical(
                view.clipped_scroll_state.clone(),
                rows.finish(),
                ScrollbarWidth::Auto,
                appearance
                    .theme()
                    .disabled_text_color(appearance.theme().background())
                    .into(),
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into(),
                Fill::None,
            )
            .finish();
        }
        Empty::new().finish()
    }
}

impl SettingsWidget for KeybindingsWidget {
    type View = KeybindingsView;

    fn search_terms(&self) -> &str {
        "keybindings keyboard shortcuts hotkeys"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let local_only_icon_state = if *CloudPreferencesSettings::as_ref(app).settings_sync_enabled
        {
            Some(LocalOnlyIconState::Visible {
                mouse_state: self.local_only_icon_mouse_state.clone(),
                custom_tooltip: Some("Keyboard shortcuts are not synced to the cloud".to_string()),
            })
        } else {
            None
        };

        let subheader = render_sub_header(
            appearance,
            "Configure keyboard shortcuts",
            local_only_icon_state,
        );
        let description = self.render_description(view.bindings.as_ref(), appearance);

        Flex::column()
            .with_child(subheader)
            .with_child(description)
            .with_child(render_columns(
                Container::new(render_text(
                    "Command",
                    Some(UiComponentStyles {
                        font_size: Some(appearance.ui_font_size() + FONT_DELTA),
                        ..Default::default()
                    }),
                    appearance,
                ))
                .with_uniform_margin(20.)
                .finish(),
                Container::new(ChildView::new(&view.search_bar).finish())
                    .with_margin_right(10.)
                    .finish(),
                0.62,
                None,
                Some(Coords {
                    top: 10.,
                    bottom: 0.,
                    right: 0.,
                    left: 0.,
                }),
            ))
            .with_child(Shrinkable::new(1., self.render_binding_list(view, appearance)).finish())
            .finish()
    }
}
