//! Ex commands: one parser ([`parse`]), one table ([`table`]) and one
//! dispatcher ([`execute_command`]) behind every entry point — the `:`
//! prompt, keymaps, Lua, the headless API and the GUI.

mod contexts;
mod debug;
mod edit;
mod files;
mod git;
mod launch;
mod lsp;
mod marked_lines;
mod options;
mod parse;
mod pattern;
mod project;
mod quickfix;
mod range;
mod session;
mod set;
mod shell;
mod table;
mod windows;

pub(crate) use parse::{parse, ParsedCmd};
pub use quickfix::jump_to_quickfix_entry;
use range::LineRange;
pub use table::command_names;

use crate::command_result::{err, ok_silent, CommandResult};
use crate::editor::{Editor, Nesting};
use contexts::BufferKind;
use table::{ExCommand, RangePolicy};

/// The file argument being typed on `line`, for path completion: the rest
/// of the line after the name of a command whose argument is a file
/// (`:e`, `:w`, `:sp`, `:r`, `:so`, `:cd`, ...), once a blank follows the
/// name. `None` for other commands and for `:r !cmd` / `:w !cmd`.
pub fn file_argument(line: &str) -> Option<&str> {
    let parsed = parse(line).ok()?;
    if parsed.command.args != table::ArgKind::File || parsed.args.starts_with('!') {
        return None;
    }
    let start = parsed.args.as_ptr() as usize - line.as_ptr() as usize;
    let before = &line[..start];
    before
        .ends_with(char::is_whitespace)
        .then(|| &line[start..])
}

/// One command as its handler sees it.
pub(crate) struct Ex<'a> {
    pub command: &'static ExCommand,
    pub bang: bool,
    pub args: &'a str,
    /// The typed range, or the command's default; `None` when it takes none.
    pub range: Option<LineRange>,
    /// Whether a range was typed (`:r` vs `:0r`, `:!cmd` vs `:.!filter`).
    pub explicit_range: bool,
}

/// Execute an ex command line (`:w`, `:2,4d`, `:%s/a/b/ | update`, ...).
pub fn execute_command(editor: &mut Editor, command: &str) -> CommandResult {
    editor.with_execution_scope(|editor| run_line(editor, command))
}

/// Run the commands of `line` in order. An error stops the rest of the line
/// (vim); otherwise the last message is the result.
pub(crate) fn run_line(editor: &mut Editor, line: &str) -> CommandResult {
    editor
        .nested(Nesting::ExLine, |editor| run_commands(editor, line))
        .unwrap_or_else(|| err("E169: Command too recursive"))
}

fn run_commands(editor: &mut Editor, line: &str) -> CommandResult {
    let mut rest = line;
    let mut result = ok_silent();
    loop {
        let text = rest.trim();
        if text.is_empty() {
            return result;
        }
        let parsed = match parse(text) {
            Ok(parsed) => parsed,
            Err(error) => return err(error.message(text)),
        };
        let outcome = run_parsed(editor, &parsed);
        match &outcome {
            CommandResult::Error(_) => return outcome,
            CommandResult::Success(success) if success.message.is_some() => result = outcome,
            CommandResult::Success(_) => {}
        }
        match parsed.next {
            Some(next) => rest = next,
            None => return result,
        }
    }
}

