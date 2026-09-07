//! Bridging two machines' clipboards.
//!
//! A yank in a remote editor fills a register on the *editor's* host. The
//! clipboard the user pastes into lives on the laptop running the window, and
//! on a headless remote there is usually no clipboard at all -- `arboard` finds
//! no display, `ClipboardProvider` falls back to an in-process cache, and the
//! yank goes nowhere a browser could reach. This module carries it across, and
//! carries the laptop's clipboard the other way so `p` in a remote editor
//! pastes what the user copied here.
//!
//! **The policy is Ovim's existing one, observed rather than reinvented.**
//! Nothing here decides when a yank belongs on the system clipboard; the editor
//! already decides that from `set clipboard=`, and the only new question is
//! *which machine's* clipboard it means.
//!
//! * **Editor to frontend.** [`GuiClipboard::generation`] counts the writes the
//!   editor made to its own `+` register. It rises exactly when a local Ovim
//!   would have put the text on the user's clipboard -- a yank or delete under
//!   `unnamedplus`, or an explicit `"+y` whatever the option says -- so
//!   mirroring every rise reproduces the local behaviour and adds no case to
//!   it. A user with `set clipboard=` sees nothing arrive from a plain `y`,
//!   because the editor never wrote its own clipboard either.
//! * **Frontend to editor.** Gated on [`GuiClipboard::shared`], the same
//!   option read as a boolean. This is the direction that sends the user's
//!   clipboard to another host, and `unnamedplus` is the declaration that
//!   makes it defensible: the user has said their system clipboard *is* the
//!   editor's default register. Somebody who has not said that gets nothing.
//!
//! Both directions run in this process rather than in the webview. The client
//! already links `ovim-core`, so it has the same `arboard` the editor uses --
//! which means identical semantics to a local Ovim, no clipboard-permission
//! prompt, no dependence on a DOM gesture, and no clipboard content passing
//! through the webview or its IPC at all.

use super::protocol::GuiSnapshot;
use super::GuiBridge;
use ovim_core::editor::Editor;
use std::sync::Arc;
use std::sync::Mutex;

/// The most text either direction will carry.
///
/// A clipboard exists to be pasted somewhere by a person. 1 MiB is on the
/// order of fifteen thousand lines of source -- far past anything anyone pastes
/// into a browser or a chat window by hand -- and it is about a second on the
/// kind of uplink a laptop has, where the 20 MiB the image path allows would be
/// twenty. The asymmetry with images is deliberate: a pasted screenshot has no
/// smaller form and no other way to travel, whereas an oversized yank is still
/// sitting in the remote register with better options than the clipboard
/// (`:w` it, or pipe it). Refusing loudly beats spending a minute of the link
/// on something the user will not paste anywhere.
pub const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// What the editor would hand a frontend asking for its clipboard.
///
/// Reads what the editor last *wrote*, not what its host's clipboard holds
/// now: the frontend is mirroring this editor, and on a headless host there is
/// frequently nothing to read back.
pub fn clipboard_text(editor: &Editor) -> Result<super::GuiClipboardText, String> {
    let text = editor.registers().last_clipboard_write();
    within_limit(text)?;
    Ok(super::GuiClipboardText {
        generation: editor.registers().clipboard_generation(),
        text: text.to_string(),
    })
}

/// Refuse text the bridge will not carry, saying how far past the line it is.
pub(super) fn within_limit(text: &str) -> Result<(), String> {
    if text.len() <= MAX_CLIPBOARD_BYTES {
        return Ok(());
    }
    Err(format!(
        "The clipboard is {} bytes, past the {} MiB clipboard bridging limit",
        text.len(),
        MAX_CLIPBOARD_BYTES / (1024 * 1024)
    ))
}

/// The system clipboard of the machine the *frontend* runs on.
///
/// A trait only so the policy above can be tested without a display; the one
/// implementation that ships is [`SystemClipboard`].
pub trait LocalClipboard: Send + Sync {
    fn read(&self) -> Option<String>;
    fn write(&self, text: &str);
}

/// The real clipboard, through the same `arboard` handle the editor uses.
pub struct SystemClipboard;

impl LocalClipboard for SystemClipboard {
    fn read(&self) -> Option<String> {
        ovim_core::editor::read_system_clipboard()
    }

    fn write(&self, text: &str) {
        ovim_core::editor::write_system_clipboard(text);
    }
}

