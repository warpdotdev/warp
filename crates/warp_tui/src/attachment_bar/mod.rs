mod image_processing;
mod model;
mod view;

pub(crate) use model::{TuiAttachmentModel, TuiAttachmentPasteDisposition};
pub(crate) use view::{
    FOCUS_ATTACHMENTS_BINDING_NAME, TuiAttachmentBar, TuiAttachmentBarEvent, init,
};
