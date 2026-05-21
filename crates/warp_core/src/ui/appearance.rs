use warpui::{
    fonts::{FamilyId, Weight},
    Entity, ModelContext, SingletonEntity,
};

use super::{builder::UiBuilder, theme::WarpTheme};

/// The standard font size to use for headers (e.g.: in dialogs).
const HEADER_FONT_SIZE: f32 = 18.;
const OVERLINE_FONT_SIZE: f32 = 10.;

pub const DEFAULT_UI_FONT_SIZE: f32 = 12.0;
pub const DEFAULT_COMMAND_PALETTE_FONT_SIZE: f32 = 14.0;

/// Holds visual settings that are so widely used that it's best
/// to invalidate all views when they change rather than forcing views
/// to individually listen for changes. The most prominent examples are
/// settings related to themes and fonts.
pub struct Appearance {
    theme: WarpTheme,
    monospace_font_family: FamilyId,
    monospace_font_size: f32,
    monospace_font_weight: Weight,
    line_height_ratio: f32,
    ui_builder: UiBuilder,

    // We cache the family id for the ui font - note that this
    // isn't actually a changeable setting right now.
    ui_font_family: FamilyId,
    ai_font_family: FamilyId,
    /// A font that is used for password fields.
    password_font_family: FamilyId,
}

/// Defines appearance change events.
///
/// For any properties that are read from appearance (e.g.: theme, font, etc.),
/// users should listen for these events rather than directly listening to
/// settings change events for the underlying properties.
///
/// NOTE: You do NOT need to set up subscriptions for these events and use them
/// to invalidate views!  All views are automatically invalidated on changes to
/// fields in [`Appearance`].  If you appear to need to subscribe to one of
/// these events and call `ctx.notify()` for proper behavior, there is probably
/// a bug in [`Appearance`].
#[derive(Debug)]
pub enum AppearanceEvent {
    ThemeChanged,
    UiFontFamilyChanged {
        previous_family_id: FamilyId,
        current_family_id: FamilyId,
    },
    MonospaceFontSizeChanged {
        previous_font_size: f32,
        current_font_size: f32,
    },
    MonospaceFontFamilyChanged {
        previous_family_id: FamilyId,
        current_family_id: FamilyId,
    },
    MonospaceFontWeightChanged {
        previous_font_weight: Weight,
        current_font_weight: Weight,
    },
    LineHeightRatioChanged {
        previous_line_height_ratio: f32,
        current_line_height_ratio: f32,
    },
}

