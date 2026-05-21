use pathfinder_color::ColorU;
use warp_editor::editor::NavigationKey;
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, ParentElement, Radius, Wrap, WrapFill,
    },
    fonts::FamilyId,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    appearance::Appearance,
    editor::{EditorView, Event, InteractionState, SingleLineEditorOptions, TextOptions},
};
use crate::{editor::PropagateAndNoOpNavigationKeys, themes::theme::Fill};

pub struct WordBlockEditorView {
    editor_view: ViewHandle<EditorView>,
    separators: Vec<char>,
    list_of_words: Vec<Word>,
    word_validator: Box<dyn Fn(&str) -> bool>,
    /// TODO(seikun): chip max width is necessary for now as the current implementation of
    /// wrap does not have a way of limiting max length of children elements along its
    /// alignment axis.
    chip_max_width: f32,
    layout: WordBlockLayout,
    style_fn: Box<dyn Fn(&AppContext) -> WordBlockEditorStyles>,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum WordBlockLayout {
    /// Word chips are shown above the text editor.
    #[default]
    Vertical,
    /// Word chips are shown alongside the text editor.
    Horizontal { editor_min_width: f32 },
}

pub struct WordBlockEditorStyles {
    pub font_family: FamilyId,
    pub editor_font_color: ColorU,
    pub background: Fill,
    pub valid_word_styles: WordBlockStyles,
    pub invalid_word_styles: WordBlockStyles,
}

pub struct WordBlockStyles {
    pub font_color: ColorU,
    pub background: Fill,
}

struct Word {
    text: String,
    mouse_state_handle: MouseStateHandle,
}

pub struct ChipEditorState {
    pub is_valid: bool,   // track whether list of words is valid (based on validator)
    pub is_empty: bool,   // track whether there are any elements in the list of words so far
    pub num_chips: usize, // track the # of chipped words
}

// Wrapper class to editor view that will separate word chips into visual blocks delimited
// by a user specified `separator` char. Can additionally pass in a validator to mark
// invalid words in an error color.
impl WordBlockEditorView {
    pub fn new(
        ctx: &mut ViewContext<Self>,
        placeholder: &str,
        ui_font_size: f32,
        separators: Vec<char>,
        chip_max_width: f32,
        word_validator: Box<dyn Fn(&str) -> bool>,
    ) -> Self {
        let editor_view = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(ui_font_size),
                    ..Default::default()
                },
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });

        editor_view.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text(placeholder, ctx);
        });

        ctx.subscribe_to_view(&editor_view, |me, _, event, ctx| {
            me.handle_editor_view_event(event, ctx)
        });

        Self {
            editor_view,
            separators,
            list_of_words: Vec::new(),
            word_validator,
            chip_max_width,
            layout: WordBlockLayout::default(),
            style_fn: Box::new(WordBlockEditorStyles::default_styles),
        }
    }

    // Return list of words that fail the validator check, includes text in the buffer
    pub fn get_list_of_invalid_words(&self, ctx: &AppContext) -> Vec<String> {
        let mut words: Vec<String> = self
            .list_of_words
            .iter()
            .filter(|word| !(self.word_validator)(&word.text))
            .map(|word| word.text.clone())
            .collect();

        let buffer_text = self.editor_view.as_ref(ctx).buffer_text(ctx);
        if !buffer_text.is_empty() && !(self.word_validator)(&buffer_text) {
            words.push(buffer_text);
        }
        words
    }

    // Return list of words that are chipped + text in the buffer as a separate word
    pub fn get_list_of_words(&self, ctx: &AppContext) -> Vec<String> {
        let mut words: Vec<String> = self
            .list_of_words
            .iter()
            .map(|word| word.text.clone())
            .collect();

        let buffer_text = self.editor_view.as_ref(ctx).buffer_text(ctx);
        if !buffer_text.is_empty() {
            words.push(buffer_text);
        }
        words
    }

    pub fn num_chips(&self) -> usize {
        self.list_of_words.len()
    }

    pub fn with_validator(
        &mut self,
        ctx: &mut ViewContext<Self>,
        word_validator: Box<dyn Fn(&str) -> bool>,
    ) {
        self.word_validator = word_validator;
        ctx.emit(WordBlockEditorViewEvent::WordListValidityChanged);
        ctx.notify();
    }

    pub fn with_layout(mut self, layout: WordBlockLayout) -> Self {
        // No need for ctx.notify - because this takes `self`, it can only be called when adding
        // the view.
        self.layout = layout;
        self
    }

    pub fn with_styles(
        mut self,
        ctx: &mut ViewContext<Self>,
        styles: impl Fn(&AppContext) -> WordBlockEditorStyles + 'static,
    ) -> Self {
        let initial_styles = styles(ctx);
        self.editor_view.update(ctx, |editor, ctx| {
            editor.set_font_family(initial_styles.font_family, ctx);
        });
        self.style_fn = Box::new(styles);
        self
    }

    /// Set the word input's [PropagateAndNoOpNavigationKeys] behavior. This allows navigation in
    /// form-like views that include a [WordBlockEditorView].
    pub fn set_propagate_navigation_keys(
        &mut self,
        propagate_navigation_keys: PropagateAndNoOpNavigationKeys,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor_view.update(ctx, |editor, _| {
            editor.set_propagate_vertical_navigation_keys(propagate_navigation_keys);
        })
    }

    pub fn clear_list_of_words(&mut self, ctx: &mut ViewContext<Self>) {
        self.list_of_words = Vec::new();
        self.editor_view.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        ctx.notify();
    }

    pub fn add_word(&mut self, word: &String, ctx: &mut ViewContext<Self>) {
        self.list_of_words.push(Word {
            text: word.to_owned(),
            mouse_state_handle: Default::default(),
        });
        ctx.emit(WordBlockEditorViewEvent::WordListValidityChanged);
        ctx.notify();
    }

    pub fn set_editor_buffer_text(&mut self, word: &str, ctx: &mut ViewContext<Self>) {
        self.editor_view.update(ctx, |editor, ctx| {
            editor.set_buffer_text(word, ctx);
        });
        ctx.notify();
    }

    /// Forwards the interaction state to the inner editor view.
    pub fn set_interaction_state(&mut self, state: InteractionState, ctx: &mut ViewContext<Self>) {
        self.editor_view.update(ctx, |editor, ctx| {
            editor.set_interaction_state(state, ctx);
            ctx.notify();
        });
    }

    fn delete_word(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.list_of_words.remove(index);
        ctx.emit(WordBlockEditorViewEvent::WordListValidityChanged);
        ctx.notify();
    }

    fn handle_editor_view_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        match event {
            Event::Edited(_) => {
                let buffer_text = self.editor_view.as_ref(ctx).buffer_text(ctx);

                // there's words to be parsed only if there's a separator character
                if buffer_text.chars().any(|c| self.separators.contains(&c)) {
                    let mut parts: Vec<&str> = buffer_text
                        .split(|c| self.separators.contains(&c))
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();

                    if !parts.is_empty() {
                        // if text does not end in a separator, keep last part in buffer
                        let remaining_buffer_text =
                            if !self.separators.iter().any(|&c| buffer_text.ends_with(c)) {
                                parts.pop()
                            } else {
                                None
                            };

                        for part in parts {
                            self.list_of_words.push(Word {
                                text: part.into(),
                                mouse_state_handle: Default::default(),
                            });
                        }

                        self.editor_view.update(ctx, |editor, ctx| {
                            editor.clear_buffer_and_reset_undo_stack(ctx);
                            if let Some(text) = remaining_buffer_text {
                                editor.set_buffer_text(text, ctx);
                            }
                        });
                    }
                }
                ctx.emit(WordBlockEditorViewEvent::WordListValidityChanged);
                ctx.notify();
            }
            Event::BackspaceOnEmptyBuffer => {
                if !self.list_of_words.is_empty() {
                    self.list_of_words.pop();
                    ctx.emit(WordBlockEditorViewEvent::WordListValidityChanged);
                    ctx.notify();
                }
            }
            Event::Escape => ctx.emit(WordBlockEditorViewEvent::Escape),
            Event::Enter => ctx.emit(WordBlockEditorViewEvent::Enter),
            Event::Navigate(key) => ctx.emit(WordBlockEditorViewEvent::Navigate(*key)),
            _ => (),
        }
    }
}

