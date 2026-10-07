//! Where everything goes in a frame, computed once from editor state and the
//! grid size.
//!
//! The renderer, the viewport resize path and the mouse hit-testing all read
//! these rectangles instead of each re-deriving "how wide is the editor with
//! the tree and the panels open", so they cannot drift apart: the wrap map is
//! sized for the width the next frame draws at, and the cursor and clicks land
//! in the column the text is in.
//!
//! Columns left to right: file tree, editor, docked AI chat, debug panel, test
//! panel. The run console sits below the editor and the chat. When the grid is
//! too narrow for everything, [`plan_columns`] shrinks the panels towards their
//! minimum widths and then hides them, least important first, so the editor
//! always keeps a usable width and the focused panel is the last to go.

use crate::editor::{DiffLayout, Editor};
use crate::mode::Mode;
use ovim_core::Rect;

/// Width of the sign column (git signs, diagnostics). Always present.
pub const SIGN_WIDTH: usize = 2;
/// Spacing between gutter and text content.
pub const GUTTER_SPACING: usize = 1;

/// Editor columns that stay usable when side panels are open.
pub const MIN_EDITOR_WIDTH: u16 = 40;
/// Least the editor is squeezed to for a focused panel that cannot fit
/// beside a comfortable editor.
const FLOOR_EDITOR_WIDTH: u16 = 10;
/// Narrowest a test or debug panel may be shrunk to when sharing the width.
const MIN_PANEL_WIDTH: u16 = 24;
/// Narrowest the file tree may be shrunk to.
const MIN_TREE_WIDTH: u16 = 20;
/// Narrowest the docked chat may be.
const MIN_CHAT_WIDTH: u16 = 30;

/// Widths of the columns in front of the text of a buffer line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GutterWidths {
    /// Line-number column alone (0 when numbers are off).
    pub line_number: usize,
    /// Blame column (0 when blame is off).
    pub blame: usize,
    /// Fold marker column (0 when hidden).
    pub fold: usize,
    /// Everything: blame + fold + sign + line number + spacing.
    pub total: usize,
}

impl GutterWidths {
    pub fn of(editor: &Editor) -> Self {
        let split_review = editor.is_diff_review_buffer()
            && editor
                .diff_review()
                .is_some_and(|review| review.layout == DiffLayout::Split);
        let show_numbers =
            !split_review && (editor.options.number || editor.options.relative_number);
        let line_number = if show_numbers {
            editor.buffer().line_count().to_string().len().max(3)
        } else {
            0
        };

        // Blame column: bracket(1) + space(1) + hash(5) + space(1) + author
        // (truncated) + space(1)
        let blame = if editor.options.blame && !split_review {
            editor.buffer().git_blame().map_or(0, |blame| {
                let author_len = blame.max_author_len().min(15);
                1 + 1 + 5 + 1 + author_len.max(3) + 1
            })
        } else {
            0
        };

        let fold = if split_review {
            0
        } else {
            editor.fold_column_width()
        };

        // The sign column is always present (git signs, diagnostics).
        let total = if split_review {
            0
        } else {
            blame + fold + SIGN_WIDTH + line_number + GUTTER_SPACING
        };
        Self {
            line_number,
            blame,
            fold,
            total,
        }
    }
}

/// A panel that shares the width with the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    FileTree,
    Test,
    Debug,
}

/// How wide a panel would like to be and how narrow it may get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelWant {
    pub preferred: u16,
    pub min: u16,
}

impl PanelWant {
    fn new(preferred: u16, min: u16) -> Self {
        Self {
            preferred,
            min: min.min(preferred),
        }
    }
}

/// Everything that asks for columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnRequest {
    pub file_tree: Option<PanelWant>,
    pub test: Option<PanelWant>,
    pub debug: Option<PanelWant>,
    /// The docked chat's preferred share (percent) of the width the editor
    /// and the chat split between them.
    pub chat_percent: Option<u16>,
    /// The side panel that has the keyboard, which is hidden last.
    pub focused: Option<Panel>,
}

/// Columns granted to each requester; `None` is hidden.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnPlan {
    pub file_tree: Option<u16>,
    pub test: Option<u16>,
    pub debug: Option<u16>,
    pub chat: Option<u16>,
}

impl ColumnPlan {
    fn panels(&self) -> u16 {
        self.file_tree.unwrap_or(0) + self.test.unwrap_or(0) + self.debug.unwrap_or(0)
    }
}

