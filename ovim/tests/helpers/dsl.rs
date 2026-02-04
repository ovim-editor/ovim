use super::EditorTest;
use ovim::mode::Mode;

#[derive(Debug, Clone)]
pub struct Fixture {
    pub mode: Mode,
    pub content: String,
    pub cursor: (usize, usize),
    pub visual_start: Option<(usize, usize)>,
    pub expected_visual_selection: Option<((usize, usize), (usize, usize))>,
}

pub fn run_editor_test_case(given: Fixture, keys: &str, expect: Fixture) {
    let mut test = build_editor_from_fixture(&given);
    test.keys(keys);
    assert_editor_matches_fixture(&test, &expect);
}

pub fn run_editor_test_steps(given: Fixture, steps: Vec<(&'static str, Fixture)>) {
    let mut test = build_editor_from_fixture(&given);
    for (keys, expect) in steps {
        test.keys(keys);
        assert_editor_matches_fixture(&test, &expect);
    }
}

pub fn fixture_from_pairs(mode: Mode, pairs: &[&str]) -> Fixture {
    parse_pairs_fixture(mode, pairs)
}

fn build_editor_from_fixture(fixture: &Fixture) -> EditorTest {
    let mut test = EditorTest::new(&fixture.content);
    test.set_cursor(fixture.cursor.0, fixture.cursor.1);
    test.editor.set_mode(fixture.mode);
    if let Some((line, col)) = fixture.visual_start {
        test.editor.set_visual_start(line, col);
    }
    test
}

fn assert_editor_matches_fixture(test: &EditorTest, expect: &Fixture) {
    test.assert_mode(expect.mode);
    test.assert_cursor(expect.cursor.0, expect.cursor.1);

    let actual_buffer = test.buffer_content();
    let expected_buffer = normalize_expected_buffer(&expect.content);
    assert_eq!(actual_buffer, expected_buffer, "Buffer content mismatch");

    if matches!(expect.mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
        assert_eq!(
            test.get_visual_selection(),
            expect.expected_visual_selection,
            "Visual selection mismatch"
        );
    }
}

fn normalize_expected_buffer(content: &str) -> String {
    let mut s = content.to_string();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn parse_pairs_fixture(mode: Mode, pairs: &[&str]) -> Fixture {
    assert!(
        pairs.len() % 2 == 0,
        "Fixture must have even number of strings (text + annotation per line)"
    );

    let mut content_lines: Vec<String> = Vec::new();
    let mut cursor: Option<(usize, usize)> = None;

    // Track selection markers for Visual (charwise) and VisualLine (linewise).
    let mut selected_cells: Vec<(usize, usize)> = Vec::new();
    let mut selected_lines: Vec<usize> = Vec::new();

    for line_idx in (0..pairs.len()).step_by(2).enumerate() {
        let (logical_line_idx, pair_idx) = line_idx;
        let text = pairs[pair_idx];
        let ann = pairs[pair_idx + 1];
        content_lines.push(text.to_string());

        let mut caret_cols: Vec<usize> = Vec::new();
        for (col, ch) in ann.chars().enumerate() {
            match ch {
                '^' => caret_cols.push(col),
                '-' => selected_cells.push((logical_line_idx, col)),
                _ => {}
            }
        }

        if caret_cols.len() > 1 {
            panic!("Annotation line must contain at most one '^' caret");
        }
        if let Some(caret_col) = caret_cols.into_iter().next() {
            if cursor.is_some() {
                panic!("Fixture must contain exactly one '^' caret total");
            }
            cursor = Some((logical_line_idx, caret_col));
            if matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
                selected_cells.push((logical_line_idx, caret_col));
                selected_lines.push(logical_line_idx);
            }
        }

        if matches!(mode, Mode::VisualLine) {
            if ann.chars().any(|c| c != ' ') {
                selected_lines.push(logical_line_idx);
            }
        }
    }

    let content = content_lines.join("\n");
    let cursor = cursor.expect("Fixture must contain exactly one '^' caret total");

    match mode {
        Mode::Normal
        | Mode::Insert
        | Mode::Replace
        | Mode::Command
        | Mode::Search
        | Mode::Picker
        | Mode::HoverPreview
        | Mode::HoverNavigate
        | Mode::FileTree
        | Mode::SubstituteConfirm
        | Mode::Dashboard
        | Mode::LspManager
        | Mode::RenameInput => Fixture {
            mode,
            content,
            cursor,
            visual_start: None,
            expected_visual_selection: None,
        },
        Mode::Visual => {
            let (visual_start, expected_visual_selection) =
                derive_visual_charwise(&content_lines, cursor, &selected_cells);
            Fixture {
                mode,
                content,
                cursor,
                visual_start: Some(visual_start),
                expected_visual_selection: Some(expected_visual_selection),
            }
        }
        Mode::VisualLine => {
            let (visual_start, expected_visual_selection) =
                derive_visual_linewise(&content_lines, cursor, &selected_lines);
            Fixture {
                mode,
                content,
                cursor,
                visual_start: Some(visual_start),
                expected_visual_selection: Some(expected_visual_selection),
            }
        }
        Mode::VisualBlock => Fixture {
            mode,
            content,
            cursor,
            visual_start: Some(cursor),
            expected_visual_selection: Some((cursor, cursor)),
        },
    }
}

fn derive_visual_charwise(
    _content_lines: &[String],
    cursor: (usize, usize),
    selected_cells: &[(usize, usize)],
) -> ((usize, usize), ((usize, usize), (usize, usize))) {
    if selected_cells.is_empty() {
        return (cursor, (cursor, cursor));
    }

    let mut sorted = selected_cells.to_vec();
    sorted.sort_unstable();
    let start = *sorted.first().unwrap();
    let end = *sorted.last().unwrap();

    let visual_start = if cursor == start {
        end
    } else if cursor == end {
        start
    } else if start == end && cursor == start {
        cursor
    } else {
        panic!("For now, caret must be at one end of the selection span");
    };

    (visual_start, (start, end))
}

fn derive_visual_linewise(
    content_lines: &[String],
    cursor: (usize, usize),
    selected_lines: &[usize],
) -> ((usize, usize), ((usize, usize), (usize, usize))) {
    let mut lines = selected_lines.to_vec();
    lines.sort_unstable();
    lines.dedup();

    let start_line = *lines.first().unwrap_or(&cursor.0);
    let end_line = *lines.last().unwrap_or(&cursor.0);

    let visual_start_line = if cursor.0 == start_line {
        end_line
    } else if cursor.0 == end_line {
        start_line
    } else {
        cursor.0
    };

    let end_col = content_lines
        .get(end_line)
        .map(|l| l.chars().count().saturating_sub(1))
        .unwrap_or(0);

    let expected_start = (start_line, 0);
    let expected_end = (end_line, end_col);

    ((visual_start_line, 0), (expected_start, expected_end))
}
