use crate::cloud_object::ObjectType;
use crate::code_review::diff_state::DiffMode;
use crate::search::mixer::SearchMixer;

pub type AIContextMenuMixer = SearchMixer<AIContextMenuSearchableAction>;

#[derive(Debug, Clone, PartialEq)]
pub enum AIContextMenuSearchableAction {
    InsertFilePath {
        /// This is the file path relative to the root of the current git
        /// repository. If this changes, this could break how we resolve
        /// the file path outside of AI mode, so just note the downstream
        /// dependencies.
        file_path: String,
    },
    InsertText {
        /// Text to insert into the input buffer.
        text: String,
    },
    InsertDriveObject {
        /// The type of the drive object (Workflow, Notebook, etc.)
        object_type: ObjectType,
        /// The UID of the drive object to insert as <object_type:{uid}>
        object_uid: String,
    },
    InsertPlan {
        /// The UID of the AI document to insert as <plan:{uid}>
        ai_document_uid: String,
    },
    InsertDiffSet {
        /// The diff mode indicating what base to compare against
        diff_mode: DiffMode,
    },
    InsertConversation {
        /// The conversation identifier to insert as <convo:{id}>.
        conversation_id: String,
    },
    InsertSkill {
        /// The skill name to insert as /{name} into the buffer.
        name: String,
    },
}
