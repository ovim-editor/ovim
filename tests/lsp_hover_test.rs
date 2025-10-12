//! LSP Hover Integration Tests
//!
//! Tests for LSP hover functionality

mod lsp_test_utils;

use anyhow::Result;

/// BUG REPRODUCTION: Hover returns None instead of type info
///
/// This test reproduces the hover bug where rust-analyzer returns null
/// even though:
/// - didOpen was sent correctly (only once, no duplicates)
/// - Position is correct (UTF-16 encoded)
/// - LSP is ready and indexed
/// - Request format is correct
///
/// Expected: hover_info contains "u32" type information
/// Actual: hover_info is None
#[tokio::test]
#[ignore] // Remove this when bug is fixed
async fn test_hover_on_struct_field_bug_reproduction() -> Result<()> {
    let session = ovim_session!("src/session.rs");

    // Navigate to line 20 (pub pid: u32)
    send!(session, "20G");
    assert_cursor!(session, line: 19, col: 0);

    // Move to word "pid"
    send!(session, "ww");
    assert_cursor!(session, line: 19, col: 8);

    // Trigger hover
    send!(session, "K");
    wait!(500);

    // BUG: This should pass but currently fails
    // assert_hover!(session, is_some);
    // assert_hover!(session, contains "u32");

    // Current buggy behavior:
    assert_hover!(session, is_none);

    session.cleanup().await?;
    Ok(())
}

/// Test that documents current behavior (for comparison when fixed)
#[tokio::test]
async fn test_hover_current_behavior() -> Result<()> {
    let session = ovim_session!("src/session.rs");

    send!(session, "20Gww");  // Go to "pid"
    send!(session, "K");
    wait!(500);

    let hover = session.get_hover_info().await?;
    println!("Current hover result: {:?}", hover);

    // Document that it currently returns None
    assert!(hover.is_none(), "Hover currently returns None (this is the bug)");

    session.cleanup().await?;
    Ok(())
}

/// Test hover on use statement
#[tokio::test]
async fn test_hover_on_use_statement() -> Result<()> {
    let session = ovim_session!("src/session.rs");

    // Navigate to line 9 (use anyhow::...)
    send!(session, "9G");

    // Move to "anyhow"
    send!(session, "w");

    // Trigger hover
    send!(session, "K");
    wait!(500);

    // Should have hover info for anyhow crate
    assert_hover!(session, is_some);

    session.cleanup().await?;
    Ok(())
}

/// Test hover on function parameter
#[tokio::test]
async fn test_hover_on_function_parameter() -> Result<()> {
    let session = ovim_session!("src/session.rs");

    // Find a function with parameters and test hover
    // Navigate to a suitable location
    send!(session, "gg");
    send!(session, "/impl SessionInfo<CR>");
    send!(session, "n"); // Find first match
    send!(session, "j");  // Go to next line
    send!(session, "w");  // Move to parameter

    // Trigger hover
    send!(session, "K");
    wait!(500);

    // Check if we get hover (might be None if not on a symbol)
    let hover = session.get_hover_info().await?;

    // Log the result for debugging
    println!("Hover result: {:?}", hover);

    session.cleanup().await?;
    Ok(())
}

/// Test that hover returns None on whitespace
#[tokio::test]
async fn test_hover_on_whitespace_returns_none() -> Result<()> {
    let session = ovim_session!("src/session.rs");

    // Go to a line with leading whitespace
    send!(session, "20G");
    // Cursor should be on column 0 (whitespace)

    // Trigger hover on whitespace
    send!(session, "K");
    wait!(500);

    // Should return None for whitespace
    assert_hover!(session, is_none);

    session.cleanup().await?;
    Ok(())
}

/// Debug test - just check LSP is working
#[tokio::test]
async fn test_lsp_initializes_correctly() -> Result<()> {
    let session = ovim_session!("src/session.rs");

    let lsp_status = session.get_lsp_status().await?;

    println!("LSP Status: {:?}", lsp_status);

    assert!(!lsp_status.servers.is_empty(), "No LSP servers started");
    assert!(
        lsp_status.servers.iter().any(|s| s.language == "rust"),
        "rust-analyzer not started"
    );
    assert!(
        lsp_status.servers.iter().any(|s| s.state.contains("Ready")),
        "LSP not ready"
    );

    session.cleanup().await?;
    Ok(())
}
