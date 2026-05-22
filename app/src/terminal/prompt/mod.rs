use warp_util::path::user_friendly_path;

use super::model::session::SessionType;
use crate::terminal::model::block::Block;
use crate::terminal::model::session::Sessions;

pub fn home_dir_for_block(block: &Block, sessions: &Sessions) -> Option<String> {
    match block.session_id() {
        Some(session_id) => {
            let session = sessions.get(session_id);
            session.and_then(|session| session.home_dir().map(|directory| directory.to_owned()))
        }
        None => None,
    }
}

pub fn user_and_host_name_string(
    session_type: SessionType,
    hostname: &str,
    user: &str,
) -> Option<String> {
    match session_type {
        SessionType::Local => None,
        SessionType::WarpifiedRemote { .. } => Some(format!("{user}@{hostname}:")),
    }
}

pub fn display_path_string(path: Option<&String>, home_dir: Option<&str>) -> String {
    if let Some(path) = path {
        user_friendly_path(path, home_dir).to_string()
    } else {
        ">".to_string()
    }
}
