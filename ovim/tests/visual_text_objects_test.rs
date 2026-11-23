#[macro_use]
#[path = "helpers/editor_test_macro.rs"]
mod editor_test_macro;
mod helpers;

#[test]
fn word_after_combining_character_uses_cursor_columns() {
    editor_test! {
        given { "é word tail" "  ^" }
        keys "viw"
        expect Visual { "é word tail" "  ---^" }
        keys "d"
        expect Normal { "é  tail" "  ^" }
        keys "u"
        expect Normal { "é word tail" "     ^" }
    }
}

#[test]
fn quote_selection_after_emoji_preserves_delimiters() {
    editor_test! {
        given { "👩‍💻 \"hi\" tail" "   ^" }
        keys "vi\""
        expect Visual { "👩‍💻 \"hi\" tail" "   -^" }
        keys "cbye<Esc>"
        expect Normal { "👩‍💻 \"bye\" tail" "     ^" }
    }
}

#[test]
fn quote_selection_includes_the_whole_last_grapheme() {
    editor_test! {
        given { "\"a👩‍💻\"!" " ^" }
        keys "vi\""
        expect Visual { "\"a👩‍💻\"!" " -^" }
        keys "d"
        expect Normal { "\"\"!" " ^" }
    }
}

#[test]
fn inner_pair_ending_at_column_zero_excludes_closing_delimiter() {
    editor_test! {
        given Normal {
            "(abc", " ^",
            ") tail", "",
        }
        keys "vi("
        expect Visual {
            "(abc", " ---^",
            ") tail", "",
        }
        keys "d"
        expect Normal { "() tail", " ^" }
    }
}

#[test]
fn inner_paragraph_excludes_the_following_blank_line() {
    editor_test! {
        given Normal {
            "one", "^",
            "two", "",
            "", "",
            "next", "",
        }
        keys "vipd"
        expect Normal {
            "", "^",
            "next", "",
        }
    }
}

#[test]
fn around_paragraph_excludes_the_first_character_of_next_paragraph() {
    editor_test! {
        given Normal {
            "one", "^",
            "", "",
            "next", "",
        }
        keys "vapd"
        expect Normal { "next", "^" }
    }
}

#[test]
fn visual_tag_uses_the_same_text_object_as_normal_mode() {
    editor_test! {
        given { "<b>hello</b>!" "   ^" }
        keys "vit"
        expect Visual { "<b>hello</b>!" "   ----^" }
        keys "d"
        expect Normal { "<b></b>!" "   ^" }
    }
}

#[test]
fn linewise_selection_uses_grapheme_columns_in_both_directions() {
    editor_test! {
        given Normal {
            "éx", "^",
            "👩‍💻y", "",
        }
        keys "Vj"
        expect VisualLine {
            "éx", "--",
            "👩‍💻y", "^-",
        }
        keys "o"
        expect VisualLine {
            "éx", "^-",
            "👩‍💻y", "--",
        }
    }
}

#[test]
fn multiline_selection_clipboard_and_yank_include_the_same_newline() {
    editor_flow_test! {
        content "(abc\n) tail";
        step "lvi(" => |test| {
            assert_eq!(test.editor.visual_selection_text().as_deref(), Some("abc\n"));
        }
        step "y" => |test| {
            assert_eq!(test.get_register_content('"').as_deref(), Some("abc\n"));
        }
    }
}

#[test]
fn block_clipboard_keeps_whole_graphemes_and_short_rows() {
    editor_flow_test! {
        content "aéx\nb👩‍💻y\nc";
        step "l<C-v>jj" => |test| {
            assert_eq!(test.editor.visual_selection_text().as_deref(), Some("é\n👩‍💻\n"));
        }
        step "y" => |test| {
            assert_eq!(test.get_register_content('"').as_deref(), Some("é\n👩‍💻\n"));
        }
    }
}

#[test]
fn reselect_preserves_a_selected_newline() {
    editor_flow_test! {
        content "(abc\n) tail";
        step "lvi(ygv" => |test| {
            assert_eq!(test.editor.visual_selection_text().as_deref(), Some("abc\n"));
        }
        step "d" => |test| {
            assert_eq!(test.buffer_content(), "() tail\n");
        }
    }
}

#[test]
fn escape_cancels_an_incomplete_visual_text_object() {
    editor_test! {
        given { "hello world" "^" }
        keys "vi<Esc>"
        expect Normal { "hello world" "^" }
    }
}

#[test]
fn reselect_restores_the_full_yanked_range_in_each_visual_mode() {
    for selection in ["ve", "Vj", "<C-v>jl"] {
        let mut test = helpers::EditorTest::new("hello world\nsecond line");
        test.keys(selection);
        let selected = test.editor.visual_selection_text();
        test.keys("ygv");
        assert_eq!(test.editor.visual_selection_text(), selected, "{selection}");
    }
}
