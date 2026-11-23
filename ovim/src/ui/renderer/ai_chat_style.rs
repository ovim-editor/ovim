use ovim_core::ai::chat_types::ChatRole;
use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Colors (pub(crate) so conversation_tree can reuse them)
// ---------------------------------------------------------------------------

pub(crate) const BG_PANEL: Color = Color::Reset;
pub(super) const BG_INPUT: Color = Color::Rgb(28, 33, 42);

pub(super) const ACCENT_USER: Color = Color::Rgb(98, 176, 255);
pub(super) const ACCENT_ASSISTANT_EDIT: Color = Color::Rgb(132, 209, 149);
pub(super) const ACCENT_ASSISTANT_QUERY: Color = Color::Rgb(120, 165, 235);
pub(super) const ACCENT_ERROR: Color = Color::Rgb(255, 107, 107);
pub(super) const ACCENT_THINKING: Color = Color::Rgb(166, 152, 208);
pub(super) const ACCENT_SELECTED: Color = Color::Rgb(255, 216, 107);

pub(crate) const TEXT_DIM: Color = Color::Rgb(128, 140, 155);
pub(super) const TEXT_THINKING: Color = Color::Rgb(100, 112, 130);
pub(crate) const TEXT_NORMAL: Color = Color::Rgb(200, 208, 220);
pub(super) const BG_USER_ROW: Color = Color::Rgb(25, 41, 64);
pub(super) const BG_ASSISTANT_EDIT_ROW: Color = Color::Rgb(24, 49, 38);
pub(super) const BG_ASSISTANT_QUERY_ROW: Color = Color::Rgb(30, 41, 63);
pub(super) const BG_THINKING_ROW: Color = Color::Rgb(34, 38, 50);
pub(super) const BG_ERROR_ROW: Color = Color::Rgb(66, 30, 33);
pub(super) const BG_SELECTED_ROW: Color = Color::Rgb(48, 56, 74);
pub(super) const BG_USER_LABEL: Color = Color::Rgb(37, 60, 91);
pub(super) const BG_ASSISTANT_EDIT_LABEL: Color = Color::Rgb(38, 67, 52);
pub(super) const BG_ASSISTANT_QUERY_LABEL: Color = Color::Rgb(42, 56, 84);
pub(super) const BG_THINKING_LABEL: Color = Color::Rgb(50, 55, 70);
pub(super) const BG_ERROR_LABEL: Color = Color::Rgb(86, 39, 42);
pub(super) const BG_SELECTED_LABEL: Color = Color::Rgb(70, 80, 103);
pub(super) const TOOL_READ: Color = Color::Rgb(112, 175, 255);
pub(super) const TOOL_NAV: Color = Color::Rgb(126, 211, 160);
pub(super) const TOOL_MUT: Color = Color::Rgb(151, 215, 110);
pub(super) const TOOL_SEARCH: Color = Color::Rgb(224, 193, 110);
pub(super) const TOOL_DIAG: Color = Color::Rgb(255, 173, 102);
pub(super) const TOOL_ERROR: Color = Color::Rgb(255, 107, 107);
pub(super) const TOOL_BG_READ: Color = Color::Rgb(28, 46, 72);
pub(super) const TOOL_BG_NAV: Color = Color::Rgb(26, 50, 39);
pub(super) const TOOL_BG_MUT: Color = Color::Rgb(30, 55, 32);
pub(super) const TOOL_BG_SEARCH: Color = Color::Rgb(58, 46, 27);
pub(super) const TOOL_BG_DIAG: Color = Color::Rgb(62, 41, 26);
pub(super) const TOOL_BG_ERROR: Color = Color::Rgb(67, 28, 31);
pub(super) const TOOL_BG_OTHER: Color = Color::Rgb(36, 40, 50);

#[derive(Clone, Copy)]
pub(super) struct MessageRowStyle {
    pub(super) accent: Color,
    pub(super) label_fg: Color,
    pub(super) label_bg: Color,
    pub(super) text_fg: Color,
    pub(super) body_bg: Color,
}

pub(super) fn message_row_style(
    role: ChatRole,
    allow_edits: bool,
    selected: bool,
) -> MessageRowStyle {
    let mut style = match role {
        ChatRole::User => MessageRowStyle {
            accent: ACCENT_USER,
            label_fg: Color::White,
            label_bg: BG_USER_LABEL,
            text_fg: TEXT_NORMAL,
            body_bg: BG_USER_ROW,
        },
        ChatRole::Assistant => {
            if allow_edits {
                MessageRowStyle {
                    accent: ACCENT_ASSISTANT_EDIT,
                    label_fg: Color::White,
                    label_bg: BG_ASSISTANT_EDIT_LABEL,
                    text_fg: TEXT_NORMAL,
                    body_bg: BG_ASSISTANT_EDIT_ROW,
                }
            } else {
                MessageRowStyle {
                    accent: ACCENT_ASSISTANT_QUERY,
                    label_fg: Color::White,
                    label_bg: BG_ASSISTANT_QUERY_LABEL,
                    text_fg: TEXT_NORMAL,
                    body_bg: BG_ASSISTANT_QUERY_ROW,
                }
            }
        }
        ChatRole::Thinking => MessageRowStyle {
            accent: ACCENT_THINKING,
            label_fg: Color::Rgb(222, 216, 245),
            label_bg: BG_THINKING_LABEL,
            text_fg: TEXT_THINKING,
            body_bg: BG_THINKING_ROW,
        },
        ChatRole::Error => MessageRowStyle {
            accent: ACCENT_ERROR,
            label_fg: Color::White,
            label_bg: BG_ERROR_LABEL,
            text_fg: Color::Rgb(255, 198, 198),
            body_bg: BG_ERROR_ROW,
        },
        ChatRole::Tool => MessageRowStyle {
            accent: ACCENT_ASSISTANT_QUERY,
            label_fg: Color::White,
            label_bg: BG_ASSISTANT_QUERY_LABEL,
            text_fg: TEXT_NORMAL,
            body_bg: BG_ASSISTANT_QUERY_ROW,
        },
    };

    if selected {
        style.accent = ACCENT_SELECTED;
        style.label_bg = BG_SELECTED_LABEL;
        style.body_bg = BG_SELECTED_ROW;
    }

    style
}
