/// Test to verify picker preview loading works correctly
/// This tests the fix for the issue where LSP location pickers
/// were using indices instead of actual file paths in the location field
use ovim::editor::{Picker, PickerResult};
use std::path::PathBuf;

#[test]
fn test_picker_new_with_results_preserves_file_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base_dir = tmp.path().join("workspace").join("ovim");
    let picker_path = base_dir.join("src").join("editor").join("picker.rs");
    let lsp_path = base_dir
        .join("src")
        .join("editor")
        .join("lsp_integration.rs");
    let base_dir_str = base_dir.to_string_lossy().to_string();
    let picker_path_str = picker_path.to_string_lossy().to_string();
    let lsp_path_str = lsp_path.to_string_lossy().to_string();

    // Create some PickerResult items with actual file paths
    let results = vec![
        PickerResult {
            display: "picker.rs:158:9".to_string(),
            location: picker_path_str.clone(),
            line: 157,
            col: 8,
            match_positions: Vec::new(),
            content: None,
        },
        PickerResult {
            display: "lsp_integration.rs:2964:5".to_string(),
            location: lsp_path_str.clone(),
            line: 2963,
            col: 4,
            match_positions: Vec::new(),
            content: None,
        },
    ];

    let base_dir = PathBuf::from(base_dir_str);

    // Create picker using new_with_results
    let picker = Picker::new_with_results(base_dir, results.clone());

    // Verify that the location field is preserved (not converted to indices)
    let filtered = picker.filtered_results();
    assert_eq!(filtered.len(), 2);

    // The location field should contain actual file paths, not indices like "0" or "1"
    assert_eq!(filtered[0].location, picker_path_str);
    assert_eq!(filtered[1].location, lsp_path_str);

    // Verify selected result also has the correct location
    let selected = picker
        .selected_result()
        .expect("Should have a selected result");
    assert_eq!(selected.location, filtered[0].location);
}

#[test]
fn test_old_new_lsp_locations_uses_indices() {
    // This test documents the old behavior (using indices) for comparison
    let display_items = vec![
        "picker.rs:158:9".to_string(),
        "lsp_integration.rs:2964:5".to_string(),
    ];

    let tmp = tempfile::tempdir().expect("tempdir");
    let base_dir = tmp.path().join("workspace").join("ovim");
    let base_dir = PathBuf::from(base_dir.to_string_lossy().to_string());

    // Create picker using old new_lsp_locations method
    let picker = Picker::new_lsp_locations(base_dir, display_items);

    // The old method converts locations to indices
    let filtered = picker.filtered_results();
    assert_eq!(filtered.len(), 2);

    // Location field contains indices, not file paths
    assert_eq!(filtered[0].location, "0");
    assert_eq!(filtered[1].location, "1");
}

#[test]
fn test_picker_preserves_line_and_col() {
    let results = vec![PickerResult {
        display: "test.rs:42:15".to_string(),
        location: "/path/to/test.rs".to_string(),
        line: 41, // 0-indexed
        col: 14,  // 0-indexed
        match_positions: Vec::new(),
        content: None,
    }];

    let base_dir = PathBuf::from("/path/to");
    let picker = Picker::new_with_results(base_dir, results);

    let selected = picker
        .selected_result()
        .expect("Should have a selected result");
    assert_eq!(selected.line, 41);
    assert_eq!(selected.col, 14);
    assert_eq!(selected.location, "/path/to/test.rs");
}
