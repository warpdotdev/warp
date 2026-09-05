/// The padding around each block, represented in fractional lines.
///
/// TODO(vorporeal): Change this to hold `Lines` instead of `f32`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BlockPadding {
    /// Padding from top of the block to the prompt.
    pub padding_top: f32,
    /// Padding from bottom of prompt to top of the command.
    pub command_padding_top: f32,
    /// Padding from bottom of command to top of output.
    pub middle: f32,
    /// Padding from bottom of output to bottom of block.
    pub bottom: f32,
}

impl BlockPadding {
    pub fn new(
        padding_top: f32,
        command_padding_top: f32,
        padding_middle: f32,
        padding_bottom: f32,
    ) -> Self {
        BlockPadding {
            padding_top,
            command_padding_top,
            middle: padding_middle,
            bottom: padding_bottom,
        }
    }
}
