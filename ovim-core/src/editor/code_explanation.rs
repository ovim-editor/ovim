use super::ai_chat_input::wrap_chat_input_rows;
use std::path::PathBuf;

pub const MAX_WALKTHROUGH_STEPS: usize = 32;
pub const MAX_WALKTHROUGH_COMMENT_BYTES: usize = 4 * 1024;
pub const MAX_WALKTHROUGH_COMMENT_ROWS: usize = 5;

const FALLBACK_SAFE_CODE_ROWS: usize = 40;
const WALKTHROUGH_CARD_RESERVED_ROWS: usize = 10;
const FALLBACK_COMMENT_WIDTH: usize = 76;
const MIN_VIEWPORT_WIDTH: u16 = 32;
const MIN_VIEWPORT_HEIGHT: u16 = 7;
const MAX_CARD_WIDTH: u16 = 100;
const MIN_CARD_HEIGHT: u16 = 7;
const MAX_CARD_HEIGHT: u16 = 10;
const CARD_NON_COMMENT_ROWS: u16 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeExplanationStep {
    pub path: String,
    pub absolute_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub comment: String,
}

/// Stable presentation data for both the terminal UI and headless snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeExplanationView {
    pub current: usize,
    pub total: usize,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub comment: String,
    pub discussion: CodeExplanationDiscussionView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeExplanationDiscussionView {
    Navigating {
        question_count: usize,
        latest_question: Option<String>,
        latest_answer: Option<String>,
        latest_failed: bool,
    },
    Composing {
        input: String,
        cursor: usize,
        question_count: usize,
    },
    Answering {
        question: String,
        answer: String,
        question_count: usize,
    },
}

/// Pure walkthrough-card geometry shared by validation and rendering.
///
/// Keeping this outside the renderer prevents the agent-facing range budget
/// from drifting away from the card that ultimately covers the code pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeExplanationCardLayout {
    pub width: u16,
    pub height: u16,
    pub comment_width: usize,
    pub comment_rows: usize,
}

impl CodeExplanationCardLayout {
    pub fn resolve(
        viewport_width: u16,
        viewport_height: u16,
        comment: &str,
        tab_width: usize,
    ) -> Option<Self> {
        if viewport_width < MIN_VIEWPORT_WIDTH || viewport_height < MIN_VIEWPORT_HEIGHT {
            return None;
        }
        let width = viewport_width.saturating_sub(2).min(MAX_CARD_WIDTH);
        let comment_width = width.saturating_sub(2).max(1) as usize;
        let comment_rows = comment_rows(comment, comment_width, tab_width);
        let height = (comment_rows as u16)
            .saturating_add(CARD_NON_COMMENT_ROWS)
            .clamp(MIN_CARD_HEIGHT, MAX_CARD_HEIGHT)
            .min(viewport_height);
        Some(Self {
            width,
            height,
            comment_width,
            comment_rows,
        })
    }
}

pub fn safe_code_rows(viewport_height: Option<usize>) -> usize {
    match viewport_height {
        Some(height) if height > 0 => height
            .saturating_sub(WALKTHROUGH_CARD_RESERVED_ROWS)
            .clamp(1, FALLBACK_SAFE_CODE_ROWS),
        _ => FALLBACK_SAFE_CODE_ROWS,
    }
}

pub fn comment_rows_for_viewport(
    viewport_width: Option<u16>,
    comment: &str,
    tab_width: usize,
) -> usize {
    let width = viewport_width
        .map(|width| width.saturating_sub(4).min(98) as usize)
        .filter(|width| *width > 0)
        .unwrap_or(FALLBACK_COMMENT_WIDTH);
    comment_rows(comment, width, tab_width)
}

fn comment_rows(comment: &str, width: usize, tab_width: usize) -> usize {
    wrap_chat_input_rows(comment, width, tab_width).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_budget_reserves_the_largest_card() {
        assert_eq!(safe_code_rows(None), 40);
        assert_eq!(safe_code_rows(Some(24)), 14);
        assert_eq!(safe_code_rows(Some(6)), 1);
    }

    #[test]
    fn card_and_validation_share_comment_wrapping() {
        let comment = "one two three four five six seven";
        let layout = CodeExplanationCardLayout::resolve(32, 24, comment, 4).unwrap();
        assert_eq!(
            layout.comment_rows,
            comment_rows_for_viewport(Some(32), comment, 4)
        );
        assert_eq!(layout.width, 30);
        assert!((7..=10).contains(&layout.height));
    }

    #[test]
    fn card_is_absent_when_the_interaction_cannot_be_rendered() {
        assert!(CodeExplanationCardLayout::resolve(31, 24, "comment", 4).is_none());
        assert!(CodeExplanationCardLayout::resolve(80, 6, "comment", 4).is_none());
    }
}
