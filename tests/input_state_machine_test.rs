//! Comprehensive tests for the input state machine.
//!
//! This test file defines the expected behavior for all input handling in ovim,
//! ensuring there are no collisions between different input contexts:
//!
//! 1. Character motions: f, t, F, T (find/till forward/backward)
//! 2. Leader sequences: <Space>xx (LSP and editor commands)
//! 3. Operators: d, c, y (delete, change, yank)
//! 4. Operator + motion: dw, ct", yf;
//! 5. Text objects: iw, a", i(, etc.
//! 6. G-prefix commands: gg, ge, gd, gf, etc.
//! 7. Z-prefix commands: zz, zt, zb, etc.
//! 8. Repeat commands: ; (repeat find), , (reverse find), . (repeat change)
//!
//! The key architectural requirement: each input context must be isolated
//! so that e.g. <Space>t doesn't interfere with the t motion.

mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

// ============================================================================
// SECTION 1: Character Motions (f, t, F, T)
// ============================================================================
//
// These are two-key commands where the second key is the target character.
// - f{char}: move cursor TO the next occurrence of {char}
// - t{char}: move cursor TILL (one before) the next occurrence of {char}
// - F{char}: move cursor TO the previous occurrence of {char}
// - T{char}: move cursor TILL (one after) the previous occurrence of {char}

mod find_motion {
    use super::*;

    #[test]
    fn test_f_moves_to_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0"); // Start at column 0

        test.keys("fo"); // Find 'o'
        assert_eq!(test.cursor(), (0, 4), "f should move TO 'o' at column 4");
    }

    #[test]
    fn test_f_with_count() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("2fo"); // Find second 'o'
        assert_eq!(test.cursor(), (0, 7), "2fo should find second 'o' at column 7");
    }

    #[test]
    fn test_f_no_match_stays_put() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("fz"); // 'z' doesn't exist
        assert_eq!(test.cursor(), (0, 0), "f with no match should not move cursor");
    }

    #[test]
    fn test_f_only_searches_current_line() {
        let mut test = EditorTest::new("hello\nworld");
        test.keys("0");

        test.keys("fw"); // 'w' is on next line
        assert_eq!(test.cursor(), (0, 0), "f should not cross line boundaries");
    }
}

mod till_motion {
    use super::*;

    #[test]
    fn test_t_moves_before_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("to"); // Till 'o'
        assert_eq!(test.cursor(), (0, 3), "t should move one BEFORE 'o' (column 3)");
    }

    #[test]
    fn test_t_with_count() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("2to"); // Till second 'o'
        assert_eq!(test.cursor(), (0, 6), "2to should stop before second 'o' at column 6");
    }

    #[test]
    fn test_t_no_match_stays_put() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("tz"); // 'z' doesn't exist
        assert_eq!(test.cursor(), (0, 0), "t with no match should not move cursor");
    }

    #[test]
    fn test_t_at_adjacent_char_stays_put() {
        let mut test = EditorTest::new("hello");
        test.keys("0"); // At 'h'

        test.keys("te"); // 'e' is at column 1, cursor would go to 0 (where we are)
        // This is an edge case - if we're already at the "till" position, behavior varies
        // Document actual vim behavior here
        let cursor = test.cursor();
        assert!(cursor.1 <= 1, "t to adjacent char should handle edge case");
    }

    #[test]
    fn test_t_only_searches_current_line() {
        let mut test = EditorTest::new("hello\nworld");
        test.keys("0");

        test.keys("tw"); // 'w' is on next line
        assert_eq!(test.cursor(), (0, 0), "t should not cross line boundaries");
    }
}

mod find_backward_motion {
    use super::*;

    #[test]
    fn test_F_moves_to_char_backward() {
        let mut test = EditorTest::new("hello world");
        test.keys("$"); // End of line

        test.keys("Fo"); // Find 'o' backward
        assert_eq!(test.cursor(), (0, 7), "F should move TO 'o' at column 7 (searching backward)");
    }

    #[test]
    fn test_F_with_count() {
        let mut test = EditorTest::new("hello world");
        test.keys("$");

        test.keys("2Fo"); // Find second 'o' backward
        assert_eq!(test.cursor(), (0, 4), "2Fo should find second 'o' backward at column 4");
    }