/// Keeps this machine's clipboard and a remote editor's `+` register in step.
pub struct ClipboardBridge {
    bridge: GuiBridge,
    local: Arc<dyn LocalClipboard>,
    /// The editor write generation already mirrored here.
    ///
    /// `None` until the first frame: adopting whatever the session had on
    /// arrival would mean a reconnect replaying an old yank over a clipboard
    /// the user has since filled from somewhere else.
    mirrored: Mutex<Option<u64>>,
    /// The text last pushed to the editor, so an unchanged clipboard is not
    /// sent again on every window activation.
    pushed: Mutex<Option<String>>,
}

impl ClipboardBridge {
    pub fn new(bridge: GuiBridge, local: Arc<dyn LocalClipboard>) -> Self {
        Self {
            bridge,
            local,
            mirrored: Mutex::new(None),
            pushed: Mutex::new(None),
        }
    }

    /// Copy a yank the editor just made onto this machine's clipboard.
    ///
    /// Called for every frame and does nothing for almost all of them: the
    /// generation only moves when the editor wrote its own clipboard.
    pub async fn mirror(&self, frame: &GuiSnapshot) {
        let generation = frame.clipboard.generation;
        {
            let mirrored = self.mirrored.lock().expect("the lock is never poisoned");
            // Compared for inequality rather than for being greater: a session
            // that was replaced starts counting from one again, and a client
            // that only ever moved forwards would then stop mirroring for the
            // rest of its life.
            if *mirrored == Some(generation) {
                return;
            }
            if mirrored.is_none() && generation == 0 {
                // Nothing has been yanked yet, so there is nothing to adopt.
                drop(mirrored);
                self.remember(generation);
                return;
            }
        }
        let fetched = match self.bridge.read_clipboard().await {
            Ok(fetched) => fetched,
            Err(error) => {
                // A refusal is either the size limit or a link that has just
                // gone; both are worth a line in the log and neither is worth
                // retrying, because the next frame asks again anyway.
                ovim_core::log_warn!("gui", "Could not mirror the remote clipboard: {}", error);
                return;
            }
        };
        // The generation the answer carried, not the one the frame did: a
        // second yank between the two would otherwise be recorded as mirrored
        // when it was the first one that arrived.
        self.remember(fetched.generation);
        // Skip a write that would change nothing. This is what keeps the two
        // directions from chasing each other -- a push into the editor's `+`
        // register raises its generation, and the mirror that follows finds
        // the text already here.
        if self.local.read().as_deref() == Some(fetched.text.as_str()) {
            return;
        }
        self.local.write(&fetched.text);
    }

    /// Send this machine's clipboard to the editor's `+` register.
    ///
    /// Called when the window is activated, which is the one moment between
    /// "copied something in a browser" and "pressed `p` in the editor" that
    /// this process can see. `p` cannot ask for it at the time -- the snapshot
    /// stream is the only channel running the other way and it carries frames,
    /// not questions -- so the sync point is the focus event instead.
    pub async fn push(&self, frame: &GuiSnapshot) {
        if !frame.clipboard.shared {
            // `set clipboard=` means the user did not ask for their system
            // clipboard to be the editor's register. Moving it to another host
            // uninvited would be a far larger step than doing it locally.
            return;
        }
        let Some(text) = self.local.read() else {
            return;
        };
        if text.len() > MAX_CLIPBOARD_BYTES {
            ovim_core::log_warn!(
                "gui",
                "The local clipboard is {} bytes, past the clipboard bridging limit",
                text.len()
            );
            return;
        }
        {
            let mut pushed = self.pushed.lock().expect("the lock is never poisoned");
            if pushed.as_deref() == Some(text.as_str()) {
                return;
            }
            *pushed = Some(text.clone());
        }
        if let Err(error) = self.bridge.write_clipboard(text).await {
            // Not queued for a retry. A link that is down drops input rather
            // than replaying it, and a clipboard is input: the next activation
            // sends whatever is on the clipboard then, which is the value the
            // user would expect anyway.
            self.pushed
                .lock()
                .expect("the lock is never poisoned")
                .take();
            ovim_core::log_warn!(
                "gui",
                "Could not send the clipboard to the editor: {}",
                error
            );
        }
    }

    /// [`ClipboardBridge::push`], against whatever frame is current.
    ///
    /// The window's activation handler has no frame in hand, and the newest
    /// one is always a `borrow` away.
    pub async fn push_latest(&self) {
        let frame = self.bridge.subscribe().borrow().clone();
        if let Some(frame) = frame {
            self.push(&frame).await;
        }
    }

    fn remember(&self, generation: u64) {
        *self.mirrored.lock().expect("the lock is never poisoned") = Some(generation);
    }
}

