use std::collections::HashMap;

/// A mode in which a key mapping applies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapMode {
    Normal,
    Insert,
    Visual,
    Command,
    /// All modes
    All,
}

impl MapMode {
    pub fn from_prefix(prefix: &str) -> Self {
        match prefix {
            "n" => MapMode::Normal,
            "i" => MapMode::Insert,
            "v" | "x" => MapMode::Visual,
            "c" => MapMode::Command,
            "" => MapMode::All,
            _ => MapMode::Normal,
        }
    }

    pub fn display_char(&self) -> char {
        match self {
            MapMode::Normal => 'n',
            MapMode::Insert => 'i',
            MapMode::Visual => 'v',
            MapMode::Command => 'c',
            MapMode::All => ' ',
        }
    }
}

/// A key mapping (lhs -> rhs)
#[derive(Debug, Clone)]
pub struct KeyMapping {
    /// The key sequence to trigger the mapping
    pub lhs: String,
    /// The key sequence to execute
    pub rhs: String,
    /// Whether this is a noremap (no recursive mapping)
    pub noremap: bool,
}

impl KeyMapping {
    pub fn new(lhs: String, rhs: String, noremap: bool) -> Self {
        Self { lhs, rhs, noremap }
    }
}

/// Manages key mappings for all modes
#[derive(Debug, Clone, Default)]
pub struct KeyMapManager {
    /// Mappings by mode
    mappings: HashMap<MapMode, Vec<KeyMapping>>,
}

impl KeyMapManager {
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
        }
    }

    /// Add a mapping for a specific mode
    pub fn add_mapping(&mut self, mode: MapMode, lhs: String, rhs: String, noremap: bool) {
        let mapping = KeyMapping::new(lhs.clone(), rhs, noremap);

        // Remove existing mapping for this lhs if present
        self.remove_mapping(mode, &lhs);

        self.mappings.entry(mode).or_default().push(mapping);
    }

    /// Remove a mapping for a specific mode
    pub fn remove_mapping(&mut self, mode: MapMode, lhs: &str) -> bool {
        if let Some(maps) = self.mappings.get_mut(&mode) {
            let len_before = maps.len();
            maps.retain(|m| m.lhs != lhs);
            return maps.len() < len_before;
        }
        false
    }

    /// Clear all mappings for a mode
    pub fn clear_mappings(&mut self, mode: MapMode) {
        self.mappings.remove(&mode);
    }

    /// Get a mapping for a key sequence in a specific mode
    pub fn get_mapping(&self, mode: MapMode, lhs: &str) -> Option<&KeyMapping> {
        // Check mode-specific mappings first
        if let Some(maps) = self.mappings.get(&mode) {
            if let Some(mapping) = maps.iter().find(|m| m.lhs == lhs) {
                return Some(mapping);
            }
        }

        // Then check All-mode mappings
        if mode != MapMode::All {
            if let Some(maps) = self.mappings.get(&MapMode::All) {
                if let Some(mapping) = maps.iter().find(|m| m.lhs == lhs) {
                    return Some(mapping);
                }
            }
        }

        None
    }

    /// Check if a key sequence could be the start of a mapping
    pub fn has_prefix(&self, mode: MapMode, prefix: &str) -> bool {
        // Check mode-specific mappings
        if let Some(maps) = self.mappings.get(&mode) {
            if maps
                .iter()
                .any(|m| m.lhs.starts_with(prefix) && m.lhs != prefix)
            {
                return true;
            }
        }

        // Check All-mode mappings
        if mode != MapMode::All {
            if let Some(maps) = self.mappings.get(&MapMode::All) {
                if maps
                    .iter()
                    .any(|m| m.lhs.starts_with(prefix) && m.lhs != prefix)
                {
                    return true;
                }
            }
        }

        false
    }

    /// List all mappings (for :map command)
    pub fn list_mappings(&self, mode: Option<MapMode>) -> Vec<(MapMode, &KeyMapping)> {
        let mut result = Vec::new();

        if let Some(specific_mode) = mode {
            if let Some(maps) = self.mappings.get(&specific_mode) {
                for mapping in maps {
                    result.push((specific_mode, mapping));
                }
            }
        } else {
            // List all modes
            for (m, maps) in &self.mappings {
                for mapping in maps {
                    result.push((*m, mapping));
                }
            }
        }

        result.sort_by(|a, b| a.1.lhs.cmp(&b.1.lhs));
        result
    }

    /// Parse special key notation like <CR>, <Esc>, <Space>, <C-a>, etc.
    pub fn parse_key_notation(s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '<' {
                // Collect until >
                let mut special = String::new();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '>' {
                        break;
                    }
                    special.push(c);
                }

                // Parse the special key
                let key = match special.to_lowercase().as_str() {
                    "cr" | "enter" | "return" => "\n",
                    "esc" | "escape" => "\x1b",
                    "space" => " ",
                    "tab" => "\t",
                    "bs" | "backspace" => "\x7f",
                    "lt" => "<",
                    "gt" => ">",
                    "bar" => "|",
                    "bslash" => "\\",
                    "up" => "\x1b[A",
                    "down" => "\x1b[B",
                    "right" => "\x1b[C",
                    "left" => "\x1b[D",
                    _ if special.starts_with("C-") || special.starts_with("c-") => {
                        // Ctrl+key
                        if let Some(key_char) = special.chars().nth(2) {
                            let ctrl_code = (key_char.to_ascii_lowercase() as u8) - b'a' + 1;
                            &*Box::leak(String::from(ctrl_code as char).into_boxed_str())
                        } else {
                            ""
                        }
                    }
                    _ => {
                        // Unknown, keep as-is
                        result.push('<');
                        result.push_str(&special);
                        result.push('>');
                        continue;
                    }
                };
                result.push_str(key);
            } else {
                result.push(ch);
            }
        }

        result
    }
}
