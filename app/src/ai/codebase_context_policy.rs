#[cfg(not(target_family = "wasm"))]
use crate::features::FeatureFlag;

pub(crate) fn codebase_indexing_enabled(codebase_context_enabled: bool) -> bool {
    codebase_context_enabled
}

pub(crate) fn codebase_auto_indexing_enabled(
    codebase_context_enabled: bool,
    auto_indexing_enabled: bool,
) -> bool {
    codebase_indexing_enabled(codebase_context_enabled) && auto_indexing_enabled
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn remote_codebase_indexing_enabled(codebase_context_enabled: bool) -> bool {
    FeatureFlag::RemoteCodebaseIndexing.is_enabled()
        && FeatureFlag::FullSourceCodeEmbedding.is_enabled()
        && codebase_indexing_enabled(codebase_context_enabled)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn remote_codebase_auto_indexing_enabled(
    codebase_context_enabled: bool,
    auto_indexing_enabled: bool,
) -> bool {
    FeatureFlag::RemoteCodebaseIndexing.is_enabled()
        && FeatureFlag::FullSourceCodeEmbedding.is_enabled()
        && codebase_auto_indexing_enabled(codebase_context_enabled, auto_indexing_enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_auto_indexing_requires_codebase_context_and_auto_indexing() {
        assert!(codebase_auto_indexing_enabled(true, true));
        assert!(!codebase_auto_indexing_enabled(false, true));
        assert!(!codebase_auto_indexing_enabled(true, false));
        assert!(!codebase_auto_indexing_enabled(false, false));
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn remote_indexing_requires_remote_and_fse_flags_and_codebase_context() {
        {
            let _remote_flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(true);
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(false);
            assert!(!remote_codebase_indexing_enabled(true));
        }
        {
            let _remote_flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(true);
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(true);
            assert!(remote_codebase_indexing_enabled(true));
            assert!(!remote_codebase_indexing_enabled(false));
        }
        {
            let _remote_flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(false);
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(true);
            assert!(!remote_codebase_indexing_enabled(true));
        }
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn remote_auto_indexing_requires_remote_indexing_and_auto_indexing() {
        let _remote_flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(true);
        let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(true);

        assert!(remote_codebase_auto_indexing_enabled(true, true));
        assert!(!remote_codebase_auto_indexing_enabled(false, true));
        assert!(!remote_codebase_auto_indexing_enabled(true, false));
        assert!(!remote_codebase_auto_indexing_enabled(false, false));
    }
}
