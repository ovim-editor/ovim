//! LSP status line text, its toast classification, and the status summary.

use super::*;
use std::time::Duration;

fn dedupe_key_for_status(status_lower: &str) -> String {
    status_lower
        .split(':')
        .next()
        .unwrap_or(status_lower)
        .trim()
        .to_string()
}

fn is_lsp_toast_candidate(status_lower: &str) -> bool {
    status_lower.starts_with("lsp:")
        || status_lower.starts_with("java:")
        || status_lower.contains("completion")
        || status_lower.contains("hover")
        || status_lower.contains("definition")
        || status_lower.contains("implementation")
        || status_lower.contains("code action")
        || status_lower.contains("semantic token")
        || status_lower.contains("workspace edit")
        || status_lower.contains("organize imports")
        || status_lower.contains("rename")
        || status_lower.contains("diagnostic")
}

struct StatusToast {
    level: ToastLevel,
    ttl: Duration,
    dedupe_key: String,
}

fn classify_status_toast(status: &str) -> Option<StatusToast> {
    if status.is_empty() {
        return None;
    }

    let lower = status.to_ascii_lowercase();
    if !is_lsp_toast_candidate(&lower) {
        return None;
    }

    if lower.contains("failed") || lower.contains("error") {
        return Some(StatusToast {
            level: ToastLevel::Error,
            ttl: Duration::from_secs(8),
            dedupe_key: dedupe_key_for_status(&lower),
        });
    }

    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("crashed")
        || lower.contains("cancelled")
        || lower.contains("canceled")
    {
        return Some(StatusToast {
            level: ToastLevel::Warning,
            ttl: Duration::from_secs(6),
            dedupe_key: dedupe_key_for_status(&lower),
        });
    }

    None
}

impl Editor {
    /// Set LSP status message
    pub fn set_lsp_status(&mut self, status: String) {
        self.lsp.state.status = status.clone();
        self.set_status_message(status.clone());

        if let Some(policy) = classify_status_toast(&status) {
            let request = ToastRequest::new(ToastSource::Lsp, policy.level, status)
                .with_title("LSP")
                .with_ttl(Some(policy.ttl))
                .with_dedupe_key(format!("lsp:{}", policy.dedupe_key));
            self.push_toast(request);
        }
    }

    /// Get current LSP status
    pub fn lsp_status(&self) -> &str {
        &self.lsp.state.status
    }

    /// Get LSP progress message (e.g., "indexing...")
    pub fn lsp_progress_message(&self) -> Option<String> {
        if let Some(lsp_manager) = &self.lsp.state.lsp_manager {
            lsp_manager.get_progress_message()
        } else {
            None
        }
    }

    /// Get LSP info for status line
    pub fn get_lsp_info(&self) -> String {
        let mut info = String::new();

        // LSP Manager status
        if self.lsp.state.lsp_manager.is_some() {
            info.push_str("LSP: enabled\n");
        } else {
            info.push_str("LSP: disabled\n");
        }

        // Active servers
        if self.lsp.state.active_lsp_servers.is_empty() {
            info.push_str("Servers: none\n");
        } else {
            info.push_str("Servers:\n");
            for (lang_id, server_name) in &self.lsp.state.active_lsp_servers {
                info.push_str(&format!("  - {} ({})\n", server_name, lang_id));
            }
        }

        // Progress messages
        if let Some(progress) = self.lsp_progress_message() {
            info.push_str(&format!("Progress: {}\n", progress));
        }

        // Diagnostic counts
        let (errors, warnings, infos, hints) = self.lsp.state.diagnostic_count;
        info.push_str(&format!(
            "Diagnostics: E:{} W:{} I:{} H:{}\n",
            errors, warnings, infos, hints
        ));

        // Current status
        if !self.lsp_status().is_empty() {
            info.push_str(&format!("\nStatus: {}\n", self.lsp_status()));
        }

        info
    }
}
