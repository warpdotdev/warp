//! "Block banners" are banners that render _inside_ a block, or its snackbar header. Currently it
//! will only render inside the active block, though that constraint can be relaxed with a bit more
//! work. The most important constraint that makes these different from other UI components is that
//! they must conform to a fixed height. This is due to an assumption we made about blocks in order
//! to efficiently viewport them: that the block height can be calculated based on Block state alone
//! without a LayoutContext. Use the exported BLOCK_BANNER_HEIGHT const when the banner height
//! needs to be taken into account.

mod warpify;

pub use warpify::*;
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, Hoverable, MouseState, MouseStateHandle,
        ParentElement, Radius, Stack,
    },
    Element,
};

use crate::themes::theme::WarpTheme;

const CONSTRAINED_BANNER_HEIGHT: f32 = 48.;
const BANNER_TOP_MARGIN: f32 = 16.;
const BANNER_SIDE_MARGIN: f32 = 20.;
const BANNER_V_PADDING: f32 = 4.;
const BANNER_H_PADDING: f32 = 8.;
pub const BLOCK_BANNER_HEIGHT: f32 = CONSTRAINED_BANNER_HEIGHT + BANNER_TOP_MARGIN;
pub const BLOCK_BANNER_DESCRIPTION_MAX_HEIGHT: f32 = 24.;

pub enum WithinBlockBanner {
    WarpifyBanner(WarpifyBannerState),
}

impl WithinBlockBanner {
    pub fn banner_height(&self) -> f32 {
        match self.warpify_mode() {
            Some(WarpificationMode::Ssh { .. }) => {
                BLOCK_BANNER_HEIGHT + BLOCK_BANNER_DESCRIPTION_MAX_HEIGHT
            }
            Some(WarpificationMode::Subshell { .. }) | None => BLOCK_BANNER_HEIGHT,
        }
    }

    pub fn warpify_mode(&self) -> Option<&WarpificationMode> {
        match self {
            WithinBlockBanner::WarpifyBanner(state) => Some(&state.mode),
        }
    }
}

/// These Elements should be common across all block banners. The specific content for each banner
/// should be passed in here. This function also enforces the height invariant.
fn render_block_banner(
    build_child: impl FnOnce(&MouseState) -> Box<dyn Element>,
    hover_state: MouseStateHandle,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    Stack::new()
        .with_child(
            Hoverable::new(hover_state, |hover_state| {
                Container::new(
                    ConstrainedBox::new(
                        Container::new(build_child(hover_state))
                            .with_vertical_padding(BANNER_V_PADDING)
                            .with_horizontal_padding(BANNER_H_PADDING)
                            .with_background_color(theme.block_banner_background().into_solid())
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                            .finish(),
                    )
                    .with_max_height(CONSTRAINED_BANNER_HEIGHT)
                    .finish(),
                )
                .with_margin_top(BANNER_TOP_MARGIN)
                .with_margin_left(BANNER_SIDE_MARGIN)
                .with_margin_right(BANNER_SIDE_MARGIN)
                .finish()
            })
            .finish(),
        )
        .finish()
}
