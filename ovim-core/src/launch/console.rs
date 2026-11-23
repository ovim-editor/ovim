//! The run console: a persistent, scrollable record of every build, run and
//! debug session.
//!
//! Output is kept per run and survives the process exiting; the header shows
//! status, exit code and duration. Lines remember which stream they came
//! from (stdout / stderr / build tool / editor notes / debugger) and, when
//! they point at source (`at com.foo.Bar.baz(Bar.java:42)`, `Foo.java:12:
//! error: ...`), where.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::diagnostics::strip_ansi;
use super::plan::LaunchMode;
use super::process::{MAX_LINE_BYTES, TRUNCATED_MARKER};
use super::stacktrace::{parse_console_location, ConsoleLocation};

/// Retained lines per run; older lines are dropped from the front.
pub const MAX_CONSOLE_LINES: usize = 20_000;
/// Retained text per run; older lines are dropped from the front.
pub const MAX_CONSOLE_BYTES: usize = 8 << 20;
/// Retained runs; the oldest finished run is dropped first.
pub const MAX_RUNS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Program standard output.
    Stdout,
    /// Program standard error.
    Stderr,
    /// Output of the build step.
    Build,
    /// Notes from the editor itself ("Build failed", "Process exited...").
    System,
    /// Debug adapter console messages.
    Debugger,
}

#[derive(Debug, Clone)]
pub struct ConsoleLine {
    pub kind: LineKind,
    pub text: String,
    pub location: Option<ConsoleLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Resolving,
    Building,
    Running,
    WaitingForDebugger,
    Debugging,
}

impl RunPhase {
    pub fn label(self) -> &'static str {
        match self {
            RunPhase::Resolving => "resolving",
            RunPhase::Building => "building",
            RunPhase::Running => "running",
            RunPhase::WaitingForDebugger => "waiting for debugger",
            RunPhase::Debugging => "debugging",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Succeeded,
    /// Program (or test task) exited non-zero.
    Failed,
    BuildFailed,
    Stopped,
    /// Never got going: resolve/spawn/attach error.
    Error(String),
    /// A debug session ended; the adapter reported no exit code.
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Active(RunPhase),
    Done(RunOutcome),
}

impl RunStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, RunStatus::Active(_))
    }
}

/// One build/run/debug session and its output.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: u64,
    pub title: String,
    pub mode: LaunchMode,
    /// The command line (or a description) shown under the title.
    pub command: String,
    pub cwd: PathBuf,
    /// Where stack-trace frames are looked up.
    pub source_roots: Vec<PathBuf>,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub lines: Vec<ConsoleLine>,
    /// Lines dropped from the front once [`MAX_CONSOLE_LINES`] or
    /// [`MAX_CONSOLE_BYTES`] was exceeded.
    pub truncated: usize,
    /// Text bytes held in `lines`.
    bytes: usize,
    pub started: Instant,
    pub duration: Option<Duration>,
    /// Text of a final line that has not been terminated by a newline yet
    /// (DAP output events are not line-aligned).
    partial: Option<(LineKind, String)>,
}

impl RunRecord {
    fn new(id: u64, title: String, mode: LaunchMode, cwd: PathBuf) -> Self {
        Self {
            id,
            title,
            mode,
            command: String::new(),
            cwd,
            source_roots: Vec::new(),
            status: RunStatus::Active(RunPhase::Resolving),
            exit_code: None,
            lines: Vec::new(),
            truncated: 0,
            bytes: 0,
            started: Instant::now(),
            duration: None,
            partial: None,
        }
    }

    /// Time since start while active, final duration once done.
    pub fn elapsed(&self) -> Duration {
        self.duration.unwrap_or_else(|| self.started.elapsed())
    }

    /// Appends one complete line.
    pub fn push_line(&mut self, kind: LineKind, text: impl Into<String>) {
        let mut text = text.into();
        // The console shows plain text: colour codes would appear as garbage.
        if text.contains('\x1b') {
            text = strip_ansi(&text).into_owned();
        }
        let text = clip_line(text);
        let location = if text.contains(".java")
            || text.contains(".kt")
            || text.contains(".scala")
            || text.contains(".groovy")
        {
            parse_console_location(&text)
        } else {
            None
        };
        self.bytes += text.len();
        self.lines.push(ConsoleLine {
            kind,
            text,
            location,
        });
        if self.lines.len() > MAX_CONSOLE_LINES || self.bytes > MAX_CONSOLE_BYTES {
            self.drop_oldest_lines();
        }
    }

