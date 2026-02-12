use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

pub fn handle_ai_prompt_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char(ch) if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            let pos = editor.ai_state.prompt.cursor;
            let mut buf = editor.ai_state.prompt.input.clone();
            buf.insert(pos, ch);
            editor.ai_state.prompt.input = buf;
            editor.ai_state.prompt.cursor = pos + ch.len_utf8();
        }
        KeyCode::Backspace => {
            let pos = editor.ai_state.prompt.cursor;
            if pos > 0 {
                let mut buf = editor.ai_state.prompt.input.clone();
                let prev = buf[..pos]
                    .char_indices()
                    .next_back()
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);
                buf.remove(prev);
                editor.ai_state.prompt.input = buf;
                editor.ai_state.prompt.cursor = prev;
            }
        }
        KeyCode::Delete => {
            let pos = editor.ai_state.prompt.cursor;
            let len = editor.ai_state.prompt.input.len();
            if pos < len {
                let mut buf = editor.ai_state.prompt.input.clone();
                buf.remove(pos);
                editor.ai_state.prompt.input = buf;
            }
        }
        KeyCode::Left => {
            let pos = editor.ai_state.prompt.cursor;
            if pos > 0 {
                let prev = editor.ai_state.prompt.input[..pos]
                    .char_indices()
                    .next_back()
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);
                editor.ai_state.prompt.cursor = prev;
            }
        }
        KeyCode::Right => {
            let pos = editor.ai_state.prompt.cursor;
            let buf = &editor.ai_state.prompt.input;
            if pos < buf.len() {
                let next = buf[pos..]
                    .char_indices()
                    .nth(1)
                    .map(|(idx, _)| pos + idx)
                    .unwrap_or(buf.len());
                editor.ai_state.prompt.cursor = next;
            }
        }
        KeyCode::Home => {
            editor.ai_state.prompt.cursor = 0;
        }
        KeyCode::End => {
            editor.ai_state.prompt.cursor = editor.ai_state.prompt.input.len();
        }
        KeyCode::Tab | KeyCode::Down => {
            editor.ai_cycle_profile(true);
        }
        KeyCode::BackTab | KeyCode::Up => {
            editor.ai_cycle_profile(false);
        }
        KeyCode::Enter => {
            editor.submit_ai_prompt_job()?;
        }
        KeyCode::Esc => {
            editor.ai_state.prompt.input.clear();
            editor.ai_state.prompt.cursor = 0;
            editor.ai_state.active_selection = None;
            editor.set_mode(Mode::Normal);
        }
        KeyCode::Char('c') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.ai_state.prompt.input.clear();
            editor.ai_state.prompt.cursor = 0;
            editor.ai_state.active_selection = None;
            editor.set_mode(Mode::Normal);
        }
        _ => {}
    }
    Ok(())
}
