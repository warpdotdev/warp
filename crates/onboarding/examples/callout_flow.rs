use std::borrow::Cow;

use anyhow::Result;
use onboarding::OnboardingIntention;
use onboarding::callout::{
    OnboardingCalloutView, OnboardingCalloutViewEvent, OnboardingKeybindings,
};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use rust_embed::RustEmbed;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::{AnsiColor, AnsiColors, Details, Fill, TerminalColors, WarpTheme};
use warpui_core::elements::{
    ChildAnchor, ChildView, ConstrainedBox, Container, Flex, MainAxisAlignment, MainAxisSize,
    OffsetPositioning, ParentElement, PositionedElementAnchor, PositionedElementOffsetBounds, Rect,
    SavePosition, Stack,
};
use warpui_core::fonts::{Cache, FamilyId, Weight};
use warpui_core::prelude::CrossAxisAlignment;
use warpui_core::ui_components::components::UiComponent;
use warpui_core::{
    AddWindowOptions, AppContext, AssetProvider, Element, Entity, SingletonEntity as _,
    TypedActionView, View, ViewContext, ViewHandle, platform,
};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "../../app/assets"]
pub struct Assets;

pub static ASSETS: Assets = Assets;

impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow::anyhow!(format!("no asset exists at path {path}")))
    }
}

// Wrapper view to demonstrate the OnboardingCalloutView with model
struct OnboardingExampleView {
    callout_view: ViewHandle<OnboardingCalloutView>,
}

impl OnboardingExampleView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let callout_view = ctx.add_typed_action_view(|ctx| {
            // Use example keybindings for the demo (Mac-style)
            let keybindings = OnboardingKeybindings {
                toggle_input_mode: "⌘-I".to_string(),
                submit_to_local_agent: "⌘-⏎".to_string(),
                submit_to_cloud_agent: "⌘-⌥-⏎".to_string(),
                return_to_terminal_mode: "ESC".to_string(),
            };
            OnboardingCalloutView::new_agent_modality(
                true, // has_project
                OnboardingIntention::AgentDrivenDevelopment,
                false, // initial_natural_language_detection_enabled
                keybindings,
                ctx,
            )
        });

        // Start with MeetInput state for the demo.
        callout_view.update(ctx, |callout_view, ctx| {
            callout_view.start_onboarding(ctx);
        });

        // Ensure the callout view receives focus so its keybindings are active.
        ctx.focus(&callout_view);

        // Re-render when the callout view updates its state.
        let callout_view_handle = callout_view.clone();
        ctx.subscribe_to_view(&callout_view, move |_me, _handle, event, ctx| {
            if matches!(event, OnboardingCalloutViewEvent::StateUpdated) {
                ctx.focus(&callout_view_handle);
                ctx.notify();
            }
        });

        Self { callout_view }
    }
}

impl Entity for OnboardingExampleView {
    type Event = ();
}

impl View for OnboardingExampleView {
    fn ui_name() -> &'static str {
        "OnboardingExampleView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let callout_view = self.callout_view.as_ref(app);
        let appearance = Appearance::as_ref(app);

        // Create the bottom command rect (140px high) with SavePosition to track its location
        let command_text = callout_view.prompt_string(app);
        let command_rect_id = "onboarding_command_rect";
        let command_rect = SavePosition::new(
            ConstrainedBox::new(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(command_text)
                        .build()
                        .finish(),
                )
                .with_uniform_padding(16.0)
                .with_background(ColorU::from_u32(0x2A2D30FF))
                .finish(),
            )
            .with_height(140.0)
            .finish(),
            command_rect_id,
        )
        .finish();

        // Create a background container
        let background = Container::new(Rect::new().finish())
            .with_background(appearance.theme().background())
            .finish();

        // Create stack with background and command rect
        let mut stack = Stack::new();

        // Add background layer
        stack.add_child(background);

        // Add command rect positioned at bottom
        let positioned_command = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(command_rect)
            .finish();
        stack.add_child(positioned_command);

        // Add callout as positioned overlay relative to the command rect if onboarding is active
        if callout_view.is_onboarding_active(app) {
            // Position callout above the command rect, centered horizontally
            let positioning = OffsetPositioning::offset_from_save_position_element(
                command_rect_id,
                vec2f(0.0, -20.0), // 20px above the command rect
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopLeft, // Anchor to top center of command rect
                ChildAnchor::BottomLeft,          // Position bottom center of callout
            );

            // Render the OnboardingCalloutView using ChildView
            stack.add_positioned_overlay_child(
                ChildView::new(&self.callout_view).finish(),
                positioning,
            );
        }

        stack.finish()
    }
}

