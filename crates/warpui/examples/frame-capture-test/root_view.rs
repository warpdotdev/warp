use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use image::ImageEncoder;
use pathfinder_color::ColorU;
use warpui::elements::{
    Align, ConstrainedBox, Container, DispatchEventResult, EventHandler, Padding, ParentElement,
    Rect, Stack, Text,
};
use warpui::fonts::{Cache as FontCache, FamilyId};
use warpui::platform::CapturedFrame;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

#[derive(Clone, Debug)]
pub enum RootViewAction {
    CaptureFrame,
}

pub struct RootView {
    window_id: warpui::WindowId,
    font_family: FamilyId,
    last_capture_msg: Arc<Mutex<Option<String>>>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let window_id = ctx.window_id();
        let font_family = FontCache::handle(ctx)
            .update(ctx, |cache: &mut FontCache, _| {
                cache.load_system_font("Arial").ok()
            })
            .unwrap_or(FamilyId(0));
        log::info!("Frame capture demo initialized. Click the button to capture!");
        println!("\n📸 Click the blue button to capture the frame!\n");
        Self {
            window_id,
            font_family,
            last_capture_msg: Arc::new(Mutex::new(None)),
        }
    }

    fn request_capture(&mut self, ctx: &mut ViewContext<Self>) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let filename = format!("frame_capture_{}.png", timestamp);

        log::info!("Requesting frame capture to: {}", filename);
        println!("\n📸 Requesting frame capture to: {}\n", filename);

        *self.last_capture_msg.lock().unwrap() = Some("Capture requested...".to_string());
        ctx.notify();

        if let Some(window) = ctx.windows().platform_window(self.window_id) {
            let msg_handle = Arc::clone(&self.last_capture_msg);
            window
                .as_ctx()
                .request_frame_capture(Box::new(move |frame| {
                    log::info!("Frame captured, saving to file");
                    match save_frame_as_png(&frame, &filename) {
                        Ok(()) => {
                            log::info!("Frame saved to: {}", filename);
                            println!("\n✅ Frame saved to: {}\n", filename);
                            *msg_handle.lock().unwrap() =
                                Some(format!("Frame written to {}", filename));
                        }
                        Err(e) => {
                            log::error!("Failed to save frame: {}", e);
                            *msg_handle.lock().unwrap() = Some(format!("Error: {}", e));
                        }
                    }
                }));
        }
    }
}

fn save_frame_as_png(frame: &CapturedFrame, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);

    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );

    encoder.write_image(
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8.into(),
    )?;

    Ok(())
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let button_color = ColorU::new(70, 120, 200, 255);
        let status_msg = self
            .last_capture_msg
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(|| "Click the button below to write a frame".to_string());

        Stack::new()
            // Dark background
            .with_child(
                Rect::new()
                    .with_background_color(ColorU::new(40, 40, 45, 255))
                    .finish(),
            )
            // Content: Colorful squares arranged vertically
            .with_child(
                Align::new(
                    Container::new(
                        Stack::new()
                            // Title text
                            .with_child(
                                Container::new(
                                    Text::new_inline(
                                        "Frame Capture Test".to_string(),
                                        self.font_family,
                                        28.0,
                                    )
                                    .with_color(ColorU::white())
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(16.0))
                                .finish(),
                            )
                            // Subtitle
                            .with_child(
                                Container::new(
                                    Text::new_inline(
                                        "WarpUI rendering sample with clickable capture button"
                                            .to_string(),
                                        self.font_family,
                                        16.0,
                                    )
                                    .with_color(ColorU::new(200, 200, 200, 255))
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(12.0))
                                .finish(),
                            )
                            // Toast / status line
                            .with_child(
                                Container::new(
                                    Text::new_inline(status_msg, self.font_family, 14.0)
                                        .with_color(ColorU::new(180, 220, 180, 255))
                                        .finish(),
                                )
                                .with_padding(Padding::uniform(12.0))
                                .finish(),
                            )
                            // Red square
                            .with_child(
                                Container::new(
                                    ConstrainedBox::new(
                                        Rect::new()
                                            .with_background_color(ColorU::new(255, 100, 100, 255))
                                            .finish(),
                                    )
                                    .with_width(150.0)
                                    .with_height(150.0)
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(15.0))
                                .finish(),
                            )
                            // Green square
                            .with_child(
                                Container::new(
                                    ConstrainedBox::new(
                                        Rect::new()
                                            .with_background_color(ColorU::new(100, 255, 100, 255))
                                            .finish(),
                                    )
                                    .with_width(150.0)
                                    .with_height(150.0)
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(15.0))
                                .finish(),
                            )
                            // Blue square
                            .with_child(
                                Container::new(
                                    ConstrainedBox::new(
                                        Rect::new()
                                            .with_background_color(ColorU::new(100, 100, 255, 255))
                                            .finish(),
                                    )
                                    .with_width(150.0)
                                    .with_height(150.0)
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(15.0))
                                .finish(),
                            )
                            // Orange square
                            .with_child(
                                Container::new(
                                    ConstrainedBox::new(
                                        Rect::new()
                                            .with_background_color(ColorU::new(255, 165, 0, 255))
                                            .finish(),
                                    )
                                    .with_width(150.0)
                                    .with_height(150.0)
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(15.0))
                                .finish(),
                            )
                            // Capture button (purple square)
                            .with_child(
                                Container::new(
                                    EventHandler::new(
                                        ConstrainedBox::new(
                                            Stack::new()
                                                // Button background
                                                .with_child(
                                                    Rect::new()
                                                        .with_background_color(button_color)
                                                        .finish(),
                                                )
                                                // Button label
                                                .with_child(
                                                    Align::new(
                                                        Text::new_inline(
                                                            "Write Frame to File System"
                                                                .to_string(),
                                                            self.font_family,
                                                            16.0,
                                                        )
                                                        .with_color(ColorU::white())
                                                        .finish(),
                                                    )
                                                    .finish(),
                                                )
                                                .finish(),
                                        )
                                        .with_width(200.0)
                                        .with_height(80.0)
                                        .finish(),
                                    )
                                    .on_left_mouse_down(|ctx, _, _| {
                                        ctx.dispatch_typed_action(RootViewAction::CaptureFrame);
                                        DispatchEventResult::StopPropagation
                                    })
                                    .finish(),
                                )
                                .with_padding(Padding::uniform(15.0))
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_padding(Padding::uniform(40.0))
                    .finish(),
                )
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = RootViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RootViewAction::CaptureFrame => self.request_capture(ctx),
        }
    }
}
