//! Prometheus-style metrics for ovim
//!
//! This module provides observability into ovim's performance and behavior through
//! Prometheus-compatible metrics. Metrics are exposed via the `/v1/metrics` endpoint.
//!
//! # Metric Types
//!
//! - **Counters**: Monotonically increasing values (requests, edits, errors)
//! - **Gauges**: Current state that can go up/down (buffer size, active sessions)
//! - **Histograms**: Distribution of values (latency, duration)
//!
//! # Usage
//!
//! ```rust
//! use ovim::metrics;
//!
//! // Increment counters
//! metrics::HTTP_REQUESTS_TOTAL.inc();
//!
//! // Time operations
//! let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
//! // ... operation ...
//! // Timer automatically records duration when dropped
//!
//! // Update gauges
//! metrics::BUFFER_SIZE_BYTES.set(buffer_size as i64);
//! ```
//!
//! # Feature Flag
//!
//! This module is only available when the `metrics` feature is enabled:
//!
//! ```toml
//! [dependencies]
//! ovim = { version = "0.1", features = ["metrics"] }
//! ```

use lazy_static::lazy_static;

#[cfg(feature = "metrics")]
use prometheus::{
    register_histogram, register_int_counter, register_int_gauge, Encoder, Histogram, IntCounter,
    IntGauge, TextEncoder,
};

#[cfg(feature = "metrics")]
lazy_static! {
    // ===== HTTP API Metrics =====

    /// Total number of HTTP API requests received
    ///
    /// Use `rate()` in PromQL to get requests per second:
    /// ```promql
    /// rate(ovim_http_requests_total[5m])
    /// ```
    pub static ref HTTP_REQUESTS_TOTAL: IntCounter = register_int_counter!(
        "ovim_http_requests_total",
        "Total HTTP API requests received"
    ).unwrap();

    /// HTTP request latency distribution in seconds
    ///
    /// Query 95th percentile:
    /// ```promql
    /// histogram_quantile(0.95, ovim_http_request_duration_seconds)
    /// ```
    pub static ref HTTP_REQUEST_DURATION: Histogram = register_histogram!(
        "ovim_http_request_duration_seconds",
        "HTTP request latency in seconds"
    ).unwrap();

    /// Total number of HTTP errors (5xx responses)
    pub static ref HTTP_ERRORS_TOTAL: IntCounter = register_int_counter!(
        "ovim_http_errors_total",
        "Total HTTP errors (5xx responses)"
    ).unwrap();

    // ===== Buffer Metrics =====

    /// Total number of buffer edit operations
    ///
    /// Tracks inserts, deletes, and modifications. Use `rate()` for edit rate:
    /// ```promql
    /// rate(ovim_buffer_edits_total[1m])
    /// ```
    pub static ref BUFFER_EDITS_TOTAL: IntCounter = register_int_counter!(
        "ovim_buffer_edits_total",
        "Total buffer edit operations"
    ).unwrap();

    /// Current buffer size in bytes
    pub static ref BUFFER_SIZE_BYTES: IntGauge = register_int_gauge!(
        "ovim_buffer_size_bytes",
        "Current buffer size in bytes"
    ).unwrap();

    /// Current buffer line count
    pub static ref BUFFER_LINES: IntGauge = register_int_gauge!(
        "ovim_buffer_lines",
        "Current number of lines in buffer"
    ).unwrap();

    // ===== LSP Metrics =====

    /// Total number of LSP requests sent to language servers
    pub static ref LSP_REQUESTS_TOTAL: IntCounter = register_int_counter!(
        "ovim_lsp_requests_total",
        "Total LSP requests sent"
    ).unwrap();

    /// LSP request latency distribution in seconds
    ///
    /// Includes time from sending request to receiving response.
    pub static ref LSP_REQUEST_DURATION: Histogram = register_histogram!(
        "ovim_lsp_request_duration_seconds",
        "LSP request latency in seconds"
    ).unwrap();

    /// Total number of LSP errors
    pub static ref LSP_ERRORS_TOTAL: IntCounter = register_int_counter!(
        "ovim_lsp_errors_total",
        "Total LSP errors"
    ).unwrap();

    /// Total number of LSP didChange notifications sent
    ///
    /// High rate indicates frequent buffer changes.
    pub static ref LSP_DIDCHANGE_TOTAL: IntCounter = register_int_counter!(
        "ovim_lsp_didchange_total",
        "Total LSP didChange notifications sent"
    ).unwrap();

    /// Total number of LSP diagnostic updates received
    pub static ref LSP_DIAGNOSTICS_TOTAL: IntCounter = register_int_counter!(
        "ovim_lsp_diagnostics_total",
        "Total LSP diagnostic updates received"
    ).unwrap();

    // ===== Render Metrics =====

    /// UI render time distribution in seconds
    ///
    /// Only relevant for TUI mode, not headless.
    pub static ref RENDER_DURATION: Histogram = register_histogram!(
        "ovim_render_duration_seconds",
        "UI render time in seconds"
    ).unwrap();

    /// Total number of renders performed
    pub static ref RENDER_COUNT: IntCounter = register_int_counter!(
        "ovim_render_count_total",
        "Total number of renders performed"
    ).unwrap();

    /// Syntax highlighting time distribution in seconds
    pub static ref SYNTAX_HIGHLIGHT_DURATION: Histogram = register_histogram!(
        "ovim_syntax_highlight_duration_seconds",
        "Syntax highlighting time in seconds"
    ).unwrap();

    // ===== Session Metrics =====

    /// Number of currently active ovim sessions
    ///
    /// Gauge that tracks how many ovim instances are running.
    pub static ref ACTIVE_SESSIONS: IntGauge = register_int_gauge!(
        "ovim_active_sessions",
        "Number of active ovim sessions"
    ).unwrap();

    /// Session uptime in seconds
    pub static ref SESSION_UPTIME_SECONDS: IntGauge = register_int_gauge!(
        "ovim_session_uptime_seconds",
        "Session uptime in seconds"
    ).unwrap();

    // ===== Memory Metrics =====

    /// Approximate memory usage in bytes
    ///
    /// This is a rough estimate based on buffer size and internal structures.
    pub static ref MEMORY_USAGE_BYTES: IntGauge = register_int_gauge!(
        "ovim_memory_usage_bytes",
        "Approximate memory usage in bytes"
    ).unwrap();

    // ===== Input Metrics =====

    /// Keystroke processing latency distribution in seconds
    ///
    /// Time from key press to editor state update.
    pub static ref INPUT_LATENCY: Histogram = register_histogram!(
        "ovim_input_latency_seconds",
        "Keystroke processing latency in seconds"
    ).unwrap();
}

