use std::collections::HashMap;

use settings::Setting as _;
use vim::vim::MotionType;
use warpui::clipboard::ClipboardContent;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::settings::AppEditorSettings;

/// Wraps the actual text that was copied as well as the motion type. That is needed because
/// linewise motions need to be pasted on their own line, NOT at the cursor position.
#[derive(Clone, Debug)]
pub struct RegisterContent {
    pub text: String,
    pub motion_type: MotionType,
}

impl RegisterContent {
    fn new(text: String, motion_type: MotionType) -> Self {
        Self { text, motion_type }
    }
}

#[derive(Default)]
pub struct VimRegisters {
    registers: HashMap<char, RegisterContent>,
}

impl VimRegisters {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VimRegisters {
    /// The registers * and + are special registers that go to the system clipboard.
    fn points_to_system_clipboard(register_name: char, app: &AppContext) -> bool {
        let unnamed_system_clipboard = *AppEditorSettings::as_ref(app)
            .vim_unnamed_system_clipboard
            .value();
        register_name == '*'
            || register_name == '+'
            || (unnamed_system_clipboard && register_name == '"')
    }

    pub fn write_to_register(
        &mut self,
        register_name: char,
        content: String,
        motion_type: MotionType,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut register_name = register_name;
        if Self::points_to_system_clipboard(register_name, ctx) {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(content.clone()));
            // Normalize '*' and '+' to both point to '+'.
            register_name = '+';
        }
        // Even if the text is for the system clipboard, we still need to save a copy of it in this
        // model in order to remember if it was linewise or charwise. That determines how it gets
        // pasted.
        self.registers
            .insert(register_name, RegisterContent::new(content, motion_type));
    }

    pub fn read_from_register(
        &self,
        register_name: char,
        ctx: &mut ModelContext<Self>,
    ) -> Option<RegisterContent> {
        // If this is coming from the system clipboard, we need to check if the content was yanked
        // using a linewise motion. If it was, it needs to be pasted on its own line.
        if Self::points_to_system_clipboard(register_name, ctx) {
            let register_content = self.registers.get(&'+');
            let clipboard_content = ctx.clipboard().read().plain_text;

            // Read this model's register entry for the system clipboard. It may be different if
            // the user wrote to the clipboard from a different app. If it's the same, we will
            // assume that we wrote this entry to the clipboard, and we return our own stored value
            // for the motion type.
            if register_content.is_some_and(|register| register.text == clipboard_content) {
                register_content.cloned()
            } else {
                // If the system clipboard content doesn't match, assume it came from a different
                // app and therefore could not have been copied from a Vim linewise motion. So,
                // treat it as charwise.
                Some(RegisterContent {
                    text: clipboard_content,
                    motion_type: MotionType::Charwise,
                })
            }
        } else {
            self.registers.get(&register_name).cloned()
        }
    }
}

impl Entity for VimRegisters {
    type Event = ();
}

impl SingletonEntity for VimRegisters {}
