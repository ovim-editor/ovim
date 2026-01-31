use super::marks::{JumpList, MarkManager, TagStack};
use super::{FindDirection, FindType};

/// Navigation state: marks, jump list, tag stack, and find repeat.
pub struct NavigationState {
    /// Mark manager for buffer marks
    pub marks: MarkManager,
    /// Jump list for Ctrl-O and Ctrl-I
    pub jump_list: JumpList,
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
            tag_stack: TagStack::new(),
            last_find: None,
        }
    }
}
