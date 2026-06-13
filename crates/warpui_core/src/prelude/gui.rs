//! GUI-backend prelude re-exports.
pub use crate::elements::{
    Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DropShadow, Element, Empty, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MinSize,
    MouseStateHandle, Padding, ParentElement as _, Radius, SavePosition, Text,
};
pub use crate::presenter::EventContext;
pub use crate::ui_components::components::Coords;

pub mod stack {
    pub use crate::elements::{
        AnchorPair, ChildAnchor, OffsetPositioning, OffsetType, ParentAnchor, ParentOffsetBounds,
        PositioningAxis, Stack, XAxisAnchor, YAxisAnchor,
    };
}