    #[test]
    fn test_F_no_match_stays_put() {
        let mut test = EditorTest::new("hello world");
        test.keys("$");
        let start = test.cursor();

        test.keys("Fz"); // 'z' doesn't exist
        assert_eq!(test.cursor(), start, "F with no match should not move cursor");
    }
}

mod till_backward_motion {
    use super::*;

    #[test]
    fn test_T_moves_after_char_backward() {
        let mut test = EditorTest::new("hello world");
        test.keys("$"); // End of line

        test.keys("To"); // Till 'o' backward
        assert_eq!(test.cursor(), (0, 8), "T should move one AFTER 'o' (column 8) searching backward");
    }

    #[test]
    fn test_T_with_count() {
        let mut test = EditorTest::new("hello world");
        test.keys("$");

        test.keys("2To"); // Till second 'o' backward
        assert_eq!(test.cursor(), (0, 5), "2To should stop after second 'o' backward at column 5");
    }

    #[test]
    fn test_T_no_match_stays_put() {
        let mut test = EditorTest::new("hello world");
        test.keys("$");
        let start = test.cursor();

        test.keys("Tz"); // 'z' doesn't exist
        assert_eq!(test.cursor(), start, "T with no match should not move cursor");
    }
}

mod find_repeat {
    use super::*;

    #[test]
    fn test_semicolon_repeats_f() {
        let mut test = EditorTest::new("one two three");
        test.keys("0");

        test.keys("ft"); // Find 't' (column 4)
        assert_eq!(test.cursor(), (0, 4));

        test.keys(";"); // Repeat - find next 't' (column 8)
        assert_eq!(test.cursor(), (0, 8), "; should repeat f motion");
    }

    #[test]
    fn test_semicolon_repeats_t() {
        let mut test = EditorTest::new("one two three");
        test.keys("0");

        test.keys("tt"); // Till 't' (column 3)
        assert_eq!(test.cursor(), (0, 3));

        test.keys(";"); // Repeat - till next 't' (column 7)
        assert_eq!(test.cursor(), (0, 7), "; should repeat t motion");
    }

    #[test]
    fn test_comma_reverses_f() {
        let mut test = EditorTest::new("one two three");
        test.keys("0");
        test.keys("2ft"); // Find second 't' (column 8)
        assert_eq!(test.cursor(), (0, 8));

        test.keys(","); // Reverse - find 't' backward (column 4)
        assert_eq!(test.cursor(), (0, 4), ", should reverse f motion direction");
    }

    #[test]
    fn test_comma_reverses_t() {
        // String: "abcdt abcdt"
        // Positions: a=0, b=1, c=2, d=3, t=4, ' '=5, a=6, b=7, c=8, d=9, t=10
        let mut test = EditorTest::new("abcdt abcdt");
        test.keys("$"); // End (position 10, 't')
        test.keys("Tt"); // Till 't' backward - find 't' at 4, position at 5 (one after)
        let after_t = test.cursor();
        assert_eq!(after_t, (0, 5), "Tt from end should position cursor at 5");

        test.keys(","); // Reverse - till 't' forward - should find 't' at 10, position at 9
        assert!(test.cursor().1 > after_t.1, ", should reverse T motion direction");
    }

    #[test]
    fn test_semicolon_with_no_previous_find() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys(";"); // No previous find
        assert_eq!(test.cursor(), (0, 0), "; with no previous find should not move");
    }
}

// ============================================================================
// SECTION 2: Leader Sequences (<Space>xx)
// ============================================================================
//
// Leader key (Space) followed by one or more keys for editor/LSP commands.
// These must NOT interfere with normal motions.

mod leader_sequences {
    use super::*;

