use pathfinder_color::ColorU;
use warpui::elements::{Align, ConstrainedBox, ParentElement, Rect, Stack, Text};
use warpui::fonts::FamilyId;
use warpui::keymap::FixedBinding;
use warpui::presenter::ChildView;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

// We could initiate global action and bindings here.
pub fn init(ctx: &mut AppContext) {
    use warpui::keymap::macros::*;

    // Add bindings to trigger actions in the subview.
    ctx.register_fixed_bindings([
        FixedBinding::new(
            "cmdorctrl-enter",
            SubViewAction::ToggleRedRect,
            id!("SubView"),
        ),
        FixedBinding::new("enter", SubViewAction::ToggleText, id!("SubView")),
    ]);
}

pub struct RootView {
    // RootView "owns" a viewhandle to the subview.
    sub_view: ViewHandle<SubView>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Adding typed action view allows the view to receive keydown events.
        let sub_view = ctx.add_typed_action_view(|ctx| {
            let menlo = warpui::fonts::Cache::handle(ctx).update(ctx, |cache, _| {
                cache.load_system_font("Menlo").expect("Should load Menlo")
            });
            let view = SubView {
                display_red_rect: true,
                display_text: true,
                menlo_font_family: menlo,
            };
            // Need the view to be focused for keydown actions to be dispatched to it.
            ctx.focus_self();
            view
        });
        Self { sub_view }
    }
}

// Implement the entity trait.
impl Entity for RootView {
    type Event = ();
}

// Implement the view trait so RootView could be considered as a view.
impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    // Renders the child view of sub_view.
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.sub_view).finish()
    }
}

#[derive(Debug, Clone)]
pub enum SubViewAction {
    ToggleRedRect,
    ToggleText,
}

pub struct SubView {
    display_red_rect: bool,
    display_text: bool,
    menlo_font_family: FamilyId,
}

// Implement the entity trait.
impl Entity for SubView {
    type Event = ();
}

// Implement the view trait so SubView could be considered as a view.
impl View for SubView {
    fn ui_name() -> &'static str {
        "SubView"
    }

    // Renders a stack of a centered solid red box and some text on top.
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        // Half transparent black background.
        let mut stack = Stack::new().with_child(
            Rect::new()
                .with_background_color(ColorU::new(0, 0, 0, 150))
                .finish(),
        );

        // If flag is true, display a solid red box.
        if self.display_red_rect {
            stack.add_child(
                Align::new(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background_color(ColorU::new(255, 0, 0, 255))
                            .finish(),
                    )
                    .with_width(300.)
                    .with_height(200.)
                    .finish(),
                )
                .finish(),
            );
        }

        // If flag is true, display some texts.
        if self.display_text {
            stack.add_child(
                Align::new(
                    ConstrainedBox::new(
                        Text::new_inline(
                            "This is some text for testing",
                            self.menlo_font_family,
                            12.,
                        )
                        .finish(),
                    )
                    .with_width(250.)
                    .finish(),
                )
                .finish(),
            )
        };

        stack.finish()
    }
}

impl TypedActionView for SubView {
    type Action = SubViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SubViewAction::ToggleRedRect => self.display_red_rect = !self.display_red_rect,
            SubViewAction::ToggleText => self.display_text = !self.display_text,
        };
        ctx.notify();
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
