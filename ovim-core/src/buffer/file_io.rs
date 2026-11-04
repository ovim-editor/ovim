use super::{Buffer, Cursor};
use crate::change::ChangeManager;
use crate::fold::FoldManager;
use crate::unicode::GraphemeCol;
use crate::GitStatus;
use anyhow::{Context, Result};
use ropey::Rope;
use std::path::{Path, PathBuf};

use super::encoding::FileEncoding;
use super::line_ending::{normalize_for_buffer, LineEnding};

fn normalize_decoded_content(content: String) -> String {
    if content.contains('\r') {
        normalize_for_buffer(&content).into_owned()
    } else {
        content
    }
}

/// Decode before inspecting line endings: UTF-16 code units may contain CR/LF
/// bytes that are not line breaks, and its real CRLF pairs have intervening NULs.
fn decode_file_content(bytes: &[u8]) -> Result<(String, FileEncoding, LineEnding)> {
    let (encoding, bom_offset) = FileEncoding::detect(bytes);
    let content = encoding.decode(bytes, bom_offset)?;
    let line_ending = LineEnding::detect(content.as_bytes());
    Ok((normalize_decoded_content(content), encoding, line_ending))
}

/// Normalizes a path to an absolute, canonical form.
///
/// CRITICAL: This function MUST be deterministic and stable across the file lifecycle.
/// The path returned here becomes the URI for LSP communication. If it changes,
/// LSP loses track of the document, causing "No definition found" and other failures.
///
/// Strategy:
/// - Always make paths absolute (resolve relative paths)
/// - For existing files: canonicalize to resolve symlinks and normalize separators
/// - For non-existent files: use absolute path as-is (no canonicalization)
/// - NEVER re-normalize after initial buffer creation
pub(super) fn normalize_path(path: &Path) -> PathBuf {
    // Step 1: Make absolute
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Resolve relative to current directory
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(_) => path.to_path_buf(), // Fallback if cwd fails
        }
    };

    // Step 2: Canonicalize ONLY if file exists
    // This prevents path changes when file is created later
    match absolute.try_exists() {
        Ok(true) => {
            // File exists - safe to canonicalize
            std::fs::canonicalize(&absolute).unwrap_or(absolute)
        }
        Ok(false) | Err(_) => {
            // File doesn't exist or we can't determine - use absolute path as-is
            // This ensures new files have stable URIs before their first save
            absolute
        }
    }
}

impl Buffer {
    /// Loads a file into the buffer (async version)
    pub async fn load_file_async<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let absolute_path = normalize_path(path_ref);
        let path_str = absolute_path.to_string_lossy().to_string();

        // Get file metadata for mtime and read-only detection
        let metadata = tokio::fs::metadata(&absolute_path).await.ok();
        let file_mtime = metadata.as_ref().and_then(|m| m.modified().ok());

        // Check if file is read-only (no write permission)
        let read_only = metadata
            .as_ref()
            .map(|m| m.permissions().readonly())
            .unwrap_or(false);

        // Read as bytes first to detect encoding and line endings
        let bytes = tokio::fs::read(&absolute_path)
            .await
            .context(format!("Failed to read file: {}", path_str))?;

        let (content, encoding, line_ending) = decode_file_content(&bytes).with_context(|| {
            format!(
                "Failed to decode file '{}'. This file may be binary or corrupted",
                path_str
            )
        })?;

        let buffer = Self {
            id: super::next_buffer_id(),
            rope: Rope::from_str(&content),
            cursor: Cursor::new(0, GraphemeCol::ZERO),
            modified: false,
            file_path: Some(path_str.clone()),
            line_ending,
            encoding,
            syntax: None,
            language_catalog: crate::language_catalog::LanguageCatalog::shared_built_in(),
            syntax_loading: false,
            cached_highlights: None,
            forced_highlights: None,
            highlight_version: 0,
            highlight_projection_generation: 0,
            highlight_range_cache: std::sync::Mutex::new(
                super::highlight_index::HighlightRangeCache::default(),
            ),
            pending_rehighlight: false,
            fold_manager: FoldManager::new(),
            git_status: GitStatus::new(),
            git_blame: None,
            change_manager: ChangeManager::new(),
            file_mtime,
            read_only,
            modifiable: true,
            refused_edit: false,
            semantic_highlights: None,
            version: 0,
            code_block_cache: None,
            injection_cache: None,
            recording: None,
            edit_log: crate::edit_log::EditLog::new(),
            text_index: std::sync::Mutex::new(crate::text_index::TextIndexCache::default()),
            line_changes: crate::text_index::LineChangeLog::default(),
            display_name: None,
            indent_options: None,
            local_marks: Default::default(),
        };

