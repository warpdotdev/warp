use crossterm::style::Color;
use std::io;
use warpui_core::geometry::vector::Vector2F;
use warpui_core::keymap::Keystroke;
use warpui_core::{App, AppContext, Entity, Event, ModelHandle};
use warpui_tui::elements::TuiElement;
use warpui_tui::{
    vertical_scroll_lines, TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiRuntime, TuiSize,
    TuiStyle, TuiView,
};

#[derive(Clone)]
enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    fn label(&self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Assistant => "warp_tui",
        }
    }
}

#[derive(Clone)]
struct ChatMessage {
    role: ChatRole,
    text: String,
    has_toggled_color: bool,
}

impl ChatMessage {
    fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            text: text.into(),
            has_toggled_color: false,
        }
    }

    fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            text: text.into(),
            has_toggled_color: false,
        }
    }

    fn style(&self) -> TuiStyle {
        if matches!(self.role, ChatRole::Assistant) && self.has_toggled_color {
            TuiStyle::default().with_foreground_color(Color::Yellow)
        } else {
            TuiStyle::default()
        }
    }
}

struct ChatModel {
    messages: Vec<ChatMessage>,
    input: String,
    transcript_scroll_offset: usize,
    pressed_assistant_message_index: Option<usize>,
    should_quit: bool,
}

impl ChatModel {
    fn insert_text(&mut self, text: &str) {
        self.input.push_str(text);
    }

    fn delete_backward(&mut self) {
        self.input.pop();
    }

    fn submit_input(&mut self) {
        let input = self.input.trim().to_owned();
        self.input.clear();

        if input.is_empty() {
            return;
        }

        self.messages.push(ChatMessage::user(input.clone()));
        self.messages.push(ChatMessage::assistant(format!(
            "Echoing from the example app: {input}"
        )));
        self.transcript_scroll_offset = 0;
    }

    fn scroll_transcript_up(&mut self, lines: usize, max_scroll_offset: usize) {
        self.transcript_scroll_offset = self
            .transcript_scroll_offset
            .saturating_add(lines)
            .min(max_scroll_offset);
    }

    fn scroll_transcript_down(&mut self, lines: usize, max_scroll_offset: usize) {
        self.transcript_scroll_offset = self
            .transcript_scroll_offset
            .min(max_scroll_offset)
            .saturating_sub(lines);
    }

    fn begin_assistant_message_click(&mut self, message_index: usize) {
        self.pressed_assistant_message_index = Some(message_index);
    }

    fn end_assistant_message_click(&mut self, released_message_index: Option<usize>) {
        let pressed_message_index = self.pressed_assistant_message_index.take();
        if pressed_message_index != released_message_index {
            return;
        }

        let Some(message_index) = released_message_index else {
            return;
        };
        let Some(message) = self.messages.get_mut(message_index) else {
            return;
        };
        if matches!(message.role, ChatRole::Assistant) {
            message.has_toggled_color = !message.has_toggled_color;
        }
    }

    fn quit(&mut self) {
        self.should_quit = true;
    }
}

impl Entity for ChatModel {
    type Event = ();
}

struct ChatView {
    model: ModelHandle<ChatModel>,
}

impl Entity for ChatView {
    type Event = ();
}

impl TuiView for ChatView {
    type RenderOutput = Box<dyn TuiElement>;

    fn ui_name() -> &'static str {
        "ChatView"
    }

    fn render_tui(&self, app: &AppContext) -> Self::RenderOutput {
        let (messages, input, transcript_scroll_offset, pressed_assistant_message_index) =
            self.model.read(app, |model, _| {
                (
                    model.messages.clone(),
                    model.input.clone(),
                    model.transcript_scroll_offset,
                    model.pressed_assistant_message_index,
                )
            });
        Box::new(ChatElement::new(
            self.model.clone(),
            messages,
            input,
            transcript_scroll_offset,
            pressed_assistant_message_index,
        ))
    }
}

fn is_text_input(keystroke: &Keystroke, chars: &str) -> bool {
    !chars.is_empty() && !keystroke.ctrl && !keystroke.alt && !keystroke.cmd && !keystroke.meta
}

struct ChatElement {
    model: ModelHandle<ChatModel>,
    messages: Vec<ChatMessage>,
    input: String,
    transcript_scroll_offset: usize,
    pressed_assistant_message_index: Option<usize>,
    size: TuiSize,
}

#[derive(Clone)]
struct TranscriptLine {
    text: String,
    message_index: Option<usize>,
    style: TuiStyle,
}

impl TranscriptLine {
    fn spacer() -> Self {
        Self {
            text: String::new(),
            message_index: None,
            style: TuiStyle::default(),
        }
    }
}

