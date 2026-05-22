use std::iter;
use std::path::Path;

use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Expanded, Flex, HighlightedRange, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Shrinkable, Text,
};
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Element, EventContext, SingletonEntity, ViewHandle};

use crate::ai::agent::ProgrammingLanguage;
use crate::ai::blocklist::inline_action::inline_action_header::{
    INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::appearance::Appearance;
use crate::code::editor::view::CodeEditorView;
use crate::code::editor_management::CodeSource;
use crate::search::files::icon::icon_from_file_path;
use crate::search::ItemHighlightState;
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::{icon_button, icon_button_with_color};
use crate::ui_components::icons::Icon;

const CODE_BLOCK_CORNER_RADIUS: f32 = 8.0;

#[derive(Default, Clone)]
pub struct CodeSnippetButtonHandles {
    pub open_button: MouseStateHandle,
    pub copy_button: MouseStateHandle,
    pub insert_button: MouseStateHandle,
}

impl CodeSnippetButtonHandles {
    // Resets the hover state of all buttons that trigger a focus change.
    pub fn reset_hover_state_on_focus_change(&self) {
        if let Ok(mut state) = self.open_button.lock() {
            state.reset_hover_state();
        }
    }
}

pub type HandleCode = Box<dyn FnMut(String, &mut EventContext)>;

fn render_file_icon(path: &Path, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(icon_from_file_path(
            path.to_str().unwrap_or_default(),
            appearance,
            ItemHighlightState::Default,
        ))
        .with_width(icon_size(app))
        .with_height(icon_size(app))
        .finish(),
    )
    .with_margin_right(8.)
    .finish()
}

fn render_button<F>(
    appearance: &Appearance,
    icon: Icon,
    tooltip_text: &str,
    mouse_handle: MouseStateHandle,
    formatted_text: String,
    on_click: F,
    color: Option<Fill>,
) -> Container
where
    F: FnMut(String, &mut EventContext) + 'static,
{
    let ui_builder = appearance.ui_builder().clone();
    let tooltip_text = tooltip_text.to_owned();
    let mut on_click = on_click;
    let button_element = if let Some(color) = color {
        icon_button_with_color(appearance, icon, false, mouse_handle, color)
    } else {
        icon_button(appearance, icon, false, mouse_handle)
    };

    Container::new(
        button_element
            .with_tooltip(move || ui_builder.tool_tip(tooltip_text.clone()).build().finish())
            .build()
            .on_click(move |ctx, _, _| {
                on_click(formatted_text.clone(), ctx);
            })
            .finish(),
    )
}

pub struct CodeBlockOptions {
    pub on_open: Option<HandleCode>,
    pub on_execute: Option<HandleCode>,
    pub on_copy: Option<HandleCode>,
    pub on_insert: Option<HandleCode>,
    pub footer_element: Option<Box<dyn Element>>,
    pub mouse_handles: Option<CodeSnippetButtonHandles>,
    pub file_path: Option<String>,
}

pub fn render_code_block_with_warp_text(
    options: CodeBlockOptions,
    view: &ViewHandle<CodeEditorView>,
    app: &AppContext,
    source: Option<CodeSource>,
) -> Box<dyn Element> {
    let code = view.as_ref(app).text(app);
    let code_element = ChildView::new(view).finish();

    render_code_block_internal(code.as_str(), code_element, options, app, source, true)
}

pub fn render_code_block_plain(
    code: &str,
    find_highlight_ranges: impl Iterator<Item = HighlightedRange>,
    options: CodeBlockOptions,
    selectable: bool,
    app: &AppContext,
    source: Option<CodeSource>,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let code_element = Text::new(
        code.to_owned(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(theme, theme.surface_1()))
    .with_highlights(find_highlight_ranges)
    .with_selectable(selectable)
    .finish();

    render_code_block_internal(code, code_element, options, app, source, false)
}

/// Renders a code snippet with a language label and optional buttons.
/// This command did not come from Agent Mode.
pub fn render_runnable_code_snippet(
    code_snippet: &str,
    language: Option<&ProgrammingLanguage>,
    on_execute: Option<HandleCode>,
    on_copy: Option<HandleCode>,
    mouse_handles: Option<CodeSnippetButtonHandles>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let language_text = language.map(|language| {
        Text::new_inline(
            language.display_name(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_3()))
        .finish()
    });
    let allow_execution = language.is_none_or(|lang| lang.is_shell());
    render_code_block_plain(
        code_snippet,
        Box::new(iter::empty()),
        CodeBlockOptions {
            on_open: None,
            on_execute: if allow_execution { on_execute } else { None },
            on_copy,
            on_insert: None,
            footer_element: language_text,
            mouse_handles,
            file_path: None,
        },
        true,
        app,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_linked_code_block_internal(
    file_icon: Box<dyn Element>,
    file_path_text: Box<dyn Element>,
    code: &str,
    code_element: Box<dyn Element>,
    on_open: Option<HandleCode>,
    on_copy: Option<HandleCode>,
    on_insert: Option<HandleCode>,
    insert_text: Option<String>,
    mouse_handles: Option<CodeSnippetButtonHandles>,
    appearance: &Appearance,
) -> Flex {
    let theme = appearance.theme();
    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let mut header_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);
    header_row.add_child(file_icon);
    header_row.add_child(
        Shrinkable::new(
            0.9,
            Container::new(file_path_text)
                .with_margin_right(8.)
                .finish(),
        )
        .finish(),
    );
    header_row.add_child(Expanded::new(0.1, Empty::new().finish()).finish());

    if let Some(mouse_handles) = mouse_handles {
        let mut action_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let code_clone = code.to_owned();

        if let (Some(on_insert), Some(insert_text)) = (on_insert, insert_text) {
            let insert_button = render_button(
                appearance,
                Icon::AtSign,
                "Add as Context",
                mouse_handles.insert_button,
                insert_text,
                on_insert,
                Some(Fill::from(blended_colors::text_main(
                    theme,
                    theme.surface_1(),
                ))),
            )
            .with_margin_left(8.)
            .finish();

            action_row.add_child(insert_button);
        }

        if let Some(on_copy) = on_copy {
            let copy_button = render_button(
                appearance,
                Icon::Copy,
                "Copy",
                mouse_handles.copy_button,
                code_clone.clone(),
                on_copy,
                Some(Fill::from(blended_colors::text_main(
                    theme,
                    theme.surface_1(),
                ))),
            )
            .with_margin_left(8.)
            .finish();

            action_row.add_child(copy_button);
        }

        if let Some(on_open) = on_open {
            let open_button = render_button(
                appearance,
                Icon::LinkExternal,
                "Open in Warp",
                mouse_handles.open_button,
                code_clone.clone(),
                on_open,
                Some(Fill::from(blended_colors::text_main(
                    theme,
                    theme.surface_1(),
                ))),
            )
            .with_margin_left(8.)
            .finish();
            action_row.add_child(open_button);
        }

        header_row.add_child(action_row.finish());
    }

    content.add_child(
        Container::new(header_row.finish())
            .with_background(theme.surface_2())
            .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(
                CODE_BLOCK_CORNER_RADIUS,
            )))
            .finish(),
    );
    content.add_child(
        Container::new(code_element)
            .with_background(theme.background())
            .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(
                CODE_BLOCK_CORNER_RADIUS,
            )))
            .finish(),
    );

    content
}

