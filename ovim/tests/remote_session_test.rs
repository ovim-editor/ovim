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
//! They all drive the one session the variable names, and some of them type
//! into it, so run them one at a time against a session started for the
//! purpose -- two of them editing one buffer at once is not a failure of
//! either.
//!
//! `OVIM_REMOTE_ENDPOINT` optionally says where to dial -- `HOST:PORT`, or a
//! bare port on loopback. That is how the same test covers a forwarded port:
//! the descriptor still fixes the Host header the session insists on, while
//! the connection goes to the local end of the tunnel.

use ovim::gui::{
    GuiBridge, GuiConnection, GuiKeyInput, GuiSnapshot, GuiTransport, RemoteEndpoint,
    RemoteTransport,
};
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

/// What a keystroke costs when nothing speaks for it locally.
///
/// The number predictive echo exists to hide, measured through the real
/// transport rather than modelled: from handing a key to the bridge to the
/// frame that shows it coming back. Set `OVIM_REMOTE_LATENCY_MS` to make a
/// loopback session feel like a distant one -- the transport applies half of it
/// in each direction -- and the reading should come out at roughly that,
/// whatever the editor itself costs on top.
///
/// The other half of the comparison is in the frontend, where the drawing
/// happens: `ovim/gui/src/predictiveEcho.measure.test.ts` reads the same
/// variable and times the same sample with the speculation switched on.
#[tokio::test]
#[ignore = "needs a running headless session; see the module comment"]
async fn a_keystroke_costs_a_round_trip_when_nothing_speaks_for_it() {
    let descriptor = PathBuf::from(
        std::env::var("OVIM_REMOTE_SESSION")
            .expect("OVIM_REMOTE_SESSION must name a session descriptor"),
    );
    let address = std::env::var("OVIM_REMOTE_ENDPOINT").ok();
    let endpoint = RemoteEndpoint::from_session_file(&descriptor, address.as_deref())
        .expect("the descriptor should describe a reachable session");
    let bridge = GuiBridge::new(Arc::new(
        RemoteTransport::open(endpoint)
            .await
            .expect("the session should accept the transport"),
    ));
    let mut updates = bridge.subscribe();
    let first = next_frame(&mut updates).await;

    bridge
        .key(typed("i"))
        .await
        .expect("insert mode should be reachable");

    let mut samples = Vec::new();
    let mut baseline = line_text(&first, 0);
    for key in ["e", "c", "h", "o"] {
        let pressed = std::time::Instant::now();
        bridge
            .key(typed(key))
            .await
            .unwrap_or_else(|error| panic!("{key} should reach the editor: {error}"));
        loop {
            let frame = next_frame(&mut updates).await;
            let line = line_text(&frame, 0);
            if line != baseline {
                baseline = line;
                break;
            }
        }
        samples.push(pressed.elapsed().as_secs_f64() * 1000.0);
    }

    bridge.key(typed("Escape")).await.expect("insert mode ends");
    println!(
        "keypress to visible over the real transport, no speculation: {samples:.1?}ms \
         (session revision {} at the start)",
        first.revision
    );
    drop(bridge);
}

/// A forwarder in front of the session, which this test can kill and restart.
///
/// `socat` rather than an in-process proxy: the failure being reproduced is a
/// separate process carrying the link dying, which is what an `ssh -L` forward
/// is, and a proxy inside the test process could not die the same way.
struct Socat {
    port: u16,
    child: Option<std::process::Child>,
    session_port: u16,
}

impl Socat {
    fn in_front_of(session_port: u16) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("a loopback port should be reservable");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut forwarder = Self {
            port,
            child: None,
            session_port,
        };
        forwarder.start();
        forwarder
    }

    fn start(&mut self) {
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new("socat")
            .arg(format!("TCP-LISTEN:{},fork,reuseaddr", self.port))
            .arg(format!("TCP:127.0.0.1:{}", self.session_port))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            // Its own process group, because `fork` means the connection in
            // flight is held by a child. Killing only the listener would leave
            // the established stream alive, which is not what a dead forward
            // does.
            .process_group(0)
            .spawn()
            .expect("socat should be installed to run this test");
        self.child = Some(child);
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        for _ in 0..100 {
            if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("socat never started listening on {}", self.port);
    }

    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        // The whole group: the listener and every connection it forked.
        // Killing only the listener would leave the established stream alive,
        // which is not what a dead forward does. Through `sh` because a
        // negative pid means "process group" to the shell's builtin kill and
        // not to every kill(1) on a machine.
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("kill -- -{}", child.id()))
            .stderr(std::process::Stdio::null())
            .status();
        let _ = child.kill();
        let _ = child.wait();
        // The group is signalled, not reaped; give the sockets a moment to go.
        std::thread::sleep(Duration::from_millis(200));
    }
}