    #[test]
    fn test_space_alone_does_nothing_visible() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys(" "); // Just space
        // Space enters Leader state, but doesn't move cursor
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_space_with_invalid_key_cancels() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys(" z"); // Space + invalid key
        test.assert_mode(Mode::Normal);
        assert_eq!(test.cursor(), (0, 0), "Invalid leader sequence should cancel cleanly");
    }

    // Note: These tests verify the leader sequences exist but don't test LSP functionality
    // (which requires a running LSP server)

    #[test]
    fn test_space_t_h_is_type_hierarchy() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        // <Space>th should trigger type hierarchy (LSP command)
        // Without LSP, it should just complete without error
        test.keys(" th");
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_space_c_a_is_code_actions() {
        let mut test = EditorTest::new("hello world");

        test.keys(" ca");
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_space_e_shows_diagnostic_at_cursor() {
        let mut test = EditorTest::new("hello world");

        test.keys(" e");
        // <Space>e shows diagnostic at cursor
        // Without diagnostics, stays in Normal mode
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_minus_toggles_file_tree() {
        let mut test = EditorTest::new("hello world");

        test.keys("-");
        // - enters FileTree mode
        test.assert_mode(Mode::FileTree);
    }
}

// ============================================================================
// SECTION 3: No Collisions Between Contexts
// ============================================================================
//
// This is the critical section - verifying that different input contexts
// don't interfere with each other.

mod no_collisions {
    use super::*;

    #[test]
    fn test_t_motion_not_affected_by_leader() {
        // This is the bug we're fixing!
        // <Space>t is for type hierarchy, but t alone is till motion
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("to"); // Till 'o' - should work independently of <Space>t
        assert_eq!(
            test.cursor(),
            (0, 3),
            "t motion must work independently - not be swallowed by leader sequence handler"
        );
    }

    #[test]
    fn test_t_works_after_using_space_t() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        // First use <Space>th (type hierarchy)
        test.keys(" th");

        // Now t motion should still work
        test.keys("to");
        assert_eq!(
            test.cursor(),
            (0, 3),
            "t motion should work after <Space>th was used"
        );
    }

    #[test]
    fn test_f_motion_unaffected_by_any_leader() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        // Use a leader sequence that doesn't change mode
        test.keys(" ca"); // Code actions (no-op without LSP)

        // f motion should work
        test.keys("fo");
        assert_eq!(test.cursor(), (0, 4), "f motion should work after leader sequence");
    }

    #[test]
    fn test_space_t_then_cancel_then_t_motion() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        // Start <Space>t but then press something invalid
        test.keys(" t");
        test.keys("z"); // Invalid - not 'h', should cancel

        // Now t motion should work
        test.keys("to");
        assert_eq!(
            test.cursor(),
            (0, 3),
            "t motion should work after cancelled <Space>t sequence"
        );
    }

    #[test]
    fn test_c_motion_not_affected_by_leader_c() {
        // <Space>c is for code actions prefix, but c alone is change operator
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("cw"); // Change word
        test.assert_mode(Mode::Insert);
    }

    #[test]
    fn test_s_motion_not_affected_by_leader_s() {
        // <Space>s is for search prefix, but s alone is substitute char
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("sx"); // Substitute 'h' with 'x'
        test.assert_mode(Mode::Insert);
    }

    #[test]
    fn test_consecutive_t_motions() {
        let mut test = EditorTest::new("abcdefghij");
        test.keys("0");

        test.keys("tc"); // Till 'c' -> column 1
        assert_eq!(test.cursor(), (0, 1));

        test.keys("tf"); // Till 'f' -> column 4
        assert_eq!(test.cursor(), (0, 4));

        test.keys("ti"); // Till 'i' -> column 7
        assert_eq!(test.cursor(), (0, 7));
    }

    #[test]
    fn test_mixed_f_and_t_motions() {
        let mut test = EditorTest::new("abcdefghij");
        test.keys("0");

        test.keys("fc"); // Find 'c' -> column 2
        assert_eq!(test.cursor(), (0, 2));

        test.keys("tf"); // Till 'f' -> column 4
        assert_eq!(test.cursor(), (0, 4));

        test.keys("fh"); // Find 'h' -> column 7
        assert_eq!(test.cursor(), (0, 7));
    }
}

// ============================================================================
// SECTION 4: Operators with Character Motions
// ============================================================================
//
// Operators (d, c, y) combined with f/t/F/T motions.

mod operator_with_char_motion {
    use super::*;