#[allow(clippy::too_many_arguments)]
fn render_plain_code_block_internal(
    code: &str,
    code_element: Box<dyn Element>,
    footer_element: Option<Box<dyn Element>>,
    on_execute: Option<HandleCode>,
    on_copy: Option<HandleCode>,
    mouse_handles: Option<CodeSnippetButtonHandles>,
    appearance: &Appearance,
    without_extra_padding_between_code_and_footer: bool,
) -> Flex {
    let theme = appearance.theme();
    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let mut footer_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(footer_element.unwrap_or_else(|| Empty::new().finish()));

    if let Some(mouse_handles) = mouse_handles {
        let mut action_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let code_clone = code.to_owned();

        if let Some(on_copy) = on_copy {
            let copy_button = render_button(
                appearance,
                Icon::Copy,
                "Copy",
                mouse_handles.copy_button,
                code_clone.clone(),
                on_copy,
                None,
            )
            .finish();

            action_row.add_child(copy_button);
        }

        if let Some(on_execute) = on_execute {
            let insert_button = render_button(
                appearance,
                Icon::TerminalInput,
                "Run in terminal",
                mouse_handles.insert_button,
                code_clone.clone(),
                on_execute,
                None,
            )
            .with_margin_left(8.)
            .finish();

            action_row.add_child(insert_button);
        }

        footer_row.add_child(action_row.finish());
    }

    let code_padding_bottom = if without_extra_padding_between_code_and_footer {
        0.
    } else {
        INLINE_ACTION_HEADER_VERTICAL_PADDING
    };
    let footer_padding_top = if without_extra_padding_between_code_and_footer {
        0.
    } else {
        6.
    };

    content.add_child(
        Container::new(code_element)
            .with_background(theme.surface_2())
            .with_padding_top(INLINE_ACTION_HEADER_VERTICAL_PADDING)
            .with_padding_bottom(code_padding_bottom)
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(
                CODE_BLOCK_CORNER_RADIUS,
            )))
            .finish(),
    );

    content.add_child(
        Container::new(
            footer_row
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .finish(),
        )
        .with_background(theme.surface_2())
        .with_padding_top(footer_padding_top)
        .with_padding_bottom(12.)
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(
            CODE_BLOCK_CORNER_RADIUS,
        )))
        .finish(),
    );

    content
}

fn render_code_block_internal(
    code: &str,
    code_element: Box<dyn Element>,
    CodeBlockOptions {
        on_open,
        on_execute,
        on_copy,
        on_insert,
        footer_element,
        mouse_handles,
        file_path,
    }: CodeBlockOptions,
    app: &AppContext,
    source: Option<CodeSource>,
    without_extra_padding_between_code_and_footer: bool,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let content = match (source.as_ref(), file_path) {
        (
            Some(CodeSource::Link {
                path,
                range_start,
                range_end,
            }),
            Some(file_path),
        ) => {
            let file_path_text = Text::new_inline(
                format!(
                    "{}{}",
                    file_path,
                    match (range_start, range_end) {
                        (Some(ls), Some(le)) => format!(" ({}-{})", ls.line_num, le.line_num),
                        _ => String::new(),
                    }
                ),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(blended_colors::text_main(theme, theme.background()))
            .finish();
            let formatted_insert_text = {
                let line_number =
                    range_start.map_or(String::new(), |start| format!(":{}", start.line_num));
                Some(format!("{file_path}{line_number}"))
            };

            render_linked_code_block_internal(
                render_file_icon(path, appearance, app),
                file_path_text,
                code,
                code_element,
                on_open,
                on_copy,
                on_insert,
                formatted_insert_text,
                mouse_handles,
                appearance,
            )
        }
        _ => render_plain_code_block_internal(
            code,
            code_element,
            footer_element,
            on_execute,
            on_copy,
            mouse_handles,
            appearance,
            without_extra_padding_between_code_and_footer,
        ),
    };

    Container::new(content.finish())
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            CODE_BLOCK_CORNER_RADIUS,
        )))
        .finish()
}