    /// Drops lines from the front down to half of either cap, so the next
    /// drop is a long way off. The newest line always stays.
    fn drop_oldest_lines(&mut self) {
        let (mut count, mut bytes) = (0, 0);
        while count + 1 < self.lines.len()
            && (self.lines.len() - count > MAX_CONSOLE_LINES / 2
                || self.bytes - bytes > MAX_CONSOLE_BYTES / 2)
        {
            bytes += self.lines[count].text.len();
            count += 1;
        }
        self.lines.drain(..count);
        self.bytes -= bytes;
        self.truncated += count;
    }

    /// Appends text that may hold several lines and end mid-line.
    pub fn push_chunk(&mut self, kind: LineKind, chunk: &str) {
        let mut text = chunk.replace("\r\n", "\n");
        if let Some((partial_kind, partial)) = self.partial.take() {
            if partial_kind == kind {
                text = format!("{partial}{text}");
            } else {
                self.push_line(partial_kind, partial);
            }
        }
        let ends_with_newline = text.ends_with('\n');
        let mut pieces: Vec<&str> = text.split('\n').collect();
        let tail = pieces.pop().unwrap_or("");
        for piece in pieces {
            self.push_line(kind, piece.to_string());
        }
        if tail.len() > MAX_LINE_BYTES {
            // Never wait for the end of a line that is already too long.
            self.push_line(kind, tail.to_string());
        } else if !tail.is_empty() && !ends_with_newline {
            self.partial = Some((kind, tail.to_string()));
        }
    }

    /// Emits any unterminated trailing text as a line.
    pub fn flush_partial(&mut self) {
        if let Some((kind, text)) = self.partial.take() {
            self.push_line(kind, text);
        }
    }

    /// Marks the run finished.
    pub fn finish(&mut self, outcome: RunOutcome, exit_code: Option<i32>) {
        self.flush_partial();
        self.status = RunStatus::Done(outcome);
        if exit_code.is_some() {
            self.exit_code = exit_code;
        }
        self.duration = Some(self.started.elapsed());
    }

    /// One-line summary such as `failed (exit 1) in 2.3s`.
    pub fn status_text(&self) -> String {
        let elapsed = crate::editor::format_duration(self.elapsed());
        match &self.status {
            RunStatus::Active(phase) => format!("{} · {}", phase.label(), elapsed),
            RunStatus::Done(outcome) => {
                let exit = self
                    .exit_code
                    .map(|c| format!(" (exit {c})"))
                    .unwrap_or_default();
                match outcome {
                    RunOutcome::Succeeded => format!("finished{exit} in {elapsed}"),
                    RunOutcome::Failed => format!("failed{exit} in {elapsed}"),
                    RunOutcome::BuildFailed => format!("build failed{exit} in {elapsed}"),
                    RunOutcome::Stopped => format!("stopped after {elapsed}"),
                    RunOutcome::Error(msg) => format!("error: {msg}"),
                    RunOutcome::Ended => format!("session ended{exit} after {elapsed}"),
                }
            }
        }
    }
}

/// Cuts `text` to [`MAX_LINE_BYTES`] (at a character boundary) with a marker.
/// A line the process reader already cut passes untouched.
fn clip_line(mut text: String) -> String {
    if text.len() <= MAX_LINE_BYTES + TRUNCATED_MARKER.len() {
        return text;
    }
    let mut cut = MAX_LINE_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str(TRUNCATED_MARKER);
    text
}

