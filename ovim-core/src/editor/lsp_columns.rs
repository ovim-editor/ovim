//! Columns of LSP positions in files other than the current buffer.
//!
//! An LSP position counts UTF-16 units into its line. The pickers and jump
//! targets want grapheme columns, which depend on the text of the line in the
//! file the position is in - the open buffer if there is one, the file on disk
//! otherwise - never on whatever the current buffer holds at that line number.

use super::Editor;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Files larger than this are not read just to place a column.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Converts LSP positions to grapheme columns, reading each file once.
pub(in crate::editor) struct ColumnResolver<'a> {
    editor: &'a Editor,
    disk: HashMap<PathBuf, Option<Vec<String>>>,
}

impl<'a> ColumnResolver<'a> {
    pub(in crate::editor) fn new(editor: &'a Editor) -> Self {
        Self {
            editor,
            disk: HashMap::new(),
        }
    }

    /// The grapheme column of `utf16_col` on `line` (0-based) of `path`. A
    /// line that cannot be read keeps the number as it is.
    pub(in crate::editor) fn grapheme_col(
        &mut self,
        path: &Path,
        line: usize,
        utf16_col: u32,
    ) -> usize {
        let text = match path
            .to_str()
            .and_then(|p| self.editor.find_buffer_by_path(p))
        {
            Some(index) => self.editor.buffers[index]
                .line_text(line)
                .map(|text| text.to_string()),
            None => self
                .disk
                .entry(path.to_path_buf())
                .or_insert_with(|| read_lines(path))
                .as_ref()
                .and_then(|lines| lines.get(line).cloned()),
        };
        match text {
            Some(text) => {
                let char_col = crate::lsp::utf16_to_char_col(&text, utf16_col);
                crate::unicode::char_to_grapheme_col(&text, crate::unicode::CharCol(char_col)).0
            }
            None => utf16_col as usize,
        }
    }
}

fn read_lines(path: &Path) -> Option<Vec<String>> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let text = String::from_utf8_lossy(&std::fs::read(path).ok()?).into_owned();
    Some(text.lines().map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The position is read against the line of ITS file: the current buffer's
    /// line with the same number is irrelevant.
    #[test]
    fn columns_come_from_the_target_files_line() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other.rs");
        // 'é' is one UTF-16 unit, '😀' two; a combining accent joins its base.
        std::fs::write(&other, "short\n    let ñ = 😀x;\n").unwrap();
        let editor = Editor::with_content("\n\n");
        let mut resolver = ColumnResolver::new(&editor);

        // `x` follows `    let ñ = 😀` = 4+4+1+1+1+1+1+2 = 15 UTF-16 units.
        assert_eq!(resolver.grapheme_col(&other, 1, 15), 14);
        assert_eq!(resolver.grapheme_col(&other, 1, 5), 5);
        // A line that is not there keeps the raw number.
        assert_eq!(resolver.grapheme_col(&other, 9, 5), 5);
    }

    /// A reference at 6:5 of another file used to jump to 6:1 when the current
    /// buffer's line 6 was shorter: the column was read against that line.
    #[test]
    fn reference_rows_place_the_column_in_the_references_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other.rs");
        std::fs::write(&other, "1\n2\n3\n4\n5\n    let value = 1;\n").unwrap();
        let editor = Editor::with_content("a\nb\nc\nd\ne\n\n");
        let location = lsp_types::Location::new(
            crate::lsp::uri_from_file_path(&other).unwrap(),
            lsp_types::Range::new(
                lsp_types::Position::new(5, 4),
                lsp_types::Position::new(5, 7),
            ),
        );

        let rows = editor.locations_to_picker_items(&[location]);

        assert_eq!((rows[0].line, rows[0].col), (5, 4));
        assert!(rows[0].display.ends_with(":6:5"), "{}", rows[0].display);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn open_buffers_win_over_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("open.rs");
        std::fs::write(&file, "on disk\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, crate::unicode::CharCol(0), "😀");
        let mut resolver = ColumnResolver::new(&editor);

        // The unsaved emoji (2 UTF-16 units) is one grapheme.
        assert_eq!(resolver.grapheme_col(&file, 0, 4), 3);
    }
}