#[derive(Debug, Clone)]
pub enum WordBlockEditorViewAction {
    DeleteWord { index: usize },
}

impl TypedActionView for WordBlockEditorView {
    type Action = WordBlockEditorViewAction;
    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WordBlockEditorViewAction::DeleteWord { index } => self.delete_word(*index, ctx),
        }
    }
}

#[derive(Clone)]
pub enum WordBlockEditorViewEvent {
    // Emitted whenever anything happens that can affect validity of all the words in the
    // list:
    // 1. words in list are removed or added
    // 2. validator changes
    WordListValidityChanged,
    Escape,
    Enter,
    Navigate(NavigationKey),
}

impl Entity for WordBlockEditorView {
    type Event = WordBlockEditorViewEvent;
}

impl View for WordBlockEditorView {
    fn ui_name() -> &'static str {
        "WordBlockEditorView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor_view);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let styles = (self.style_fn)(app);

        // Add existing words.
        let word_chips = self.list_of_words.iter().enumerate().map(|(idx, word)| {
            let is_word_valid = (self.word_validator)(&word.text);

            let (background_color, text_color) = if is_word_valid {
                (
                    styles.valid_word_styles.background,
                    styles.valid_word_styles.font_color,
                )
            } else {
                (
                    styles.invalid_word_styles.background,
                    styles.invalid_word_styles.font_color,
                )
            };

            let mut word_and_button = Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);

            // Add word text
            word_and_button.add_child(
                ConstrainedBox::new(
                    appearance
                        .ui_builder()
                        .span(word.text.clone())
                        .with_style(UiComponentStyles {
                            font_color: Some(text_color),
                            font_family_id: Some(styles.font_family),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_max_width(self.chip_max_width)
                .finish(),
            );

            // Add action button
            word_and_button.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .close_button(16., word.mouse_state_handle.clone())
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(WordBlockEditorViewAction::DeleteWord {
                                index: idx,
                            })
                        })
                        .finish(),
                )
                .with_margin_left(5.)
                .finish(),
            );

