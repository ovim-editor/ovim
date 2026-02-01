use crate::KeyEvent;
use std::collections::HashMap;

/// Manages macro recording and playback
#[derive(Clone, Debug)]
pub struct MacroManager {
    /// Stored macros (a-z)
    macros: HashMap<char, Vec<KeyEvent>>,
    /// Currently recording macro register
    recording: Option<char>,
    /// Events being recorded
    current_recording: Vec<KeyEvent>,
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
        }
    }

    /// Starts recording a macro
    pub fn start_recording(&mut self, register: char) -> bool {
        if register.is_ascii_lowercase() {
            self.recording = Some(register);
            self.current_recording.clear();
            true
        } else {
            false
        }
    }

    /// Stops recording and saves the macro
    pub fn stop_recording(&mut self) {
        if let Some(register) = self.recording {
            if !self.current_recording.is_empty() {
                self.macros.insert(register, self.current_recording.clone());
            }
            self.recording = None;
            self.current_recording.clear();
        }
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

    /// Gets a macro by register
    pub fn get_macro(&self, register: char) -> Option<&Vec<KeyEvent>> {
        self.macros.get(&register)
    }

    /// Clears all macros
    pub fn clear(&mut self) {
        self.macros.clear();
        self.recording = None;
        self.current_recording.clear();
    }
}
