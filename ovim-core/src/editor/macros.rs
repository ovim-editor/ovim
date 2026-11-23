use crate::{KeyCode, KeyEvent, Modifiers};
use std::collections::HashMap;

/// The keys a macro register was last recorded with, and the text the
/// register held then. The register is the source of truth (it can be yanked
/// into or pasted from), so these only stand in for it while it still holds
/// that text: they keep keys that have no character, like the arrows.
#[derive(Clone, Debug)]
struct RecordedKeys {
    events: Vec<KeyEvent>,
    text: String,
}

/// The register text for typed keys: characters as they are, control keys as
/// control characters, Esc as `^[` and Enter as `^M`, as Vim stores them.
/// Keys with no character (arrows, function keys) have no text.
pub fn keys_to_text(events: &[KeyEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event.code {
            KeyCode::Char(c) if event.modifiers.contains(Modifiers::CONTROL) => {
                let c = c.to_ascii_lowercase();
                matches!(c, 'a'..='z' | '[' | '\\' | ']' | '^' | '_')
                    .then(|| char::from(c as u8 & 0x1f))
            }
            KeyCode::Char(c) => Some(c),
            KeyCode::Esc => Some('\x1b'),
            KeyCode::Enter => Some('\r'),
            KeyCode::Tab => Some('\t'),
            KeyCode::Backspace => Some('\x7f'),
            _ => None,
        })
        .collect()
}

/// The keys that executing register text (`@a`) types: the inverse of
/// [`keys_to_text`], with a line feed typed as Enter.
pub fn text_to_keys(text: &str) -> Vec<KeyEvent> {
    text.chars()
        .map(|c| match c {
            '\x1b' => KeyEvent::new(KeyCode::Esc, Modifiers::NONE),
            '\r' | '\n' => KeyEvent::new(KeyCode::Enter, Modifiers::NONE),
            '\t' => KeyEvent::new(KeyCode::Tab, Modifiers::NONE),
            '\x7f' | '\x08' => KeyEvent::new(KeyCode::Backspace, Modifiers::NONE),
            '\x01'..='\x1a' => KeyEvent::new(
                KeyCode::Char(char::from(c as u8 + b'a' - 1)),
                Modifiers::CONTROL,
            ),
            '\x1c'..='\x1f' => KeyEvent::new(
                KeyCode::Char(char::from(c as u8 + b'@')),
                Modifiers::CONTROL,
            ),
            c => KeyEvent::new(KeyCode::Char(c), Modifiers::NONE),
        })
        .collect()
}

/// Manages macro recording and playback
#[derive(Clone, Debug)]
pub struct MacroManager {
    /// Keys recorded into each register (a-z)
    macros: HashMap<char, RecordedKeys>,
    /// Currently recording macro register
    recording: Option<char>,
    /// Events being recorded
    current_recording: Vec<KeyEvent>,
    /// Last played macro register (for @@)
    last_played: Option<char>,
    /// Set by motions that fail to move during macro playback
    aborted: bool,
}

impl Default for MacroManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MacroManager {
    /// Creates a new macro manager
    pub fn new() -> Self {
        Self {
            macros: HashMap::new(),
            recording: None,
            current_recording: Vec::new(),
            last_played: None,
            aborted: false,
        }
    }

    /// Starts recording a macro (`qA` appends to register `a`)
    pub fn start_recording(&mut self, register: char) -> bool {
        if register.is_ascii_alphabetic() {
            self.recording = Some(register);
            self.current_recording.clear();
            true
        } else {
            false
        }
    }

    /// Stops recording; the register and the keys typed into it.
    pub fn stop_recording(&mut self) -> Option<(char, Vec<KeyEvent>)> {
        let register = self.recording.take()?;
        Some((register, std::mem::take(&mut self.current_recording)))
    }

    /// Records a key event (if currently recording)
    pub fn record_event(&mut self, event: KeyEvent) {
        if self.recording.is_some() {
            self.current_recording.push(event);
        }
    }

    /// Returns whether currently recording
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Gets the register being recorded
    pub fn recording_register(&self) -> Option<char> {
        self.recording
    }

    /// The keys recorded into `register` if it still holds `text`.
    fn recorded_keys(&self, register: char, text: &str) -> Option<&[KeyEvent]> {
        self.macros
            .get(&register)
            .filter(|recorded| recorded.text == text)
            .map(|recorded| recorded.events.as_slice())
    }

