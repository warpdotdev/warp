use std::{borrow::Cow, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use pathfinder_geometry::vector::Vector2F;
use rust_embed::RustEmbed;
use ui_components::{
    Component as _, Options, button, dialog,
    lightbox::{self, LightboxImage, LightboxImageSource, NavigationDirection},
    switch, tooltip,
};
use warp_core::ui::{Icon, appearance::Appearance, theme::color::internal_colors};
use warpui::{
    AssetProvider, SingletonEntity, Tracked,
    assets::asset_cache::{AssetCache, AssetSource, AssetState},
    r#async::Timer,
    elements::Stack,
    image_cache::ImageType,
    keymap::FixedBinding,
    platform,
    prelude::*,
};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "../../app/assets"]
#[include = "bundled/**"] // Should be kept in sync with BUNDLED_ASSETS_DIR.
#[include = "async/**"] // Should be kept in sync with ASYNC_ASSETS_DIR.
#[cfg_attr(target_family = "wasm", exclude = "async/**")] // Excludes take precedence.
pub struct Assets;

pub static ASSETS: Assets = Assets;

impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

fn main() -> warpui::platform::app::TerminationResult {
    // Initialize the TLS provider so reqwest can make HTTPS requests.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("must be able to initialize crypto provider for TLS support");

    let app_builder =
        platform::AppBuilder::new(platform::AppCallbacks::default(), Box::new(ASSETS), None);
    app_builder.run(move |ctx| {
        let font_name = if cfg!(target_os = "macos") {
            ".AppleSystemUIFont".to_string()
        } else if cfg!(target_os = "windows") {
            "Segoe UI".to_string()
        } else {
            "Noto Sans".to_string()
        };

        let font_family = warpui::fonts::Cache::handle(ctx).update(ctx, |cache, _ctx| {
            cache.load_system_font(&font_name).unwrap()
        });
        ctx.add_singleton_model(|ctx| {
            let mut appearance = Appearance::mock();
            appearance.set_ui_font_family(font_family, ctx);
            appearance
        });

        {
            use warpui::keymap::macros::*;
            let lightbox_open = id!("RootView") & id!("RootView_LightboxOpen");
            ctx.register_fixed_bindings([
                FixedBinding::new(
                    "escape",
                    Action::CloseDialog,
                    id!("RootView") & id!("RootView_DialogOpen"),
                ),
                FixedBinding::new("escape", Action::CloseLightbox, lightbox_open.clone()),
                FixedBinding::new(
                    "left",
                    Action::LightboxNavigatePrevious,
                    lightbox_open.clone(),
                ),
                FixedBinding::new("right", Action::LightboxNavigateNext, lightbox_open),
            ]);
        }

        ctx.add_window(warpui::AddWindowOptions::default(), RootView::new);
    })
}

pub struct RootView {
    // Switch.
    switch: switch::Switch,
    switch_checked: Tracked<bool>,

    // Buttons.
    default_button_row: ButtonRow,
    small_button_row: ButtonRow,

    // Dialog.
    dialog: dialog::Dialog,
    dialog_open: Tracked<bool>,
    open_dialog_button: button::Button,

    // Lightbox.
    lightbox: lightbox::Lightbox,
    lightbox_open: Tracked<bool>,
    lightbox_images: Vec<LightboxImage>,
    lightbox_current_index: usize,
    open_lightbox_button: button::Button,

    // Async lightbox.
    async_lightbox: lightbox::Lightbox,
    async_lightbox_open: Tracked<bool>,
    async_lightbox_images: Vec<LightboxImage>,
    async_lightbox_current_index: usize,
    open_async_lightbox_button: button::Button,
}

