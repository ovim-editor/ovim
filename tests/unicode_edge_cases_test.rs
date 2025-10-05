mod helpers;
use helpers::EditorTest;
// use insta::assert_snapshot;

// ============================================================================
// Unicode - Basic multi-byte characters
// ============================================================================

#[test]
fn test_unicode_basic() {
    let mut test = EditorTest::new("héllo wörld");

    test.keys("w");       // Move to wörld

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_unicode_cursor_movement() {
    let mut test = EditorTest::new("café résumé");

    test.press('l')       // Move right through 'c'
        .press('l')       // Move right through 'a'
        .press('l')       // Move right through 'f'
        .press('l');      // Move right through 'é'

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_unicode_delete() {
    let mut test = EditorTest::new("héllo");

    test.press('x');      // Delete 'h'

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_unicode_delete_accented() {
    let mut test = EditorTest::new("hello é world");

    test.keys("w")        // Move to 'é'
        .press('x');      // Delete 'é'

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Emoji
// ============================================================================

#[test]
fn test_emoji_basic() {
    let mut test = EditorTest::new("hello 😀 world");

    test.keys("w");       // Move to emoji

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_emoji_delete() {
    let mut test = EditorTest::new("test 😀 test");

    test.keys("w")        // Move to emoji
        .press('x');      // Delete emoji

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_emoji_multiple() {
    let mut test = EditorTest::new("😀😁😂😃");

    test.press('l')       // Move through emojis
        .press('l');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_emoji_yank_paste() {
    let mut test = EditorTest::new("test 😀 end");

    test.keys("w")        // Move to emoji
        .keys("yiw")      // Yank emoji
        .keys("$")
        .press('p');      // Paste

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Asian characters (CJK)
// ============================================================================

#[test]
fn test_chinese_characters() {
    let mut test = EditorTest::new("你好世界");

    test.press('l')       // Move through Chinese chars
        .press('l');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_chinese_word_motion() {
    let mut test = EditorTest::new("hello 世界 test");

    test.press('w')       // Move to Chinese
        .press('w');      // Move past Chinese

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_japanese_hiragana() {
    let mut test = EditorTest::new("こんにちは");

    test.press('l')
        .press('l');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_japanese_mixed() {
    let mut test = EditorTest::new("Hello こんにちは World");

    test.press('w')       // To こんにちは
        .press('w');      // To World

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_korean_hangul() {
    let mut test = EditorTest::new("안녕하세요");

    test.press('l')
        .press('l');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - RTL (Right-to-Left) text
// ============================================================================

#[test]
fn test_arabic_text() {
    let mut test = EditorTest::new("مرحبا");

    test.press('l')
        .press('l');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_hebrew_text() {
    let mut test = EditorTest::new("שלום");

    test.press('l');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_mixed_ltr_rtl() {
    let mut test = EditorTest::new("hello مرحبا world");

    test.press('w')       // To Arabic
        .press('w');      // To world

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Combining characters
// ============================================================================

#[test]
fn test_combining_diacritic() {
    // e + combining acute accent
    let mut test = EditorTest::new("e\u{0301}");

    test.press('x');      // Delete combined character

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_combining_multiple() {
    // a + combining grave + combining tilde
    let mut test = EditorTest::new("a\u{0300}\u{0303}");

    test.press('l');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Zero-width characters
// ============================================================================

#[test]
fn test_zero_width_joiner() {
    // Family emoji using ZWJ
    let mut test = EditorTest::new("👨‍👩‍👧");

    test.press('x');      // Delete complex emoji

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_zero_width_space() {
    let mut test = EditorTest::new("hello\u{200B}world");

    test.keys("w");

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Word boundaries
// ============================================================================

#[test]
fn test_unicode_word_boundary() {
    let mut test = EditorTest::new("hello_世界_test");

    test.press('w')       // Should treat as word
        .press('w');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_unicode_diw() {
    let mut test = EditorTest::new("hello 世界 test");

    test.press('w')       // Move to 世界
        .keys("diw");     // Delete inner word

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_unicode_daw() {
    let mut test = EditorTest::new("hello 世界 test");

    test.press('w')
        .keys("daw");     // Delete around word

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Search and find
// ============================================================================

#[test]
fn test_search_unicode() {
    let mut test = EditorTest::new("hello 世界 test 世界");

    test.press('/')
        .type_text("世界")
        .press_enter();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_find_unicode_char() {
    let mut test = EditorTest::new("hello 世 world");

    test.press('f')
        .type_text("世");

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_search_emoji() {
    let mut test = EditorTest::new("test 😀 more 😀 text");

    test.press('/')
        .type_text("😀")
        .press_enter()
        .press('n');      // Next match

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Yank and paste
// ============================================================================

#[test]
fn test_yank_unicode_word() {
    let mut test = EditorTest::new("hello 世界 test");

    test.press('w')
        .keys("yiw")      // Yank 世界
        .keys("$")
        .press('p');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_yank_emoji_sequence() {
    let mut test = EditorTest::new("😀😁😂");

    test.keys("yy")       // Yank line
        .press('p');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_yank_mixed_unicode() {
    let mut test = EditorTest::new("hello 世界 😀 test");

    test.keys("yy")
        .press('p');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Visual mode
// ============================================================================

#[test]
fn test_visual_select_unicode() {
    let mut test = EditorTest::new("hello 世界 test");

    test.press('w')       // Move to 世界
        .press('v')
        .keys("ll");      // Select characters

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_visual_select_emoji() {
    let mut test = EditorTest::new("test 😀😁😂 end");

    test.press('w')
        .press('v')
        .keys("lll");     // Select emojis

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Insert mode
// ============================================================================

#[test]
fn test_insert_unicode() {
    let mut test = EditorTest::new("hello");

    test.press('a')
        .type_text(" 世界")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_insert_emoji() {
    let mut test = EditorTest::new("test");

    test.press('a')
        .type_text(" 😀")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Line length and column positions
// ============================================================================

#[test]
fn test_dollar_with_unicode() {
    let mut test = EditorTest::new("hello 世界");

    test.keys("$");       // End of line with unicode

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_zero_with_unicode() {
    let mut test = EditorTest::new("世界 hello");

    test.keys("$")
        .keys("0");       // Beginning with unicode

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode - Change and replace
// ============================================================================

#[test]
fn test_change_unicode_word() {
    let mut test = EditorTest::new("hello 世界 test");

    test.press('w')
        .keys("ciw")      // Change 世界
        .type_text("world")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_replace_with_unicode() {
    let mut test = EditorTest::new("hello world");

    test.press('r')
        .type_text("世");  // Replace with Chinese char

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_replace_unicode_with_ascii() {
    let mut test = EditorTest::new("世界");

    test.press('r')
        .press('x');      // Replace Chinese with 'x'

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Special characters - Tabs and control characters
// ============================================================================

#[test]
fn test_tab_character() {
    let mut test = EditorTest::new("hello\tworld");

    test.press('l')
        .press('l')
        .press('l')
        .press('l')
        .press('l');      // Move through tab

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_word_motion_with_tab() {
    let mut test = EditorTest::new("hello\tworld");

    test.press('w');      // Should skip tab

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_delete_tab() {
    let mut test = EditorTest::new("hello\tworld");

    test.keys("w")        // Move to tab area
        .press('h')       // Back to tab
        .press('x');      // Delete tab

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Edge cases - Empty strings and whitespace
// ============================================================================

#[test]
fn test_only_unicode() {
    let mut test = EditorTest::new("世界");

    test.press('x');      // Delete first char

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_only_emoji() {
    let mut test = EditorTest::new("😀");

    test.press('x');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Mixed ASCII and Unicode
// ============================================================================

#[test]
fn test_mixed_word_boundaries() {
    let mut test = EditorTest::new("hello世界test");

    test.press('w')       // Move through mixed text
        .press('w');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_alternating_ascii_unicode() {
    let mut test = EditorTest::new("a世b界c");

    test.press('w')
        .press('w')
        .press('w');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Very long unicode sequences
// ============================================================================

#[test]
fn test_long_unicode_line() {
    let mut test = EditorTest::new(&"世".repeat(100));

    test.keys("50l");     // Move 50 chars right

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_long_emoji_sequence() {
    let mut test = EditorTest::new(&"😀".repeat(50));

    test.keys("10w");

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Unicode normalization
// ============================================================================

#[test]
fn test_precomposed_vs_decomposed() {
    // é as single char vs e + combining acute
    let mut test1 = EditorTest::new("café");      // é is U+00E9
    let mut test2 = EditorTest::new("cafe\u{0301}"); // e + U+0301

    test1.keys("$");
    test2.keys("$");

    assert_snapshot!("precomposed", test1.snapshot_state());
    assert_snapshot!("decomposed", test2.snapshot_state());
}

// ============================================================================
// Grapheme clusters
// ============================================================================

#[test]
fn test_grapheme_cluster_flag() {
    // Flag emoji (regional indicators)
    let mut test = EditorTest::new("🇺🇸");

    test.press('x');      // Should delete whole flag

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_grapheme_cluster_skin_tone() {
    // Emoji with skin tone modifier
    let mut test = EditorTest::new("👋🏽");

    test.press('x');      // Should delete emoji with modifier

    assert_snapshot!(test.snapshot_state());
}
