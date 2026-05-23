use url::Url;

use crate::cloud_object::extract_server_id_and_object_type_from_warp_drive_link;
use crate::drive::OpenWarpDriveObjectArgs;
use crate::ChannelState;

#[derive(PartialEq, Debug)]
pub enum WarpWebLink {
    Session,
    DriveObject(Box<OpenWarpDriveObjectArgs>),
}

pub fn get_item_data_from_warp_link(url: &Url) -> Option<WarpWebLink> {
    if url.origin() == ChannelState::server_root_domain() {
        url.path_segments().and_then(|mut path_segments| {
            path_segments.next().and_then(|segment| match segment {
                "drive" => extract_server_id_and_object_type_from_warp_drive_link(url)
                    .map(|args| WarpWebLink::DriveObject(Box::new(args))),
                "session" => Some(WarpWebLink::Session),
                _ => None,
            })
        })
    } else {
        None
    }
}
