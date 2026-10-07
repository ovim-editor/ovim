//! Project-wide commands: replace in files, `:grep`, `:Problems`,
//! `:Symbols`.

use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::{Editor, QuickfixEntry, QuickfixEntryType};
use crate::project_search::{self, SearchOptions};
use std::sync::atomic::AtomicBool;

/// `:Problems [all|warnings|errors]` / `:Diagnostics`.
pub(super) fn problems(editor: &mut Editor, ex: &Ex) -> CommandResult {
    match crate::editor::problems::ProblemFilter::parse(ex.args) {
        Some(filter) => {
            editor.open_problems_picker(filter);
            ok_silent()
        }
        None => err("Problems: use all, warnings or errors"),
    }
}

/// `:Symbols [query]` / `:WorkspaceSymbols`.
pub(super) fn symbols(editor: &mut Editor, ex: &Ex) -> CommandResult {
    editor.open_workspace_symbol_picker();
    if !ex.args.is_empty() {
        if let Some(picker) = editor.picker_mut() {
            picker.set_query(ex.args.to_string());
            picker.mark_filter_pending();
        }
    }
    ok_silent()
}

/// Splits `/find/replace/flags rest` on an unescaped delimiter.
///
/// Returns `(find, replace, flags, rest)`. `\<delim>` yields a literal
/// delimiter; every other backslash sequence is kept for the regex engine.
fn parse_substitution(args: &str) -> Option<(String, String, String, String)> {
    let mut chars = args.chars();
    let delimiter = chars.next()?;
    if delimiter.is_alphanumeric() || delimiter.is_whitespace() || delimiter == '\\' {
        return None;
    }
    let mut parts: Vec<String> = vec![String::new()];
    let mut rest = String::new();
    let mut escaped = false;
    let mut delimiters = 0;
    let mut in_tail = false;
    for c in chars {
        if in_tail {
            rest.push(c);
            continue;
        }
        let current = parts.last_mut()?;
        if escaped {
            if c != delimiter {
                current.push('\\');
            }
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter && delimiters < 2 {
            delimiters += 1;
            parts.push(String::new());
        } else if delimiters >= 2 && c.is_whitespace() {
            in_tail = true;
        } else {
            current.push(c);
        }
    }
    if delimiters < 1 {
        return None;
    }
    let mut parts = parts.into_iter();
    let find = parts.next().unwrap_or_default();
    let replace = parts.next().unwrap_or_default();
    let flags = parts.next().unwrap_or_default();
    Some((find, replace, flags, rest.trim().to_string()))
}

/// Splits `/pattern/[g][j] files` on the first unescaped closing `/`
/// (`:vimgrep`). Returns `(pattern, flags, files)`; `\/` yields a literal
/// slash. `None` when there is no closing slash, or text follows it directly
/// (`/usr/bin`), so such a pattern stays literal. Other delimiters stay
/// literal too: `:grep "text"` searches for the quotes.
fn parse_delimited_pattern(args: &str) -> Option<(String, &str, &str)> {
    const DELIMITER: char = '/';
    let body = args.strip_prefix(DELIMITER)?;
    let mut pattern = String::new();
    let mut escaped = false;
    for (index, c) in body.char_indices() {
        if escaped {
            if c != DELIMITER {
                pattern.push('\\');
            }
            pattern.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == DELIMITER {
            let after = &body[index + c.len_utf8()..];
            let flags_len = after
                .find(|c| !matches!(c, 'g' | 'j'))
                .unwrap_or(after.len());
            let (flags, files) = after.split_at(flags_len);
            if !files.is_empty() && !files.starts_with(char::is_whitespace) {
                return None;
            }
            return Some((pattern, flags, files.trim()));
        } else {
            pattern.push(c);
        }
    }
    None
}

/// `:SearchReplace [/find/replace/flags [globs]]`
///
/// Flags: `c` case sensitive, `w` whole word, `r` regular expression. Without
/// arguments the last review (or an empty one) opens. With arguments the
/// search runs immediately so the review is populated when the command
/// returns.
pub(super) fn search_replace(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let args = ex.args;
    if args.is_empty() {
        editor.open_search_replace(None);
        return ok("Replace in files");
    }
    let Some((find, replace, flags, globs)) = parse_substitution(args) else {
        // Bare text: use it as the find string.
        editor.open_search_replace(Some(args.to_string()));
        return ok("Replace in files");
    };
    editor.open_search_replace(None);
    if let Some(panel) = editor.search_replace_panel_mut() {
        panel.find = crate::editor::SingleLineInput::new(find);
        panel.find.move_end();
        panel.replace = crate::editor::SingleLineInput::new(replace);
        panel.replace.move_end();
        panel.files = crate::editor::SingleLineInput::new(globs);
        panel.files.move_end();
        panel.case_sensitive = flags.contains('c');
        panel.whole_word = flags.contains('w');
        panel.regex = flags.contains('r');
        panel.focus = crate::editor::search_replace::SearchReplaceField::Results;
    }
    editor.run_search_replace_now();
    match editor.search_replace_panel() {
        Some(panel) => match &panel.error {
            Some(error) => err(format!("Search failed: {error}")),
            None => ok(format!(
                "{} matches in {} files; Alt-Enter or :ReplaceApply to replace",
                panel.total_matches(),
                panel.results.len()
            )),
        },
        None => ok("Replace in files"),
    }
}

pub(super) fn replace_apply(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    match editor.apply_search_replace() {
        Ok(report) => {
            editor.discard_search_replace();
            ok(report.summary())
        }
        Err(message) => err(message),
    }
}

pub(super) fn replace_undo(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    match editor.undo_last_search_replace() {
        Ok((undone, skipped)) => {
            let mut message = format!(
                "Undid the last replace in {undone} file{}",
                if undone == 1 { "" } else { "s" }
            );
            if skipped > 0 {
                message.push_str(&format!("; left {skipped} alone (changed or undone since)"));
            }
            ok(message)
        }
        Err(message) => err(message),
    }
}

/// `:grep pattern [-- globs]` — fill the quickfix list with matching lines
/// (regex, smart case, respects .gitignore). Pair with `:cdo` / `:cfdo`.
pub(super) fn grep(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let args = ex.args;
    if args.is_empty() {
        return err("E471: Argument required");
    }
    let (pattern, globs) = match args.split_once(" -- ") {
        Some((pattern, globs)) => (pattern.trim(), globs.trim()),
        None => (args, ""),
    };
    // Accept `:vimgrep /pat/[gj] glob` style as well.
    let (pattern, globs, jump) = match parse_delimited_pattern(pattern) {
        Some((find, flags, rest)) => (
            find,
            if globs.is_empty() {
                rest.to_string()
            } else {
                globs.to_string()
            },
            !flags.contains('j'),
        ),
        None => (pattern.to_string(), globs.to_string(), true),
    };
    let mut options = SearchOptions {
        pattern: pattern.clone(),
        regex: true,
        case_sensitive: pattern.chars().any(|c| c.is_uppercase()),
        whole_word: false,
        globs,
    };
    if options.build_regex().is_err() {
        options.regex = false;
    }
    let root = editor.picker_dirs().0;
    let overlays = editor.open_buffer_overlays();
    let outcome =
        match project_search::search_project(&root, &options, &overlays, &AtomicBool::new(false)) {
            Ok(outcome) => outcome,
            Err(message) => return err(format!("E486: {message}")),
        };
    let entries: Vec<QuickfixEntry> = outcome
        .files
        .iter()
        .flat_map(|file| {
            file.matches.iter().map(move |found| {
                QuickfixEntry::new(
                    Some(file.path.clone()),
                    found.line + 1,
                    found.start_col + 1,
                    QuickfixEntryType::Info,
                    found.line_text.trim().to_string(),
                )
            })
        })
        .collect();
    if entries.is_empty() {
        return err(format!("E486: Pattern not found: {pattern}"));
    }
    let count = entries.len();
    editor.set_quickfix_list(entries, format!(":grep {pattern}"));
    editor.open_quickfix_window();
    if jump {
        if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
            let _ = super::jump_to_quickfix_entry(editor, &entry);
        }
    }
    ok(format!(
        "{count} match{} for {pattern}{}",
        if count == 1 { "" } else { "es" },
        if outcome.truncated {
            " (truncated)"
        } else {
            ""
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::{parse_delimited_pattern, parse_substitution};

    #[test]
    fn delimited_pattern_ends_at_the_first_unescaped_slash() {
        assert_eq!(
            parse_delimited_pattern("/foo/ *.rs *.toml"),
            Some(("foo".to_string(), "", "*.rs *.toml"))
        );
        assert_eq!(
            parse_delimited_pattern(r"/a\/b/gj  src"),
            Some(("a/b".to_string(), "gj", "src"))
        );
        assert_eq!(
            parse_delimited_pattern(r"/\d+\//"),
            Some((r"\d+/".to_string(), "", ""))
        );
        // No closing slash, or text glued to it: a literal pattern.
        assert_eq!(parse_delimited_pattern("/foo"), None);
        assert_eq!(parse_delimited_pattern("/usr/bin"), None);
        assert_eq!(parse_delimited_pattern("foo/ bar"), None);
    }

    #[test]
    fn parses_find_replace_flags_and_globs() {
        let parsed = parse_substitution("/foo/bar/cw *.java, !build").unwrap();
        assert_eq!(
            parsed,
            (
                "foo".to_string(),
                "bar".to_string(),
                "cw".to_string(),
                "*.java, !build".to_string()
            )
        );
    }

    #[test]
    fn escaped_delimiters_are_literal_and_other_escapes_kept() {
        let parsed = parse_substitution(r"/a\/b\d/x\/y/r").unwrap();
        assert_eq!(parsed.0, r"a/b\d");
        assert_eq!(parsed.1, "x/y");
        assert_eq!(parsed.2, "r");
    }

    #[test]
    fn missing_trailing_delimiter_means_empty_replacement_flags() {
        let parsed = parse_substitution("/foo/bar").unwrap();
        assert_eq!(
            (parsed.0.as_str(), parsed.1.as_str(), parsed.2.as_str()),
            ("foo", "bar", "")
        );
    }

    #[test]
    fn plain_words_are_not_substitutions() {
        assert!(parse_substitution("foo").is_none());
    }
}
