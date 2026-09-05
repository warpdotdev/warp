use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct HookTrustKey {
    pub(crate) git_root: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) definition_hash: String,
}

pub(crate) trait HookTrustStore: Send + Sync {
    fn is_trusted(&self, key: &HookTrustKey) -> bool;
}

#[derive(Default)]
#[allow(dead_code)]
pub(crate) struct DenyProjectHookTrust;

impl HookTrustStore for DenyProjectHookTrust {
    fn is_trusted(&self, _key: &HookTrustKey) -> bool {
        false
    }
}

#[derive(Default)]
pub(crate) struct ExactHookTrustStore {
    trusted: RwLock<HashSet<HookTrustKey>>,
}

impl ExactHookTrustStore {
    pub(crate) fn trust(&self, key: HookTrustKey) {
        self.trusted.write().unwrap().insert(key);
    }

    #[allow(dead_code)]
    pub(crate) fn revoke(&self, key: &HookTrustKey) {
        self.trusted.write().unwrap().remove(key);
    }
}

impl HookTrustStore for ExactHookTrustStore {
    fn is_trusted(&self, key: &HookTrustKey) -> bool {
        self.trusted.read().unwrap().contains(key)
    }
}
