/// Tests for diagnostic navigation (]d / [d) column positioning.
///
/// Bug: goto_next_diagnostic() and goto_prev_diagnostic() hardcoded col: 0,
/// ignoring the diagnostic's actual column position.
///
/// Fix: Extract both line and character from the diagnostic range, convert
/// UTF-16 character offset to char column via utf16_to_col(), and use it
/// in set_position().
mod helpers;

use helpers::EditorTest;
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

fn make_diagnostic(line: u32, character: u32, message: &str) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position { line, character },
            end: Position {
                line,
                character: character + 1,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        message: message.to_string(),
        ..Default::default()
    }
}

#[test]
fn test_next_diagnostic_jumps_to_column() {
    let mut test = EditorTest::new("fn main() {\n    let x = bad_call();\n}\n");

    // Diagnostic at line 1 (0-indexed), column 12 (UTF-16) — pointing at "bad_call"
    test.editor
        .set_test_diagnostics(vec![make_diagnostic(1, 12, "undefined function")]);

    // Cursor starts at (0, 0), ]d should jump to (1, 12)
    test.keys("]d");
    test.assert_cursor(1, 12);
}

#[test]
fn test_prev_diagnostic_jumps_to_column() {
    let mut test = EditorTest::new("fn main() {\n    let x = bad_call();\n}\n");

    test.editor
        .set_test_diagnostics(vec![make_diagnostic(1, 12, "undefined function")]);

    // Start cursor past the diagnostic
    test.set_cursor(2, 0);

    // [d should jump back to (1, 12)
    test.keys("[d");
    test.assert_cursor(1, 12);
}

#[test]
fn test_diagnostic_nav_wraps_with_column() {
    let mut test = EditorTest::new("let a = 1;\nlet b = bad();\nlet c = 3;\nlet d = worse();\n");

    test.editor.set_test_diagnostics(vec![
        make_diagnostic(1, 8, "error on b"),
        make_diagnostic(3, 8, "error on d"),
    ]);

    // Position cursor after last diagnostic
    test.set_cursor(3, 10);

    // ]d should wrap to first diagnostic at (1, 8)
    test.keys("]d");
    test.assert_cursor(1, 8);

    // Now [d from before first diagnostic should wrap to last at (3, 8)
    test.set_cursor(0, 0);
    test.keys("[d");
    test.assert_cursor(3, 8);
}

#[test]
fn test_diagnostic_nav_same_line_different_columns() {
    let mut test = EditorTest::new("let a = bad1() + bad2();\n");

    // Two diagnostics on the same line at different columns
    test.editor.set_test_diagnostics(vec![
        make_diagnostic(0, 8, "error at bad1"),
        make_diagnostic(0, 17, "error at bad2"),
    ]);

    // Start at col 0, ]d should go to first diagnostic at col 8
    test.keys("]d");
    test.assert_cursor(0, 8);

    // ]d again should advance to col 17 (same line, next diagnostic)
    test.keys("]d");
    test.assert_cursor(0, 17);

    // [d should go back to col 8
    test.keys("[d");
    test.assert_cursor(0, 8);
}

#[test]
fn test_show_diagnostic_at_cursor_chooses_nearest_on_line() {
    let mut test = EditorTest::new("console.log(pendingCount);\n");

    test.editor.set_test_diagnostics(vec![
        make_diagnostic(0, 0, "first diagnostic"),
        make_diagnostic(0, 20, "near pendingCount"),
    ]);

    // Cursor is not inside either range, but is much closer to column 20.
    test.set_cursor(0, 18);
    test.editor.show_diagnostic_at_cursor();

    let hover = test.editor.hover_info().unwrap_or_default();
    assert!(hover.contains("near pendingCount"));
}

#[test]
fn test_show_diagnostic_at_cursor_shows_stale_diagnostics_after_buffer_edit() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("console.log(pendingCount);\n");
    test.set_file_path(path);

    // Seed cached diagnostics.
    test.editor
        .set_test_diagnostics(vec![make_diagnostic(0, 12, "stale diagnostic")]);

    // Edit the buffer — diagnostics are now stale but should remain visible
    // ("show until replaced" model — better than hiding all feedback).
    test.editor
        .buffer_mut()
        .insert_text_at(0, ovim::unicode::CharCol::ZERO, "x");

    // Diagnostic was at col 12, but the edit shifted it. The cursor at col 13
    // still finds the diagnostic (it's at the old position, stale but visible).
    test.set_cursor(0, 13);
    test.editor.show_diagnostic_at_cursor();

    // Diagnostics should still be visible (stale data is shown until replaced).
    assert_eq!(test.editor.mode(), ovim::mode::Mode::HoverPreview);
}

#[test]
fn test_show_diagnostic_at_cursor_hides_diagnostics_after_file_path_change() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("a.rs");
    let file2 = dir.path().join("b.rs");
    std::fs::write(&file1, "let x = bad();\n").unwrap();
    std::fs::write(&file2, "let y = ok();\n").unwrap();

    let file1 = std::fs::canonicalize(&file1)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let file2 = std::fs::canonicalize(&file2)
        .unwrap()
        .to_string_lossy()
        .to_string();

    let mut test = EditorTest::new("let x = bad();\n");
    test.set_file_path(file1);
    test.editor
        .set_test_diagnostics(vec![make_diagnostic(0, 8, "old file diagnostic")]);

    // Simulate path reassociation (e.g. save-as) without refreshing diagnostics.
    test.set_file_path(file2);
    test.set_cursor(0, 8);
    test.editor.show_diagnostic_at_cursor();

    assert_eq!(test.editor.mode(), ovim::mode::Mode::Normal);
    assert_eq!(test.editor.status_message(), "No diagnostics at cursor");
}

#[test]
fn uppercase_diagnostics_skip_other_severities_preserve_columns_and_wrap() {
    let mut test = EditorTest::new("a😀bcdefghijklmnop\nsecond line\n");
    let with_severity = |line, character, severity| {
        let mut diagnostic = make_diagnostic(line, character, "test");
        diagnostic.severity = severity;
        diagnostic
    };
    // Deliberately unsorted, with two errors and a warning on the same line.
    test.editor.set_test_diagnostics(vec![
        with_severity(0, 7, Some(DiagnosticSeverity::ERROR)),
        with_severity(0, 5, Some(DiagnosticSeverity::WARNING)),
        with_severity(0, 3, Some(DiagnosticSeverity::ERROR)),
        with_severity(0, 8, Some(DiagnosticSeverity::INFORMATION)),
        with_severity(0, 9, Some(DiagnosticSeverity::HINT)),
        // Missing severity is already treated as an error throughout ovim.
        with_severity(1, 2, None),
    ]);
    test.keys("]D");
    test.assert_cursor(0, 2);
    test.keys("]d");
    test.assert_cursor(0, 4);
    test.keys("]D");
    test.assert_cursor(0, 6);
    test.keys("]D");
    test.assert_cursor(1, 2);
    test.keys("]D");
    test.assert_cursor(0, 2);
    test.keys("[D");
    test.assert_cursor(1, 2);
    test.keys("2[D");
    test.assert_cursor(0, 2);
}

#[test]
fn uppercase_diagnostics_stay_put_when_there_are_no_errors() {
    let mut test = EditorTest::new("line one\nline two\n");
    let mut warning = make_diagnostic(1, 2, "warning");
    warning.severity = Some(DiagnosticSeverity::WARNING);
    test.editor.set_test_diagnostics(vec![warning]);
    test.set_cursor(0, 3);
    test.keys("]D[D");
    test.assert_cursor(0, 3);
    test.keys("]d");
    test.assert_cursor(1, 2);
}
