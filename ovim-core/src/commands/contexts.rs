//! Buffer kinds that change what ex commands may do.
//!
//! A pseudocode reading view only allows the commands whose table entry
//! lists it; the chat scratch and commit message buffers allow everything
//! but give the write/quit family ([`Lifecycle`]) their own meaning.

use crate::command_result::CommandResult;
use crate::editor::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferKind {
    Normal,
    /// A source-mapped pseudocode reading view.
    Pseudocode,
    /// A scratch buffer whose text goes to the AI chat input on `:w`.
    ChatScratch,
    /// A git commit message: `:w` commits, `:q!` aborts.
    CommitMessage,
}

impl BufferKind {
    pub fn of(editor: &Editor) -> Self {
        if editor.is_pseudocode_buffer() {
            BufferKind::Pseudocode
        } else if editor.is_chat_scratch_buffer() {
            BufferKind::ChatScratch
        } else if editor.is_commit_message_buffer() {
            BufferKind::CommitMessage
        } else {
            BufferKind::Normal
        }
    }

    fn bit(self) -> u8 {
        match self {
            BufferKind::Normal => 1,
            BufferKind::Pseudocode => 2,
            BufferKind::ChatScratch => 4,
            BufferKind::CommitMessage => 8,
        }
    }

    /// Why a command is refused in this kind of buffer.
    pub fn refusal(self) -> &'static str {
        match self {
            BufferKind::Pseudocode => {
                "Pseudocode is a reading view; press Enter to edit or save source"
            }
            _ => "Not allowed in this buffer",
        }
    }
}

/// The buffer kinds a command runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contexts(u8);

impl Contexts {
    /// Every buffer that holds editable text.
    pub const EDITABLE: Contexts = Contexts(1 | 4 | 8);
    /// Also the pseudocode reading view (navigation, options, windows).
    pub const ANY: Contexts = Contexts(1 | 2 | 4 | 8);

    pub fn allows(self, kind: BufferKind) -> bool {
        self.0 & kind.bit() != 0
    }
}

/// What a write/quit-family command means to a special buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// `:w`, `:wq`, `:x`: keep the text.
    Write,
    /// `:q`: leave the buffer.
    Quit,
    /// `:bd`: leave the buffer.
    Delete,
    /// `:close`: leave the window.
    Close,
}

/// Run a write/quit-family command in a special buffer, or `None` when the
/// buffer kind gives it no special meaning.
pub fn finish_special(
    editor: &mut Editor,
    kind: BufferKind,
    lifecycle: Lifecycle,
    bang: bool,
) -> Option<CommandResult> {
    use crate::command_result::{err, ok, ok_silent};
    match (kind, lifecycle) {
        (BufferKind::Normal, _) => None,
        // `:q` and `:bd` leave the reading view for the source buffer.
        (BufferKind::Pseudocode, Lifecycle::Quit | Lifecycle::Delete) => {
            Some(match editor.set_pseudocode(false) {
                Ok(()) => ok_silent(),
                Err(error) => err(error.to_string()),
            })
        }
        (BufferKind::Pseudocode, _) => None,
        (BufferKind::ChatScratch, Lifecycle::Write) => {
            Some(match editor.finish_chat_scratch(true) {
                Ok(()) => ok("Scratch content transferred to chat input"),
                Err(error) => err(format!("Could not finish chat scratch: {error}")),
            })
        }
        (BufferKind::ChatScratch, _) => Some(match editor.finish_chat_scratch(false) {
            Ok(()) => ok("Scratch buffer discarded"),
            Err(error) => err(format!("Could not discard chat scratch: {error}")),
        }),
        (BufferKind::CommitMessage, Lifecycle::Write) => {
            editor.finish_commit_message(true);
            Some(ok_silent())
        }
        (BufferKind::CommitMessage, Lifecycle::Quit | Lifecycle::Delete)
            if !bang && editor.is_modified() =>
        {
            Some(err(
                "E37: No write since last change (:w commits, :q! aborts)",
            ))
        }
        (BufferKind::CommitMessage, _) => {
            editor.finish_commit_message(false);
            Some(ok_silent())
        }
    }
}
