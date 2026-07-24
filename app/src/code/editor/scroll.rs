use warp_editor::content::version::BufferVersion;
use warp_util::path::LineAndColumnArg;

#[derive(Debug, Clone, Copy)]
pub enum ScrollWheelBehavior {
    #[allow(dead_code)]
    OnlyHandleOnFocus,
    #[allow(dead_code)]
    AlwaysHandle,
    #[allow(dead_code)]
    NeverHandle,
}

impl ScrollWheelBehavior {
    #[allow(dead_code)]
    pub fn should_handle(&self, focused: bool) -> bool {
        match self {
            Self::OnlyHandleOnFocus => focused,
            Self::AlwaysHandle => true,
            Self::NeverHandle => false,
        }
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Clone)]
pub enum ScrollPosition {
    LineAndColumn(LineAndColumnArg),
    FocusedDiffHunk,
    /// Scroll to a fraction of the scrollable range, in `0..=1`. Used to restore scroll position
    /// across a markdown raw<->rendered toggle, where the documents differ and a line/column can't
    /// map between them.
    Fraction(f32),
}

/// We don't want to scroll to the provided line number until the content has
/// been loaded from the file and layout has occurred to update the viewport size.
/// This struct is used to track the state of the scroll trigger.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct ScrollTrigger {
    pub minimum_applicable_version: BufferVersion,
    pub position: ScrollPosition,
}

impl ScrollTrigger {
    /// Create a new scroll trigger that will jump to the provided line number
    /// after the provided version has been loaded and a layout update has occurred.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn new(position: ScrollPosition, version: BufferVersion) -> Self {
        Self {
            position,
            minimum_applicable_version: version,
        }
    }
}
