use std::borrow::Cow;

use anyhow::{anyhow, Result};
use onboarding::components::onboarding_callout::{
    Button as CalloutButton, OnboardingCallout, Options as CalloutOptions, Params as CalloutParams,
    StepStatus,
};
use rust_embed::RustEmbed;
use ui_components::Component as _;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::{AnsiColor, AnsiColors, Details, Fill, TerminalColors, WarpTheme};
use warpui_core::color::ColorU;
use warpui_core::elements::{Rect, Stack};
use warpui_core::fonts::{Cache, FamilyId, Weight};
use warpui_core::prelude::*;
use warpui_core::{platform, AddWindowOptions, AssetProvider, ModelContext};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "../../app/assets"]
pub struct Assets;

pub static ASSETS: Assets = Assets;

impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

fn main() -> platform::app::TerminationResult {
    let app_builder = warpui::platform::AppBuilder::new(
        platform::AppCallbacks::default(),
        Box::new(ASSETS),
        None,
    );

    app_builder.run(move |ctx| {
        let font_name = if cfg!(target_os = "macos") {
            ".AppleSystemUIFont".to_string()
        } else if cfg!(target_os = "windows") {
            "Segoe UI".to_string()
        } else {
            "Noto Sans".to_string()
        };

        let font_family = Cache::handle(ctx).update(ctx, |cache, _ctx| {
            cache.load_system_font(&font_name).unwrap()
        });

        ctx.add_singleton_model(|ctx| build_appearance(mock_theme(), font_family, ctx));

        let window_options = AddWindowOptions::default();
        ctx.add_window(window_options, RootView::new);
    })
}

pub struct RootView {
    callout: OnboardingCallout,
}

impl RootView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            callout: Default::default(),
        }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "OnboardingCalloutExample"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);

        let options = <CalloutOptions as ui_components::Options>::default(appearance);

        let callout = self.callout.render(
            appearance,
            CalloutParams {
                title: "Meet your Warp input".into(),
                text: "Your terminal input can detect natural language as well as commands.".into(),
                step: StepStatus::new(1, 2),
                right_button: CalloutButton {
                    text: "Submit".into(),
                    keystroke: None,
                    handler: Box::new(|_ctx, _app_ctx, _mouse_pos| {}),
                },
                options,
            },
        );

        let background = Container::new(Rect::new().finish())
            .with_background(appearance.theme().background())
            .finish();

        let centered_callout = Align::new(callout).finish();

        let mut stack = Stack::new();
        stack.add_child(background);
        stack.add_child(Container::new(centered_callout).finish());
        stack.finish()
    }
}

#[derive(Clone, Debug)]
pub enum Action {}

impl TypedActionView for RootView {
    type Action = Action;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

fn mock_theme() -> WarpTheme {
    let normal = AnsiColors::new(
        AnsiColor::from_u32(0x121212FF),
        AnsiColor::from_u32(0xC76156FF),
        AnsiColor::from_u32(0x57C78AFF),
        AnsiColor::from_u32(0xC8A35AFF),
        AnsiColor::from_u32(0x5785C7FF),
        AnsiColor::from_u32(0xC756A9FF),
        AnsiColor::from_u32(0x57C7C3FF),
        AnsiColor::from_u32(0xEEEDEBFF),
    );

    let bright = AnsiColors::new(
        AnsiColor::from_u32(0x292929FF),
        AnsiColor::from_u32(0xE3493BFF),
        AnsiColor::from_u32(0x1CA05AFF),
        AnsiColor::from_u32(0xE3AA3BFF),
        AnsiColor::from_u32(0x3BE38AFF),
        AnsiColor::from_u32(0xC8A35AFF),
        AnsiColor::from_u32(0x3BE3DDFF),
        AnsiColor::from_u32(0xFFFFFFFF),
    );

    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x1D2022FF)),
        ColorU::from_u32(0xE4EEF5FF),
        Fill::Solid(ColorU::from_u32(0x6C96B4FF)),
        None,
        Some(Details::Darker),
        TerminalColors::new(normal, bright),
        None,
        Some("Onboarding Example".to_string()),
    )
}

fn build_appearance(
    theme: WarpTheme,
    ui_font_family: FamilyId,
    ctx: &mut ModelContext<Appearance>,
) -> Appearance {
    // For this example, use the same family for all fonts.
    let monospace_font_family = ui_font_family;
    let ai_font_family = ui_font_family;
    let password_font_family = ui_font_family;

    let mut appearance = Appearance::new(
        theme,
        monospace_font_family,
        13.0,
        Weight::Normal,
        ui_font_family,
        1.4,
        ai_font_family,
        password_font_family,
    );

    appearance.set_ui_font_family(ui_font_family, ctx);

    appearance
}
