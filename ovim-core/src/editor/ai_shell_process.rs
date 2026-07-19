use std::path::PathBuf;
use std::time::Duration;

use super::ai_chat_state::{ShellInspectorState, ShellTranscript, ShellTranscriptPhase};
use super::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellProcessPhase {
    Preparing,
    Running,
    InterruptRequested,
    CapturingChanges,
    Succeeded,
    Failed,
    Interrupted,
    OutcomeUnknown,
}

impl ShellProcessPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "preparing snapshot",
            Self::Running => "running",
            Self::InterruptRequested => "interrupt requested",
            Self::CapturingChanges => "capturing changes",
            Self::Succeeded => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::OutcomeUnknown => "outcome unknown",
        }
    }

    pub fn is_running(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Running | Self::InterruptRequested | Self::CapturingChanges
        )
    }
}

impl From<ShellTranscriptPhase> for ShellProcessPhase {
    fn from(value: ShellTranscriptPhase) -> Self {
        match value {
            ShellTranscriptPhase::Preparing => Self::Preparing,
            ShellTranscriptPhase::Running => Self::Running,
            ShellTranscriptPhase::InterruptRequested => Self::InterruptRequested,
            ShellTranscriptPhase::CapturingChanges => Self::CapturingChanges,
            ShellTranscriptPhase::Succeeded => Self::Succeeded,
            ShellTranscriptPhase::Failed => Self::Failed,
            ShellTranscriptPhase::Interrupted => Self::Interrupted,
            ShellTranscriptPhase::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellInspectorView {
    pub tool_call_id: String,
    pub command: String,
    pub workdir: PathBuf,
    pub phase: ShellProcessPhase,
    pub pid: Option<u32>,
    pub elapsed: Duration,
    pub last_output_age: Option<Duration>,
    pub output: String,
    pub dropped_bytes: usize,
    pub expired: bool,
    pub row_scroll_from_bottom: usize,
    pub follow_latest: bool,
    pub search_query: Option<String>,
    pub search_input: Option<String>,
    pub search_match_line: Option<usize>,
}

impl Editor {
    pub fn ai_shell_process_row_label(&self, tool_call_id: &str) -> Option<String> {
        let transcript = self
            .ai_state
            .chat
            .as_ref()?
            .shell_transcripts
            .get(tool_call_id)?;
        let phase = ShellProcessPhase::from(transcript.phase).label();
        let command = transcript
            .command
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let output_age = transcript.last_output_at.map(|last| {
            format!(
                " · output {} ago",
                compact_duration(std::time::Instant::now().saturating_duration_since(last))
            )
        });
        Some(format!(
            "{phase} · {command}{}",
            output_age.unwrap_or_default()
        ))
    }

    pub fn ai_shell_inspector_is_open(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.shell_inspector.is_some())
    }

    pub fn open_ai_shell_process_inspector(&mut self, tool_call_id: &str) -> bool {
        let Some(chat) = self.ai_state.chat.as_mut() else {
            return false;
        };
        if !chat.shell_transcripts.contains_key(tool_call_id) {
            return false;
        }
        chat.shell_inspector = Some(ShellInspectorState {
            tool_call_id: tool_call_id.to_string(),
            follow_latest: true,
            ..Default::default()
        });
        true
    }

    pub fn close_ai_shell_process_inspector(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.shell_inspector = None;
        }
    }

    pub fn ai_shell_inspector_view(&self) -> Option<ShellInspectorView> {
        let chat = self.ai_state.chat.as_ref()?;
        let inspector = chat.shell_inspector.as_ref()?;
        let transcript = chat.shell_transcripts.get(&inspector.tool_call_id)?;
        let now = std::time::Instant::now();
        let finished_at = transcript.completed_at.unwrap_or(now);
        Some(ShellInspectorView {
            tool_call_id: inspector.tool_call_id.clone(),
            command: transcript.command.clone(),
            workdir: transcript.workdir.clone(),
            phase: transcript.phase.into(),
            pid: transcript.pid,
            elapsed: finished_at.saturating_duration_since(transcript.started_at),
            last_output_age: transcript
                .last_output_at
                .map(|last| now.saturating_duration_since(last)),
            output: normalize_shell_output(transcript),
            dropped_bytes: transcript.dropped_bytes,
            expired: transcript.expired,
            row_scroll_from_bottom: inspector.row_scroll_from_bottom,
            follow_latest: inspector.follow_latest,
            search_query: inspector.search_query.clone(),
            search_input: inspector.search_input.clone(),
            search_match_line: inspector.search_match_line,
        })
    }

    pub fn ai_shell_inspector_scroll_up(&mut self, rows: usize) {
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.follow_latest = false;
            inspector.row_scroll_from_bottom =
                inspector.row_scroll_from_bottom.saturating_add(rows);
        }
    }

