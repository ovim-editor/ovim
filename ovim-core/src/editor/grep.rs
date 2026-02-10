//! In-process grep search using grep-searcher + grep-regex (ripgrep internals).
//!
//! Replaces the old subprocess-based `rg` invocation with direct library calls
//! for better performance (no process spawn overhead) and no external binary dependency.

use super::picker::PickerResult;
use grep_matcher::Matcher;
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
    preferred_dir: PathBuf,
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

        let mut roots: Vec<(PathBuf, bool)> = Vec::new();
        if preferred_dir != base_dir && preferred_dir.starts_with(&base_dir) {
            roots.push((preferred_dir.clone(), true));
        }
        roots.push((base_dir.clone(), false));

        let mut result_count = 0usize;

        for (root, is_preferred_root) in roots {
            let preferred_for_filter = preferred_dir.clone();
            let base_for_filter = base_dir.clone();
            let walker = WalkBuilder::new(&root)
                .hidden(true) // skip hidden files
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .filter_entry(move |entry| {
                    // For the base-dir pass, skip the preferred subtree entirely to avoid duplicates.
                    if !is_preferred_root && preferred_for_filter != base_for_filter {
                        if entry.path().starts_with(&preferred_for_filter) {
                            return false;
                        }
                    }
                    true
                })
                .build();

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

                        // Find match position within the line for column positioning
                        let col = matcher
                            .find(line_content.as_bytes())
                            .ok()
                            .flatten()
                            .map(|m| {
                                // Convert byte offset to char offset
                                line_content[..m.start()].chars().count()
                            })
                            .unwrap_or(0);

                        let result = PickerResult {
                            display: format!("{}:{}:{}", rel_path, line, col + 1),
                            location: abs_path.to_string(),
                            line: line.saturating_sub(1), // 0-indexed
                            col,
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

            if cancel.load(Ordering::Relaxed) || result_count >= MAX_RESULTS {
                break;
            }
        }
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_grep_prefers_preferred_dir_results_first() {
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let preferred = root.join("preferred");
        let other = root.join("other");
        fs::create_dir_all(&preferred).unwrap();
        fs::create_dir_all(&other).unwrap();

        fs::write(preferred.join("a.txt"), "needle\n").unwrap();
        fs::write(preferred.join("b.txt"), "needle\n").unwrap();
        fs::write(other.join("c.txt"), "needle\n").unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let mut rx = spawn_grep_search(
            "needle".to_string(),
            root.to_path_buf(),
            preferred.clone(),
            cancel,
        );

        let mut displays = Vec::new();
        while let Some(r) = rx.recv().await {
            displays.push(r.display);
        }

        assert!(!displays.is_empty());

        // All preferred-dir matches should appear before any non-preferred-dir match.
        let mut seen_non_preferred = false;
        for d in displays {
            let rel_path = d.split(':').next().unwrap_or("");
            let is_preferred = rel_path.starts_with("preferred/");
            if !is_preferred {
                seen_non_preferred = true;
            }
            if seen_non_preferred {
                assert!(
                    !is_preferred,
                    "saw preferred-dir match after non-preferred: {rel_path}"
                );
            }
        }
    }
}
