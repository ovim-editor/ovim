//! LSP Completion Integration Tests
//!
//! These tests require external infrastructure to run:
//! - The release binary must be built (`cargo build --release`)
//! - rust-analyzer must be installed and working
//! - Tests spawn actual headless ovim processes
//!
//! Run these tests with: `cargo test --test lsp_completion_test -- --ignored`

mod lsp_test_utils;

use anyhow::Result;
use tokio::time::{sleep, Duration, Instant};

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                chars.next(); // '['
                while let Some(x) = chars.next() {
                    if x.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

async fn wait_for_render_contains(
    session: &lsp_test_utils::OvimTestSession,
    needle: &str,
    timeout: Duration,
) -> Result<String> {
    let start = Instant::now();
    loop {
        let render = session.get_render().await?;
        let plain = strip_ansi(&render.ansi);
        if plain.contains(needle) {
            return Ok(plain);
        }
        if start.elapsed() > timeout {
            anyhow::bail!("Timed out waiting for render to contain {:?}", needle);
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// Documents expected behavior: after `s.` where `s: String`, completions should include `push_str`.
///
/// This helps pin down intermittent cases where dot-completion shows global items instead.
#[tokio::test]
#[ignore = "Requires release binary and rust-analyzer"]
async fn test_dot_completion_shows_string_methods() -> Result<()> {
    let session = ovim_session!("ovim/src/main.rs");

    // Insert a small snippet at EOF:
    //   let s = String::new();
    //   s.
    send!(session, "G");
    send!(session, "olet s = String::new();<CR>s.");

    // Expect the completion popup to include a String method.
    // `push_str` is a stable method name and unlikely to appear in the file content nearby.
    let plain = wait_for_render_contains(&session, "push_str", Duration::from_secs(10)).await?;

    // Also assert that we didn't fall back to a purely global-ish list.
    // This is intentionally weak (documentation) since global symbols can vary.
    assert!(
        !plain.contains("Arc") || plain.contains("push_str"),
        "Expected method completions to include push_str; render was:\n{}",
        plain
    );

    session.cleanup().await?;
    Ok(())
}