        // Don't enable syntax highlighting immediately - defer for lazy loading
        // This makes file loading instant even for large files
        // Syntax highlighting will be triggered later when the buffer is displayed.
        // Git signs arrive from a background refresh once the editor adopts it.

        Ok(buffer)
    }

    /// Loads a file into the buffer (blocking wrapper for async contexts)
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let absolute_path = normalize_path(path_ref);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(Self::load_file_async(&absolute_path))
        })
    }

    /// Saves the buffer to its file path (async version)
    pub async fn save_async(&mut self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("No file path set for buffer")?;
        self.save_as_async(path.clone()).await?;
        Ok(())
    }

    /// Saves the buffer to a specific file path (async version), making it
    /// the buffer's file. See [`Self::write_contents_to`] for how.
    pub async fn save_as_async<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_ref = path.as_ref();
        let path_str_input = path_ref.to_string_lossy();

        // CRITICAL: Only normalize if this is a NEW path
        // Re-normalizing an existing path can change URIs, breaking LSP tracking
        let absolute_path = if let Some(existing_path) = &self.file_path {
            // Check if input path matches existing path (regular save)
            if path_str_input == existing_path.as_str() {
                // Regular save - use existing path without re-normalization
                PathBuf::from(existing_path)
            } else {
                // Save As with different path - normalize the new path
                normalize_path(path_ref)
            }
        } else {
            // No existing path (new buffer) - normalize it
            normalize_path(path_ref)
        };

        let path_ref = absolute_path.as_path();
        let path_str = path_ref.to_string_lossy().to_string();
        self.write_contents_to(path_ref).await?;

        // CRITICAL: Only update file_path if it changed (Save As scenario)
        // Preserves URI stability for LSP tracking
        if self.file_path.as_deref() != Some(&path_str) {
            self.file_path = Some(path_str);
        }
        if self.line_ending == LineEnding::Mixed {
            self.line_ending = LineEnding::Lf;
        }
        self.modified = false;

        // Update mtime after successful save
        self.file_mtime = tokio::fs::metadata(path_ref)
            .await
            .ok()
            .and_then(|m| m.modified().ok());

        Ok(())
    }

    /// The file bytes: line endings and encoding as loaded.
    fn file_bytes(&self) -> Result<Vec<u8>> {
        self.encode_for_file(self.rope.to_string())
    }

    /// `content` (lines ending in `\n`) with the line endings and encoding
    /// the buffer was loaded with.
    fn encode_for_file(&self, content: String) -> Result<Vec<u8>> {
        let content = match self.line_ending {
            LineEnding::Lf | LineEnding::Mixed => content,
            LineEnding::Crlf => content.replace('\n', "\r\n"),
            LineEnding::Cr => content.replace('\n', "\r"),
        };
        self.encoding.encode(&content).context(format!(
            "Failed to encode file content as {:?}",
            self.encoding
        ))
    }

    /// Overwrite `path` in place (open, truncate, write, fsync), which keeps
    /// permissions, ownership, ACLs, hard links and extended attributes —
    /// the strategy Vim and VS Code use for user-edited files.
    async fn write_contents_to(&self, path: &Path) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let bytes = self.file_bytes()?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
            .context(format!(
                "Failed to open file for writing: {}",
                path.display()
            ))?;
        file.write_all(&bytes)
            .await
            .context("Failed to write file content")?;
        file.sync_all()
            .await
            .context("Failed to sync file to disk")?;
        Ok(())
    }

    /// Write the text to another file without making it the buffer's file
    /// (vim's `:w {file}` on a named buffer). The buffer stays modified.
    pub fn write_copy<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = normalize_path(path.as_ref());
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.write_contents_to(&path))
        })
    }

    /// Write lines `first..=last` (0-based) to a file, replacing its content
    /// or appending to it, without making it the buffer's file (vim's
    /// `:{range}w {file}` and `:w >> {file}`). Every line ends in a line
    /// break. Returns the number of characters written.
    pub fn write_lines<P: AsRef<Path>>(
        &self,
        path: P,
        first: usize,
        last: usize,
        append: bool,
    ) -> Result<usize> {
        use std::io::Write;

        let path = normalize_path(path.as_ref());
        let mut content = String::new();
        for index in first..=last {
            if let Some(line) = self.line_text(index) {
                content.push_str(&line);
                content.push('\n');
            }
        }
        let chars = content.chars().count();
        let bytes = self.encode_for_file(content)?;
        let mut options = std::fs::OpenOptions::new();
        if append {
            options.append(true).create(true);
        } else {
            options.write(true).create(true).truncate(true);
        }
        let mut file = options.open(&path).context(format!(
            "Failed to open file for writing: {}",
            path.display()
        ))?;
        // The encoder includes the file's BOM. It belongs only at byte zero,
        // never between an existing document and the appended lines.
        let bytes = if append && file.metadata()?.len() > 0 {
            let bom_len = match self.encoding {
                FileEncoding::Utf8Bom => 3,
                FileEncoding::Utf16Le | FileEncoding::Utf16Be => 2,
                _ => 0,
            };
            &bytes[bom_len..]
        } else {
            &bytes
        };
        file.write_all(bytes)
            .context("Failed to write file content")?;
        file.sync_all().context("Failed to sync file to disk")?;
        Ok(chars)
    }

    /// Saves the buffer to its file path (blocking wrapper)
    pub fn save(&mut self) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.save_async())
        })
    }

    /// Saves the buffer to a specific file path (blocking wrapper)
    pub fn save_as<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.save_as_async(path))
        })
        // Git status refresh is handled asynchronously by the event loop
        // after SaveCompleted — not here, to avoid blocking the UI thread.
    }

    // ========== Swap/Backup File Support ==========

    /// Returns the path to the swap file for this buffer
    /// Swap files are named .{filename}.swp in the same directory
    pub fn swap_file_path(&self) -> Option<PathBuf> {
        let file_path = self.file_path.as_ref()?;
        let path = Path::new(file_path);
        let parent = path.parent().unwrap_or(Path::new("."));
        let file_name = path.file_name()?.to_str()?;
        Some(parent.join(format!(".{}.swp", file_name)))
    }

    /// Returns the path to the backup file for this buffer
    /// Backup files are named {filename}~ in the same directory
    pub fn backup_file_path(&self) -> Option<PathBuf> {
        let file_path = self.file_path.as_ref()?;
        Some(PathBuf::from(format!("{}~", file_path)))
    }

    /// Checks if a swap file exists for this buffer
    pub fn has_swap_file(&self) -> bool {
        self.swap_file_path().map(|p| p.exists()).unwrap_or(false)
    }

    /// Writes the current buffer content to the swap file
    pub fn write_swap_file(&self) -> Result<()> {
        let swap_path = self
            .swap_file_path()
            .context("Cannot write swap file: no file path set")?;

        let content = self.rope.to_string();
        std::fs::write(&swap_path, &content).context(format!(
            "Failed to write swap file: {}",
            swap_path.display()
        ))?;

        Ok(())
    }

    /// Deletes the swap file if it exists
    pub fn delete_swap_file(&self) -> Result<()> {
        if let Some(swap_path) = self.swap_file_path() {
            if swap_path.exists() {
                std::fs::remove_file(&swap_path).context(format!(
                    "Failed to delete swap file: {}",
                    swap_path.display()
                ))?;
            }
        }
        Ok(())
    }

    /// Creates a backup of the original file before saving
    pub fn create_backup_file(&self) -> Result<()> {
        let file_path = self
            .file_path
            .as_ref()
            .context("Cannot create backup: no file path set")?;
        let backup_path = self
            .backup_file_path()
            .context("Cannot create backup path")?;

        let source = Path::new(file_path);
        if source.exists() {
            std::fs::copy(source, &backup_path).context(format!(
                "Failed to create backup: {}",
                backup_path.display()
            ))?;
        }

        Ok(())
    }

    /// Reads content from the swap file if it exists
    pub fn read_swap_file(&self) -> Result<Option<String>> {
        let swap_path = match self.swap_file_path() {
            Some(p) if p.exists() => p,
            _ => return Ok(None),
        };

        let content = std::fs::read_to_string(&swap_path)
            .context(format!("Failed to read swap file: {}", swap_path.display()))?;

        Ok(Some(content))
    }

    /// Recovers buffer content from the swap file
    pub fn recover_from_swap_file(&mut self) -> Result<bool> {
        if let Some(content) = self.read_swap_file()? {
            // Ensure content ends with newline
            let content = if content.ends_with('\n') {
                content
            } else {
                format!("{}\n", content)
            };

            self.rope = Rope::from_str(&content);
            self.modified = true; // Mark as modified so user knows to save
            self.reset_derived_state(&content);

            // Delete the swap file after successful recovery
            self.delete_swap_file()?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ========== External File Change Detection ==========

    /// Returns the last known file modification time
    pub fn file_mtime(&self) -> Option<std::time::SystemTime> {
        self.file_mtime
    }

    /// Updates the stored file modification time
    pub fn set_file_mtime(&mut self, mtime: Option<std::time::SystemTime>) {
        self.file_mtime = mtime;
    }

    /// Checks if the file has been modified externally since we loaded/saved it
    /// Returns Ok(true) if file changed, Ok(false) if unchanged, Err if can't determine
    pub fn check_external_modification(&self) -> Result<bool> {
        let file_path = self.file_path.as_ref().context("No file path set")?;

        let path = Path::new(file_path);
        if !path.exists() {
            // Deleted externally - but only a buffer that was loaded from or
            // saved to that path can lose it. Scratch buffers (`[LspInfo]`) and
            // brand-new files never had an mtime and were never on disk
            // (OV-00457: the poll reported "Failed to read file: .../[LspInfo]").
            return Ok(self.file_mtime.is_some());
        }

        let current_mtime = std::fs::metadata(path)
            .context("Failed to get file metadata")?
            .modified()
            .context("Failed to get file modification time")?;

        match self.file_mtime {
            Some(stored_mtime) => Ok(current_mtime != stored_mtime),
            None => Ok(false), // No stored mtime means this is a new buffer
        }
    }

    /// Reloads the buffer from disk if it was modified externally
    /// Returns true if reload happened, false if no change
    pub async fn reload_if_changed(&mut self) -> Result<bool> {
        if !self.check_external_modification()? {
            return Ok(false);
        }

        let file_path = self.file_path.clone().context("No file path set")?;

        // Read file content
        let bytes = tokio::fs::read(&file_path)
            .await
            .context(format!("Failed to read file: {}", file_path))?;

        let file_mtime = tokio::fs::metadata(&file_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok());
        let (content, encoding, line_ending) = decode_file_content(&bytes)
            .with_context(|| format!("Failed to decode file '{}'", file_path))?;

        // Commit metadata only after the new text is valid. A failed reload
        // must preserve the old encoding and remain detectable on the next poll.
        self.file_mtime = file_mtime;
        self.encoding = encoding;
        self.line_ending = line_ending;

        // Update rope and reset all derived state
        self.rope = Rope::from_str(&content);
        self.modified = false;
        self.reset_derived_state(&content);
        self.validate_cursor_position();

        Ok(true)
    }

    /// Blocking version of reload_if_changed
    pub fn reload_if_changed_sync(&mut self) -> Result<bool> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.reload_if_changed())
        })
    }

    /// Unconditionally reloads the buffer from disk, discarding any changes.
    /// Used by :e and :e! commands to reload.
    /// Preserves cursor position (clamped to new file bounds).
    pub fn reload_from_disk(&mut self) -> Result<()> {
        let file_path = self.file_path.clone().context("No file path set")?;

        // Save cursor position before reload
        let old_line = self.cursor.line();
        let old_col = self.cursor.col();

        // Read file content synchronously
        let bytes =
            std::fs::read(&file_path).context(format!("Failed to read file: {}", file_path))?;

        let file_mtime = std::fs::metadata(&file_path)
            .ok()
            .and_then(|m| m.modified().ok());
        let (content, encoding, line_ending) = decode_file_content(&bytes)
            .with_context(|| format!("Failed to decode file '{}'", file_path))?;

        self.file_mtime = file_mtime;
        self.encoding = encoding;
        self.line_ending = line_ending;

        // Update rope and reset all derived state
        self.rope = Rope::from_str(&content);
        self.modified = false;
        self.reset_derived_state(&content);

        // Restore cursor, clamped to new bounds
        let max_line = self.line_count().saturating_sub(1);
        self.cursor.set_position(old_line.min(max_line), old_col);
        self.validate_cursor_position();

        Ok(())
    }
}

