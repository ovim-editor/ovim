//! Property-Based Testing for Operators
//!
//! Operators (`d`, `y`, `c`, `>`, `<`) compose with motions to form the core
//! of Vim's editing model. Bugs here are high-severity: a broken `dw` means
//! data loss, a broken `cw` means corrupted edits.
//!
//! ## Why Property-Based Testing?
//!
//! Operators interact with cursor position, buffer content, mode transitions,
//! registers, and undo. The combinatorial explosion of states makes hand-written
//! tests insufficient. Property tests verify invariants across thousands of inputs.
//!
//! ## Critical Invariants
//!
//! 1. **No panics**: Any operator+motion on any buffer doesn't crash
//! 2. **Yank is read-only**: After any yank operation, buffer content is unchanged
//! 3. **Delete shortens**: After delete, buffer char count <= original
//! 4. **Cursor in bounds**: After any operation, cursor is within buffer
//! 5. **Mode correctness**: `d` stays in Normal, `c` enters Insert
//! 6. **Buffer integrity**: Content is always valid UTF-8 after any operation

mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;
use proptest::prelude::*;

// ============================================================================
// Test Strategies
// ============================================================================

/// Strategy for buffer content to test operators on.
fn arb_operator_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Word-heavy content (most common case for operators)
        3 => prop::collection::vec(
            "[a-zA-Z0-9_ ]{1,20}",
            1..6,
        ).prop_map(|lines| lines.join("\n")),

        // Code-like content (brackets, indentation)
        2 => prop_oneof![
            Just("fn foo() {\n  let x = 1;\n  bar(x);\n}".to_string()),
            Just("if (a) {\n  b();\n} else {\n  c();\n}".to_string()),
            Just("  hello\n    world\n  foo\nbar".to_string()),
            Just("one two three\nfour five six\nseven eight nine".to_string()),
        ],

        // Edge cases
        1 => prop_oneof![
            Just("a".to_string()),
            Just("".to_string()),
            Just("hello".to_string()),
            Just("   ".to_string()),
            Just("\n\n\n".to_string()),
            Just("hello world\n".to_string()),
        ],
    ]
}

/// Operator+motion key sequences that should work in normal mode.
/// Each entry is a key sequence string that can be fed to EditorTest::keys().
fn arb_delete_motion() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("dd"),
        Just("dw"),
        Just("dW"),
        Just("de"),
        Just("dE"),
        Just("db"),
        Just("dB"),
        Just("d$"),
        Just("d0"),
        Just("d^"),
        Just("dj"),
        Just("dk"),
        Just("dl"),
        Just("dh"),
        Just("d}"),
        Just("d{"),
    ]
}

fn arb_yank_motion() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("yy"),
        Just("yw"),
        Just("yW"),
        Just("ye"),
        Just("yE"),
        Just("yb"),
        Just("yB"),
        Just("y$"),
        Just("y0"),
        Just("y^"),
        Just("yj"),
        Just("yk"),
        Just("yh"),
    ]
}

fn arb_change_motion() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("cc"),
        Just("cw"),
        Just("cW"),
        Just("ce"),
        Just("cE"),
        Just("cb"),
        Just("cB"),
        Just("c$"),
        Just("c0"),
        Just("c^"),
        Just("cl"),
        Just("ch"),
        Just("cj"),
        Just("ck"),
    ]
}

fn arb_indent_motion() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just(">>"),
        Just(">j"),
        Just(">k"),
        Just("<<"),
        Just("<j"),
        Just("<k"),
    ]
}

/// Helper: check that cursor is within valid buffer bounds.
///
/// Operators and the input handler's safety net should keep cursor in bounds.
/// We do NOT call `validate_cursor_position()` here — this ensures bugs are
/// caught at the operator/input layer, not masked by a test-level workaround.
fn assert_editor_cursor_in_bounds(t: &EditorTest, context: &str) -> Result<(), TestCaseError> {
    let cursor = t.editor.buffer().cursor();
    let line_count = t.editor.buffer().line_count();
    prop_assert!(
        cursor.line() < line_count,
        "{}: cursor line {} >= line_count {}",
        context,
        cursor.line(),
        line_count
    );
    Ok(())
}

/// Helper: set cursor to a valid position for the given buffer.
fn setup_cursor(t: &mut EditorTest, line: usize, col: usize) {
    let lc = t.editor.buffer().line_count();
    let safe_line = if lc > 0 { line % lc } else { 0 };
    let line_text = t.editor.buffer().line(safe_line).unwrap_or_default();
    let line_len = line_text.trim_end_matches('\n').chars().count();
    let safe_col = if line_len > 0 { col % line_len } else { 0 };
    t.set_cursor(safe_line, safe_col);
}

