/// Maximum number of input latency samples to keep for percentile calculation
pub const MAX_LATENCY_SAMPLES: usize = 1000;

use super::Editor;

/// Performance metrics for the editor
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// Performance metrics: render count
    pub render_count: u64,
    /// Performance metrics: last render duration in microseconds
    pub last_render_duration_micros: Option<u64>,
    /// Performance metrics: last syntax highlighting duration in microseconds
    pub last_syntax_duration_micros: Option<u64>,
    /// Render dirty flag - set when UI needs redraw
    pub render_dirty: bool,
    /// Input latency samples in microseconds (circular buffer, max 1000 samples)
    pub input_latency_samples: Vec<u64>,
    /// Last LSP serialize (rope->string) duration in microseconds
    pub last_lsp_serialize_micros: Option<u64>,
    /// Last git status refresh duration in microseconds
    pub last_git_status_micros: Option<u64>,
    /// Last fold calculation duration in microseconds
    pub last_fold_calc_micros: Option<u64>,
    /// Last diagnostic query duration in microseconds
    pub last_diagnostic_query_micros: Option<u64>,
}

impl PerformanceMetrics {
    /// Create new performance metrics with default values
    pub fn new() -> Self {
        Self {
            render_count: 0,
            last_render_duration_micros: None,
            last_syntax_duration_micros: None,
            render_dirty: true, // Start dirty to trigger initial render
            input_latency_samples: Vec::new(),
            last_lsp_serialize_micros: None,
            last_git_status_micros: None,
            last_fold_calc_micros: None,
            last_diagnostic_query_micros: None,
        }
    }

    /// Performance metrics: increment render count
    pub fn increment_render_count(&mut self) {
        self.render_count = self.render_count.saturating_add(1);
    }

    /// Performance metrics: record render duration
    pub fn record_render_duration(&mut self, duration_micros: u64) {
        self.last_render_duration_micros = Some(duration_micros);
    }

    /// Performance metrics: record syntax highlighting duration
    pub fn record_syntax_duration(&mut self, duration_micros: u64) {
        self.last_syntax_duration_micros = Some(duration_micros);
    }

    /// Performance metrics: get render count
    pub fn render_count(&self) -> u64 {
        self.render_count
    }

    /// Performance metrics: get last render duration
    pub fn last_render_duration_micros(&self) -> Option<u64> {
        self.last_render_duration_micros
    }

    /// Performance metrics: get last syntax duration
    pub fn last_syntax_duration_micros(&self) -> Option<u64> {
        self.last_syntax_duration_micros
    }

    /// Performance metrics: record input latency sample
    pub fn record_input_latency(&mut self, latency_micros: u64) {
        self.input_latency_samples.push(latency_micros);
        // Keep only the most recent MAX_LATENCY_SAMPLES samples (circular buffer)
        if self.input_latency_samples.len() > MAX_LATENCY_SAMPLES {
            self.input_latency_samples.remove(0);
        }
    }

