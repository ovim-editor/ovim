use super::marks::{JumpEntry, JumpList, MarkManager, TagStack};
use super::{FindDirection, FindType};

/// Navigation state: marks, jump list, tag stack, and find repeat.
pub struct NavigationState {
    /// Mark manager for buffer marks
    pub marks: MarkManager,
    /// Jump list for Ctrl-O and Ctrl-I
    pub jump_list: JumpList,
    /// The position before the latest jump (`''` and `` ` ` ``); `None` until
    /// there has been one, when it is the start of the buffer.
    pub context_mark: Option<JumpEntry>,
    /// Tag stack for Ctrl-T (LSP goto definition/implementation/type navigation)
    pub tag_stack: TagStack,
    /// Last find motion (for ; and , repeat)
    /// (char, FindType::Find/Till, FindDirection::Forward/Backward)
    pub last_find: Option<(char, FindType, FindDirection)>,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self {
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            context_mark: None,
            tag_stack: TagStack::new(),
            last_find: None,
        }
    }
}
