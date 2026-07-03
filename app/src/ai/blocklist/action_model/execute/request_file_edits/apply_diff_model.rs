//! Entity submodel that encapsulates all filesystem access for diff application.
//!
//! The executor holds a [`ModelHandle<ApplyDiffModel>`] and calls
//! [`ApplyDiffModel::apply_diffs`] without knowing whether the session is local
//! or remote. Internally the method resolves the session context, then dispatches:
//!
//! - **Local**: calls [`apply_edits`] with a `std::fs`-backed closure.
//! - **Remote**: returns unsupported; hosted remote file access has been removed.

use ai::diff_validation::AIRequestedCodeDiff;
use futures::FutureExt;
use vec1::Vec1;
use warpui::r#async::BoxFuture;
use warpui::{Entity, ModelContext, ModelHandle};

use crate::ai::agent::FileEdit;
use crate::ai::blocklist::SessionContext;
use crate::terminal::model::session::active_session::ActiveSession;

use super::diff_application::{apply_edits, DiffApplicationError, FileReadResult};

/// Entity submodel that encapsulates filesystem access for diff application.
///
/// Held as a [`ModelHandle`] by the [`super::RequestFileEditsExecutor`].
pub(crate) struct ApplyDiffModel {
    active_session: ModelHandle<ActiveSession>,
}

impl Entity for ApplyDiffModel {
    type Event = ();
}

impl ApplyDiffModel {
    pub fn new(active_session: ModelHandle<ActiveSession>) -> Self {
        Self { active_session }
    }

    /// Resolves session context, then returns a future that applies local edits.
    pub fn apply_diffs(
        &self,
        edits: Vec<FileEdit>,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, Result<Vec<AIRequestedCodeDiff>, Vec1<DiffApplicationError>>> {
        let session_context = SessionContext::from_session(self.active_session.as_ref(ctx), ctx);

        let is_remote = session_context.is_remote();
        let fut = async move {
            if is_remote {
                Err(vec1::vec1![
                    DiffApplicationError::RemoteFileOperationsUnsupported
                ])
            } else {
                apply_edits(edits, &session_context, |path| async move {
                    FileReadResult::from(std::fs::read_to_string(path))
                })
                .await
            }
        };
        fut.boxed()
    }
}