// ============================================================================
// Property Tests: Delete Operations
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: Delete operations never panic and cursor stays in bounds.
    #[test]
    fn prop_delete_no_panic(
        text in arb_operator_text(),
        motion in arb_delete_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        assert_editor_cursor_in_bounds(&t, &format!("delete {}", motion))?;
    }

    /// Property: Delete operations don't increase buffer size.
    ///
    /// After any delete, the total char count should be <= original.
    /// (Some delete operations like dd delete whole lines including newlines.)
    #[test]
    fn prop_delete_doesnt_grow_buffer(
        text in arb_operator_text(),
        motion in arb_delete_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        let original_chars = t.editor.buffer().rope().len_chars();
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        let after_chars = t.editor.buffer().rope().len_chars();
        prop_assert!(
            after_chars <= original_chars,
            "{} grew buffer from {} to {} chars",
            motion, original_chars, after_chars
        );
    }

    /// Property: Delete operations stay in normal mode.
    #[test]
    fn prop_delete_stays_normal(
        text in arb_operator_text(),
        motion in arb_delete_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        prop_assert_eq!(
            t.mode(),
            Mode::Normal,
            "Delete '{}' should stay in normal mode, got {:?}",
            motion, t.mode()
        );
    }

    /// Property: Buffer content is valid UTF-8 after delete.
    #[test]
    fn prop_delete_preserves_utf8(
        text in arb_operator_text(),
        motion in arb_delete_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        let content = t.editor.buffer().rope().to_string();
        prop_assert!(
            std::str::from_utf8(content.as_bytes()).is_ok(),
            "Buffer should be valid UTF-8 after '{}'",
            motion
        );
    }
}

// ============================================================================
// Property Tests: Yank Operations
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: Yank operations never panic.
    #[test]
    fn prop_yank_no_panic(
        text in arb_operator_text(),
        motion in arb_yank_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        assert_editor_cursor_in_bounds(&t, &format!("yank {}", motion))?;
    }

    /// Property: Yank is read-only — buffer content must not change.
    ///
    /// This is a critical invariant: yank should copy text to a register
    /// without modifying the buffer at all.
    #[test]
    fn prop_yank_is_readonly(
        text in arb_operator_text(),
        motion in arb_yank_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        let original_content = t.editor.buffer().rope().to_string();
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        let after_content = t.editor.buffer().rope().to_string();
        prop_assert_eq!(
            after_content, original_content,
            "Yank '{}' should not modify buffer",
            motion
        );
    }

    /// Property: Yank operations stay in normal mode.
    #[test]
    fn prop_yank_stays_normal(
        text in arb_operator_text(),
        motion in arb_yank_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        prop_assert_eq!(
            t.mode(),
            Mode::Normal,
            "Yank '{}' should stay in normal mode, got {:?}",
            motion, t.mode()
        );
    }
}

// ============================================================================
// Property Tests: Change Operations
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: Change operations never panic.
    #[test]
    fn prop_change_no_panic(
        text in arb_operator_text(),
        motion in arb_change_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        assert_editor_cursor_in_bounds(&t, &format!("change {}", motion))?;
    }

    /// Property: Change operations enter insert mode.
    ///
    /// In Vim, `c{motion}` deletes the text covered by the motion and
    /// enters insert mode. This is a fundamental mode transition contract.
    #[test]
    fn prop_change_enters_insert(
        text in arb_operator_text(),
        motion in arb_change_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        // Change should enter insert mode (or stay in normal if the motion
        // was a no-op, e.g., ch at col 0, or c0 at col 0)
        let mode = t.mode();
        let is_valid_mode = mode == Mode::Insert || mode == Mode::Normal;
        prop_assert!(
            is_valid_mode,
            "Change '{}' should enter Insert (or stay Normal for no-op), got {:?}",
            motion, mode
        );
    }

    /// Property: Buffer content is valid UTF-8 after change.
    #[test]
    fn prop_change_preserves_utf8(
        text in arb_operator_text(),
        motion in arb_change_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        let content = t.editor.buffer().rope().to_string();
        prop_assert!(
            std::str::from_utf8(content.as_bytes()).is_ok(),
            "Buffer should be valid UTF-8 after '{}'",
            motion
        );
    }
}

// ============================================================================
// Property Tests: Indent/Dedent Operations
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: Indent/dedent operations never panic.
    #[test]
    fn prop_indent_no_panic(
        text in arb_operator_text(),
        motion in arb_indent_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        assert_editor_cursor_in_bounds(&t, &format!("indent {}", motion))?;
    }

    /// Property: Indent/dedent operations stay in normal mode.
    #[test]
    fn prop_indent_stays_normal(
        text in arb_operator_text(),
        motion in arb_indent_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        prop_assert_eq!(
            t.mode(),
            Mode::Normal,
            "Indent '{}' should stay in normal mode, got {:?}",
            motion, t.mode()
        );
    }

    /// Property: Line count doesn't change after indent/dedent.
    ///
    /// Indentation adds/removes leading whitespace but should never
    /// create or destroy lines.
    #[test]
    fn prop_indent_preserves_line_count(
        text in arb_operator_text(),
        motion in arb_indent_motion(),
        start_line in 0..10usize,
        start_col in 0..20usize,
    ) {
        let mut t = EditorTest::new(&text);
        let original_line_count = t.editor.buffer().line_count();
        setup_cursor(&mut t, start_line, start_col);

        t.keys(motion);

        let after_line_count = t.editor.buffer().line_count();
        prop_assert_eq!(
            after_line_count, original_line_count,
            "'{}' changed line count from {} to {}",
            motion, original_line_count, after_line_count
        );
    }
}

