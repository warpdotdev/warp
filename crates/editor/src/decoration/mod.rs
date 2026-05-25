use warpui::ModelContext;

use crate::content::buffer::BufferSnapshot;
use crate::content::edit::PreciseDelta;
use crate::content::version::BufferVersion;

pub trait DecorationLayer {
    fn update_internal_state_with_delta(
        &mut self,
        deltas: &[PreciseDelta],
        content_version: BufferVersion,
        content: BufferSnapshot,
        ctx: &mut ModelContext<Self>,
    );
}
