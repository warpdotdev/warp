//! A UI sample demonstrating how the SelectableArea element can be used.

use markdown_parser::{parse_markdown, FormattedTextFragment, FormattedTextLine};
use warpui::color::ColorU;
use warpui::elements::{
    Align, ChildView, ConstrainedBox, Flex, FormattedTextElement, HeadingFontSizeMultipliers,
    HighlightedHyperlink, HyperlinkLens, ParentElement, Rect, SelectableArea, SelectionHandle,
    Stack, Text,
};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

pub struct RootView {
    sub_view: ViewHandle<FormattedTextView>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let sub_view = ctx.add_view(|ctx| {
            let font_family = warpui::fonts::Cache::handle(ctx).update(ctx, |cache, _| {
                cache.load_system_font("Menlo").expect("Should load Menlo")
            });
            let view = FormattedTextView {
                font_family,
                highlighted_link: Default::default(),
                selectable_area_state_handle: Default::default(),
            };
            ctx.focus_self();
            view
        });
        Self { sub_view }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.sub_view).finish()
    }
}

pub struct FormattedTextView {
    font_family: FamilyId,
    highlighted_link: HighlightedHyperlink,
    selectable_area_state_handle: SelectionHandle,
}

impl Entity for FormattedTextView {
    type Event = ();
}

impl View for FormattedTextView {
    fn ui_name() -> &'static str {
        "SelectableExampleView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(
                SelectableArea::new(
                    self.selectable_area_state_handle.clone(),
                    |selection_args, _, _| {
                        println!("Selected text: {:?}", selection_args.selection);
                    },
                    Align::new(
                        ConstrainedBox::new(
                            Flex::column()
                                .with_children([
                                    FormattedTextElement::new(
                                        parse_markdown(concat!(
                                            "## This is a markdown header\n",
                                            "### This is a subheader\n",
                                            "This is a ~~strikethrough~~ text.\n",
                                            "* list item 1\n",
                                            "* list item 2\n",
                                            "* list item 3\n",
                                            "* list item 4\n",
                                            "```rust\n",
                                            "fn main() {\n",
                                            "    println!(\"Hello, world!\");\n",
                                            "}\n",
                                            "```\n",
                                            "fi\n",
                                            "cd\n",
                                            "this is a [link](https://www.google.com)\n",
                                        ))
                                        .unwrap()
                                        .append_line(
                                            FormattedTextLine::Line(vec![
                                                FormattedTextFragment::plain_text(
                                                    "\nThis is a link that dispatches an action: ",
                                                ),
                                                FormattedTextFragment::hyperlink_action(
                                                    "press enter",
                                                    RootViewAction::LinkClicked,
                                                ),
                                            ]),
                                        ),
                                        13.,
                                        self.font_family,
                                        self.font_family,
                                        ColorU::white(),
                                        self.highlighted_link.clone(),
                                    )
                                    .with_line_height_ratio(1.2)
                                    .with_heading_to_font_size_multipliers(
                                        HeadingFontSizeMultipliers {
                                            h1: 1.8,
                                            h2: 1.5,
                                            h3: 1.2,
                                            ..Default::default()
                                        },
                                    )
                                    .register_default_click_handlers_with_action_support(
                                        |hyperlink_lens, evt, ctx| match hyperlink_lens {
                                            HyperlinkLens::Url(url) => {
                                                ctx.open_url(url);
                                            }
                                            HyperlinkLens::Action(action_ref) => {
                                                if let Some(root_action) = action_ref
                                                    .as_any()
                                                    .downcast_ref::<RootViewAction>(
                                                ) {
                                                    evt.dispatch_typed_action(root_action.clone());
                                                }
                                            }
                                        },
                                    )
                                    .set_selectable(true)
                                    .finish(),
                                    Text::new(
                                        "This is normal Text (large font, not a header)",
                                        self.font_family,
                                        13. * 1.5,
                                    )
                                    .finish(),
                                ])
                                .finish(),
                        )
                        .with_max_width(700.)
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum RootViewAction {
    LinkClicked,
}

impl TypedActionView for RootView {
    type Action = RootViewAction;

    fn handle_action(&mut self, action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        match action {
            RootViewAction::LinkClicked => {
                println!("Link clicked");
            }
        }
    }
}