/// Divides `total` columns between the editor and the requested panels.
///
/// The editor keeps [`MIN_EDITOR_WIDTH`] columns whenever the grid has them.
/// The docked chat (its input has the keyboard) is guaranteed
/// [`MIN_CHAT_WIDTH`]; the side panels share what is left, each growing from
/// its minimum towards its preferred width. When even the minimums do not fit,
/// panels are hidden in the order file tree, test panel, debug panel, except
/// that the focused one goes last.
pub fn plan_columns(total: u16, request: &ColumnRequest) -> ColumnPlan {
    let chat_min = request
        .chat_percent
        .map(|_| MIN_CHAT_WIDTH.min(total.saturating_sub(FLOOR_EDITOR_WIDTH)));
    let room = i32::from(total) - i32::from(MIN_EDITOR_WIDTH) - i32::from(chat_min.unwrap_or(0));
    let room = room.max(0) as u32;

    let mut order = [Panel::FileTree, Panel::Test, Panel::Debug];
    order.sort_by_key(|panel| Some(*panel) == request.focused);
    let want = |panel: Panel| match panel {
        Panel::FileTree => request.file_tree,
        Panel::Test => request.test,
        Panel::Debug => request.debug,
    };
    // In the order they are given up.
    let mut kept: Vec<(Panel, PanelWant)> = order
        .iter()
        .filter_map(|&panel| want(panel).map(|want| (panel, want)))
        .collect();
    let min_sum = |kept: &[(Panel, PanelWant)]| -> u32 {
        kept.iter().map(|(_, want)| u32::from(want.min)).sum()
    };
    while !kept.is_empty() && min_sum(&kept) > room {
        kept.remove(0);
    }

    let mut plan = ColumnPlan::default();
    let extra = room - min_sum(&kept);
    let growth: u32 = kept
        .iter()
        .map(|(_, want)| u32::from(want.preferred - want.min))
        .sum();
    for (panel, want) in &kept {
        let grow = u32::from(want.preferred - want.min);
        let grant = if growth <= extra {
            grow
        } else {
            grow * extra / growth
        };
        let width = want.min + grant as u16;
        match panel {
            Panel::FileTree => plan.file_tree = Some(width),
            Panel::Test => plan.test = Some(width),
            Panel::Debug => plan.debug = Some(width),
        }
    }

    // A focused panel that did not fit beside a comfortable editor still gets
    // its minimum: typing into a panel nobody can see is worse than a narrow
    // editor.
    if let Some(panel) = request.focused.filter(|_| request.chat_percent.is_none()) {
        let hidden = match panel {
            Panel::FileTree => plan.file_tree.is_none(),
            Panel::Test => plan.test.is_none(),
            Panel::Debug => plan.debug.is_none(),
        };
        if let (true, Some(want)) = (hidden, want(panel)) {
            let width = want.min.min(total.saturating_sub(FLOOR_EDITOR_WIDTH));
            let width = (width > 0).then_some(width);
            match panel {
                Panel::FileTree => plan.file_tree = width,
                Panel::Test => plan.test = width,
                Panel::Debug => plan.debug = width,
            }
        }
    }

    if let (Some(percent), Some(chat_min)) = (request.chat_percent, chat_min) {
        let shared = total - plan.panels();
        let preferred = (u32::from(shared) * u32::from(percent) / 100) as u16;
        let most = shared.saturating_sub(MIN_EDITOR_WIDTH).max(chat_min);
        plan.chat = Some(preferred.max(chat_min).min(most));
    }
    plan
}

/// Whether the AI chat is docked beside the editor rather than hidden by
/// review mode or an interactive walkthrough.
pub fn should_dock_ai_chat(is_ai_chat: bool, review_mode: bool, walkthrough_open: bool) -> bool {
    is_ai_chat && !review_mode && !walkthrough_open
}

/// Height of the console panel for a content area of `content_height` rows.
pub fn run_console_height(content_height: u16, delta: i16) -> u16 {
    if content_height < 10 {
        return 0;
    }
    let base = i32::from((content_height / 3).clamp(6, 20));
    let wanted = (base + i32::from(delta)).max(4) as u16;
    wanted.min(content_height * 2 / 3)
}

/// The rectangles of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLayout {
    pub tab_bar: Option<Rect>,
    pub file_tree: Option<Rect>,
    /// The editor's windows: the column above the progress and status lines.
    pub buffer: Rect,
    pub progress: Option<Rect>,
    pub status: Rect,
    pub command: Rect,
    pub chat: Option<Rect>,
    pub test_panel: Option<Rect>,
    pub debug_panel: Option<Rect>,
    pub run_console: Option<Rect>,
}

