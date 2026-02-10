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
    let mut test =
        EditorTest::new("let a = 1;\nlet b = bad();\nlet c = 3;\nlet d = worse();\n");

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
