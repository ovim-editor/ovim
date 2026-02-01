use super::fuzzy_backend::FuzzyListKind;
use super::grep_backend::GrepState;
use super::nucleo_backend::NucleoState;

/// Typed backend for the picker. Each variant owns the state specific to its mode.
pub enum PickerBackend {
    Nucleo(NucleoState),
    Grep(GrepState),
    FuzzyList(FuzzyListKind),
}