// Empty action enum since this wrapper view doesn't handle actions directly
#[derive(Clone, Debug)]
pub enum OnboardingExampleAction {}

impl TypedActionView for OnboardingExampleView {
    type Action = OnboardingExampleAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        // This view doesn't handle any actions - all actions are handled by OnboardingCalloutView
    }
}

fn main() -> Result<()> {
    // Initialize logging for the onboarding binary.
    warp_logging::init(warp_logging::LogConfig {
        is_cli: false,
        log_destination: None,
        ..Default::default()
    })?;

    let app_builder = warpui::platform::AppBuilder::new(
        platform::AppCallbacks::default(),
        Box::new(ASSETS),
        None,
    );
    let _ = app_builder.run(move |ctx| {
        // Register Appearance singleton so views can access Appearance::handle(ctx).
        ctx.add_singleton_model(|ctx| build_appearance(adeberry(), ctx));

        // Register onboarding keybindings.
        onboarding::init(ctx);

        ctx.add_window(AddWindowOptions::default(), |ctx| {
            OnboardingExampleView::new(ctx)
        });
    });

    Ok(())
}

const ADEBERRY_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x121212FF),
    AnsiColor::from_u32(0xC76156FF),
    AnsiColor::from_u32(0x57C78AFF),
    AnsiColor::from_u32(0xC8A35AFF),
    AnsiColor::from_u32(0x5785C7FF),
    AnsiColor::from_u32(0xC756A9FF),
    AnsiColor::from_u32(0x57C7C3FF),
    AnsiColor::from_u32(0xEEEDEBFF),
);

const ADEBERRY_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x292929FF),
    AnsiColor::from_u32(0xE3493BFF),
    AnsiColor::from_u32(0x1CA05AFF),
    AnsiColor::from_u32(0xE3AA3BFF),
    AnsiColor::from_u32(0x3BE38AFF),
    AnsiColor::from_u32(0xC8A35AFF),
    AnsiColor::from_u32(0x3BE3DDFF),
    AnsiColor::from_u32(0xFFFFFFFF),
);

fn adeberry_colors() -> TerminalColors {
    TerminalColors::new(ADEBERRY_NORMAL_COLORS, ADEBERRY_BRIGHT_COLORS)
}

fn adeberry() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x1D2022FF)),
        ColorU::from_u32(0xE4EEF5FF),
        Fill::Solid(ColorU::from_u32(0x6C96B4FF)),
        None,
        Some(Details::Darker),
        adeberry_colors(),
        None,
        Some("Adeberry".to_string()),
    )
}

fn build_appearance(theme: WarpTheme, ctx: &mut AppContext) -> Appearance {
    let ui_font_family =
        load_default_ui_font_family(ctx).expect("unable to load default ui font family");

    Appearance::new(
        theme,
        ui_font_family,
        13.0,
        Weight::Normal,
        ui_font_family,
        1.2,
        ui_font_family,
        ui_font_family,
    )
}

fn load_default_ui_font_family(ctx: &mut AppContext) -> anyhow::Result<FamilyId> {
    Cache::handle(ctx).update(ctx, |font_cache, _| {
        // On Windows, default to use Segoe UI as the UI font.
        #[cfg(windows)]
        if let Ok(font_family_id) = font_cache.load_system_font("Segoe UI") {
            return Ok(font_family_id);
        }

        font_cache.load_family_from_bytes(
            "Roboto",
            vec![
                ASSETS
                    .get("bundled/fonts/roboto/Roboto-Italic.ttf")?
                    .to_vec(),
                ASSETS.get("bundled/fonts/roboto/Roboto-Bold.ttf")?.to_vec(),
                ASSETS
                    .get("bundled/fonts/roboto/Roboto-Regular.ttf")?
                    .to_vec(),
                ASSETS
                    .get("bundled/fonts/roboto/Roboto-Medium.ttf")?
                    .to_vec(),
                ASSETS
                    .get("bundled/fonts/roboto/RobotoFlex-Semibold.ttf")?
                    .to_vec(),
                ASSETS
                    .get("bundled/fonts/roboto/Roboto-BoldItalic.ttf")?
                    .to_vec(),
            ],
        )
    })
}
