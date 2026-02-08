//! LSP diagnostics initial-load integration test
//!
//! Requires:
//! - `cargo build --release`
//! - rust-analyzer installed
//!
//! Run with: `cargo test --test lsp_diagnostics_initial_load_test -- --ignored`

mod lsp_test_utils;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct DiagnosticCounts {
    errors: usize,
    warnings: usize,
    info: usize,
    hints: usize,
}

#[derive(Debug, Deserialize)]
struct DiagnosticsInfo {
    counts: DiagnosticCounts,
}

#[tokio::test]
#[ignore = "Requires release binary and rust-analyzer"]
async fn diagnostics_appear_without_edits_on_initial_load() -> Result<()> {
    let dir = tempfile::tempdir().context("tempdir")?;
    std::fs::create_dir_all(dir.path().join("src")).context("create src/")?;

    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "ovim_diag_test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .context("write Cargo.toml")?;

    // Intentional type error.
    std::fs::write(
        dir.path().join("src").join("main.rs"),
        r#"fn main() {
    let x: i32 = "oops";
    println!("{x}");
}
"#,
    )
    .context("write main.rs")?;

    let file_path = dir
        .path()
        .join("src")
        .join("main.rs")
        .to_string_lossy()
        .to_string();

    let session = lsp_test_utils::OvimTestSession::start(&file_path).await?;

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/diagnostics", session.port);

    let mut last_counts: Option<DiagnosticCounts> = None;
    for _ in 0..60 {
        let response = client
            .get(&url)
            .send()
            .await
            .context("GET /diagnostics")?
            .json::<DiagnosticsInfo>()
            .await
            .context("parse /diagnostics")?;

        last_counts = Some(response.counts);
        if last_counts.as_ref().is_some_and(|c| c.errors > 0) {
            session.cleanup().await?;
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    session.cleanup().await?;

    anyhow::bail!(
        "Expected diagnostics to appear without edits; last_counts={:?}",
        last_counts
    );
}