impl Drop for Socat {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn connection_reaches(
    connection: &mut watch::Receiver<GuiConnection>,
    ready: impl Fn(&GuiConnection) -> bool,
) -> GuiConnection {
    for _ in 0..600 {
        let state = connection.borrow_and_update().clone();
        if ready(&state) {
            return state;
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), connection.changed()).await;
    }
    panic!(
        "the connection never reached the expected state (last: {:?})",
        connection.borrow().clone()
    );
}

#[tokio::test]
#[ignore = "needs a running headless session and socat; see the module comment"]
async fn a_forward_that_dies_and_comes_back_leaves_the_session_untouched() {
    // The whole reason the editor lives on the far side of the link. The
    // forward is killed the way a closed lid kills one, the session keeps
    // being edited while nobody can see it, and the client has to come back to
    // that same session rather than to a fresh one.
    let descriptor = PathBuf::from(
        std::env::var("OVIM_REMOTE_SESSION")
            .expect("OVIM_REMOTE_SESSION must name a session descriptor"),
    );
    let session: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&descriptor).expect("a readable descriptor"))
            .expect("a descriptor is JSON");
    let session_port = session["port"].as_u64().expect("a port") as u16;
    let mut forwarder = Socat::in_front_of(session_port);

    let endpoint =
        RemoteEndpoint::from_session_file(&descriptor, Some(&forwarder.port.to_string()))
            .expect("the descriptor should describe a reachable session");
    let transport = RemoteTransport::open(endpoint)
        .await
        .expect("the session should accept the transport through the forwarder");
    let mut connection = transport.connection();
    let bridge = GuiBridge::new(Arc::new(transport));
    let mut updates = bridge.subscribe();
    let first = next_frame(&mut updates).await;
    println!(
        "connected through port {}: revision {}, first line {:?}",
        forwarder.port,
        first.revision,
        line_text(&first, 0)
    );

    forwarder.stop();
    let down = connection_reaches(&mut connection, |state| !state.is_connected()).await;
    println!("forward killed: {down:?}");
    // A key typed now must be dropped, not queued: replaying it later would
    // apply it to a buffer that has moved underneath.
    let refused = bridge.key(typed("x")).await.unwrap_err();
    println!("a keystroke during the outage: {refused}");

    // The session carries on regardless, which is the property being proved.
    let direct = RemoteTransport::open(
        RemoteEndpoint::from_session_file(&descriptor, None)
            .expect("the session is reachable on its own port"),
    )
    .await
    .expect("the session should still be running with nobody watching");
    let aside = GuiBridge::new(Arc::new(direct));
    for key in ["i", "l", "i", "v", "e", "Escape"] {
        aside
            .key(typed(key))
            .await
            .unwrap_or_else(|error| panic!("{key} should reach the session: {error}"));
    }
    drop(aside);

    forwarder.start();
    let back = connection_reaches(&mut connection, GuiConnection::is_connected).await;
    println!("forward restarted: {back:?}");

    let mut recovered = next_frame(&mut updates).await;
    while !line_text(&recovered, 0).contains("live") {
        recovered = next_frame(&mut updates).await;
    }
    println!(
        "after reconnecting: revision {}, first line {:?}",
        recovered.revision,
        line_text(&recovered, 0)
    );
    assert!(
        recovered.revision >= first.revision,
        "revisions must not go backwards across a reconnect: {} then {}",
        first.revision,
        recovered.revision
    );
    // And typing works again, against the very same session. `gg` rather than
    // anything that edits: this test shares its session with the one above,
    // and leaving the cursor where it found it is the neighbourly thing to do.
    for key in ["g", "g"] {
        bridge
            .key(typed(key))
            .await
            .expect("the reattached session should take keys again");
    }
}