/// A bridge, and the loop that follows the editor's frames for it.
///
/// `None` for an in-process editor, which needs no bridge at all: it writes the
/// very clipboard this process would write, through the same `arboard` handle,
/// and a second writer could only get in its way.
///
/// The loop is handed back rather than spawned here so the caller can put it on
/// whatever runtime it has -- the shell's `setup` hook has no ambient one.
pub fn start(
    bridge: &GuiBridge,
    local: Arc<dyn LocalClipboard>,
) -> Option<(
    Arc<ClipboardBridge>,
    impl std::future::Future<Output = ()> + Send + 'static,
)> {
    if !bridge.is_remote() {
        return None;
    }
    let clipboard = Arc::new(ClipboardBridge::new(bridge.clone(), local));
    let mut frames = bridge.subscribe();
    let following = Arc::clone(&clipboard);
    let follow = async move {
        while frames.changed().await.is_ok() {
            let frame = frames.borrow_and_update().clone();
            if let Some(frame) = frame {
                following.mirror(&frame).await;
            }
        }
    };
    Some((clipboard, follow))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::bridge::tests::RecordingTransport;
    use crate::gui::protocol::{GuiClipboard, GuiClipboardText, GuiCommand, GuiReply};
    use std::sync::Mutex as StdMutex;

    /// A clipboard in memory, standing in for one on a screen.
    #[derive(Default)]
    struct FakeClipboard {
        held: StdMutex<Option<String>>,
        writes: StdMutex<u32>,
    }

    impl FakeClipboard {
        fn holding(text: &str) -> Self {
            Self {
                held: StdMutex::new(Some(text.to_string())),
                writes: StdMutex::new(0),
            }
        }

        fn contents(&self) -> Option<String> {
            self.held.lock().unwrap().clone()
        }

        fn writes(&self) -> u32 {
            *self.writes.lock().unwrap()
        }
    }

    impl LocalClipboard for FakeClipboard {
        fn read(&self) -> Option<String> {
            self.held.lock().unwrap().clone()
        }

        fn write(&self, text: &str) {
            *self.held.lock().unwrap() = Some(text.to_string());
            *self.writes.lock().unwrap() += 1;
        }
    }

    /// A frame carrying nothing but a clipboard policy.
    fn frame(shared: bool, generation: u64) -> GuiSnapshot {
        let mut snapshot = super::super::snapshot(&Editor::with_content("fn main() {}\n"), 1, 0);
        snapshot.clipboard = GuiClipboard { shared, generation };
        snapshot
    }

    /// A bridge whose editor answers `ReadClipboard` with `text`.
    fn bridged(text: &str, generation: u64) -> (GuiBridge, Arc<RecordingTransport>) {
        let answer = GuiClipboardText {
            generation,
            text: text.to_string(),
        };
        let transport = Arc::new(RecordingTransport::remote_answering(move |command| {
            matches!(command, GuiCommand::ReadClipboard)
                .then(|| GuiReply::Clipboard(Ok(answer.clone())))
        }));
        (GuiBridge::new(transport.clone()), transport)
    }

    #[tokio::test]
    async fn a_yank_on_the_editor_reaches_this_machine_s_clipboard() {
        let local = Arc::new(FakeClipboard::default());
        let (bridge, _transport) = bridged("yanked line\n", 1);
        let clipboard = ClipboardBridge::new(bridge, local.clone());

        clipboard.mirror(&frame(true, 1)).await;

        assert_eq!(local.contents().as_deref(), Some("yanked line\n"));
    }

    #[tokio::test]
    async fn a_frame_that_yanked_nothing_new_costs_no_round_trip() {
        // Every frame passes through here, so the common case has to be free.
        let local = Arc::new(FakeClipboard::default());
        let (bridge, transport) = bridged("yanked line\n", 1);
        let clipboard = ClipboardBridge::new(bridge, local.clone());

        clipboard.mirror(&frame(true, 1)).await;
        for _ in 0..20 {
            clipboard.mirror(&frame(true, 1)).await;
        }

        assert_eq!(
            transport
                .received()
                .iter()
                .filter(|command| matches!(command, GuiCommand::ReadClipboard))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_session_that_started_counting_again_is_still_mirrored() {
        // A replaced session's generation restarts at one, so a client that
        // only ever accepted a higher number would go quiet for good -- the
        // same trap `Link::publish` guards the revision against.
        let local = Arc::new(FakeClipboard::default());
        let (bridge, _transport) = bridged("after the restart", 1);
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        clipboard.remember(9);

        clipboard.mirror(&frame(true, 1)).await;

        assert_eq!(local.contents().as_deref(), Some("after the restart"));
    }

    #[tokio::test]
    async fn a_clipboard_that_already_holds_the_text_is_left_alone() {
        // Pushing this machine's clipboard raises the editor's generation, so
        // the mirror that follows must recognise its own text rather than
        // writing it back and disturbing a clipboard manager for nothing.
        let local = Arc::new(FakeClipboard::holding("already here"));
        let (bridge, _transport) = bridged("already here", 4);
        let clipboard = ClipboardBridge::new(bridge, local.clone());

        clipboard.mirror(&frame(true, 4)).await;

        assert_eq!(local.writes(), 0);
    }

    #[tokio::test]
    async fn nothing_is_mirrored_from_a_session_that_has_never_yanked() {
        let local = Arc::new(FakeClipboard::default());
        let (bridge, transport) = bridged("", 0);
        let clipboard = ClipboardBridge::new(bridge, local.clone());

        clipboard.mirror(&frame(true, 0)).await;

        assert!(transport.received().is_empty());
        assert_eq!(local.contents(), None);
    }

    #[tokio::test]
    async fn this_machine_s_clipboard_reaches_the_editor_when_the_option_asks_for_it() {
        let local = Arc::new(FakeClipboard::holding("copied in a browser"));
        let (bridge, transport) = bridged("", 0);
        let clipboard = ClipboardBridge::new(bridge, local);

        clipboard.push(&frame(true, 0)).await;

        assert_eq!(
            transport.received(),
            vec![GuiCommand::WriteClipboard {
                text: "copied in a browser".to_string()
            }]
        );
    }

    #[tokio::test]
    async fn a_user_who_did_not_opt_in_locally_does_not_opt_in_by_going_remote() {
        // `set clipboard=` is a decision about the system clipboard, and the
        // editor being on another host does not reopen it.
        let local = Arc::new(FakeClipboard::holding("a password, for all this knows"));
        let (bridge, transport) = bridged("", 0);
        let clipboard = ClipboardBridge::new(bridge, local);

        clipboard.push(&frame(false, 0)).await;

        assert!(transport.received().is_empty());
    }

    #[tokio::test]
    async fn an_unchanged_clipboard_is_not_sent_again_on_every_activation() {
        let local = Arc::new(FakeClipboard::holding("copied once"));
        let (bridge, transport) = bridged("", 0);
        let clipboard = ClipboardBridge::new(bridge, local);

        for _ in 0..5 {
            clipboard.push(&frame(true, 0)).await;
        }

        assert_eq!(transport.received().len(), 1);
    }

    #[tokio::test]
    async fn a_clipboard_too_large_to_carry_is_refused_rather_than_sent() {
        let oversized = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
        let local = Arc::new(FakeClipboard::holding(&oversized));
        let (bridge, transport) = bridged("", 0);
        let clipboard = ClipboardBridge::new(bridge, local);

        clipboard.push(&frame(true, 0)).await;

        assert!(transport.received().is_empty());
    }

    #[test]
    fn an_editor_running_in_this_process_gets_no_bridge_at_all() {
        // It writes the very clipboard this process would write, through the
        // same `arboard` handle. A second writer could only get in its way.
        let bridge = GuiBridge::new(RecordingTransport::new());

        assert!(start(&bridge, Arc::new(FakeClipboard::default())).is_none());
    }

    #[tokio::test]
    async fn an_editor_on_another_host_is_followed_without_anyone_asking() {
        let (bridge, _transport) = bridged("yanked line\n", 1);

        let (clipboard, _follow) = start(&bridge, Arc::new(FakeClipboard::default()))
            .expect("a remote editor needs the bridge");

        // The handle is what the window's activation handler pushes through.
        clipboard.push_latest().await;
    }

    #[test]
    fn a_yank_past_the_limit_is_refused_where_the_text_is() {
        // The bound has to hold on the editor's side too: a client cannot
        // decline a payload it has already been sent. Checked here rather than
        // through a register, because seeding one would put a megabyte on the
        // machine's real clipboard.
        assert!(within_limit(&"y".repeat(MAX_CLIPBOARD_BYTES)).is_ok());

        let error = within_limit(&"y".repeat(MAX_CLIPBOARD_BYTES + 1)).unwrap_err();

        assert!(error.contains("limit"), "{error}");
    }

    #[test]
    fn what_an_editor_hands_over_is_what_it_wrote_not_what_its_host_holds() {
        // On a headless host `get_clipboard` reads back nothing at all, so
        // reading the provider's own record is the only way the yank survives.
        let mut editor = Editor::with_content("fn main() {}\n");
        editor.registers_mut().set_clipboard("yanked".to_string());

        let handed = clipboard_text(&editor).unwrap();

        assert_eq!(handed.text, "yanked");
        assert_eq!(handed.generation, 1);
    }
}