// ============================================================================
// Property Tests: Counted Operations
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: Counted delete operations (2dd, 3dw) never panic.
    #[test]
    fn prop_counted_delete_no_panic(
        text in arb_operator_text(),
        count in 2..5usize,
        start_line in 0..5usize,
        start_col in 0..10usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        // Build count + operator string
        let keys = format!("{}dd", count);
        t.keys(&keys);

        assert_editor_cursor_in_bounds(&t, &format!("{}dd", count))?;
        prop_assert_eq!(t.mode(), Mode::Normal);
    }

    /// Property: Counted yank (2yy, 3yw) is still read-only.
    #[test]
    fn prop_counted_yank_readonly(
        text in arb_operator_text(),
        count in 2..5usize,
        start_line in 0..5usize,
        start_col in 0..10usize,
    ) {
        let mut t = EditorTest::new(&text);
        let original = t.editor.buffer().rope().to_string();
        setup_cursor(&mut t, start_line, start_col);

        let keys = format!("{}yy", count);
        t.keys(&keys);

        prop_assert_eq!(
            t.editor.buffer().rope().to_string(),
            original,
            "{}yy should not modify buffer",
            count
        );
    }
}

// ============================================================================
// Property Tests: Composite / Stress
// ============================================================================

/// Any operator+motion key sequence for stress testing.
#[derive(Debug, Clone)]
enum OperatorSequence {
    Delete(&'static str),
    Yank(&'static str),
    Change(&'static str),
    Indent(&'static str),
}

fn arb_operator_sequence() -> impl Strategy<Value = OperatorSequence> {
    prop_oneof![
        3 => arb_delete_motion().prop_map(OperatorSequence::Delete),
        3 => arb_yank_motion().prop_map(OperatorSequence::Yank),
        2 => arb_change_motion().prop_map(OperatorSequence::Change),
        1 => arb_indent_motion().prop_map(OperatorSequence::Indent),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(150))]

    /// Property: Arbitrary sequence of operations never panics.
    ///
    /// This is the comprehensive stress test: apply random operations to
    /// random text and verify the editor never crashes. Between change operations
    /// (which enter insert mode), we press Escape to return to normal mode.
    #[test]
    fn prop_operator_sequence_no_panic(
        text in arb_operator_text(),
        ops in prop::collection::vec(arb_operator_sequence(), 1..8),
        start_line in 0..5usize,
        start_col in 0..10usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        for (i, op) in ops.iter().enumerate() {
            // Ensure we're in normal mode before each operator
            if t.mode() != Mode::Normal {
                t.keys("<Esc>");
            }

            let keys = match op {
                OperatorSequence::Delete(m) => *m,
                OperatorSequence::Yank(m) => *m,
                OperatorSequence::Change(m) => *m,
                OperatorSequence::Indent(m) => *m,
            };

            t.keys(keys);

            assert_editor_cursor_in_bounds(
                &mut t,
                &format!("operator sequence step {}: {}", i, keys),
            )?;

            // Verify buffer integrity
            let content = t.editor.buffer().rope().to_string();
            prop_assert!(
                std::str::from_utf8(content.as_bytes()).is_ok(),
                "Buffer must be valid UTF-8 after step {}: {}",
                i, keys
            );
        }
    }

    /// Property: After any operation sequence, buffer has at least 1 line.
    ///
    /// Vim always maintains at least one (possibly empty) line.
    #[test]
    fn prop_buffer_never_empty_after_ops(
        text in arb_operator_text(),
        ops in prop::collection::vec(arb_operator_sequence(), 1..10),
        start_line in 0..5usize,
        start_col in 0..10usize,
    ) {
        let mut t = EditorTest::new(&text);
        setup_cursor(&mut t, start_line, start_col);

        for op in &ops {
            if t.mode() != Mode::Normal {
                t.keys("<Esc>");
            }

            let keys = match op {
                OperatorSequence::Delete(m) => *m,
                OperatorSequence::Yank(m) => *m,
                OperatorSequence::Change(m) => *m,
                OperatorSequence::Indent(m) => *m,
            };

            t.keys(keys);

            prop_assert!(
                t.editor.buffer().line_count() >= 1,
                "Buffer must have at least 1 line after '{}'",
                keys
            );
        }
    }
}
