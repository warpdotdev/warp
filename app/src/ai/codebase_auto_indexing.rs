use std::collections::HashSet;
use std::hash::Hash;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use crate::settings::CodeSettings;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CodebaseAutoIndexingSurface {
    Local,
    Remote,
}

impl CodebaseAutoIndexingSurface {
    fn required_feature_enabled(self) -> bool {
        match self {
            Self::Local => true,
            Self::Remote => FeatureFlag::RemoteCodebaseIndexing.is_enabled(),
        }
    }
}

pub(crate) fn should_use_codebase_indexing(
    surface: CodebaseAutoIndexingSurface,
    ctx: &AppContext,
) -> bool {
    codebase_indexing_enabled(
        surface,
        UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx),
    )
}

pub(crate) fn should_auto_index_codebase(
    surface: CodebaseAutoIndexingSurface,
    ctx: &AppContext,
) -> bool {
    codebase_auto_indexing_enabled(
        surface,
        UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx),
        *CodeSettings::as_ref(ctx).auto_indexing_enabled,
    )
}

fn codebase_indexing_enabled(
    surface: CodebaseAutoIndexingSurface,
    codebase_context_enabled: bool,
) -> bool {
    FeatureFlag::FullSourceCodeEmbedding.is_enabled()
        && surface.required_feature_enabled()
        && codebase_context_enabled
}

pub(crate) fn codebase_auto_indexing_enabled(
    surface: CodebaseAutoIndexingSurface,
    codebase_context_enabled: bool,
    auto_indexing_enabled: bool,
) -> bool {
    codebase_indexing_enabled(surface, codebase_context_enabled) && auto_indexing_enabled
}

pub(crate) fn auto_index_candidate_roots<Root>(
    roots: impl IntoIterator<Item = Root>,
    mut should_request_index: impl FnMut(&Root) -> bool,
) -> Vec<Root>
where
    Root: Clone + Eq + Hash,
{
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for root in roots {
        if seen.insert(root.clone()) && should_request_index(&root) {
            candidates.push(root);
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_auto_indexing_requires_full_source_code_embedding_codebase_context_and_auto_indexing()
    {
        {
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(false);
            assert!(!codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Local,
                true,
                true,
            ));
        }
        {
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(true);
            assert!(codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Local,
                true,
                true,
            ));
            assert!(!codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Local,
                false,
                true,
            ));
            assert!(!codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Local,
                true,
                false,
            ));
            assert!(!codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Local,
                false,
                false,
            ));
        }
    }

    #[test]
    fn remote_auto_indexing_requires_remote_feature() {
        {
            let _remote_flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(false);
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(true);
            assert!(!codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Remote,
                true,
                true,
            ));
        }
        {
            let _remote_flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(true);
            let _flag = FeatureFlag::FullSourceCodeEmbedding.override_enabled(true);
            assert!(codebase_auto_indexing_enabled(
                CodebaseAutoIndexingSurface::Remote,
                true,
                true,
            ));
        }
    }

    #[test]
    fn candidate_roots_are_deduped_before_filtering() {
        let roots = vec!["/repo", "/repo", "/other"];
        let candidates = auto_index_candidate_roots(roots, |root| *root != "/other");

        assert_eq!(candidates, vec!["/repo"]);
    }
}