#[cfg(feature = "metrics")]
/// Export all metrics in Prometheus text format
///
/// Returns metrics in the Prometheus exposition format, suitable for scraping.
///
/// # Example
///
/// ```rust
/// let metrics_text = ovim::metrics::export_metrics();
/// println!("{}", metrics_text);
/// ```
///
/// Output:
/// ```text
/// # HELP ovim_http_requests_total Total HTTP API requests received
/// # TYPE ovim_http_requests_total counter
/// ovim_http_requests_total 42
/// ...
/// ```
pub fn export_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();

    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .expect("Failed to encode metrics");

    String::from_utf8(buffer).expect("Metrics are not valid UTF-8")
}

#[cfg(not(feature = "metrics"))]
/// Export metrics (no-op when metrics feature is disabled)
///
/// Returns a message indicating metrics are disabled.
pub fn export_metrics() -> String {
    "# Metrics disabled. Enable with --features metrics\n".to_string()
}

// No-op implementations when metrics feature is disabled
#[cfg(not(feature = "metrics"))]
pub struct NoOpCounter;
#[cfg(not(feature = "metrics"))]
impl NoOpCounter {
    pub fn inc(&self) {}
    pub fn inc_by(&self, _: u64) {}
}

#[cfg(not(feature = "metrics"))]
pub struct NoOpGauge;
#[cfg(not(feature = "metrics"))]
impl NoOpGauge {
    pub fn set(&self, _: i64) {}
    pub fn inc(&self) {}
    pub fn dec(&self) {}
}

