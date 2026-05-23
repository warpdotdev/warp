use black_editor::content::version::BufferVersion;
use black_util::path::LineAndColumnArg;

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
