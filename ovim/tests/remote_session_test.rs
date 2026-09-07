//! Driving a real session over the remote transport.
//!
//! Ignored by default because it needs an editor that is already running, and
//! a test that starts one would be timing out on somebody's laptop rather than
//! testing the transport. Run it against a live session with:
//!
//! ```text
//! ovim notes.txt --headless --session demo
//! OVIM_REMOTE_SESSION="$HOME/.cache/ovim/sessions/demo.json" \
//!     cargo test --test remote_session_test -- --ignored --nocapture
//! ```
//!
//! `OVIM_REMOTE_ENDPOINT` optionally says where to dial -- `HOST:PORT`, or a
//! bare port on loopback. That is how the same test covers a forwarded port:
//! the descriptor still fixes the Host header the session insists on, while
//! the connection goes to the local end of the tunnel.

use ovim::gui::{GuiBridge, GuiKeyInput, GuiSnapshot, RemoteEndpoint, RemoteTransport};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

fn typed(key: &str) -> GuiKeyInput {
    GuiKeyInput {
        key: key.to_string(),
        shift: false,
        control: false,
        alt: false,
        meta: false,
    }
}

/// The next frame the session projects.
async fn next_frame(updates: &mut watch::Receiver<Option<GuiSnapshot>>) -> GuiSnapshot {
    loop {
        tokio::time::timeout(Duration::from_secs(10), updates.changed())
            .await
            .expect("the session should project a frame within ten seconds")
            .expect("the snapshot stream should stay open");
        let frame = updates.borrow_and_update().clone();
        if let Some(frame) = frame {
            return frame;
        }
    }
}

fn line_text(frame: &GuiSnapshot, index: usize) -> String {
    frame.lines[index]
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect()
}

#[tokio::test]
#[ignore = "needs a running headless session; see the module comment"]
async fn keystrokes_reach_a_live_session_and_its_snapshots_come_back() {
    let descriptor = PathBuf::from(
        std::env::var("OVIM_REMOTE_SESSION")
            .expect("OVIM_REMOTE_SESSION must name a session descriptor"),
    );
    let address = std::env::var("OVIM_REMOTE_ENDPOINT").ok();
    let endpoint = RemoteEndpoint::from_session_file(&descriptor, address.as_deref())
        .expect("the descriptor should describe a reachable session");

    let transport = RemoteTransport::open(endpoint)
        .await
        .expect("the session should accept the transport");
    let bridge = GuiBridge::new(Arc::new(transport));
    let mut updates = bridge.subscribe();

    // Subscribing is what makes an otherwise unwatched session project at all.
    let first = next_frame(&mut updates).await;
    println!(
        "first frame: revision {}, mode {}, {} lines, first line {:?}",
        first.revision,
        first.mode,
        first.total_lines,
        line_text(&first, 0)
    );

    // Typed rather than pasted, so this exercises the same path a keystroke
    // from a window takes.
    let baseline = line_text(&first, 0);
    for key in ["i", "r", "e", "m", "o", "t", "e", "Escape"] {
        bridge
            .key(typed(key))
            .await
            .unwrap_or_else(|error| panic!("{key} should reach the editor: {error}"));
    }

    // Compared against what the line held on arrival rather than against a
    // fixed string: a session that has been edited before is still a session,
    // and reconnecting to one is the point of the exercise.
    let mut edited = next_frame(&mut updates).await;
    while line_text(&edited, 0) == baseline {
        edited = next_frame(&mut updates).await;
    }
    println!(
        "after typing: revision {}, mode {}, first line {:?}",
        edited.revision,
        edited.mode,
        line_text(&edited, 0)
    );
    assert!(
        edited.revision > first.revision,
        "revisions must advance as the buffer changes"
    );
    assert_eq!(
        line_text(&edited, 0).len(),
        baseline.len() + "remote".len(),
        "every typed character should have landed exactly once"
    );

    // Dropping the bridge closes the stream and nothing else. The session is
    // deliberately left running: a window closing must not end it, which is
    // what a later reconnect depends on.
    drop(bridge);
}