impl ChatElement {
    fn new(
        model: ModelHandle<ChatModel>,
        messages: Vec<ChatMessage>,
        input: String,
        transcript_scroll_offset: usize,
        pressed_assistant_message_index: Option<usize>,
    ) -> Self {
        Self {
            model,
            messages,
            input,
            transcript_scroll_offset,
            pressed_assistant_message_index,
            size: TuiSize::default(),
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<TranscriptLine> {
        let mut lines = Vec::new();
        for (message_index, message) in self.messages.iter().enumerate() {
            let style = message.style();
            lines.extend(
                wrapped_lines(
                    &format!("{}: {}", message.role.label(), message.text),
                    width,
                )
                .into_iter()
                .map(|text| TranscriptLine {
                    text,
                    message_index: Some(message_index),
                    style,
                }),
            );
            lines.push(TranscriptLine::spacer());
        }
        lines.pop();
        lines
    }

    fn input_line(&self, width: u16) -> (String, usize) {
        let width = usize::from(width);
        if width == 0 {
            return (String::new(), 0);
        }

        let prompt = "› ";
        let prompt_width = prompt.chars().count().min(width);
        let available_input_width = width.saturating_sub(prompt_width);
        let input_chars = self.input.chars().collect::<Vec<_>>();
        let start_index = input_chars.len().saturating_sub(available_input_width);
        let visible_input = input_chars[start_index..].iter().collect::<String>();

        (
            format!("{prompt}{visible_input}"),
            prompt_width.saturating_add(visible_input.chars().count()),
        )
    }

    fn input_y(&self, area: TuiRect) -> u16 {
        area.bottom().saturating_sub(1)
    }

    fn header_height(&self, area: TuiRect) -> u16 {
        match area.height {
            0..=3 => 0,
            4 => 1,
            _ => 2,
        }
    }

    fn transcript_area(&self, area: TuiRect) -> TuiRect {
        let input_height = area.height.min(2);
        let input_top = area.bottom().saturating_sub(input_height);
        let transcript_top = area.y.saturating_add(self.header_height(area));
        TuiRect::new(
            area.x,
            transcript_top,
            area.width,
            input_top.saturating_sub(transcript_top),
        )
    }

    fn max_transcript_scroll_offset(&self) -> usize {
        let area = TuiRect::new(0, 0, self.size.width, self.size.height);
        let transcript_area = self.transcript_area(area);
        self.transcript_lines(transcript_area.width)
            .len()
            .saturating_sub(usize::from(transcript_area.height))
    }

    fn visible_transcript_lines(&self, area: TuiRect) -> Vec<TranscriptLine> {
        let transcript_lines = self.transcript_lines(area.width);
        let visible_transcript_height = usize::from(area.height);
        let max_scroll_offset = transcript_lines
            .len()
            .saturating_sub(visible_transcript_height);
        let scroll_offset = self.transcript_scroll_offset.min(max_scroll_offset);
        let end_index = transcript_lines.len().saturating_sub(scroll_offset);
        let start_index = end_index.saturating_sub(visible_transcript_height);
        transcript_lines[start_index..end_index].to_vec()
    }

    fn assistant_message_index_at_position(
        &self,
        area: TuiRect,
        position: Vector2F,
    ) -> Option<usize> {
        let transcript_area = self.transcript_area(area);
        if !transcript_area.contains_position(position) {
            return None;
        }

        let row = (position.y() as u16).saturating_sub(transcript_area.y);
        let message_index = self
            .visible_transcript_lines(transcript_area)
            .get(usize::from(row))?
            .message_index?;
        let message = self.messages.get(message_index)?;
        matches!(message.role, ChatRole::Assistant).then_some(message_index)
    }
}

impl TuiElement for ChatElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        self.size = constraint.max;
        self.size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        if area.is_empty() {
            return;
        }

        let header_height = self.header_height(area);
        if header_height >= 1 {
            buffer.write_str(area.x, area.y, area.width, "warpui_tui chat example");
        }
        if header_height >= 2 {
            buffer.write_str(
                area.x,
                area.y.saturating_add(1),
                area.width,
                "Enter: submit · PageUp/PageDown or scroll: transcript · Esc/Ctrl-C: quit",
            );
        }

        let transcript_area = self.transcript_area(area);
        let visible_transcript_height = usize::from(transcript_area.height);
        if visible_transcript_height > 0 {
            for (row, line) in self
                .visible_transcript_lines(transcript_area)
                .iter()
                .enumerate()
            {
                let Ok(row) = u16::try_from(row) else {
                    break;
                };
                buffer.write_str_with_style(
                    transcript_area.x,
                    transcript_area.y.saturating_add(row),
                    transcript_area.width,
                    &line.text,
                    line.style,
                );
            }
        }

        let input_height = area.height.min(2);
        let input_top = area.bottom().saturating_sub(input_height);
        if input_height == 2 {
            for x in area.x..area.right() {
                buffer.set_symbol(x, input_top, '─');
            }
        }

        let input_y = self.input_y(area);
        let (input_line, _) = self.input_line(area.width);
        buffer.write_str(area.x, input_y, area.width, &input_line);
    }