impl RootView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            // Switch.
            switch: Default::default(),
            switch_checked: Tracked::new(false),

            // Buttons.
            default_button_row: ButtonRow::default(),
            small_button_row: ButtonRow::default(),

            // Dialog.
            dialog: Default::default(),
            dialog_open: Tracked::new(false),
            open_dialog_button: Default::default(),

            // Lightbox.
            lightbox: Default::default(),
            lightbox_open: Tracked::new(false),
            lightbox_images: vec![
                LightboxImage {
                    source: LightboxImageSource::Resolved {
                        asset_source: AssetSource::Bundled {
                            path: "bundled/png/dev.png",
                        },
                    },
                    description: Some("First image (dev.png)".to_string()),
                },
                LightboxImage {
                    source: LightboxImageSource::Resolved {
                        asset_source: AssetSource::Bundled {
                            path: "bundled/png/dev.png",
                        },
                    },
                    description: Some("Second image (also dev.png)".to_string()),
                },
            ],
            lightbox_current_index: 0,
            open_lightbox_button: Default::default(),

            // Async lightbox.
            async_lightbox: Default::default(),
            async_lightbox_open: Tracked::new(false),
            async_lightbox_images: Vec::new(),
            async_lightbox_current_index: 0,
            open_async_lightbox_button: Default::default(),
        }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn keymap_context(&self, _: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if *self.dialog_open {
            context.set.insert("RootView_DialogOpen");
        }
        if *self.lightbox_open {
            context.set.insert("RootView_LightboxOpen");
        }
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_spacing(8.);

        for render_fn in &[
            Self::render_switch,
            Self::render_tooltip,
            Self::render_buttons,
            Self::render_dialog_button,
            Self::render_lightbox_buttons,
        ] {
            column.add_child(example_row(render_fn(self, appearance), appearance));
        }

        let content = Container::new(Align::new(column.finish()).finish())
            .with_background_color(ColorU::new(68, 68, 68, 255))
            .finish();

        if *self.dialog_open {
            Stack::new()
                .with_child(content)
                .with_child(Align::new(self.render_dialog(appearance)).finish())
                .finish()
        } else if *self.lightbox_open {
            Stack::new()
                .with_child(content)
                .with_child(self.render_lightbox(appearance, ctx))
                .finish()
        } else if *self.async_lightbox_open {
            Stack::new()
                .with_child(content)
                .with_child(self.render_async_lightbox(appearance, ctx))
                .finish()
        } else {
            content
        }
    }
}

