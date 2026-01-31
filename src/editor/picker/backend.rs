use super::fuzzy_backend::FuzzyListKind;
use super::grep_backend::GrepState;
use super::nucleo_backend::NucleoState;
use super::result::PickerMode;

/// Typed backend for the picker. Each variant owns the state specific to its mode.
pub enum PickerBackend {
    Nucleo(NucleoState),
    Grep(GrepState),
    FuzzyList(FuzzyListKind),
}

impl PickerBackend {
    /// Returns the PickerMode corresponding to this backend.
    pub fn mode(&self) -> PickerMode {
        match self {
            PickerBackend::Nucleo(_) => PickerMode::FindFiles,
            PickerBackend::Grep(_) => PickerMode::LiveGrep,
            PickerBackend::FuzzyList(kind) => match kind {
                FuzzyListKind::Custom => PickerMode::Custom,
                FuzzyListKind::Completion => PickerMode::Completion,
                FuzzyListKind::LspLocations => PickerMode::LspLocations,
            },
        }
    }
}