    fn desired_height(&self, _: u16) -> u16 {
        self.size.height
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        if area.is_empty() {
            return None;
        }

        let (_, cursor_offset) = self.input_line(area.width);
        let cursor_x = area
            .x
            .saturating_add(u16::try_from(cursor_offset).unwrap_or(u16::MAX))
            .min(area.right().saturating_sub(1));
        Some((cursor_x, self.input_y(area)))
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        _: &AppContext,
    ) -> bool {
        let model = self.model.clone();
        match event {
            Event::LeftMouseDown { position, .. } => {
                let Some(message_index) = self.assistant_message_index_at_position(area, *position)
                else {
                    return false;
                };
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.begin_assistant_message_click(message_index);
                        ctx.notify();
                    });
                });
                true
            }
            Event::LeftMouseUp { position, .. }
                if self.pressed_assistant_message_index.is_some() =>
            {
                let released_message_index =
                    self.assistant_message_index_at_position(area, *position);
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.end_assistant_message_click(released_message_index);
                        ctx.notify();
                    });
                });
                true
            }
            Event::KeyDown {
                keystroke, chars, ..
            } if is_text_input(keystroke, chars) => {
                let chars = chars.clone();
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.insert_text(&chars);
                        ctx.notify();
                    });
                });
                true
            }
            Event::KeyDown { keystroke, .. } if keystroke.is_unmodified_enter() => {
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.submit_input();
                        ctx.notify();
                    });
                });
                true
            }
            Event::KeyDown { keystroke, .. } if keystroke.is_unmodified_key("backspace") => {
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.delete_backward();
                        ctx.notify();
                    });
                });
                true
            }
            Event::KeyDown { keystroke, .. } if keystroke.is_unmodified_key("pageup") => {
                let max_scroll_offset = self.max_transcript_scroll_offset();
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.scroll_transcript_up(8, max_scroll_offset);
                        ctx.notify();
                    });
                });
                true
            }
            Event::KeyDown { keystroke, .. } if keystroke.is_unmodified_key("pagedown") => {
                let max_scroll_offset = self.max_transcript_scroll_offset();
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        model.scroll_transcript_down(8, max_scroll_offset);
                        ctx.notify();
                    });
                });
                true
            }
            Event::KeyDown { keystroke, .. }
                if keystroke.is_unmodified_key("escape")
                    || (keystroke.ctrl && keystroke.key == "c") =>
            {
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, _| {
                        model.quit();
                    });
                });
                true
            }
            Event::ScrollWheel { delta, .. } => {
                let lines = vertical_scroll_lines(*delta);
                if lines == 0 {
                    return false;
                }

                let max_scroll_offset = self.max_transcript_scroll_offset();
                ctx.dispatch_app_update(move |app| {
                    model.update(app, |model, ctx| {
                        if lines > 0 {
                            model.scroll_transcript_up(
                                lines.unsigned_abs().into(),
                                max_scroll_offset,
                            );
                        } else {
                            model.scroll_transcript_down(
                                lines.unsigned_abs().into(),
                                max_scroll_offset,
                            );
                        }
                        ctx.notify();
                    });
                });
                true
            }
            _ => false,
        }
    }
}

fn wrapped_lines(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let width = usize::from(width);
    text.split('\n')
        .flat_map(|line| {
            if line.is_empty() {
                return vec![String::new()];
            }

            line.chars()
                .collect::<Vec<_>>()
                .chunks(width)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn main() -> io::Result<()> {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| ChatModel {
            messages: vec![
                ChatMessage::assistant("This is a tiny chat shell rendered through warpui_tui."),
                ChatMessage::assistant(
                    "Type a message below and press Enter to append it to the transcript.",
                ),
            ],
            input: String::new(),
            transcript_scroll_offset: 0,
            pressed_assistant_message_index: None,
            should_quit: false,
        });
        let (window_id, root_view) = app.add_tui_window(|_| ChatView {
            model: model.clone(),
        });
        let mut runtime = TuiRuntime::enter(&app, window_id, root_view)?;
        runtime.run_until(&mut app, |app| {
            model.read(app, |model, _| model.should_quit)
        })?;

        Ok(())
    })
}