impl RootView {
    fn render_switch(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.switch.render(
            appearance,
            switch::Params {
                checked: *self.switch_checked,
                on_click: Some(Box::new(|ctx, _app, _pos| {
                    ctx.dispatch_typed_action(Action::SwitchToggled);
                })),
                options: switch::Options {
                    hover_border_size: Some(10.),
                    label: Some(Box::new(move |appearance: &Appearance| {
                        warpui::elements::Text::new(
                            "Switch",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(ColorU::white())
                        .finish()
                    })),
                    ..Options::default(appearance)
                },
            },
        )
    }

    fn render_tooltip(&self, appearance: &Appearance) -> Box<dyn Element> {
        tooltip::Tooltip.render(
            appearance,
            tooltip::Params {
                label: "Tooltip label".into(),
                options: tooltip::Options {
                    keyboard_shortcut: Some(warpui::keymap::Keystroke {
                        ctrl: true,
                        key: "k".to_string(),
                        ..Default::default()
                    }),
                },
            },
        )
    }

    fn render_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        let default_size_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_spacing(8.)
            .with_children([
                self.default_button_row.default.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("Primary".into()),
                        theme: &button::themes::Primary,
                        options: button::Options {
                            keystroke: Some(warpui::keymap::Keystroke {
                                ctrl: true,
                                key: "k".to_string(),
                                ..Default::default()
                            }),
                            tooltip: Some(button::Tooltip {
                                params: tooltip::Params {
                                    label: "Primary / Default".into(),
                                    options: Options::default(appearance),
                                },
                                alignment: Default::default(),
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
                self.default_button_row.secondary.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("Secondary".into()),
                        theme: &button::themes::Secondary,
                        options: button::Options {
                            keystroke: Some(warpui::keymap::Keystroke {
                                cmd: true,
                                key: "enter".to_string(),
                                ..Default::default()
                            }),
                            tooltip: Some(button::Tooltip {
                                params: tooltip::Params {
                                    label: "Secondary / Default".into(),
                                    options: Options::default(appearance),
                                },
                                alignment: Default::default(),
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
                self.default_button_row.disabled.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("Disabled".into()),
                        theme: &button::themes::Primary,
                        options: button::Options {
                            disabled: true,
                            keystroke: Some(warpui::keymap::Keystroke {
                                shift: true,
                                key: "d".to_string(),
                                ..Default::default()
                            }),
                            tooltip: Some(button::Tooltip {
                                params: tooltip::Params {
                                    label: "Disabled / Default".into(),
                                    options: Options::default(appearance),
                                },
                                alignment: Default::default(),
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
                self.default_button_row.icon.render(
                    appearance,
                    button::Params {
                        content: button::Content::Icon(Icon::X),
                        theme: &button::themes::Primary,
                        options: button::Options {
                            ..Options::default(appearance)
                        },
                    },
                ),
            ])
            .finish();

        let small_size_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_spacing(8.)
            .with_children([
                self.small_button_row.default.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("Primary".into()),
                        theme: &button::themes::Primary,
                        options: button::Options {
                            size: button::Size::Small,
                            keystroke: Some(warpui::keymap::Keystroke {
                                ctrl: true,
                                key: "k".to_string(),
                                ..Default::default()
                            }),
                            tooltip: Some(button::Tooltip {
                                params: tooltip::Params {
                                    label: "Primary / Small".into(),
                                    options: Options::default(appearance),
                                },
                                alignment: Default::default(),
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
                self.small_button_row.secondary.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("Secondary".into()),
                        theme: &button::themes::Secondary,
                        options: button::Options {
                            size: button::Size::Small,
                            keystroke: Some(warpui::keymap::Keystroke {
                                cmd: true,
                                key: "enter".to_string(),
                                ..Default::default()
                            }),
                            tooltip: Some(button::Tooltip {
                                params: tooltip::Params {
                                    label: "Secondary / Small".into(),
                                    options: Options::default(appearance),
                                },
                                alignment: Default::default(),
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
                self.small_button_row.disabled.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("Disabled".into()),
                        theme: &button::themes::Primary,
                        options: button::Options {
                            disabled: true,
                            size: button::Size::Small,
                            keystroke: Some(warpui::keymap::Keystroke {
                                shift: true,
                                key: "d".to_string(),
                                ..Default::default()
                            }),
                            tooltip: Some(button::Tooltip {
                                params: tooltip::Params {
                                    label: "Disabled / Small".into(),
                                    options: Options::default(appearance),
                                },
                                alignment: Default::default(),
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
                self.small_button_row.icon.render(
                    appearance,
                    button::Params {
                        content: button::Content::Icon(Icon::X),
                        theme: &button::themes::Secondary,
                        options: button::Options {
                            size: button::Size::Small,
                            keystroke: Some(warpui::keymap::Keystroke {
                                key: "escape".to_string(),
                                ..Default::default()
                            }),
                            ..Options::default(appearance)
                        },
                    },
                ),
            ])
            .finish();

        let small_size_row = Container::new(small_size_row).with_margin_top(8.).finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_children([default_size_row, small_size_row])
            .finish()
    }

    fn render_dialog_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.open_dialog_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Open Dialog".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(Action::OpenDialog);
                    })),
                    ..Options::default(appearance)
                },
            },
        )
    }

    fn render_dialog(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.dialog.render(
            appearance,
            dialog::Params {
                title: "Dialog Title".into(),
                content: Box::new(|appearance: &Appearance| {
                    Container::new(
                        Text::new(
                            "This is a dialog.",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(ColorU::white())
                        .finish(),
                    )
                    .with_horizontal_padding(dialog::HORIZONTAL_PADDING)
                    .with_padding_bottom(dialog::BASE_PADDING)
                    .finish()
                }),
                options: dialog::Options {
                    width: Some(500.),
                    on_dismiss: Some(Arc::new(|ctx, _app| {
                        ctx.dispatch_typed_action(Action::CloseDialog);
                    })),
                    dismiss_keystroke: Some(warpui::keymap::Keystroke {
                        key: "escape".to_string(),
                        ..Default::default()
                    }),
                    footer: Some(Box::new(|appearance: &Appearance| {
                        Text::new(
                            "This is the footer",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(ColorU::white())
                        .finish()
                    })),
                },
            },
        )
    }

    fn render_lightbox_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        Flex::row()
            .with_spacing(8.)
            .with_child(self.open_lightbox_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label("Open Lightbox".into()),
                    theme: &button::themes::Primary,
                    options: button::Options {
                        on_click: Some(Box::new(|ctx, _app, _pos| {
                            ctx.dispatch_typed_action(Action::OpenLightbox);
                        })),
                        ..Options::default(appearance)
                    },
                },
            ))
            .with_child(self.open_async_lightbox_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label("Open Lightbox (Async)".into()),
                    theme: &button::themes::Secondary,
                    options: button::Options {
                        on_click: Some(Box::new(|ctx, _app, _pos| {
                            ctx.dispatch_typed_action(Action::OpenAsyncLightbox);
                        })),
                        ..Options::default(appearance)
                    },
                },
            ))
            .finish()
    }

    fn render_lightbox(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let current_image_native_size = self
            .lightbox_images
            .get(self.lightbox_current_index)
            .and_then(|img| native_size_for_image(img, app));

        self.lightbox.render(
            appearance,
            lightbox::Params {
                images: &self.lightbox_images,
                current_index: self.lightbox_current_index,
                on_dismiss: Arc::new(|ctx, _app| {
                    ctx.dispatch_typed_action(Action::CloseLightbox);
                }),
                current_image_native_size,
                animation_start_time: None,
                zoom_factor: 1.0,
                pan_offset: pathfinder_geometry::vector::Vector2F::zero(),
                metadata_line: None,
                thumbnail_rail: None,
                options: lightbox::Options {
                    on_zoom: None,
                    on_pan: None,
                    on_double_tap_zoom: None,
                    dismiss_keystroke: Some(warpui::keymap::Keystroke {
                        key: "escape".to_string(),
                        ..Default::default()
                    }),
                    on_navigate: Some(Arc::new(|direction, ctx, _app| match direction {
                        NavigationDirection::Previous => {
                            ctx.dispatch_typed_action(Action::LightboxNavigatePrevious);
                        }
                        NavigationDirection::Next => {
                            ctx.dispatch_typed_action(Action::LightboxNavigateNext);
                        }
                    })),
                },
            },
        )
    }

    fn render_async_lightbox(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let current_image_native_size = self
            .async_lightbox_images
            .get(self.async_lightbox_current_index)
            .and_then(|img| native_size_for_image(img, app));

        self.async_lightbox.render(
            appearance,
            lightbox::Params {
                images: &self.async_lightbox_images,
                current_index: self.async_lightbox_current_index,
                on_dismiss: Arc::new(|ctx, _app| {
                    ctx.dispatch_typed_action(Action::CloseLightbox);
                }),
                current_image_native_size,
                animation_start_time: None,
                zoom_factor: 1.0,
                pan_offset: pathfinder_geometry::vector::Vector2F::zero(),
                metadata_line: None,
                thumbnail_rail: None,
                options: lightbox::Options {
                    on_zoom: None,
                    on_pan: None,
                    on_double_tap_zoom: None,
                    dismiss_keystroke: Some(warpui::keymap::Keystroke {
                        key: "escape".to_string(),
                        ..Default::default()
                    }),
                    on_navigate: Some(Arc::new(|direction, ctx, _app| match direction {
                        NavigationDirection::Previous => {
                            ctx.dispatch_typed_action(Action::LightboxNavigatePrevious);
                        }
                        NavigationDirection::Next => {
                            ctx.dispatch_typed_action(Action::LightboxNavigateNext);
                        }
                    })),
                },
            },
        )
    }
}

#[derive(Debug)]
pub enum Action {
    SwitchToggled,
    OpenDialog,
    CloseDialog,
    OpenLightbox,
    OpenAsyncLightbox,
    CloseLightbox,
    LightboxNavigatePrevious,
    LightboxNavigateNext,
}

impl TypedActionView for RootView {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::SwitchToggled => {
                *self.switch_checked = !*self.switch_checked;
            }
            Action::OpenDialog => {
                *self.dialog_open = true;
            }
            Action::CloseDialog => {
                *self.dialog_open = false;
            }
            Action::OpenLightbox => {
                self.lightbox_current_index = 0;
                *self.lightbox_open = true;
            }
            Action::OpenAsyncLightbox => {
                // Start with 3 images in Loading state.
                self.async_lightbox_images = vec![
                    LightboxImage {
                        source: LightboxImageSource::Loading,
                        description: Some("Image 1".to_string()),
                    },
                    LightboxImage {
                        source: LightboxImageSource::Loading,
                        description: Some("Image 2".to_string()),
                    },
                    LightboxImage {
                        source: LightboxImageSource::Loading,
                        description: Some("Image 3".to_string()),
                    },
                ];
                self.async_lightbox_current_index = 0;
                *self.async_lightbox_open = true;

                // Simulate async loading: each image "loads" after a staggered delay.
                for i in 0..3usize {
                    let delay = Duration::from_secs((i as u64 + 1) * 2);
                    ctx.spawn(
                        async move {
                            Timer::after(delay).await;
                            i
                        },
                        |view, index, ctx| {
                            if let Some(image) = view.async_lightbox_images.get_mut(index) {
                                image.source = LightboxImageSource::Resolved {
                                    asset_source: ::asset_cache::url_source(
                                        "https://cdn.terminaltrove.com/m/b1c31938-6e80-4f28-a2cd-d2047eddcdb2.png",
                                    ),
                                };
                                image.description =
                                    Some(format!("Image {} \u{2014} loaded!", index + 1));
                            }
                            ctx.notify();
                        },
                    );
                }
            }
            Action::CloseLightbox => {
                *self.lightbox_open = false;
                *self.async_lightbox_open = false;
            }
            Action::LightboxNavigatePrevious => {
                if *self.lightbox_open && self.lightbox_current_index > 0 {
                    self.lightbox_current_index -= 1;
                }
                if *self.async_lightbox_open && self.async_lightbox_current_index > 0 {
                    self.async_lightbox_current_index -= 1;
                }
            }
            Action::LightboxNavigateNext => {
                if *self.lightbox_open
                    && self.lightbox_current_index + 1 < self.lightbox_images.len()
                {
                    self.lightbox_current_index += 1;
                }
                if *self.async_lightbox_open
                    && self.async_lightbox_current_index + 1 < self.async_lightbox_images.len()
                {
                    self.async_lightbox_current_index += 1;
                }
            }
        }
    }
}

#[derive(Default)]
struct ButtonRow {
    default: button::Button,
    secondary: button::Button,
    disabled: button::Button,
    icon: button::Button,
}

/// Queries the `AssetCache` for the native pixel dimensions of a lightbox image.
/// Returns `Some` when the image bytes have been fully loaded and decoded.
fn native_size_for_image(image: &LightboxImage, app: &AppContext) -> Option<Vector2F> {
    match &image.source {
        LightboxImageSource::Resolved { asset_source } => {
            let asset_cache = AssetCache::as_ref(app);
            match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loaded { data } => data
                    .image_size()
                    .map(|size| Vector2F::new(size.x() as f32, size.y() as f32)),
                _ => None,
            }
        }
        LightboxImageSource::Loading => None,
        // Error entries have no native size.
        LightboxImageSource::Error { .. } => None,
    }
}

fn example_row(contents: Box<dyn Element>, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(contents)
        .with_uniform_padding(16.)
        .with_border(internal_colors::neutral_4(appearance.theme()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
}
