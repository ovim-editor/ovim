//! The code pane beside the picker results: the selected file around the
//! matched line, with the same syntax colours as the editor.

use serde::Serialize;

use crate::editor::{Editor, Picker, PickerMode, PickerResult, PreviewCache};

/// Lines sent to the webview. The pane shows what fits and keeps the target
/// line in its middle, so this only has to cover a tall window.
const PREVIEW_LINES: usize = 80;
/// Characters kept per line. The pane clips wider lines anyway.
const PREVIEW_LINE_CHARS: usize = 400;
const PREVIEW_TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiPickerPreview {
    /// Path of the file shown, relative to the picker's directory.
    pub path: String,
    /// 1-based line of the selected match. `None` for a file result.
    pub target_line: Option<usize>,
    /// The file shown is the previous result, while the selected one loads.
    pub stale: bool,
    pub lines: Vec<GuiPreviewLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiPreviewLine {
    /// 1-based line number.
    pub number: usize,
    pub target: bool,
    pub segments: Vec<GuiPreviewSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiPreviewSegment {
    pub text: String,
    pub token: Option<String>,
}

pub(super) fn shows_preview(picker: &Picker) -> bool {
    matches!(
        picker.mode(),
        PickerMode::FindFiles | PickerMode::LiveGrep | PickerMode::LspLocations
    )
}

pub(super) fn picker_preview(editor: &Editor) -> Option<GuiPickerPreview> {
    let picker = editor.picker()?;
    if !shows_preview(picker) {
        return None;
    }
    let has_location = *picker.mode() != PickerMode::FindFiles;
    let result = picker.selected_result()?;
    let (preview, stale) = editor.get_preview_with_fallback(&result.location)?;
    let total_lines = preview.content.lines().count();
    // A stale file cannot supply context for the newly selected location.
    let target = (has_location && !stale && result.line < total_lines).then_some(result.line);
    let start = target
        .map(|line| {
            line.saturating_sub(PREVIEW_LINES / 2)
                .min(total_lines.saturating_sub(PREVIEW_LINES))
        })
        .unwrap_or(0);
    let end = total_lines.min(start + PREVIEW_LINES);
    // Highlighting parses the whole file. While the selection races through
    // results, plain text keeps the picker responsive.
    if !editor.is_picker_scrolling_rapidly() {
        highlight_window(preview, start..end);
    }
    let highlights = preview.highlighted_lines.borrow();
    let lines = preview
        .content
        .lines()
        .enumerate()
        .skip(start)
        .take(end - start)
        .map(|(index, text)| GuiPreviewLine {
            number: index + 1,
            target: target == Some(index),
            segments: segments(
                text,
                highlights.get(&index).map(Vec::as_slice).unwrap_or(&[]),
            ),
        })
        .collect();
    Some(GuiPickerPreview {
        path: shown_path(picker, result, stale),
        target_line: target.map(|line| line + 1),
        stale,
        lines,
    })
}

fn shown_path(picker: &Picker, result: &PickerResult, stale: bool) -> String {
    if stale {
        return String::new();
    }
    let path = std::path::Path::new(&result.location);
    path.strip_prefix(picker.base_dir())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Fill the preview's highlight cache for the lines shown. The cache lives
/// with the preview, so each file is parsed once while the picker is open.
fn highlight_window(preview: &PreviewCache, window: std::ops::Range<usize>) {
    let Some(language) = preview.language else {
        return;
    };
    if window
        .clone()
        .all(|line| preview.highlighted_lines.borrow().contains_key(&line))
    {
        return;
    }
    let Ok(mut highlighter) = crate::syntax::SyntaxHighlighter::new(language) else {
        return;
    };
    highlighter.parse(&preview.content);
    let mut cache = preview.highlighted_lines.borrow_mut();
    for line in window {
        cache
            .entry(line)
            .or_insert_with(|| highlighter.highlights_for_line(line, &preview.content));
    }
}

/// Split one line into runs of one syntax token, with tabs as spaces.
fn segments(
    text: &str,
    highlights: &[(std::ops::Range<usize>, crate::syntax::HighlightGroup)],
) -> Vec<GuiPreviewSegment> {
    let mut segments: Vec<GuiPreviewSegment> = Vec::new();
    let mut column = 0usize;
    for (byte, char) in text.char_indices().take(PREVIEW_LINE_CHARS) {
        // The innermost capture is the last range that covers the byte.
        let token = highlights
            .iter()
            .rev()
            .find(|(range, _)| range.contains(&byte))
            .map(|(_, group)| super::syntax_name(*group));
        let piece = if char == '\t' {
            let cells = PREVIEW_TAB_WIDTH - column % PREVIEW_TAB_WIDTH;
            column += cells;
            " ".repeat(cells)
        } else {
            column += 1;
            char.to_string()
        };
        match segments.last_mut() {
            Some(last) if last.token.as_deref() == token => last.text.push_str(&piece),
            _ => segments.push(GuiPreviewSegment {
                text: piece,
                token: token.map(str::to_string),
            }),
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::HighlightGroup;

    fn location(root: &std::path::Path, line: usize) -> PickerResult {
        PickerResult {
            display: format!("main.rs:{}", line + 1),
            location: root.join("main.rs").to_string_lossy().into_owned(),
            line,
            col: 0,
            match_positions: Vec::new(),
            content: None,
        }
    }

    #[test]
    fn a_location_shows_its_file_around_the_target_line_with_colours() {
        let root = std::env::temp_dir();
        let mut editor = Editor::new();
        let result = location(&root, 119);
        let content: String = (1..=200).map(|n| format!("fn f{n}() {{}}\n")).collect();
        editor.insert_preview(
            result.location.clone(),
            PreviewCache {
                content,
                highlighted_lines: Default::default(),
                language: Some(crate::syntax::Language::Rust),
            },
        );
        editor.set_picker(Picker::new_with_results(root, vec![result]));

        let preview = picker_preview(&editor).unwrap();

        assert_eq!(preview.path, "main.rs");
        assert_eq!(preview.target_line, Some(120));
        assert!(!preview.stale);
        assert_eq!(preview.lines.len(), PREVIEW_LINES);
        assert_eq!(preview.lines[0].number, 120 - PREVIEW_LINES / 2);
        let target = preview.lines.iter().find(|line| line.target).unwrap();
        assert_eq!(target.number, 120);
        assert_eq!(target.segments[0].text, "fn");
        assert_eq!(target.segments[0].token.as_deref(), Some("keyword"));
    }

    #[test]
    fn a_file_still_loading_shows_the_previous_file_without_a_target() {
        let root = std::env::temp_dir();
        let mut editor = Editor::new();
        editor.insert_preview(
            "previous.txt".to_string(),
            PreviewCache {
                content: "one\ntwo\n".to_string(),
                highlighted_lines: Default::default(),
                language: None,
            },
        );
        editor.set_last_shown_preview("previous.txt");
        editor.set_picker(Picker::new_with_results(
            root.clone(),
            vec![location(&root, 1)],
        ));

        let preview = picker_preview(&editor).unwrap();

        assert!(preview.stale);
        assert_eq!(preview.target_line, None);
        assert_eq!(preview.lines.len(), 2);
        assert!(preview.lines.iter().all(|line| !line.target));
    }

    #[test]
    fn a_line_splits_into_one_run_per_token_with_tabs_as_spaces() {
        let highlights = vec![(0..2, HighlightGroup::Keyword)];
        let runs = segments("fn\tmain", &highlights);
        assert_eq!(
            runs,
            vec![
                GuiPreviewSegment {
                    text: "fn".to_string(),
                    token: Some("keyword".to_string()),
                },
                GuiPreviewSegment {
                    text: "  main".to_string(),
                    token: None,
                },
            ]
        );
    }

    #[test]
    fn the_innermost_capture_colours_a_character() {
        let highlights = vec![
            (0..4, HighlightGroup::String),
            (1..3, HighlightGroup::Keyword),
        ];
        let runs = segments("\"ab\"cd", &highlights);
        let tokens: Vec<_> = runs
            .iter()
            .map(|run| (run.text.as_str(), run.token.as_deref()))
            .collect();
        assert_eq!(
            tokens,
            vec![
                ("\"", Some("string")),
                ("ab", Some("keyword")),
                ("\"", Some("string")),
                ("cd", None),
            ]
        );
    }
}
