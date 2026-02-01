//! In-process grep search using grep-searcher + grep-regex (ripgrep internals).
//!
//! Replaces the old subprocess-based `rg` invocation with direct library calls
//! for better performance (no process spawn overhead) and no external binary dependency.

use super::picker::PickerResult;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use ignore::WalkBuilder;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Maximum number of results to prevent OOM on broad queries.
const MAX_RESULTS: usize = 5000;

/// Spawns an in-process grep search in a blocking thread.
///
/// Returns a channel receiver that streams `PickerResult`s as they're found.
/// The search respects `.gitignore`, skips `.git` directories, and uses
/// smart-case matching (case-insensitive when query is all lowercase).
///
/// Pass a `cancel` flag to abort the search early (e.g., when query changes).
pub fn spawn_grep_search(
    query: String,
    base_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) -> mpsc::Receiver<PickerResult> {
    let (tx, rx) = mpsc::channel(512);

    tokio::task::spawn_blocking(move || {
        if query.is_empty() {
            return;
        }

        // Smart-case: case-insensitive when query is all lowercase
        let case_insensitive = query.chars().all(|c| !c.is_uppercase());

        // Build regex matcher, falling back to literal if regex is invalid
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(case_insensitive)
            .build(&query)
            .or_else(|_| {
                // Partial typing like `foo(` produces invalid regex — escape and retry
                RegexMatcherBuilder::new()
                    .case_insensitive(case_insensitive)
                    .build(&regex::escape(&query))
            });

        let matcher = match matcher {
            Ok(m) => m,
            Err(_) => return,
        };

        let walker = WalkBuilder::new(&base_dir)
            .hidden(true) // skip hidden files
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build();

        let mut result_count = 0usize;

        for entry in walker {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip directories
            if entry.file_type().map_or(true, |ft| !ft.is_file()) {
                continue;
            }

            let file_path = entry.path().to_path_buf();

            let mut searcher = Searcher::new();
            searcher.set_binary_detection(grep_searcher::BinaryDetection::quit(0));

            let tx_ref = &tx;
            let cancel_ref = &cancel;
            let base_dir_ref = &base_dir;
            let result_count_ref = &mut result_count;

            let _ = searcher.search_path(
                &matcher,
                &file_path,
                UTF8(|line_num, line_content| {
                    if cancel_ref.load(Ordering::Relaxed) {
                        return Ok(false); // stop searching this file
                    }

                    if *result_count_ref >= MAX_RESULTS {
                        return Ok(false);
                    }

                    let rel_path = file_path
                        .strip_prefix(base_dir_ref)
                        .unwrap_or(&file_path)
                        .to_string_lossy();
                    let abs_path = file_path.to_string_lossy();

                    let content = line_content.trim_end().to_string();
                    let line = line_num as usize;

                    let result = PickerResult {
                        display: format!("{}:{}", rel_path, line),
                        location: abs_path.to_string(),
                        line: line.saturating_sub(1), // 0-indexed
                        col: 0,
                        match_positions: Vec::new(),
                        content: Some(content),
                    };

                    *result_count_ref += 1;

                    // If the receiver is dropped, stop searching
                    if tx_ref.blocking_send(result).is_err() {
                        return Ok(false);
                    }

                    Ok(true)
                }),
            );

            if result_count >= MAX_RESULTS {
                break;
            }
        }
    });

    rx
}