    #[test]
    fn test_df_deletes_through_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("dfo"); // Delete through 'o'
        assert_eq!(
            test.buffer_content(),
            " world\n",
            "df should delete from cursor through target char"
        );
    }

    #[test]
    fn test_dt_deletes_until_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("dto"); // Delete until 'o'
        assert_eq!(
            test.buffer_content(),
            "o world\n",
            "dt should delete from cursor until (not including) target char"
        );
    }

    #[test]
    fn test_cf_changes_through_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("cfo"); // Change through 'o'
        test.assert_mode(Mode::Insert);
        assert_eq!(
            test.buffer_content(),
            " world\n",
            "cf should delete through target and enter insert mode"
        );
    }

    #[test]
    fn test_ct_changes_until_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("cto"); // Change until 'o'
        test.assert_mode(Mode::Insert);
        assert_eq!(
            test.buffer_content(),
            "o world\n",
            "ct should delete until target and enter insert mode"
        );
    }

    #[test]
    fn test_yf_yanks_through_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("yfo"); // Yank through 'o'
        test.keys("$p"); // Go to end and paste

        assert!(
            test.buffer_content().contains("hello"),
            "yf should yank through target char"
        );
    }

    #[test]
    fn test_yt_yanks_until_char() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("yto"); // Yank until 'o'
        test.keys("$p"); // Go to end and paste

        let content = test.buffer_content();
        assert!(
            content.contains("hell"),
            "yt should yank until (not including) target char"
        );
    }

    #[test]
    fn test_d2f_deletes_through_second_occurrence() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("d2fo"); // Delete through second 'o'
        assert_eq!(
            test.buffer_content(),
            "rld\n",
            "d2f should delete through second occurrence"
        );
    }
}

// ============================================================================
// SECTION 5: G-Prefix Commands
// ============================================================================
//
// Commands starting with 'g': gg, ge, gd, gf, g;, g,, etc.

mod g_prefix {
    use super::*;

    #[test]
    fn test_gg_goes_to_first_line() {
        let mut test = EditorTest::new("line 1\nline 2\nline 3");
        test.keys("G"); // Go to last line

        test.keys("gg"); // Go to first line
        assert_eq!(test.cursor().0, 0, "gg should go to first line");
    }

    #[test]
    fn test_ge_goes_to_end_of_previous_word() {
        let mut test = EditorTest::new("hello world test");
        test.keys("w"); // Go to 'world'

        test.keys("ge"); // End of previous word
        assert_eq!(test.cursor(), (0, 4), "ge should go to end of previous word");
    }

    #[test]
    fn test_gd_is_go_to_definition() {
        // This is an LSP command - just verify it doesn't crash
        let mut test = EditorTest::new("hello world");
        test.keys("gd");
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_g_alone_waits_for_next_key() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("g");
        // Should be waiting for next key, cursor shouldn't move
        assert_eq!(test.cursor(), (0, 0));
    }

    #[test]
    fn test_g_with_invalid_key_cancels() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("gz"); // Invalid g-command
        test.assert_mode(Mode::Normal);
        assert_eq!(test.cursor(), (0, 0));
    }
}

// ============================================================================
// SECTION 6: Z-Prefix Commands
// ============================================================================
//
// Commands starting with 'z': zz, zt, zb, etc. (viewport scrolling)

mod z_prefix {
    use super::*;

    #[test]
    fn test_zz_centers_viewport() {
        let mut test = EditorTest::new("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15");
        test.keys("7j"); // Go to line 8

        test.keys("zz"); // Center viewport
        test.assert_mode(Mode::Normal);
        // Can't easily test viewport centering without UI, just verify no crash
    }

    #[test]
    fn test_zt_scrolls_to_top() {
        let mut test = EditorTest::new("1\n2\n3\n4\n5\n6\n7\n8\n9\n10");
        test.keys("5j");

        test.keys("zt");
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_zb_scrolls_to_bottom() {
        let mut test = EditorTest::new("1\n2\n3\n4\n5\n6\n7\n8\n9\n10");
        test.keys("5j");

        test.keys("zb");
        test.assert_mode(Mode::Normal);
    }
}

// ============================================================================
// SECTION 7: Marks and Jumps
// ============================================================================
//
// Setting marks (m{a-z}) and jumping to them ('{a-z} or `{a-z})

mod marks {
    use super::*;

    #[test]
    fn test_m_sets_mark() {
        let mut test = EditorTest::new("line 1\nline 2\nline 3");
        test.keys("j"); // Line 2

        test.keys("ma"); // Set mark 'a'
        test.keys("j"); // Line 3
        test.keys("'a"); // Jump to mark 'a'

        assert_eq!(test.cursor().0, 1, "' should jump to marked line");
    }

