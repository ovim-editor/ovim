use crate::editor::{Editor, TextObjectType};
use crate::KeyCode;

/// Decode the same text objects for operators and visual selections. Range
/// resolution belongs to TextObjectType, including its change-only empty range
/// handling; input handlers only decide how to consume that range.
pub(super) fn from_key(editor: &Editor, key: KeyCode, inner: bool) -> Option<TextObjectType> {
    Some(match key {
        KeyCode::Char('w') => TextObjectType::Word { inner, big: false },
        KeyCode::Char('W') => TextObjectType::Word { inner, big: true },
        KeyCode::Char(char @ ('"' | '\'' | '`')) => TextObjectType::Quote { char, inner },
        KeyCode::Char('(' | ')' | 'b') => TextObjectType::Paired {
            open: '(',
            close: ')',
            inner,
        },
        KeyCode::Char('[' | ']') => TextObjectType::Paired {
            open: '[',
            close: ']',
            inner,
        },
        KeyCode::Char('{' | '}' | 'B') => TextObjectType::Paired {
            open: '{',
            close: '}',
            inner,
        },
        KeyCode::Char('<' | '>') => TextObjectType::Paired {
            open: '<',
            close: '>',
            inner,
        },
        KeyCode::Char('p') => TextObjectType::Paragraph { inner },
        KeyCode::Char('s') => TextObjectType::Sentence { inner },
        KeyCode::Char('t') => TextObjectType::Tag { inner },
        KeyCode::Char('i') => TextObjectType::Indent {
            inner,
            tab_width: editor.indent_options().tab_width,
        },
        KeyCode::Char('f') => TextObjectType::Function { inner },
        _ => return None,
    })
}