/// Gate on the buffer kind, resolve the range and run the handler.
fn run_parsed(editor: &mut Editor, parsed: &ParsedCmd) -> CommandResult {
    let command = parsed.command;
    let kind = BufferKind::of(editor);
    if let Some(lifecycle) = command.lifecycle {
        if let Some(result) = contexts::finish_special(editor, kind, lifecycle, parsed.bang) {
            return result;
        }
    }
    if !command.contexts.allows(kind) {
        return err(kind.refusal());
    }
    let last = range::last_line(editor);
    let range = match (&parsed.range, command.range) {
        (Some(_), RangePolicy::None) => return err("E481: No range allowed"),
        (Some(spec), policy) => match range::eval_range(editor, spec) {
            Ok(range) if policy == RangePolicy::Goto => Some(range),
            Ok(range) if range.end > last => return err("E16: Invalid range"),
            // Line 0 means "above the first line" only where that makes sense.
            Ok(range) if policy == RangePolicy::LineOrZero => Some(range),
            Ok(range) => Some(LineRange {
                start: range.start.max(1),
                end: range.end.max(1),
            }),
            Err(message) => return err(message),
        },
        (None, RangePolicy::None | RangePolicy::Goto) => None,
        (None, RangePolicy::Line | RangePolicy::LineOrZero) => {
            Some(LineRange::line(range::cursor_line(editor)))
        }
        (None, RangePolicy::Whole) => Some(LineRange {
            start: 1,
            end: last,
        }),
    };
    let expanded;
    let mut args = parsed.args;
    // `:r !cmd` / `:w !cmd` expand `%` and `#` themselves, with shell quoting.
    if command.args == table::ArgKind::File && !args.is_empty() && !args.starts_with('!') {
        expanded = match expand_file_argument(editor, args) {
            Ok(expanded) => expanded,
            Err(message) => return err(message),
        };
        args = &expanded;
    }
    let ex = Ex {
        command,
        bang: parsed.bang,
        args,
        range,
        explicit_range: parsed.range.is_some(),
    };
    if command.args == table::ArgKind::None && !ex.args.is_empty() {
        return err(format!("E488: Trailing characters: {}", ex.args));
    }
    (command.handler)(editor, &ex)
}

/// Expand `~`, `%`, `#` and their modifiers in the file argument of `:e`,
/// `:w`, `:sp`, `:r`, `:cd`, ... once, before the handler sees it.
fn expand_file_argument(editor: &Editor, args: &str) -> Result<String, String> {
    let home = files::expand_tilde(args)
        .map_err(|error| format!("Failed to expand path '{args}': {error}"))?;
    let current_file = editor.buffer().file_path().unwrap_or("");
    let alternate_file = editor.registers().get(Some('#'));
    crate::editor::shell_expansion::expand_file_argument(
        &home.to_string_lossy(),
        current_file,
        &alternate_file,
    )
}

/// Run a command line the user typed (`:` prompt, keymaps, Lua, `ZZ`) and
/// show the outcome: errors and one-line messages on the status line,
/// longer output in the hover popup.
pub fn execute_and_show(editor: &mut Editor, line: &str) {
    match execute_command(editor, line) {
        CommandResult::Success(success) => {
            if let Some(message) = success.message {
                let message = message.into_owned();
                if message.contains('\n') {
                    editor.set_hover_info(message);
                } else {
                    editor.set_status_message(message);
                }
            }
        }
        CommandResult::Error(error) => editor.set_status_message(error.error),
    }
}

/// Execute a command line for callers without a terminal (the headless API
/// and the GUI): a queued `:!cmd` runs with captured output and a queued
/// `:terminal` is refused.
pub fn execute_command_api(editor: &mut Editor, line: &str) -> CommandResult {
    let result = execute_command(editor, line);
    if let Some(shell) = editor.take_pending_shell_command() {
        if let CommandResult::Error(_) = result {
            return result;
        }
        return shell::run_captured(editor, &shell.command);
    }
    if editor.take_pending_terminal_session().is_some() {
        return err("Interactive terminal sessions require the TUI frontend");
    }
    result
}

#[cfg(test)]
mod characterization_tests;

#[cfg(test)]
mod tests {
    use super::execute_command;
    use crate::command_result::CommandResult;
    use crate::editor::{Editor, EditorServices, Nesting};
    use crate::unicode::CharCol;