/// A clipboard in memory, standing in for the laptop's.
///
/// Deliberately not the real one. The session in this test runs on the same
/// machine as the test, so a real clipboard on both ends would be one
/// clipboard, and the bridge would appear to work while doing nothing.
#[derive(Default)]
struct TestClipboard {
    held: std::sync::Mutex<Option<String>>,
}

impl TestClipboard {
    fn contents(&self) -> Option<String> {
        self.held.lock().unwrap().clone()
    }

    fn put(&self, text: &str) {
        *self.held.lock().unwrap() = Some(text.to_string());
    }
}

impl ovim::gui::clipboard::LocalClipboard for TestClipboard {
    fn read(&self) -> Option<String> {
        self.held.lock().unwrap().clone()
    }

    fn write(&self, text: &str) {
        *self.held.lock().unwrap() = Some(text.to_string());
    }
}

/// Wait until this machine's clipboard holds something matching `wanted`.
async fn clipboard_reaches(local: &TestClipboard, wanted: impl Fn(&str) -> bool) -> String {
    for _ in 0..200 {
        if let Some(held) = local.contents() {
            if wanted(&held) {
                return held;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "the clipboard never reached the expected contents (last: {:?})",
        local.contents()
    );
}

#[tokio::test]
#[ignore = "needs a running headless session and socat; see the module comment"]
async fn a_yank_crosses_the_link_to_this_clipboard_and_this_clipboard_crosses_back() {
    // The papercut this chunk exists for, end to end and through a real
    // forward: yank on the session and the text is on this machine's
    // clipboard; copy on this machine and `p` on the session pastes it.
    let descriptor = PathBuf::from(
        std::env::var("OVIM_REMOTE_SESSION")
            .expect("OVIM_REMOTE_SESSION must name a session descriptor"),
    );
    let session: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&descriptor).expect("a readable descriptor"))
            .expect("a descriptor is JSON");
    let session_port = session["port"].as_u64().expect("a port") as u16;
    let forwarder = Socat::in_front_of(session_port);

    let endpoint =
        RemoteEndpoint::from_session_file(&descriptor, Some(&forwarder.port.to_string()))
            .expect("the descriptor should describe a reachable session");
    let bridge =
        GuiBridge::new(Arc::new(RemoteTransport::open(endpoint).await.expect(
            "the session should accept the transport through the forwarder",
        )));
    let local = Arc::new(TestClipboard::default());
    let (clipboard, follow) = ovim::gui::clipboard::start(&bridge, local.clone())
        .expect("a session on the far side of a transport needs the bridge");
    tokio::spawn(follow);
    let mut updates = bridge.subscribe();
    let first = next_frame(&mut updates).await;
    println!(
        "connected through port {}: first line {:?}, clipboard {:?}",
        forwarder.port,
        line_text(&first, 0),
        first.clipboard
    );
    assert!(
        first.clipboard.shared,
        "this test assumes the session's default clipboard=unnamedplus"
    );

    // Session to laptop: yank the first line and watch it arrive here.
    for key in ["Escape", "g", "g", "y", "y"] {
        bridge
            .key(typed(key))
            .await
            .unwrap_or_else(|error| panic!("{key} should reach the session: {error}"));
    }
    let arrived = clipboard_reaches(&local, |held| held.contains("yank me")).await;
    println!("the session's yank reached this machine: {arrived:?}");

    // Laptop to session: copy here, hand it over on activation, and paste.
    local.put("pasted from the laptop");
    clipboard.push_latest().await;
    for key in ["G", "p"] {
        bridge
            .key(typed(key))
            .await
            .unwrap_or_else(|error| panic!("{key} should reach the session: {error}"));
    }
    let mut pasted = next_frame(&mut updates).await;
    for _ in 0..40 {
        if (0..pasted.lines.len())
            .any(|index| line_text(&pasted, index).contains("pasted from the laptop"))
        {
            println!("this machine's clipboard was pasted into the session");
            // Put the buffer and the cursor back: these tests may share one
            // session, and one that edits has to leave what it found.
            for key in ["u", "g", "g"] {
                bridge
                    .key(typed(key))
                    .await
                    .expect("the session should take the tidying up");
            }
            return;
        }
        pasted = next_frame(&mut updates).await;
    }
    panic!(
        "the session never pasted this machine's clipboard: {:?}",
        (0..pasted.lines.len())
            .map(|index| line_text(&pasted, index))
            .collect::<Vec<_>>()
    );
}
