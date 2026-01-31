use super::result::PickerResult;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;

/// State for the LiveGrep picker mode.
pub struct GrepState {
    pub grep_rx: Option<mpsc::Receiver<PickerResult>>,
    pub grep_cancel: Option<Arc<AtomicBool>>,
    pub last_grep_query: String,
    pub grep_stale: bool,
    pub loading: bool,
    pub file_filter: String,
    pub file_filter_cursor: usize,
    pub last_filtered_file_filter: String,
}

impl GrepState {
    pub fn new() -> Self {
        Self {
            grep_rx: None,
            grep_cancel: None,
            last_grep_query: String::new(),
            grep_stale: false,
            loading: false,
            file_filter: String::new(),
            file_filter_cursor: 0,
            last_filtered_file_filter: String::new(),
        }
    }
}