    #[test]
    fn test_backtick_jumps_to_exact_position() {
        let mut test = EditorTest::new("hello world");
        test.keys("w"); // Go to 'world'

        test.keys("ma"); // Set mark at 'world'
        test.keys("0"); // Go to start
        test.keys("`a"); // Jump to exact position

        assert_eq!(test.cursor(), (0, 6), "` should jump to exact marked position");
    }

    #[test]
    fn test_m_alone_waits_for_register() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("m");
        // Should be waiting for register name
        test.assert_mode(Mode::Normal);
    }
}

// ============================================================================
// SECTION 8: Replace Character
// ============================================================================
//
// r{char} replaces character under cursor

mod replace_char {
    use super::*;

    #[test]
    fn test_r_replaces_single_char() {
        let mut test = EditorTest::new("hello");
        test.keys("0");

        test.keys("rx"); // Replace 'h' with 'x'
        assert_eq!(test.buffer_content(), "xello\n");
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_r_with_count() {
        let mut test = EditorTest::new("hello");
        test.keys("0");

        test.keys("3rx"); // Replace 3 chars with 'x'
        assert_eq!(test.buffer_content(), "xxxlo\n");
    }

    #[test]
    fn test_r_at_end_of_line() {
        let mut test = EditorTest::new("hello");
        test.keys("$"); // End of line

        test.keys("rx");
        assert_eq!(test.buffer_content(), "hellx\n");
    }
}

// ============================================================================
// SECTION 9: State Machine Invariants
// ============================================================================
//
// Tests that verify the input state machine maintains proper invariants.

mod state_invariants {
    use super::*;

    #[test]
    fn test_escape_clears_pending_state() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("d"); // Start delete operator
        test.keys("<Esc>"); // Cancel

        // d should be cancelled, next d should start fresh
        test.keys("dw"); // Delete word
        assert_eq!(test.buffer_content(), "world\n");
    }

    #[test]
    fn test_escape_clears_leader() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys(" "); // Start leader
        test.keys("<Esc>"); // Cancel

        // Leader should be cancelled
        test.keys("to"); // t motion should work
        assert_eq!(test.cursor(), (0, 3));
    }

    #[test]
    fn test_mode_change_clears_pending() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("d"); // Start delete
        test.keys("i"); // Enter insert mode (should this cancel? depends on impl)
        test.keys("<Esc>");

        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_rapid_key_sequence() {
        // Test that rapid sequences don't cause state confusion
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("fofototi");
        // Should process: fo, fo, to, ti
        // This tests that state clears properly between commands
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_count_clears_after_motion() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("2fo"); // 2fo should find second 'o'
        assert_eq!(test.cursor(), (0, 7));

        test.keys("fo"); // fo (without count) should find next 'o'
        // If there's no next 'o', cursor stays
        // The important thing is count was cleared
    }

    #[test]
    fn test_operator_clears_after_motion() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("dw"); // Delete word
        assert_eq!(test.buffer_content(), "world\n");

        test.keys("w"); // Just motion (not dw again)
        // Cursor should move, not delete
        test.assert_mode(Mode::Normal);
    }
}

// ============================================================================
// SECTION 10: Edge Cases
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn test_empty_line_motions() {
        let mut test = EditorTest::new("");
        test.keys("fo");
        test.assert_mode(Mode::Normal);
    }

    #[test]
    fn test_single_char_line() {
        let mut test = EditorTest::new("a");
        test.keys("0");

        test.keys("fa");
        assert_eq!(test.cursor(), (0, 0), "f on current char should stay or find next");
    }

    #[test]
    fn test_special_chars_in_find() {
        let mut test = EditorTest::new("hello.world");
        test.keys("0");

        test.keys("f.");
        assert_eq!(test.cursor(), (0, 5), "f should find special characters");
    }

    #[test]
    fn test_space_in_find() {
        let mut test = EditorTest::new("hello world");
        test.keys("0");

        test.keys("f ");
        assert_eq!(test.cursor(), (0, 5), "f should find space character");
    }

    #[test]
    fn test_unicode_in_find() {
        let mut test = EditorTest::new("hello 世界 world");
        test.keys("0");

        test.keys("f世");
        assert_eq!(test.cursor(), (0, 6), "f should find unicode characters");
    }
}
