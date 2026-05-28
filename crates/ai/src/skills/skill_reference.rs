use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

const API_PATH_REFERENCE_PREFIX: &str = "warp-skill-location:v1:";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "location", rename_all = "snake_case")]
enum ApiPathReference {
    Local { path: String },
    Remote { host_id: String, path: String },
}

pub(crate) fn encode_api_path_reference(path: &LocalOrRemotePath) -> String {
    let reference = match path {
        LocalOrRemotePath::Local(path) => ApiPathReference::Local {
            path: path.to_string_lossy().into_owned(),
        },
        LocalOrRemotePath::Remote(path) => ApiPathReference::Remote {
            host_id: path.host_id.as_str().to_string(),
            path: path.path.as_str().to_string(),
        },
    };
    let payload = serde_json::to_string(&reference)
        .expect("API path skill references only serialize string fields");
    format!("{API_PATH_REFERENCE_PREFIX}{payload}")
}

pub(crate) fn decode_api_path_reference(path: &str) -> Result<Option<LocalOrRemotePath>, ()> {
    let Some(payload) = path.strip_prefix(API_PATH_REFERENCE_PREFIX) else {
        return Ok(None);
    };
    let reference: ApiPathReference = serde_json::from_str(payload).map_err(|_| ())?;
    let path = match reference {
        ApiPathReference::Local { path } => LocalOrRemotePath::Local(PathBuf::from(path)),
        ApiPathReference::Remote { host_id, path } => LocalOrRemotePath::Remote(RemotePath::new(
            HostId::new(host_id),
            StandardizedPath::try_new(&path).map_err(|_| ())?,
        )),
    };
    Ok(Some(path))
}

/// An unique reference to a skill.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum SkillReference {
    /// A skill identified by the path to its SKILL.md file.
    Path(LocalOrRemotePath),
    /// A bundled skill distributed with Warp.
    BundledSkillId(String),
}

impl fmt::Display for SkillReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SkillReference::Path(path) => path.display_path().fmt(f),
            SkillReference::BundledSkillId(id) => write!(f, "@warp-skill:{id}"),
        }
    }
}

impl From<SkillReference> for warp_multi_agent_api::skill_descriptor::SkillReference {
    fn from(reference: SkillReference) -> Self {
        match reference {
            SkillReference::Path(path) => {
                warp_multi_agent_api::skill_descriptor::SkillReference::Path(
                    encode_api_path_reference(&path),
                )
            }
            SkillReference::BundledSkillId(id) => {
                warp_multi_agent_api::skill_descriptor::SkillReference::BundledSkillId(id)
            }
        }
    }
}