    /// Performance metrics: compute latency percentile
    fn compute_percentile(samples: &[u64], percentile: f64) -> Option<u64> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let index = ((percentile / 100.0) * (sorted.len() as f64 - 1.0)) as usize;
        Some(sorted[index])
    }

    /// Performance metrics: get input latency p50
    pub fn input_latency_p50_micros(&self) -> Option<u64> {
        Self::compute_percentile(&self.input_latency_samples, 50.0)
    }

    /// Performance metrics: get input latency p95
    pub fn input_latency_p95_micros(&self) -> Option<u64> {
        Self::compute_percentile(&self.input_latency_samples, 95.0)
    }

    /// Performance metrics: get input latency p99
    pub fn input_latency_p99_micros(&self) -> Option<u64> {
        Self::compute_percentile(&self.input_latency_samples, 99.0)
    }

    /// Performance metrics: get number of input latency samples
    pub fn input_latency_sample_count(&self) -> usize {
        self.input_latency_samples.len()
    }

    /// Performance metrics: record LSP serialize duration
    pub fn record_lsp_serialize_duration(&mut self, duration_micros: u64) {
        self.last_lsp_serialize_micros = Some(duration_micros);
    }

    /// Performance metrics: get last LSP serialize duration
    pub fn last_lsp_serialize_micros(&self) -> Option<u64> {
        self.last_lsp_serialize_micros
    }

    /// Performance metrics: record git status duration
    pub fn record_git_status_duration(&mut self, duration_micros: u64) {
        self.last_git_status_micros = Some(duration_micros);
    }

    /// Performance metrics: get last git status duration
    pub fn last_git_status_micros(&self) -> Option<u64> {
        self.last_git_status_micros
    }

    /// Performance metrics: record fold calculation duration
    pub fn record_fold_calc_duration(&mut self, duration_micros: u64) {
        self.last_fold_calc_micros = Some(duration_micros);
    }

    /// Performance metrics: get last fold calculation duration
    pub fn last_fold_calc_micros(&self) -> Option<u64> {
        self.last_fold_calc_micros
    }

    /// Performance metrics: record diagnostic query duration
    pub fn record_diagnostic_query_duration(&mut self, duration_micros: u64) {
        self.last_diagnostic_query_micros = Some(duration_micros);
    }

    /// Performance metrics: get last diagnostic query duration
    pub fn last_diagnostic_query_micros(&self) -> Option<u64> {
        self.last_diagnostic_query_micros
    }

    /// Marks the editor as needing a redraw
    pub fn mark_dirty(&mut self) {
        self.render_dirty = true;
    }

    /// Checks if the editor needs a redraw
    pub fn is_dirty(&self) -> bool {
        self.render_dirty
    }

    /// Marks the editor as clean (just rendered)
    pub fn mark_clean(&mut self) {
        self.render_dirty = false;
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// Performance metrics: increment render count
    pub fn increment_render_count(&mut self) {
        self.metrics.increment_render_count();
    }

    /// Performance metrics: record render duration
    pub fn record_render_duration(&mut self, duration_micros: u64) {
        self.metrics.record_render_duration(duration_micros);
    }

    /// Performance metrics: record syntax highlighting duration
    pub fn record_syntax_duration(&mut self, duration_micros: u64) {
        self.metrics.record_syntax_duration(duration_micros);
    }

    /// Performance metrics: get render count
    pub fn render_count(&self) -> u64 {
        self.metrics.render_count()
    }

    /// Performance metrics: get last render duration
    pub fn last_render_duration_micros(&self) -> Option<u64> {
        self.metrics.last_render_duration_micros()
    }

    /// Performance metrics: get last syntax duration
    pub fn last_syntax_duration_micros(&self) -> Option<u64> {
        self.metrics.last_syntax_duration_micros()
    }

    /// Performance metrics: record input latency sample
    pub fn record_input_latency(&mut self, latency_micros: u64) {
        self.metrics.record_input_latency(latency_micros);
    }

    /// Performance metrics: get input latency p50
    pub fn input_latency_p50_micros(&self) -> Option<u64> {
        self.metrics.input_latency_p50_micros()
    }

    /// Performance metrics: get input latency p95
    pub fn input_latency_p95_micros(&self) -> Option<u64> {
        self.metrics.input_latency_p95_micros()
    }

    /// Performance metrics: get input latency p99
    pub fn input_latency_p99_micros(&self) -> Option<u64> {
        self.metrics.input_latency_p99_micros()
    }

    /// Performance metrics: get number of input latency samples
    pub fn input_latency_sample_count(&self) -> usize {
        self.metrics.input_latency_sample_count()
    }

    /// Performance metrics: record LSP serialize duration
    pub fn record_lsp_serialize_duration(&mut self, duration_micros: u64) {
        self.metrics.record_lsp_serialize_duration(duration_micros);
    }

    /// Performance metrics: get last LSP serialize duration
    pub fn last_lsp_serialize_micros(&self) -> Option<u64> {
        self.metrics.last_lsp_serialize_micros()
    }

    /// Performance metrics: record git status duration
    pub fn record_git_status_duration(&mut self, duration_micros: u64) {
        self.metrics.record_git_status_duration(duration_micros);
    }

    /// Performance metrics: get last git status duration
    pub fn last_git_status_micros(&self) -> Option<u64> {
        self.metrics.last_git_status_micros()
    }

    /// Performance metrics: record fold calculation duration
    pub fn record_fold_calc_duration(&mut self, duration_micros: u64) {
        self.metrics.record_fold_calc_duration(duration_micros);
    }

    /// Performance metrics: get last fold calculation duration
    pub fn last_fold_calc_micros(&self) -> Option<u64> {
        self.metrics.last_fold_calc_micros()
    }

    /// Performance metrics: record diagnostic query duration
    pub fn record_diagnostic_query_duration(&mut self, duration_micros: u64) {
        self.metrics.record_diagnostic_query_duration(duration_micros);
    }

    /// Performance metrics: get last diagnostic query duration
    pub fn last_diagnostic_query_micros(&self) -> Option<u64> {
        self.metrics.last_diagnostic_query_micros()
    }

    /// Marks the editor as needing a redraw
    pub fn mark_dirty(&mut self) {
        self.metrics.mark_dirty();
    }

    /// Checks if the editor needs a redraw
    pub fn is_dirty(&self) -> bool {
        self.metrics.is_dirty()
    }

    /// Marks the editor as clean (just rendered)
    pub fn mark_clean(&mut self) {
        self.metrics.mark_clean();
    }
}