    /// Remembers the keys `register` was recorded with and the text it holds.
    fn remember(&mut self, register: char, events: Vec<KeyEvent>, text: String) {
        self.macros.insert(register, RecordedKeys { events, text });
    }

    /// Sets the last played register (for @@)
    pub fn set_last_played(&mut self, register: char) {
        self.last_played = Some(register);
    }

    /// Gets the last played register (for @@)
    pub fn last_played(&self) -> Option<char> {
        self.last_played
    }

    /// Clears all macros
    pub fn clear(&mut self) {
        self.macros.clear();
        self.recording = None;
        self.current_recording.clear();
        self.last_played = None;
        self.aborted = false;
    }

    /// Returns whether macro playback should abort (a motion failed to move).
    pub fn aborted(&self) -> bool {
        self.aborted
    }

    /// Signal that a motion failed (cursor didn't move), aborting macro playback.
    pub fn signal_abort(&mut self) {
        self.aborted = true;
    }

    /// Clear the macro abort flag.
    pub fn clear_abort(&mut self) {
        self.aborted = false;
    }
}

impl super::Editor {
    /// Stops recording: the typed keys become the text of the register
    /// (`qA` appends to it), so `"ap` pastes them and `"ayy` can replace them.
    pub fn stop_macro_recording(&mut self) {
        use super::RegisterType;
        let Some((register, events)) = self.macro_manager.stop_recording() else {
            return;
        };
        let name = register.to_ascii_lowercase();
        let mut keys = Vec::new();
        if register.is_ascii_uppercase() {
            keys = self.get_macro(name).unwrap_or_default();
        }
        self.registers.set_with_type(
            Some(register),
            keys_to_text(&events),
            RegisterType::Character,
        );
        keys.extend(events);
        let text = self.registers.get(Some(name));
        self.macro_manager.remember(name, keys, text);
    }

    /// The keys executing `register` types: what was recorded into it, or its
    /// text if it has changed since (yanked into, for one).
    pub fn get_macro(&self, register: char) -> Option<Vec<KeyEvent>> {
        let text = self.registers.get(Some(register));
        if text.is_empty() {
            return None;
        }
        Some(match self.macro_manager.recorded_keys(register, &text) {
            Some(events) => events.to_vec(),
            None => text_to_keys(&text),
        })
    }

    /// Replay a register in the caller's execution scope. Counted and nested
    /// playback share clipboard state until the outermost dispatch completes.
    pub(crate) fn play_macro(&mut self, register: char, count: usize) -> anyhow::Result<()> {
        let Some(events) = self.get_macro(register) else {
            return Ok(());
        };
        self.with_execution_scope(|editor| {
            editor.clear_macro_abort();
            let result = (|| {
                for _ in 0..count {
                    for event in &events {
                        super::InputHandler::handle_key_event(editor, *event)?;
                        if editor.macro_aborted() {
                            return Ok(());
                        }
                    }
                }
                Ok(())
            })();
            editor.clear_macro_abort();
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: Modifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_become_register_text_and_back() {
        let events = [
            key(KeyCode::Char('A'), Modifiers::SHIFT),
            key(KeyCode::Char('!'), Modifiers::NONE),
            key(KeyCode::Esc, Modifiers::NONE),
            key(KeyCode::Char('a'), Modifiers::CONTROL),
            key(KeyCode::Enter, Modifiers::NONE),
        ];
        let text = keys_to_text(&events);
        assert_eq!(text, "A!\x1b\x01\r");
        let keys = text_to_keys(&text);
        assert_eq!(keys[1], events[1]);
        assert_eq!(keys[2], events[2]);
        assert_eq!(keys[3], events[3]);
        assert_eq!(keys[4], events[4]);
    }

    #[test]
    fn keys_without_a_character_have_no_text() {
        let events = [
            key(KeyCode::Left, Modifiers::NONE),
            key(KeyCode::Char('x'), Modifiers::NONE),
            key(KeyCode::F(2), Modifiers::NONE),
        ];
        assert_eq!(keys_to_text(&events), "x");
    }

    #[test]
    fn a_line_feed_in_a_register_is_typed_as_enter() {
        assert_eq!(
            text_to_keys("j\n"),
            [
                key(KeyCode::Char('j'), Modifiers::NONE),
                key(KeyCode::Enter, Modifiers::NONE)
            ]
        );
    }
}
