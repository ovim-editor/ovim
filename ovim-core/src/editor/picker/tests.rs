use super::*;
use std::path::PathBuf;

#[test]
fn test_glob_match_star() {
    assert!(filter::glob_match("*.rs", "main.rs"));
    assert!(filter::glob_match("*.rs", "MAIN.RS"));
    assert!(!filter::glob_match("*.rs", "main.ts"));
    assert!(filter::glob_match("src/*", "src/lib.rs"));
    assert!(filter::glob_match("*test*", "my_test_file.rs"));
}

#[test]
fn test_glob_match_question() {
    assert!(filter::glob_match("?.rs", "a.rs"));
    assert!(!filter::glob_match("?.rs", "ab.rs"));
    assert!(filter::glob_match("??.rs", "ab.rs"));
}

#[test]
fn test_glob_match_combined() {
    assert!(filter::glob_match("*_test.?s", "my_test.rs"));
    assert!(filter::glob_match("*_test.?s", "my_test.ts"));
    assert!(!filter::glob_match("*_test.?s", "my_test.css"));
}

#[test]
fn test_matches_file_filter_empty() {
    assert!(filter::matches_file_filter("", "src/main.rs"));
    assert!(filter::matches_file_filter("   ", "src/main.rs"));
}

#[test]
fn test_matches_file_filter_substring() {
    assert!(filter::matches_file_filter("mod", "src/mod.rs"));
    assert!(filter::matches_file_filter("mod", "src/editor/mod.rs"));
    assert!(!filter::matches_file_filter("xyz", "src/main.rs"));
}

#[test]
fn test_matches_file_filter_glob() {
    assert!(filter::matches_file_filter("*.rs", "src/main.rs"));
    assert!(!filter::matches_file_filter("*.ts", "src/main.rs"));
}

#[test]
fn test_matches_file_filter_multiple_tokens() {
    assert!(filter::matches_file_filter("*.rs mod", "mod.rs"));
    assert!(!filter::matches_file_filter("*.rs xyz", "mod.rs"));
}

#[test]
fn test_matches_file_filter_path_token() {
    assert!(filter::matches_file_filter("src/", "src/main.rs"));
    assert!(!filter::matches_file_filter("src/", "lib/main.rs"));
}

#[test]
fn test_toggle_field() {
    let mut picker = Picker::new_live_grep(PathBuf::from("."), PathBuf::from("."));
    assert_eq!(picker.active_field(), PickerField::Query);

    picker.toggle_field();
    assert_eq!(picker.active_field(), PickerField::FileFilter);

    picker.toggle_field();
    assert_eq!(picker.active_field(), PickerField::Query);
}

#[test]
fn test_toggle_field_no_op_for_find_files() {
    let mut picker = Picker::new_file_finder(PathBuf::from("."), PathBuf::from("."));
    assert_eq!(picker.active_field(), PickerField::Query);

    picker.toggle_field();
    assert_eq!(picker.active_field(), PickerField::Query);
}

#[test]
fn test_toggle_field_no_op_for_custom() {
    let mut picker = Picker::new_custom(PathBuf::from("."), vec!["a".into(), "b".into()]);
    assert_eq!(picker.active_field(), PickerField::Query);

    picker.toggle_field();
    assert_eq!(picker.active_field(), PickerField::Query);
}

#[test]
fn test_has_file_filter() {
    assert!(!Picker::new_file_finder(PathBuf::from("."), PathBuf::from(".")).has_file_filter());
    assert!(Picker::new_live_grep(PathBuf::from("."), PathBuf::from(".")).has_file_filter());
    assert!(!Picker::new_custom(PathBuf::from("."), vec![]).has_file_filter());
    assert!(!Picker::new_completion(PathBuf::from("."), vec![]).has_file_filter());
    assert!(!Picker::new_lsp_locations(PathBuf::from("."), vec![]).has_file_filter());
}

#[test]
fn test_active_field_mut_delegates_to_query() {
    let mut picker = Picker::new_file_finder(PathBuf::from("."), PathBuf::from("."));
    picker.insert_char('a');
    picker.insert_char('b');
    assert_eq!(picker.query(), "ab");
    assert_eq!(picker.file_filter(), "");
}

#[test]
fn test_active_field_mut_delegates_to_filter() {
    let mut picker = Picker::new_live_grep(PathBuf::from("."), PathBuf::from("."));
    picker.toggle_field();

    picker.insert_char('*');
    picker.insert_char('.');
    picker.insert_char('r');
    picker.insert_char('s');
    assert_eq!(picker.file_filter(), "*.rs");
    assert_eq!(picker.query(), "");
}

#[test]
fn test_backspace_in_filter_field() {
    let mut picker = Picker::new_live_grep(PathBuf::from("."), PathBuf::from("."));
    picker.toggle_field();
    picker.insert_char('a');
    picker.insert_char('b');
    picker.backspace_query();
    assert_eq!(picker.file_filter(), "a");
    assert_eq!(picker.file_filter_cursor(), 1);
}

#[test]
fn test_insert_text_into_query() {
    let mut picker = Picker::new_file_finder(PathBuf::from("."), PathBuf::from("."));
    picker.insert_text("hello");
    assert_eq!(picker.query(), "hello");
    assert_eq!(picker.query_cursor(), 5);

    picker.insert_text(" world");
    assert_eq!(picker.query(), "hello world");
    assert_eq!(picker.query_cursor(), 11);
}

#[test]
fn test_insert_text_at_cursor_midpoint() {
    let mut picker = Picker::new_file_finder(PathBuf::from("."), PathBuf::from("."));
    picker.insert_text("ac");
    picker.move_cursor_left();
    picker.insert_text("b");
    assert_eq!(picker.query(), "abc");
    assert_eq!(picker.query_cursor(), 2);
}

#[test]
fn test_insert_text_into_file_filter() {
    let mut picker = Picker::new_live_grep(PathBuf::from("."), PathBuf::from("."));
    picker.toggle_field();
    picker.insert_text("*.rs");
    assert_eq!(picker.file_filter(), "*.rs");
    assert_eq!(picker.file_filter_cursor(), 4);
    assert_eq!(picker.query(), "");
}

#[test]
fn test_cursor_movement_in_filter_field() {
    let mut picker = Picker::new_live_grep(PathBuf::from("."), PathBuf::from("."));
    picker.toggle_field();
    picker.insert_char('a');
    picker.insert_char('b');
    picker.insert_char('c');
    assert_eq!(picker.file_filter_cursor(), 3);

    picker.move_cursor_left();
    assert_eq!(picker.file_filter_cursor(), 2);

    picker.move_cursor_home();
    assert_eq!(picker.file_filter_cursor(), 0);

    picker.move_cursor_end();
    assert_eq!(picker.file_filter_cursor(), 3);
}