/// State of the run console panel.
#[derive(Debug, Default)]
pub struct RunConsoleState {
    /// Whether the panel is shown.
    pub open: bool,
    /// Runs, oldest first.
    pub runs: Vec<RunRecord>,
    /// Index of the run being viewed; `None` follows the newest run.
    pub selected: Option<usize>,
    /// Lines scrolled up from the bottom; `0` follows the output.
    pub scroll: usize,
    /// Highlighted line (index into the viewed run) while the console has focus.
    pub cursor: usize,
    /// Rows available for output in the last render (set by the frontend).
    pub view_height: usize,
    /// Rows added to (or taken from) the default panel height.
    pub height_delta: i16,
    next_id: u64,
}

impl RunConsoleState {
    /// Starts a new run record, opens the panel, and follows the new run.
    pub fn start_run(&mut self, title: String, mode: LaunchMode, cwd: PathBuf) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.runs.push(RunRecord::new(id, title, mode, cwd));
        while self.runs.len() > MAX_RUNS {
            match self.runs.iter().position(|r| !r.status.is_active()) {
                Some(idx) => {
                    self.runs.remove(idx);
                }
                None => break,
            }
        }
        self.selected = None;
        self.scroll = 0;
        self.open = true;
        id
    }

    pub fn run_mut(&mut self, id: u64) -> Option<&mut RunRecord> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    pub fn run(&self, id: u64) -> Option<&RunRecord> {
        self.runs.iter().find(|r| r.id == id)
    }

    /// Index of the run currently shown.
    pub fn viewed_index(&self) -> Option<usize> {
        if self.runs.is_empty() {
            return None;
        }
        Some(
            self.selected
                .map(|i| i.min(self.runs.len() - 1))
                .unwrap_or(self.runs.len() - 1),
        )
    }

    pub fn viewed(&self) -> Option<&RunRecord> {
        self.viewed_index().and_then(|i| self.runs.get(i))
    }

    /// Switch to an older run.
    pub fn view_previous(&mut self) {
        if let Some(i) = self.viewed_index() {
            if i > 0 {
                self.selected = Some(i - 1);
                self.scroll = 0;
                self.cursor = 0;
            }
        }
    }

    /// Switch to a newer run; reaching the newest resumes following.
    pub fn view_next(&mut self) {
        if let Some(i) = self.viewed_index() {
            if i + 1 >= self.runs.len() {
                return;
            }
            self.selected = if i + 2 >= self.runs.len() {
                None
            } else {
                Some(i + 1)
            };
            self.scroll = 0;
            self.cursor = 0;
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let max = self.viewed().map(|r| r.lines.len()).unwrap_or(0);
        self.scroll = (self.scroll + lines).min(max.saturating_sub(1));
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = self
            .viewed()
            .map(|r| r.lines.len().saturating_sub(1))
            .unwrap_or(0);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
    }

    /// Index of the first visible line for a viewport of `height` rows.
    pub fn top_line(&self, height: usize) -> usize {
        let len = self.viewed().map(|r| r.lines.len()).unwrap_or(0);
        let bottom = len.saturating_sub(self.scroll.min(len.saturating_sub(1)));
        bottom.saturating_sub(height)
    }

    /// Moves the highlighted line by `delta` (clamped) and scrolls so it
    /// stays visible.
    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.viewed().map(|r| r.lines.len()).unwrap_or(0);
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        self.set_cursor(next);
    }

    /// Highlights line `index` and scrolls it into view.
    pub fn set_cursor(&mut self, index: usize) {
        let len = self.viewed().map(|r| r.lines.len()).unwrap_or(0);
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = index.min(len - 1);
        let height = self.view_height.max(1);
        let top = self.top_line(height);
        if self.cursor < top {
            // Put the cursor on the first row.
            self.scroll = len.saturating_sub(self.cursor + height);
        } else if self.cursor >= top + height {
            self.scroll = len.saturating_sub(self.cursor + 1);
        }
    }

    /// Removes finished runs (active runs stay).
    pub fn clear_finished(&mut self) {
        self.runs.retain(|r| r.status.is_active());
        self.selected = None;
        self.scroll = 0;
        self.cursor = 0;
    }

    pub fn any_active(&self) -> bool {
        self.runs.iter().any(|r| r.status.is_active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> RunRecord {
        RunRecord::new(1, "Main".into(), LaunchMode::Run, PathBuf::from("/p"))
    }

    #[test]
    fn chunks_are_split_into_lines_and_partial_tails_are_joined() {
        let mut r = record();
        r.push_chunk(LineKind::Stdout, "one\ntw");
        r.push_chunk(LineKind::Stdout, "o\nthree");
        assert_eq!(
            r.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        r.flush_partial();
        assert_eq!(r.lines.last().unwrap().text, "three");
    }

    #[test]
    fn a_partial_line_is_flushed_when_the_stream_kind_changes() {
        let mut r = record();
        r.push_chunk(LineKind::Stdout, "out");
        r.push_chunk(LineKind::Stderr, "err\n");
        assert_eq!(r.lines[0].text, "out");
        assert_eq!(r.lines[0].kind, LineKind::Stdout);
        assert_eq!(r.lines[1].kind, LineKind::Stderr);
    }

    #[test]
    fn stack_frames_and_compiler_lines_carry_locations() {
        let mut r = record();
        r.push_line(LineKind::Stderr, "\tat com.x.Foo.bar(Foo.java:7)");
        r.push_line(LineKind::Stdout, "plain text");
        assert!(matches!(
            r.lines[0].location,
            Some(ConsoleLocation::Frame { line: 7, .. })
        ));
        assert!(r.lines[1].location.is_none());
    }

    #[test]
    fn line_cap_keeps_the_tail_and_counts_drops() {
        let mut r = record();
        for i in 0..(MAX_CONSOLE_LINES + 5) {
            r.push_line(LineKind::Stdout, format!("l{i}"));
        }
        assert!(r.lines.len() <= MAX_CONSOLE_LINES);
        assert_eq!(
            r.lines.last().unwrap().text,
            format!("l{}", MAX_CONSOLE_LINES + 4)
        );
        assert_eq!(r.truncated + r.lines.len(), MAX_CONSOLE_LINES + 5);
    }

    #[test]
    fn the_byte_cap_drops_old_lines_even_when_there_are_few_of_them() {
        let mut r = record();
        let long = "x".repeat(MAX_LINE_BYTES);
        let lines = MAX_CONSOLE_BYTES / MAX_LINE_BYTES + 5;
        for i in 0..lines {
            r.push_line(LineKind::Stdout, format!("{i}{long}"));
            assert!(r.bytes <= MAX_CONSOLE_BYTES + 2 * MAX_LINE_BYTES);
        }
        assert!(r.lines.len() < lines);
        assert!(r
            .lines
            .last()
            .unwrap()
            .text
            .starts_with(&(lines - 1).to_string()));
        assert_eq!(r.truncated + r.lines.len(), lines);
        assert_eq!(r.bytes, r.lines.iter().map(|l| l.text.len()).sum::<usize>());
    }

    #[test]
    fn colour_codes_are_stripped_but_locations_still_found() {
        let mut r = record();
        r.push_line(LineKind::Stdout, "\x1b[31mred text\x1b[0m plain");
        r.push_chunk(
            LineKind::Stderr,
            "\x1b[1;31m\tat com.x.Foo.bar(Foo.java:7)\x1b[m\n",
        );
        assert_eq!(r.lines[0].text, "red text plain");
        assert_eq!(r.lines[1].text, "\tat com.x.Foo.bar(Foo.java:7)");
        assert!(matches!(
            r.lines[1].location,
            Some(ConsoleLocation::Frame { line: 7, .. })
        ));
    }

    #[test]
    fn an_overlong_line_is_cut_at_a_character_boundary_with_a_marker() {
        let mut r = record();
        r.push_line(LineKind::Stdout, "\u{20ac}".repeat(MAX_LINE_BYTES));
        let text = &r.lines[0].text;
        assert!(text.ends_with(TRUNCATED_MARKER));
        assert!(text.len() <= MAX_LINE_BYTES + TRUNCATED_MARKER.len());
        assert!(!text.contains('\u{FFFD}'));
        // A line the process reader already cut is not cut twice.
        let cut = format!("{}{TRUNCATED_MARKER}", "a".repeat(MAX_LINE_BYTES));
        r.push_line(LineKind::Stdout, cut.clone());
        assert_eq!(r.lines[1].text, cut);
    }

    #[test]
    fn an_endless_unterminated_chunk_stream_does_not_pile_up_as_a_partial_line() {
        let mut r = record();
        for _ in 0..10 {
            r.push_chunk(LineKind::Stdout, &"y".repeat(MAX_LINE_BYTES));
        }
        r.push_chunk(LineKind::Stdout, &"y".repeat(MAX_LINE_BYTES + 1));
        assert!(r
            .partial
            .as_ref()
            .is_none_or(|(_, p)| p.len() <= MAX_LINE_BYTES));
        assert!(!r.lines.is_empty());
    }

    #[test]
    fn finished_runs_persist_and_report_exit_code_and_duration() {
        let mut state = RunConsoleState::default();
        let id = state.start_run("Main".into(), LaunchMode::Run, "/p".into());
        state.run_mut(id).unwrap().push_line(LineKind::Stdout, "hi");
        state
            .run_mut(id)
            .unwrap()
            .finish(RunOutcome::Failed, Some(2));
        let run = state.viewed().unwrap();
        assert!(
            run.status_text().starts_with("failed (exit 2) in "),
            "{}",
            run.status_text()
        );
        assert_eq!(run.lines.len(), 1);
        assert!(!state.any_active());
    }

    #[test]
    fn history_drops_oldest_finished_run_first_and_never_an_active_one() {
        let mut state = RunConsoleState::default();
        let first = state.start_run("active".into(), LaunchMode::Run, "/p".into());
        for i in 0..MAX_RUNS + 2 {
            let id = state.start_run(format!("r{i}"), LaunchMode::Run, "/p".into());
            state
                .run_mut(id)
                .unwrap()
                .finish(RunOutcome::Succeeded, Some(0));
        }
        assert_eq!(state.runs.len(), MAX_RUNS);
        assert!(state.run(first).is_some(), "active run must be kept");
    }

    #[test]
    fn scrolling_is_clamped_and_switching_runs_resets_it() {
        let mut state = RunConsoleState::default();
        let a = state.start_run("a".into(), LaunchMode::Run, "/p".into());
        state.start_run("b".into(), LaunchMode::Run, "/p".into());
        for i in 0..10 {
            state
                .run_mut(a)
                .unwrap()
                .push_line(LineKind::Stdout, format!("{i}"));
        }
        state.view_previous();
        assert_eq!(state.viewed().unwrap().id, a);
        state.scroll_up(100);
        assert_eq!(state.scroll, 9);
        state.scroll_down(4);
        assert_eq!(state.scroll, 5);
        state.view_next();
        assert_eq!(state.scroll, 0);
        assert_eq!(
            state.selected, None,
            "reaching the newest run resumes following"
        );
    }

    #[test]
    fn cursor_movement_keeps_the_highlighted_line_in_view() {
        let mut state = RunConsoleState::default();
        let id = state.start_run("a".into(), LaunchMode::Run, "/p".into());
        for i in 0..100 {
            state
                .run_mut(id)
                .unwrap()
                .push_line(LineKind::Stdout, format!("{i}"));
        }
        state.view_height = 10;
        state.set_cursor(99);
        assert_eq!(state.top_line(10), 90);
        state.set_cursor(50);
        assert_eq!(
            state.top_line(10),
            50,
            "cursor above the window becomes the first row"
        );
        state.set_cursor(59);
        assert_eq!(
            state.top_line(10),
            50,
            "still visible: window does not move"
        );
        state.move_cursor(1);
        assert_eq!(state.cursor, 60);
        assert_eq!(state.top_line(10), 51);
        state.move_cursor(-1000);
        assert_eq!(state.cursor, 0);
        assert_eq!(state.top_line(10), 0);
    }

    #[test]
    fn clear_removes_finished_runs_only() {
        let mut state = RunConsoleState::default();
        let a = state.start_run("done".into(), LaunchMode::Run, "/p".into());
        state
            .run_mut(a)
            .unwrap()
            .finish(RunOutcome::Succeeded, Some(0));
        let b = state.start_run("live".into(), LaunchMode::Run, "/p".into());
        state.clear_finished();
        assert_eq!(state.runs.len(), 1);
        assert_eq!(state.runs[0].id, b);
    }
}