    pub fn ai_shell_inspector_scroll_down(&mut self, rows: usize) {
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.row_scroll_from_bottom =
                inspector.row_scroll_from_bottom.saturating_sub(rows);
            if inspector.row_scroll_from_bottom == 0 {
                inspector.follow_latest = true;
            }
        }
    }

    pub fn ai_shell_inspector_follow_latest(&mut self) {
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.row_scroll_from_bottom = 0;
            inspector.follow_latest = true;
        }
    }

    pub fn ai_shell_inspector_scroll_to_top(&mut self) {
        let line_count = self
            .ai_shell_inspector_view()
            .map_or(0, |view| view.output.lines().count());
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.follow_latest = false;
            inspector.row_scroll_from_bottom = line_count.saturating_sub(1);
        }
    }

    pub fn ai_shell_inspector_begin_search(&mut self) {
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.search_input = Some(inspector.search_query.clone().unwrap_or_default());
        }
    }

    pub fn ai_shell_inspector_search_insert(&mut self, character: char) {
        if let Some(input) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
            .and_then(|inspector| inspector.search_input.as_mut())
        {
            input.push(character);
        }
    }

    pub fn ai_shell_inspector_search_backspace(&mut self) {
        if let Some(input) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
            .and_then(|inspector| inspector.search_input.as_mut())
        {
            input.pop();
        }
    }

    pub fn ai_shell_inspector_cancel_search(&mut self) {
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.search_input = None;
        }
    }

    pub fn ai_shell_inspector_submit_search(&mut self) {
        let query = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.shell_inspector.as_ref())
            .and_then(|inspector| inspector.search_input.clone())
            .unwrap_or_default();
        if query.is_empty() {
            self.ai_shell_inspector_cancel_search();
            return;
        }
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.search_input = None;
            inspector.search_query = Some(query);
            inspector.search_match_line = None;
        }
        self.ai_shell_inspector_next_match();
    }

    pub fn ai_shell_inspector_next_match(&mut self) {
        let Some(view) = self.ai_shell_inspector_view() else {
            return;
        };
        let Some(query) = view
            .search_query
            .as_deref()
            .filter(|query| !query.is_empty())
        else {
            return;
        };
        let lines = view.output.lines().collect::<Vec<_>>();
        let before = view.search_match_line.unwrap_or(lines.len());
        let found = (0..before)
            .rev()
            .find(|&index| lines[index].contains(query))
            .or_else(|| {
                (before..lines.len())
                    .rev()
                    .find(|&index| lines[index].contains(query))
            });
        let Some(found) = found else {
            return;
        };
        if let Some(inspector) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.shell_inspector.as_mut())
        {
            inspector.search_match_line = Some(found);
            inspector.follow_latest = false;
            inspector.row_scroll_from_bottom = lines.len().saturating_sub(found + 1);
        }
    }
}

fn compact_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m", seconds / 60)
    }
}

fn normalize_shell_output(transcript: &ShellTranscript) -> String {
    let bytes = transcript
        .chunks
        .iter()
        .flat_map(|chunk| chunk.bytes.iter().copied())
        .collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&bytes);
    let mut output = String::with_capacity(text.len());
    let mut line_start = 0;
    let mut state = EscapeState::Text;
    for character in text.chars() {
        state = match state {
            EscapeState::Text => match character {
                '\u{1b}' => EscapeState::Escape,
                '\r' => {
                    output.truncate(line_start);
                    EscapeState::Text
                }
                '\n' => {
                    output.push('\n');
                    line_start = output.len();
                    EscapeState::Text
                }
                '\u{8}' => {
                    if output.len() > line_start {
                        output.pop();
                    }
                    EscapeState::Text
                }
                '\t' => {
                    output.push('\t');
                    EscapeState::Text
                }
                value if value.is_control() => EscapeState::Text,
                value => {
                    output.push(value);
                    EscapeState::Text
                }
            },
            EscapeState::Escape => match character {
                '[' => EscapeState::Csi,
                ']' => EscapeState::Osc,
                _ => EscapeState::Text,
            },
            EscapeState::Csi => {
                if ('@'..='~').contains(&character) {
                    EscapeState::Text
                } else {
                    EscapeState::Csi
                }
            }
            EscapeState::Osc => match character {
                '\u{7}' => EscapeState::Text,
                '\u{1b}' => EscapeState::OscEscape,
                _ => EscapeState::Osc,
            },
            EscapeState::OscEscape => {
                if character == '\\' {
                    EscapeState::Text
                } else {
                    EscapeState::Osc
                }
            }
        };
    }
    output
}

#[derive(Clone, Copy)]
enum EscapeState {
    Text,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ToolCallInfo;
    use crate::editor::ai_chat_state::ShellOutputStream;

    fn transcript(output: &[u8]) -> ShellTranscript {
        let mut transcript = ShellTranscript::new(
            ToolCallInfo {
                id: "shell-1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "test" }),
            },
            "test".into(),
            ".".into(),
        );
        transcript.append(ShellOutputStream::Stdout, output.to_vec());
        transcript
    }

    #[test]
    fn output_normalization_removes_ansi_and_applies_carriage_returns() {
        let transcript = transcript(b"\x1b[32mdownloading\x1b[0m 10%\rdownloading 80%\ndone\n");
        assert_eq!(
            normalize_shell_output(&transcript),
            "downloading 80%\ndone\n"
        );
    }

    #[test]
    fn output_normalization_handles_escape_sequences_split_across_chunks() {
        let mut transcript = transcript(b"before \x1b[");
        transcript.append(ShellOutputStream::Stderr, b"31merror\x1b[0m after".to_vec());
        assert_eq!(normalize_shell_output(&transcript), "before error after");
    }

    #[test]
    fn inspector_search_moves_to_matching_output_and_leaves_follow_mode() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(crate::ai::chat_types::ChatOpts {
                name: "chat".into(),
                allow_edits: true,
                ..Default::default()
            })
            .unwrap();
        let transcript = transcript(b"first\nneedle here\nlast\n");
        editor
            .ai_state
            .chat
            .as_mut()
            .unwrap()
            .shell_transcripts
            .insert("shell-1".into(), transcript);

        assert!(editor.open_ai_shell_process_inspector("shell-1"));
        editor.ai_shell_inspector_begin_search();
        for character in "needle".chars() {
            editor.ai_shell_inspector_search_insert(character);
        }
        editor.ai_shell_inspector_submit_search();

        let view = editor.ai_shell_inspector_view().unwrap();
        assert_eq!(view.search_query.as_deref(), Some("needle"));
        assert_eq!(view.search_match_line, Some(1));
        assert!(!view.follow_latest);
        assert_eq!(view.row_scroll_from_bottom, 1);
    }
}