            Container::new(word_and_button.finish())
                .with_padding_left(5.)
                .with_vertical_padding(4.)
                .with_padding_right(4.)
                .with_horizontal_margin(3.)
                .with_background(background_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                .finish()
        });

        let editor = Container::new(
            appearance
                .ui_builder()
                .text_input(self.editor_view.clone())
                .with_style(UiComponentStyles {
                    background: Some(styles.background.into()),
                    font_color: Some(styles.editor_font_color),
                    border_width: Some(0.),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
        .finish();

        if self.list_of_words.is_empty() {
            editor
        } else {
            let wrapping_section = Wrap::row()
                .with_run_spacing(10.)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Min)
                .with_children(word_chips);
            match self.layout {
                WordBlockLayout::Vertical => {
                    let wrapping_section_container = Container::new(wrapping_section.finish())
                        .with_margin_bottom(4.)
                        .with_margin_top(3.)
                        .with_horizontal_margin(4.)
                        .finish();
                    Flex::column()
                        .with_children([wrapping_section_container, editor])
                        .finish()
                }
                WordBlockLayout::Horizontal { editor_min_width } => {
                    let wrapping_section = wrapping_section
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(WrapFill::new(editor_min_width, editor).finish())
                        .finish();
                    Container::new(wrapping_section)
                        .with_vertical_padding(6.)
                        .with_horizontal_padding(4.)
                        .finish()
                }
            }
        }
    }
}

impl WordBlockEditorStyles {
    fn default_styles(app: &AppContext) -> WordBlockEditorStyles {
        let appearance = Appearance::as_ref(app);
        let text_color = appearance
            .theme()
            .main_text_color(appearance.theme().background())
            .into_solid();

        WordBlockEditorStyles {
            font_family: appearance.monospace_font_family(),
            editor_font_color: text_color,
            background: appearance.theme().background(),
            valid_word_styles: WordBlockStyles {
                font_color: text_color,
                background: appearance.theme().nonactive_ui_detail(),
            },
            invalid_word_styles: WordBlockStyles {
                font_color: text_color,
                background: Fill::from(appearance.theme().ui_error_color()).with_opacity(30),
            },
        }
    }
}