#[cfg(test)]
mod external_change_tests {
    use super::*;

    /// OV-00457: a scratch buffer (`[LspInfo]`) has a path that never exists
    /// on disk; that is not an "external deletion".
    #[test]
    fn scratch_buffer_is_not_reported_as_externally_deleted() {
        let mut buffer = Buffer::new_from_str("info");
        buffer.set_file_path("/nonexistent-dir-ovim/[LspInfo]".to_string());
        assert!(!buffer.check_external_modification().unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_loaded_file_that_vanishes_is_reported_as_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "x\n").unwrap();
        let buffer = Buffer::load_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(buffer.check_external_modification().unwrap());
    }
}

#[cfg(test)]
mod encoding_roundtrip_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn utf16_load_and_save_preserve_real_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("encoded.txt");
        for encoding in [FileEncoding::Utf16Le, FileEncoding::Utf16Be] {
            for (text, ending) in [
                ("one\r\ntwo\r\n", LineEnding::Crlf),
                ("one\rtwo\r", LineEnding::Cr),
                ("one\ntwo\n", LineEnding::Lf),
                // These code units contain CR/LF bytes but are not line breaks.
                ("Ċč", LineEnding::Lf),
            ] {
                let bytes = encoding.encode(text).unwrap();
                std::fs::write(&path, &bytes).unwrap();
                let mut buffer = Buffer::load_file_async(&path).await.unwrap();
                assert_eq!(buffer.encoding(), encoding);
                assert_eq!(buffer.line_ending(), ending);
                buffer.save_async().await.unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn both_reload_paths_decode_line_endings_before_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reload.txt");
        for encoding in [FileEncoding::Utf16Le, FileEncoding::Utf16Be] {
            for asynchronous in [false, true] {
                std::fs::write(&path, "original\n").unwrap();
                let mut buffer = Buffer::load_file_async(&path).await.unwrap();
                let bytes = encoding.encode("café\r\n🦀\r\n").unwrap();
                std::fs::write(&path, &bytes).unwrap();
                buffer.set_file_mtime(Some(std::time::UNIX_EPOCH));
                if asynchronous {
                    assert!(buffer.reload_if_changed().await.unwrap());
                } else {
                    buffer.reload_from_disk().unwrap();
                }
                assert_eq!(buffer.rope().to_string(), "café\n🦀\n");
                assert_eq!(buffer.line_ending(), LineEnding::Crlf);
                assert_eq!(buffer.encoding(), encoding);
                buffer.save_async().await.unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn failed_reloads_preserve_the_entire_old_buffer_and_remain_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.txt");
        for asynchronous in [false, true] {
            std::fs::write(&path, "original\r\n").unwrap();
            let mut buffer = Buffer::load_file_async(&path).await.unwrap();
            buffer.insert_text_at(0, crate::unicode::CharCol::ZERO, "unsaved ");
            buffer.set_file_mtime(Some(std::time::UNIX_EPOCH));
            let version = buffer.version();
            // UTF-16LE BOM followed by an unpaired high surrogate.
            std::fs::write(&path, [0xff, 0xfe, 0x00, 0xd8]).unwrap();
            if asynchronous {
                assert!(buffer.reload_if_changed().await.is_err());
            } else {
                assert!(buffer.reload_from_disk().is_err());
            }
            assert_eq!(buffer.rope().to_string(), "unsaved original\n");
            assert_eq!(buffer.encoding(), FileEncoding::Utf8);
            assert_eq!(buffer.line_ending(), LineEnding::Crlf);
            assert_eq!(buffer.file_mtime(), Some(std::time::UNIX_EPOCH));
            assert_eq!(buffer.version(), version);
            assert!(buffer.is_modified());
            assert!(buffer.check_external_modification().unwrap());
            std::fs::write(&path, "fixed\n").unwrap();
            assert!(buffer.reload_if_changed().await.unwrap());
            assert_eq!(buffer.rope().to_string(), "fixed\n");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn appending_encoded_lines_writes_a_bom_only_at_the_start_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        for encoding in [
            FileEncoding::Utf8Bom,
            FileEncoding::Utf16Le,
            FileEncoding::Utf16Be,
        ] {
            std::fs::write(&source, encoding.encode("next\r\n").unwrap()).unwrap();
            let buffer = Buffer::load_file_async(&source).await.unwrap();
            for existing in ["", "first\r\n"] {
                std::fs::write(&target, encoding.encode(existing).unwrap()).unwrap();
                buffer.write_lines(&target, 0, 0, true).unwrap();
                buffer.write_lines(&target, 0, 0, true).unwrap();
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    encoding
                        .encode(&format!("{existing}next\r\nnext\r\n"))
                        .unwrap()
                );
            }
            // An empty destination also needs its initial BOM.
            std::fs::write(&target, []).unwrap();
            buffer.write_lines(&target, 0, 0, true).unwrap();
            assert_eq!(
                std::fs::read(&target).unwrap(),
                encoding.encode("next\r\n").unwrap()
            );
        }
    }
}