impl FrameLayout {
    /// Lays out a frame of `area`. `None` when the dashboard replaces the
    /// editor and there is nothing to lay out.
    pub fn compute(editor: &Editor, area: Rect) -> Option<Self> {
        if editor.should_show_dashboard() {
            return None;
        }

        // Tab bar (if multiple tabs) + rest
        let (tab_bar, rest) = if editor.tab_count() > 1 {
            let height = area.height.min(1);
            (
                Some(Rect { height, ..area }),
                Rect {
                    y: area.y + height,
                    height: area.height - height,
                    ..area
                },
            )
        } else {
            (None, area)
        };

        let docks_chat = should_dock_ai_chat(
            editor.mode() == Mode::AiChat,
            editor.ai_chat_review_mode(),
            // Interactive walkthroughs temporarily dedicate the shared content
            // width to code. The chat remains alive and resumes as soon as the
            // walkthrough completes or is dismissed.
            editor.ai_chat_has_pending_code_explanation(),
        );
        let tree_width = editor
            .file_tree()
            .is_visible()
            .then(|| editor.file_tree().preferred_width(rest.width));
        let after_tree = rest.width - tree_width.unwrap_or(0).min(rest.width);
        let panel_width = |range: (u16, u16), delta: i16| {
            let preferred = (i32::from((after_tree / 3).clamp(range.0, range.1)) + i32::from(delta))
                .max(20) as u16;
            PanelWant::new(preferred, MIN_PANEL_WIDTH)
        };
        let request = ColumnRequest {
            file_tree: tree_width.map(|width| PanelWant::new(width, MIN_TREE_WIDTH)),
            test: editor
                .is_test_panel_open()
                .then(|| panel_width((30, 50), editor.test_panel().width_delta)),
            debug: editor
                .debug_state()
                .panels_visible
                .then(|| panel_width((30, 46), editor.debug_state().panel.width_delta)),
            chat_percent: docks_chat.then(|| {
                editor
                    .ai_chat_panel_width_percent()
                    .unwrap_or(if editor.ai_chat_allow_edits() { 40 } else { 35 })
                    .clamp(1, 99)
            }),
            focused: match editor.mode() {
                Mode::FileTree => Some(Panel::FileTree),
                Mode::DebugPanel => Some(Panel::Debug),
                _ => None,
            },
        };
        let plan = plan_columns(rest.width, &request);

        let column = |x: u16, width: u16| Rect { x, width, ..rest };
        let mut left = rest.x;
        let mut right = rest.x + rest.width;
        let file_tree = plan.file_tree.map(|width| {
            let tree = column(left, width);
            left += width;
            tree
        });
        let test_panel = plan.test.map(|width| {
            right -= width;
            column(right, width)
        });
        let debug_panel = plan.debug.map(|width| {
            right -= width;
            column(right, width)
        });
        let main = Rect {
            x: left,
            width: right - left,
            ..rest
        };

        // Run console (bottom): build / run / debug output, kept after exit
        let console_height = if editor.run_console().open {
            run_console_height(main.height, editor.run_console().height_delta)
        } else {
            0
        };
        let run_console = (console_height > 0).then(|| Rect {
            y: main.y + main.height - console_height,
            height: console_height,
            ..main
        });
        let main = Rect {
            height: main.height - console_height,
            ..main
        };

        let chat = plan.chat.map(|width| Rect {
            x: main.x + main.width - width,
            width,
            ..main
        });
        let editor_column = Rect {
            width: main.width - plan.chat.unwrap_or(0),
            ..main
        };

        // Windows, then the optional progress line, the status line and the
        // command/message line, bottom up.
        let command_height = editor_column.height.min(1);
        let status_height = (editor_column.height - command_height).min(1);
        let progress_height = if editor.lsp_progress_message().is_some() {
            (editor_column.height - command_height - status_height).min(1)
        } else {
            0
        };
        let buffer_height = editor_column.height - command_height - status_height - progress_height;
        let row = |y: u16, height: u16| Rect {
            y,
            height,
            ..editor_column
        };
        let buffer = row(editor_column.y, buffer_height);
        let progress_y = editor_column.y + buffer_height;
        let status_y = progress_y + progress_height;
        let command_y = status_y + status_height;

        Some(Self {
            tab_bar,
            file_tree,
            buffer,
            progress: (progress_height > 0).then(|| row(progress_y, progress_height)),
            status: row(status_y, status_height),
            command: row(command_y, command_height),
            chat,
            test_panel,
            debug_panel,
            run_console,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WANT: PanelWant = PanelWant {
        preferred: 40,
        min: 24,
    };

    fn request(tree: bool, test: bool, debug: bool) -> ColumnRequest {
        ColumnRequest {
            file_tree: tree.then_some(PanelWant {
                preferred: 30,
                min: 20,
            }),
            test: test.then_some(WANT),
            debug: debug.then_some(WANT),
            ..ColumnRequest::default()
        }
    }

    #[test]
    fn panels_get_their_preferred_width_when_there_is_room() {
        let plan = plan_columns(200, &request(true, true, true));
        assert_eq!(plan.file_tree, Some(30));
        assert_eq!(plan.test, Some(40));
        assert_eq!(plan.debug, Some(40));
    }

    #[test]
    fn a_panel_alone_never_takes_the_editors_minimum() {
        let plan = plan_columns(80, &request(false, true, false));
        assert_eq!(plan.test, Some(40));
        let plan = plan_columns(70, &request(false, true, false));
        assert_eq!(
            plan.test,
            Some(30),
            "grows only into what the editor spares"
        );
    }

    #[test]
    fn panels_share_the_width_proportionally_above_their_minimums() {
        // 100 columns: 60 for panels, minimums 24 + 24, the 12 spare columns
        // are split by how much each would still like to grow (16 each).
        let plan = plan_columns(100, &request(false, true, true));
        assert_eq!((plan.test, plan.debug), (Some(30), Some(30)));
    }

    #[test]
    fn the_least_important_panel_is_hidden_first() {
        // Minimums are 20 (tree) + 24 + 24; the editor keeps 40.
        let plan = plan_columns(120, &request(true, true, true));
        assert!(plan.file_tree.is_some() && plan.test.is_some() && plan.debug.is_some());
        let plan = plan_columns(100, &request(true, true, true));
        assert_eq!(plan.file_tree, None, "the tree is given up first");
        assert!(plan.test.is_some() && plan.debug.is_some());
        let plan = plan_columns(80, &request(true, true, true));
        assert_eq!(
            (plan.file_tree, plan.test),
            (None, None),
            "then the test panel"
        );
        assert!(plan.debug.is_some());
        let plan = plan_columns(60, &request(true, true, true));
        assert_eq!(
            plan,
            ColumnPlan::default(),
            "nothing fits beside 40 columns"
        );
    }

    #[test]
    fn the_focused_panel_is_hidden_last_and_never_below_its_minimum() {
        let mut focused = request(true, true, true);
        focused.focused = Some(Panel::FileTree);
        let plan = plan_columns(70, &focused);
        assert_eq!(plan.file_tree, Some(30));
        assert_eq!((plan.test, plan.debug), (None, None));
        // Too narrow for a comfortable editor: the editor gives way.
        let plan = plan_columns(50, &focused);
        assert_eq!(plan.file_tree, Some(20));
        let plan = plan_columns(25, &focused);
        assert_eq!(
            plan.file_tree,
            Some(15),
            "the editor keeps its 10 column floor"
        );
    }

    #[test]
    fn the_docked_chat_is_guaranteed_room_over_every_side_panel() {
        let mut with_chat = request(true, true, true);
        with_chat.chat_percent = Some(40);
        for total in [60u16, 80, 100, 120, 160, 200] {
            let plan = plan_columns(total, &with_chat);
            let chat = plan.chat.unwrap();
            assert!(chat >= 30, "{total}: chat {chat}");
            let editor = total - plan.panels() - chat;
            assert!(editor >= 30.min(total), "{total}: editor {editor}");
        }
    }

    #[test]
    fn the_chat_split_honors_its_preference_without_starving_the_editor() {
        let chat = |percent| {
            plan_columns(
                100,
                &ColumnRequest {
                    chat_percent: Some(percent),
                    ..ColumnRequest::default()
                },
            )
            .chat
            .unwrap()
        };
        assert_eq!(chat(55), 55);
        assert_eq!(chat(90), 60, "the editor keeps 40");
        assert_eq!(chat(35), 35);
        assert_eq!(chat(5), 30, "never below the minimum");
    }

    #[test]
    fn a_tiny_grid_gives_everything_to_the_editor_or_the_chat() {
        assert_eq!(
            plan_columns(30, &request(true, true, true)),
            ColumnPlan::default()
        );
        let chat = plan_columns(
            35,
            &ColumnRequest {
                chat_percent: Some(40),
                ..request(true, true, true)
            },
        );
        assert_eq!(chat.chat, Some(25), "the editor keeps 10 columns");
        assert_eq!(chat.panels(), 0);
        assert_eq!(
            plan_columns(0, &request(true, true, true)),
            ColumnPlan::default()
        );
    }

    #[test]
    fn run_console_height_matches_the_old_panel_rules() {
        assert_eq!(run_console_height(9, 0), 0);
        assert_eq!(run_console_height(30, 0), 10);
        assert_eq!(run_console_height(30, -50), 4);
        assert_eq!(run_console_height(12, 40), 8);
    }
}