impl Appearance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme: WarpTheme,
        monospace_font_family: FamilyId,
        monospace_font_size: f32,
        monospace_font_weight: Weight,
        ui_font_family: FamilyId,
        line_height_ratio: f32,
        ai_font_family: FamilyId,
        password_font_family: FamilyId,
    ) -> Self {
        Self {
            theme: theme.clone(),
            monospace_font_family,
            monospace_font_size,
            monospace_font_weight,
            ui_font_family,
            line_height_ratio,
            ui_builder: UiBuilder::new(
                theme,
                ui_font_family,
                DEFAULT_UI_FONT_SIZE,
                DEFAULT_COMMAND_PALETTE_FONT_SIZE,
                line_height_ratio,
            ),
            ai_font_family,
            password_font_family,
        }
    }

    #[cfg(feature = "test-util")]
    pub fn mock() -> Self {
        use warpui::color::ColorU;

        use crate::ui::theme::{mock_terminal_colors, Details, Fill};

        let mock_theme = WarpTheme::new(
            Fill::Solid(ColorU::from_u32(0x000000ff)),
            ColorU::from_u32(0xffffffff),
            Fill::Solid(ColorU::new(18, 123, 156, 255)),
            None,
            Some(Details::Darker),
            mock_terminal_colors(),
            None,
            Some("Dark".to_string()),
        );
        let line_height_ratio = 1.4;
        let ui_font_family = FamilyId(1);

        Self {
            theme: mock_theme.clone(),
            monospace_font_family: FamilyId(0),
            monospace_font_size: 13.,
            monospace_font_weight: Weight::Normal,
            line_height_ratio,
            ui_builder: UiBuilder::new(
                mock_theme,
                ui_font_family,
                DEFAULT_UI_FONT_SIZE,
                DEFAULT_COMMAND_PALETTE_FONT_SIZE,
                line_height_ratio,
            ),
            ui_font_family,
            ai_font_family: FamilyId(0),
            password_font_family: FamilyId(0),
        }
    }

    pub fn set_theme(&mut self, new_theme: WarpTheme, ctx: &mut ModelContext<Self>) {
        self.theme = new_theme;
        self.ui_builder = UiBuilder::new(
            self.theme.clone(),
            self.ui_font_family,
            self.ui_font_size(),
            DEFAULT_COMMAND_PALETTE_FONT_SIZE,
            self.line_height_ratio,
        );

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        // Allow listeners who specifically care about theme changes to know the theme has changed.
        ctx.emit(AppearanceEvent::ThemeChanged);

        // Notify listeners that appearance-related configuration
        // has changed.
        ctx.notify();
    }

    pub fn set_monospace_font_family(
        &mut self,
        new_family: FamilyId,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_family_id = self.monospace_font_family;
        self.monospace_font_family = new_family;

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        ctx.emit(AppearanceEvent::MonospaceFontFamilyChanged {
            previous_family_id,
            current_family_id: new_family,
        });
    }

    pub fn set_ui_font_family(&mut self, new_family: FamilyId, ctx: &mut ModelContext<Self>) {
        let previous_family_id = self.ui_font_family;
        self.ui_font_family = new_family;

        self.ui_builder = UiBuilder::new(
            self.theme.clone(),
            self.ui_font_family,
            self.ui_font_size(),
            DEFAULT_COMMAND_PALETTE_FONT_SIZE,
            self.line_height_ratio,
        );

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        // We fire the same event as monospace font family change - performance is likely not going to be an issue.
        ctx.emit(AppearanceEvent::UiFontFamilyChanged {
            previous_family_id,
            current_family_id: new_family,
        });
    }

    pub fn set_ai_font_family(&mut self, new_family: FamilyId, ctx: &mut ModelContext<Self>) {
        let previous_family_id = self.ai_font_family;
        self.ai_font_family = new_family;

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        // We fire the same event as monospace font family change - performance is likely not going to be an issue.
        ctx.emit(AppearanceEvent::MonospaceFontFamilyChanged {
            previous_family_id,
            current_family_id: new_family,
        });
    }

    pub fn set_monospace_font_size(&mut self, new_font_size: f32, ctx: &mut ModelContext<Self>) {
        let previous_font_size = self.monospace_font_size;
        self.monospace_font_size = new_font_size;

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        ctx.emit(AppearanceEvent::MonospaceFontSizeChanged {
            current_font_size: self.monospace_font_size,
            previous_font_size,
        });
    }

    pub fn set_monospace_font_weight(
        &mut self,
        new_font_weight: Weight,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_font_weight = self.monospace_font_weight;
        self.monospace_font_weight = new_font_weight;

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        ctx.emit(AppearanceEvent::MonospaceFontWeightChanged {
            current_font_weight: self.monospace_font_weight,
            previous_font_weight,
        });
    }

    #[cfg(feature = "test-util")]
    pub fn set_monospace_font_size_test(&mut self, new_font_size: f32) {
        self.monospace_font_size = new_font_size;
    }

    pub fn set_line_height_ratio(
        &mut self,
        new_line_height_ratio: f32,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_line_height_ratio = self.line_height_ratio;
        self.line_height_ratio = new_line_height_ratio;
        self.ui_builder = UiBuilder::new(
            self.theme.clone(),
            self.ui_font_family,
            DEFAULT_UI_FONT_SIZE,
            DEFAULT_COMMAND_PALETTE_FONT_SIZE,
            self.line_height_ratio,
        );

        // Request a redraw of all windows.
        ctx.invalidate_all_views();

        ctx.emit(AppearanceEvent::LineHeightRatioChanged {
            current_line_height_ratio: self.line_height_ratio,
            previous_line_height_ratio,
        });
    }

    pub fn ui_builder(&self) -> &UiBuilder {
        &self.ui_builder
    }

    pub fn theme(&self) -> &WarpTheme {
        &self.theme
    }

    pub fn monospace_font_family(&self) -> FamilyId {
        self.monospace_font_family
    }

    pub fn ai_font_family(&self) -> FamilyId {
        self.ai_font_family
    }

    pub fn monospace_font_size(&self) -> f32 {
        self.monospace_font_size
    }

    pub fn monospace_ui_scalar(&self) -> f32 {
        self.monospace_font_size / DEFAULT_UI_FONT_SIZE
    }

    pub fn monospace_font_weight(&self) -> Weight {
        self.monospace_font_weight
    }

    pub fn ui_font_family(&self) -> FamilyId {
        self.ui_font_family
    }

    pub fn ui_font_size(&self) -> f32 {
        DEFAULT_UI_FONT_SIZE
    }

    pub fn header_font_family(&self) -> FamilyId {
        self.ui_font_family
    }

    pub fn header_font_size(&self) -> f32 {
        HEADER_FONT_SIZE
    }

    pub fn overline_font_family(&self) -> FamilyId {
        self.ui_font_family
    }

    pub fn overline_font_size(&self) -> f32 {
        OVERLINE_FONT_SIZE
    }

    pub fn line_height_ratio(&self) -> f32 {
        self.line_height_ratio
    }

    pub fn password_font_family(&self) -> FamilyId {
        self.password_font_family
    }
}

impl Entity for Appearance {
    type Event = AppearanceEvent;
}

impl SingletonEntity for Appearance {}