#[cfg(not(feature = "metrics"))]
pub struct NoOpHistogram;
#[cfg(not(feature = "metrics"))]
impl NoOpHistogram {
    pub fn observe(&self, _: f64) {}
    pub fn start_timer(&self) -> NoOpTimer {
        NoOpTimer
    }
}

#[cfg(not(feature = "metrics"))]
pub struct NoOpTimer;
#[cfg(not(feature = "metrics"))]
impl Drop for NoOpTimer {
    fn drop(&mut self) {}
}

// No-op static refs when metrics disabled
#[cfg(not(feature = "metrics"))]
lazy_static! {
    pub static ref HTTP_REQUESTS_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref HTTP_REQUEST_DURATION: NoOpHistogram = NoOpHistogram;
    pub static ref HTTP_ERRORS_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref BUFFER_EDITS_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref BUFFER_SIZE_BYTES: NoOpGauge = NoOpGauge;
    pub static ref BUFFER_LINES: NoOpGauge = NoOpGauge;
    pub static ref LSP_REQUESTS_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref LSP_REQUEST_DURATION: NoOpHistogram = NoOpHistogram;
    pub static ref LSP_ERRORS_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref LSP_DIDCHANGE_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref LSP_DIAGNOSTICS_TOTAL: NoOpCounter = NoOpCounter;
    pub static ref RENDER_DURATION: NoOpHistogram = NoOpHistogram;
    pub static ref RENDER_COUNT: NoOpCounter = NoOpCounter;
    pub static ref SYNTAX_HIGHLIGHT_DURATION: NoOpHistogram = NoOpHistogram;
    pub static ref ACTIVE_SESSIONS: NoOpGauge = NoOpGauge;
    pub static ref SESSION_UPTIME_SECONDS: NoOpGauge = NoOpGauge;
    pub static ref MEMORY_USAGE_BYTES: NoOpGauge = NoOpGauge;
    pub static ref INPUT_LATENCY: NoOpHistogram = NoOpHistogram;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "metrics")]
    fn test_counter_increment() {
        HTTP_REQUESTS_TOTAL.inc();
        let metrics = export_metrics();
        assert!(metrics.contains("ovim_http_requests_total"));
    }

    #[test]
    #[cfg(feature = "metrics")]
    fn test_gauge_set() {
        BUFFER_SIZE_BYTES.set(1024);
        let metrics = export_metrics();
        assert!(metrics.contains("ovim_buffer_size_bytes"));
    }

    #[test]
    #[cfg(feature = "metrics")]
    fn test_histogram_observe() {
        HTTP_REQUEST_DURATION.observe(0.001);
        let metrics = export_metrics();
        assert!(metrics.contains("ovim_http_request_duration_seconds"));
    }

    #[test]
    #[cfg(feature = "metrics")]
    fn test_export_format() {
        // Increment some metrics
        HTTP_REQUESTS_TOTAL.inc();
        BUFFER_EDITS_TOTAL.inc_by(5);
        BUFFER_SIZE_BYTES.set(2048);

        let exported = export_metrics();

        // Should contain HELP and TYPE directives
        assert!(exported.contains("# HELP"));
        assert!(exported.contains("# TYPE"));

        // Should contain our metrics
        assert!(exported.contains("ovim_http_requests_total"));
        assert!(exported.contains("ovim_buffer_edits_total"));
        assert!(exported.contains("ovim_buffer_size_bytes"));
    }

    #[test]
    #[cfg(not(feature = "metrics"))]
    fn test_no_op_when_disabled() {
        // Should not panic
        HTTP_REQUESTS_TOTAL.inc();
        BUFFER_SIZE_BYTES.set(1024);
        let _timer = HTTP_REQUEST_DURATION.start_timer();

        let exported = export_metrics();
        assert!(exported.contains("disabled"));
    }
}