    #[test]
    fn edit_directory_opens_the_explorer_instead_of_a_text_buffer() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("main.rs"), "fn main() {}").unwrap();
        let mut editor = Editor::new();

        let result = execute_command(&mut editor, &format!("edit {}", directory.path().display()));

        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(editor.mode(), crate::mode::Mode::FileTree);
        assert_eq!(
            editor.file_tree().root_path(),
            Some(directory.path().canonicalize().unwrap().as_path())
        );
        assert_eq!(editor.buffer().file_path(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn write_refuses_to_overwrite_external_changes_without_bang() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("conflict.txt");
        std::fs::write(&path, "original\n").unwrap();

        let mut editor = Editor::new();
        editor.load_file_async(&path).await.unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, CharCol::ZERO, "local ");

        // Ensure even coarse filesystems observe a distinct modification time.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "external\n").unwrap();

        let result = execute_command(&mut editor, "w");
        assert!(
            matches!(result, CommandResult::Error(ref error) if error.error.contains("E211")),
            "unexpected result: {result:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external\n");

        let result = execute_command(&mut editor, "w!");
        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "local original\n");
    }

    #[test]
    fn command_lines_nested_past_the_limit_fail_with_e169() {
        // vim: "E169: Command too recursive" once command lines nest too deep.
        fn deepest(editor: &mut Editor) -> CommandResult {
            match editor.nested(Nesting::ExLine, deepest) {
                Some(result) => result,
                None => execute_command(editor, "set number"),
            }
        }
        let mut editor = Editor::new();

        let result = deepest(&mut editor);

        assert!(
            matches!(result, CommandResult::Error(ref e) if e.error.starts_with("E169")),
            "unexpected result: {result:?}"
        );
        // Unwinding restores the depth, so ordinary commands work again.
        assert!(matches!(
            execute_command(&mut editor, "set number"),
            CommandResult::Success(_)
        ));
    }

    #[test]
    fn format_command_requests_lsp_document_formatting() {
        let mut editor = Editor::new();

        let result = execute_command(&mut editor, "format");

        assert!(matches!(result, CommandResult::Success(_)));
        assert!(editor.pending_intents().format_document);
    }

    #[tokio::test]
    async fn browser_command_opens_a_new_frontend_session() {
        let (browser, mut host) = crate::browser::browser_channel();
        let mut editor =
            Editor::new().with_services(EditorServices::default().with_browser(browser));

        let result = execute_command(&mut editor, "browser");
        assert!(matches!(result, CommandResult::Success(_)));

        let host_task = tokio::spawn(async move {
            let request = host.recv().await.expect("browser start request");
            assert_eq!(
                request.command(),
                &crate::browser::BrowserCommand::Start { url: None }
            );
            request.respond(Ok(crate::browser::BrowserResponse::Session(
                crate::browser::BrowserSession {
                    session_id: "browser-1".into(),
                    url: String::new(),
                    title: String::new(),
                    visible: false,
                    loading: false,
                    document_id: 0,
                },
            )));
        });
        editor.dispatch_pending_intents().await;
        host_task.await.unwrap();
    }

    #[tokio::test]
    async fn browser_command_surfaces_a_host_rejection() {
        let (browser, mut host) = crate::browser::browser_channel();
        let mut editor =
            Editor::new().with_services(EditorServices::default().with_browser(browser));
        assert!(matches!(
            execute_command(&mut editor, "browser"),
            CommandResult::Success(_)
        ));

        let host_task = tokio::spawn(async move {
            let request = host.recv().await.expect("browser start request");
            request.respond(Err(crate::browser::BrowserError::new(
                crate::browser::BrowserErrorKind::InvalidRequest,
                "Browser tab limit reached",
            )));
        });
        editor.dispatch_pending_intents().await;
        host_task.await.unwrap();

        assert_eq!(
            editor.status_message(),
            "Could not open embedded browser: Browser tab limit reached"
        );
    }

    #[test]
    fn browser_command_reports_an_unavailable_frontend() {
        let mut editor = Editor::new();

        let result = execute_command(&mut editor, "browser");

        assert!(matches!(
            result,
            CommandResult::Error(ref error)
                if error.error == "Could not open embedded browser: The embedded browser is unavailable in this frontend"
        ));
    }

    /// OV-00331: `:qa` used to check only the CURRENT buffer, silently
    /// discarding hidden modified buffers (e.g. files edited by a multi-file
    /// rename). Vim semantics verified in `nvim --clean` (2026-08-14): with
    /// buffer 1 modified and an unmodified buffer 2 current, `:qa` fails
    /// with "E37: No write since last change"; `:qa!` quits.
    #[tokio::test(flavor = "multi_thread")]
    async fn qa_refuses_when_a_non_current_buffer_is_modified() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        std::fs::write(&first, "aaa\n").unwrap();
        std::fs::write(&second, "bbb\n").unwrap();

        let mut editor = Editor::new();
        editor.load_file_async(&first).await.unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, CharCol::ZERO, "changed ");
        editor.load_file_async(&second).await.unwrap();
        assert!(
            !editor.is_modified(),
            "current buffer must be clean so only the hidden buffer blocks"
        );

        let result = execute_command(&mut editor, "qa");
        assert!(
            matches!(result, CommandResult::Error(ref e) if e.error.contains("No write since last change")),
            "unexpected result: {result:?}"
        );
        assert!(!editor.should_quit());

        let result = execute_command(&mut editor, "qa!");
        assert!(matches!(result, CommandResult::Success(_)));
        assert!(editor.should_quit());
    }

    /// OV-00331: `:q` on the last window exits the editor, so hidden
    /// modified buffers must block it exactly like `:qa`. Verified in
    /// `nvim --clean` (2026-08-14): fails with "E37: No write since last
    /// change".
    #[tokio::test(flavor = "multi_thread")]
    async fn q_on_last_window_refuses_when_a_hidden_buffer_is_modified() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        std::fs::write(&first, "aaa\n").unwrap();
        std::fs::write(&second, "bbb\n").unwrap();

        let mut editor = Editor::new();
        editor.load_file_async(&first).await.unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, CharCol::ZERO, "changed ");
        editor.load_file_async(&second).await.unwrap();

        let result = execute_command(&mut editor, "q");
        assert!(
            matches!(result, CommandResult::Error(ref e) if e.error.contains("No write since last change")),
            "unexpected result: {result:?}"
        );
        assert!(!editor.should_quit());
    }

    /// OV-00331: `:wa` had no handler at all (it only appeared as a
    /// completion candidate). Vim semantics verified in `nvim --clean`
    /// (2026-08-14): `:wa` writes every modified named buffer.
    #[tokio::test(flavor = "multi_thread")]
    async fn wa_writes_all_named_modified_buffers() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        std::fs::write(&first, "aaa\n").unwrap();
        std::fs::write(&second, "bbb\n").unwrap();

        let mut editor = Editor::new();
        editor.load_file_async(&first).await.unwrap();
        editor.buffer_mut().insert_text_at(0, CharCol::ZERO, "one ");
        editor.load_file_async(&second).await.unwrap();
        editor.buffer_mut().insert_text_at(0, CharCol::ZERO, "two ");

        let result = execute_command(&mut editor, "wa");
        assert!(
            matches!(result, CommandResult::Success(_)),
            "unexpected result: {result:?}"
        );
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "one aaa\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "two bbb\n");
        assert!(!editor.any_buffer_modified());

        // With everything written, :qa proceeds.
        let result = execute_command(&mut editor, "qa");
        assert!(matches!(result, CommandResult::Success(_)));
        assert!(editor.should_quit());
    }

    /// External review on OV-00331: a REAL on-disk file whose basename looks
    /// like a scratch buffer (`[draft]`) must still be written by `:wa` and
    /// must still block `:qa` — the bracket-name heuristic alone excluded it
    /// from both, silently losing the changes on quit. Scratch buffers never
    /// exist on disk, which is the disambiguator.
    #[tokio::test(flavor = "multi_thread")]
    async fn real_file_with_bracket_name_is_written_and_blocks_quit() {
        let temp = tempfile::tempdir().unwrap();
        let bracketed = temp.path().join("[draft]");
        std::fs::write(&bracketed, "aaa\n").unwrap();

        let mut editor = Editor::new();
        editor.load_file_async(&bracketed).await.unwrap();
        editor.buffer_mut().insert_text_at(0, CharCol::ZERO, "one ");

        assert!(
            editor.any_buffer_modified(),
            "a modified real [draft] file must count as modified"
        );
        let result = execute_command(&mut editor, "qa");
        assert!(
            matches!(result, CommandResult::Error(_)),
            ":qa must refuse while the real [draft] file is modified: {result:?}"
        );

        let result = execute_command(&mut editor, "wa");
        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(std::fs::read_to_string(&bracketed).unwrap(), "one aaa\n");
        assert!(!editor.any_buffer_modified());
    }

    /// External review round 2: a real bracket-named file must KEEP its quit
    /// protection when an external process deletes it after load — the
    /// buffer was loaded from disk (file_mtime recorded), so a vanished path
    /// must not reclassify it as a scratch buffer.
    #[tokio::test(flavor = "multi_thread")]
    async fn externally_deleted_bracket_file_still_blocks_quit() {
        let temp = tempfile::tempdir().unwrap();
        let bracketed = temp.path().join("[draft]");
        std::fs::write(&bracketed, "aaa\n").unwrap();

        let mut editor = Editor::new();
        editor.load_file_async(&bracketed).await.unwrap();
        editor.buffer_mut().insert_text_at(0, CharCol::ZERO, "one ");

        std::fs::remove_file(&bracketed).unwrap();

        assert!(
            editor.any_buffer_modified(),
            "externally deleted [draft] must still count as modified"
        );
        let result = execute_command(&mut editor, "qa");
        assert!(
            matches!(result, CommandResult::Error(_)),
            ":qa must refuse — the only copy of the edits is this buffer: {result:?}"
        );
    }

    /// Vim semantics verified in `nvim --clean` (2026-08-14): `:wa` with a
    /// modified unnamed buffer reports "E141: No file name for buffer N"
    /// and STILL writes the named buffers.
    #[tokio::test(flavor = "multi_thread")]
    async fn wa_reports_e141_for_unnamed_buffer_but_still_writes_named() {
        let temp = tempfile::tempdir().unwrap();
        let named = temp.path().join("named.txt");
        std::fs::write(&named, "aaa\n").unwrap();

        let mut editor = Editor::new();
        // Buffer 1 is the initial unnamed buffer — modify it.
        editor
            .buffer_mut()
            .insert_text_at(0, CharCol::ZERO, "draft");
        editor.load_file_async(&named).await.unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, CharCol::ZERO, "changed ");

        let result = execute_command(&mut editor, "wa");
        assert!(
            matches!(result, CommandResult::Error(ref e) if e.error.contains("E141: No file name for buffer 1")),
            "unexpected result: {result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&named).unwrap(),
            "changed aaa\n",
            "named buffer must be written despite the unnamed-buffer error"
        );
    }

    #[test]
    fn session_start_rejects_overlong_name() {
        let mut editor = Editor::new();
        let name = "x".repeat(65);

        let result = execute_command(&mut editor, &format!("session start {name}"));
        assert!(
            matches!(result, CommandResult::Error(ref e) if e.error.contains("Invalid session name")),
            "65-char name must be rejected, got: {result:?}"
        );
    }

    #[test]
    fn session_start_rejects_punctuation_only_name() {
        let mut editor = Editor::new();

        let result = execute_command(&mut editor, "session start !!!...");
        assert!(
            matches!(result, CommandResult::Error(ref e) if e.error.contains("Invalid session name")),
            "punctuation-only name must be rejected, got: {result:?}"
        );
    }

    #[test]
    fn session_start_accepts_valid_names_past_validation() {
        // No API port is set, so a valid name proceeds past validation and
        // fails on the API-server check instead — proving the name itself
        // was accepted without touching the real session directory.
        let max_len = "y".repeat(64);
        for name in ["dev", "my_session-2", max_len.as_str()] {
            let mut editor = Editor::new();
            let result = execute_command(&mut editor, &format!("session start {name}"));
            assert!(
                matches!(result, CommandResult::Error(ref e) if e.error.contains("do not expose the automation API")),
                "valid name {name:?} must pass validation, got: {result:?}"
            );
        }
    }
}
