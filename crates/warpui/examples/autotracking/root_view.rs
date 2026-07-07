use pathfinder_color::ColorU;
use warpui::elements::{
    Align, Border, ChildView, Container, CornerRadius, DispatchEventResult, EventHandler, Flex,
    ParentElement, Radius, Rect, Stack, Text,
};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, Tracked, TypedActionView, View,
    ViewContext, ViewHandle,
};

pub fn init(ctx: &mut AppContext) {
    ctx.add_singleton_model(|_| Settings {
        dark_mode: Tracked::new(false),
    });
}

pub struct RootView {
    main: ViewHandle<MainView>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let main = ctx.add_typed_action_view(MainView::new);

        RootView { main }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let dark_mode = *Settings::as_ref(app).dark_mode;

        Stack::new()
            .with_child(
                Rect::new()
                    .with_background_color(if dark_mode {
                        ColorU::black()
                    } else {
                        ColorU::white()
                    })
                    .finish(),
            )
            .with_child(ChildView::new(&self.main).finish())
            .finish()
    }
}

struct Settings {
    dark_mode: Tracked<bool>,
}

impl Entity for Settings {
    type Event = ();
}

impl SingletonEntity for Settings {}

#[derive(Default)]
struct Counter {
    value: Tracked<isize>,
}

impl Counter {
    fn increment(&mut self) {
        *self.value += 1;
    }

    fn decrement(&mut self) {
        *self.value -= 1;
    }

    fn value(&self) -> isize {
        *self.value
    }
}

impl Entity for Counter {
    type Event = ();
}

struct MainView {
    model: ModelHandle<Counter>,
    stored: Tracked<Option<isize>>,
    font_family: FamilyId,
}

#[derive(Clone, Copy, Debug)]
enum MainViewAction {
    Increment,
    Decrement,
    Save,
    Restore,
    ToggleDarkMode,
}

impl MainView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model = ctx.add_model(|_| Counter::default());
        let font_family = warpui::fonts::Cache::handle(ctx)
            .update(ctx, |cache, _| cache.load_system_font("Arial").unwrap());
        MainView {
            model,
            stored: Tracked::new(None),
            font_family,
        }
    }
}

impl Entity for MainView {
    type Event = ();
}

impl TypedActionView for MainView {
    type Action = MainViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use MainViewAction::*;

        match action {
            Increment => self.model.update(ctx, |model, _| model.increment()),
            Decrement => self.model.update(ctx, |model, _| model.decrement()),
            Save => {
                let current = self.model.read(ctx, |model, _| model.value());
                *self.stored = Some(current);
            }
            Restore => {
                if let Some(stored) = self.stored.take() {
                    self.model.update(ctx, |model, _| *model.value = stored);
                }
            }
            ToggleDarkMode => {
                Settings::handle(ctx).update(ctx, |settings, _| {
                    *settings.dark_mode = !*settings.dark_mode;
                });
            }
        }
    }
}

impl View for MainView {
    fn ui_name() -> &'static str {
        "MainView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let dark_mode = *Settings::as_ref(app).dark_mode;
        let text_color = if dark_mode {
            ColorU::white()
        } else {
            ColorU::black()
        };
        let counter = self.model.as_ref(app).value();

        Align::new(
            Flex::column()
                .with_child(
                    Align::new(
                        Flex::row()
                            .with_child(render_button(
                                Text::new_inline("-", self.font_family, 16.)
                                    .with_color(text_color)
                                    .finish(),
                                MainViewAction::Decrement,
                            ))
                            .with_child(
                                Text::new_inline(format!("{counter}"), self.font_family, 16.)
                                    .with_color(text_color)
                                    .finish(),
                            )
                            .with_child(render_button(
                                Text::new_inline("+", self.font_family, 16.)
                                    .with_color(text_color)
                                    .finish(),
                                MainViewAction::Increment,
                            ))
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Align::new(
                        Flex::row()
                            .with_child(render_button(
                                Text::new_inline("Toggle Dark Mode", self.font_family, 16.)
                                    .with_color(text_color)
                                    .finish(),
                                MainViewAction::ToggleDarkMode,
                            ))
                            .with_child(if self.stored.is_some() {
                                render_button(
                                    Text::new_inline("Restore", self.font_family, 16.)
                                        .with_color(text_color)
                                        .finish(),
                                    MainViewAction::Restore,
                                )
                            } else {
                                render_button(
                                    Text::new_inline("Save Value", self.font_family, 16.)
                                        .with_color(text_color)
                                        .finish(),
                                    MainViewAction::Save,
                                )
                            })
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .finish()
    }
}

fn render_button(inner: Box<dyn Element>, action: MainViewAction) -> Box<dyn Element> {
    Container::new(
        EventHandler::new(
            Container::new(inner)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(Border::all(1.).with_border_color(ColorU::new(128, 128, 128, 255)))
                .with_uniform_padding(4.)
                .finish(),
        )
        .on_left_mouse_down(move |ctx, _, _| {
            ctx.dispatch_typed_action(action);
            DispatchEventResult::StopPropagation
        })
        .finish(),
    )
    .with_uniform_margin(4.)
    .finish()
}

impl TypedActionView for RootView {
    type Action = ();
}
