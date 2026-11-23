//! Bridging two machines' clipboards.
//!
//! A yank in a remote editor fills a register on the *editor's* host. The
//! clipboard the user pastes into lives on the laptop running the window, and
//! on a headless remote there is usually no clipboard at all. This module
//! carries an editor's yank to the laptop, and carries the laptop's clipboard
//! to the editor when -- and only when -- the user pastes from it.
//!
//! Both directions are decided here, in the window, from facts the remote
//! cannot fake. A session that has been compromised can still lie about
//! everything it sends; what it cannot do is make this window type keys.
//!
//! * **Frontend to editor.** The laptop's clipboard is sent only immediately
//!   ahead of a key the user typed to paste from it: `"+` or `"*` followed by
//!   a put (`p`, `P`, `gp`, `gP`, `]p`, `[p`, ...), `Ctrl-R +` / `Ctrl-R *` in
//!   Insert or command-line mode, or `:put +` / `:put *`. The paste key and the
//!   Edit menu's Paste carry their text themselves. Nothing is sent on focus,
//!   and no option on the remote side can turn sending on. See
//!   [`PasteDetector`].
//! * **Editor to frontend.** [`GuiClipboard::generation`] counts the editor's
//!   writes to its `+` register, and the session says which client's command
//!   made the latest one ([`GuiClipboard::writer`]). A write is copied here
//!   only if this window made it, after this window attached, and this window
//!   has sent a command since the last write it copied. The first frame after
//!   every connect or reconnect is only a baseline.
//!
//! Both directions run in this process rather than in the webview, through the
//! same `arboard` the editor uses: no clipboard-permission prompt, no DOM
//! gesture, and no clipboard content passing through the webview's IPC.

use super::GuiBridge;
use super::GuiSnapshot;
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
    within_limit(&text)?;
    Ok(super::GuiClipboardText {
        generation: editor.registers().clipboard_generation(),
        text,
        writer: None,
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

/// Recognises, from the keys the user types, the ones that paste from the
/// system clipboard.
///
/// It reads only what this window sends and the mode of the latest frame. The
/// mode decides between Normal-mode register syntax and text entry; a remote
/// that lies about it can at most make a sequence the user typed -- `"+p`, or
/// `Ctrl-R +` -- count as a paste in the other mode. It cannot make an
/// ordinary key one.
///
/// Plain `p` is deliberately not a paste here, even under `clipboard=unnamedplus`:
/// that option lives on the remote side, and honouring it would let the remote
/// decide when this machine's clipboard leaves it.
#[derive(Debug, Default)]
pub struct PasteDetector {
    register: RegisterState,
    /// `Ctrl-R` was typed in a text-entry mode; the next `+` or `*` pastes.
    ctrl_r: bool,
    /// An Ex command line typed after `:`, as far as this window can follow
    /// it; `None` once it has been edited in a way it cannot (history, `Tab`).
    command_line: Option<Option<String>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum RegisterState {
    #[default]
    None,
    /// `"` typed; the next key names a register.
    Naming,
    /// `"+` or `"*` typed, plus an optional `g`, `[` or `]` prefix.
    Clipboard { prefix: Option<char> },
}

impl PasteDetector {
    /// Feed one key the user typed. True when that key pastes from the
    /// system clipboard and the clipboard must reach the editor first.
    pub fn observe(&mut self, input: &super::GuiKeyInput, mode: &str) -> bool {
        let plain = !input.control && !input.alt && !input.meta;
        let character = {
            let mut chars = input.key.chars();
            match (chars.next(), chars.next()) {
                (Some(character), None) => Some(character),
                _ => None,
            }
        };
        let control = |letter: char| {
            input.control
                && !input.alt
                && !input.meta
                && character.is_some_and(|c| c.eq_ignore_ascii_case(&letter))
        };

        if self.ctrl_r {
            // `Ctrl-R Ctrl-R`, `Ctrl-R Ctrl-O` and `Ctrl-R Ctrl-P` still name a
            // register next.
            if control('r') || control('o') || control('p') {
                return false;
            }
            self.ctrl_r = false;
            let pastes = plain && matches!(character, Some('+' | '*'));
            if let Some(Some(line)) = self.command_line.as_mut() {
                // Whatever the register held is now on the line; it can no
                // longer be read from here.
                if pastes {
                    self.command_line = Some(None);
                } else if let Some(c) = character.filter(|_| plain) {
                    line.push(c);
                }
            }
            return pastes;
        }

        if let Some(line) = self.command_line.as_mut() {
            if control('r') {
                self.ctrl_r = true;
                return false;
            }
            match input.key.as_str() {
                "Enter" => {
                    let pastes = line.as_deref().is_some_and(puts_from_clipboard);
                    self.command_line = None;
                    return pastes;
                }
                "Escape" => self.command_line = None,
                "Backspace" => match line {
                    Some(text) if text.is_empty() => self.command_line = None,
                    Some(text) => {
                        text.pop();
                    }
                    None => {}
                },
                _ => match (character, line.as_mut()) {
                    (Some(c), Some(text)) if plain => text.push(c),
                    _ => *line = None,
                },
            }
            return false;
        }

        match mode {
            "INSERT" | "REPLACE" | "COMMAND" | "SEARCH" => {
                self.register = RegisterState::None;
                if control('r') {
                    self.ctrl_r = true;
                }
                false
            }
            "NORMAL" | "VISUAL" | "VISUAL_LINE" | "VISUAL_BLOCK" => {
                if !plain {
                    self.register = RegisterState::None;
                    return false;
                }
                let Some(c) = character else {
                    self.register = RegisterState::None;
                    return false;
                };
                match (self.register, c) {
                    (RegisterState::Naming, '+' | '*') => {
                        self.register = RegisterState::Clipboard { prefix: None };
                        false
                    }
                    (RegisterState::Clipboard { prefix: None }, '0'..='9') => false,
                    (RegisterState::Clipboard { prefix: None }, 'g' | '[' | ']') => {
                        self.register = RegisterState::Clipboard { prefix: Some(c) };
                        false
                    }
                    (RegisterState::Clipboard { .. }, 'p' | 'P') => {
                        self.register = RegisterState::None;
                        true
                    }
                    (_, '"') => {
                        self.register = RegisterState::Naming;
                        false
                    }
                    (RegisterState::None, ':') => {
                        self.command_line = Some(Some(String::new()));
                        false
                    }
                    _ => {
                        self.register = RegisterState::None;
                        false
                    }
                }
            }
            _ => {
                self.register = RegisterState::None;
                false
            }
        }
    }
}

/// Whether an Ex command line is `:[range]pu[t][!] +` or `*`.
fn puts_from_clipboard(line: &str) -> bool {
    let rest = line
        .trim_start()
        .trim_start_matches(|c: char| c.is_ascii_digit() || ".,;$%'<>-".contains(c));
    let Some(rest) = rest.strip_prefix("pu") else {
        return false;
    };
    let rest = rest.strip_prefix('t').unwrap_or(rest);
    let rest = rest.strip_prefix('!').unwrap_or(rest);
    matches!(rest.trim(), "+" | "*")
}

/// Keeps this machine's clipboard and a remote editor's `+` register in step.
pub struct ClipboardBridge {
    bridge: GuiBridge,
    local: Arc<dyn LocalClipboard>,
    /// The editor write generation already accounted for, and how many
    /// commands this window had sent at that point.
    mirrored: Mutex<Option<(u64, u64)>>,
    /// The next frame is the first since a connect or reconnect, and only
    /// records where the session's clipboard stands.
    baseline: std::sync::atomic::AtomicBool,
    detector: Mutex<PasteDetector>,
}

impl ClipboardBridge {
    pub fn new(bridge: GuiBridge, local: Arc<dyn LocalClipboard>) -> Self {
        Self {
            bridge,
            local,
            mirrored: Mutex::new(None),
            baseline: std::sync::atomic::AtomicBool::new(true),
            detector: Mutex::new(PasteDetector::default()),
        }
    }

    /// The link was just (re)established: the next frame is a baseline.
    pub fn attached(&self) {
        self.baseline
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Copy a yank this window just made onto this machine's clipboard.
    ///
    /// Called for every frame and does nothing for almost all of them.
    pub async fn mirror(&self, frame: &GuiSnapshot) {
        let generation = frame.clipboard.generation;
        if self
            .baseline
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            // Attaching never writes this machine's clipboard: whatever the
            // session yanked before this window was looking is not something
            // this window asked for.
            self.remember(generation);
            return;
        }
        let acted_since = {
            let mirrored = self.mirrored.lock().expect("the lock is never poisoned");
            // Compared for inequality rather than for being greater: a session
            // that was replaced starts counting from one again.
            if mirrored.map(|(seen, _)| seen) == Some(generation) {
                return;
            }
            mirrored.is_none_or(|(_, commands)| commands != self.bridge.commands_sent())
        };
        let me = self.bridge.client_id();
        if me.is_none() || frame.clipboard.writer != me || !acted_since {
            // Somebody else's write, or one this window did nothing to cause.
            self.remember(generation);
            return;
        }
        let fetched = match self.bridge.read_clipboard().await {
            Ok(fetched) => fetched,
            Err(error) => {
                // A refusal is either the size limit or a link that has just
                // gone; neither is worth retrying, because the next frame
                // asks again anyway.
                ovim_core::log_warn!("gui", "Could not mirror the remote clipboard: {}", error);
                return;
            }
        };
        // The generation the answer carried, not the one the frame did: a
        // second write between the two must be judged on its own.
        self.remember(fetched.generation);
        if fetched.writer != me {
            return;
        }
        // The session enforces the limit too, but this is the side whose
        // clipboard it is.
        if let Err(error) = within_limit(&fetched.text) {
            ovim_core::log_warn!("gui", "Refused the remote clipboard: {}", error);
            return;
        }
        // Skip a write that would change nothing, which is what a paste this
        // window sent produces when its generation comes back.
        if self.local.read().as_deref() == Some(fetched.text.as_str()) {
            return;
        }
        self.local.write(&fetched.text);
    }

    /// Deliver a key the user typed, with this machine's clipboard ahead of it
    /// when the key pastes from the clipboard.
    ///
    /// Costs one extra command in the ordered lane for a paste, and nothing
    /// for any other key.
    pub async fn key(&self, input: super::GuiKeyInput) -> Result<(), String> {
        let mode = self
            .bridge
            .subscribe()
            .borrow()
            .as_ref()
            .map(|frame| frame.mode.clone())
            .unwrap_or_default();
        let pastes = self
            .detector
            .lock()
            .expect("the lock is never poisoned")
            .observe(&input, &mode);
        if pastes {
            match self.local.read() {
                Some(text) if within_limit(&text).is_ok() => {
                    return self.bridge.clipboard_then_key(text, input).await;
                }
                Some(text) => ovim_core::log_warn!(
                    "gui",
                    "The local clipboard is {} bytes, past the clipboard bridging limit",
                    text.len()
                ),
                None => {}
            }
        }
        self.bridge.key(input).await
    }

    fn remember(&self, generation: u64) {
        *self.mirrored.lock().expect("the lock is never poisoned") =
            Some((generation, self.bridge.commands_sent()));
    }
}

/// A bridge, and the loop that follows the editor's frames for it.
///
/// `None` for an in-process editor, which needs no bridge at all: it writes the
/// very clipboard this process would write, through the same `arboard` handle,
/// and a second writer could only get in its way.
///
/// The loop is handed back rather than spawned here so the caller can put it on
/// whatever runtime it has.
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
    let mut connection = bridge.connection();
    let following = Arc::clone(&clipboard);
    let follow = async move {
        while frames.changed().await.is_ok() {
            // The link is marked connected before the new stream's first
            // frame is published, so a reconnect is always visible here by
            // the time that frame is.
            if connection.has_changed().unwrap_or(false)
                && connection.borrow_and_update().is_connected()
            {
                following.attached();
            }
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
    use crate::gui::bridge::tests::{RecordingTransport, THIS_WINDOW};
    use crate::gui::protocol::{GuiClipboard, GuiClipboardText, GuiCommand, GuiReply};
    use crate::gui::{GuiConnection, GuiKeyInput};
    use std::sync::Mutex as StdMutex;

    /// A clipboard in memory, standing in for one on a screen.
    #[derive(Default)]
    struct FakeClipboard {
        held: StdMutex<Option<String>>,
        writes: StdMutex<u32>,
        reads: StdMutex<u32>,
    }

    impl FakeClipboard {
        fn holding(text: &str) -> Self {
            Self {
                held: StdMutex::new(Some(text.to_string())),
                ..Self::default()
            }
        }

        fn contents(&self) -> Option<String> {
            self.held.lock().unwrap().clone()
        }

        fn writes(&self) -> u32 {
            *self.writes.lock().unwrap()
        }

        fn reads(&self) -> u32 {
            *self.reads.lock().unwrap()
        }
    }

    impl LocalClipboard for FakeClipboard {
        fn read(&self) -> Option<String> {
            *self.reads.lock().unwrap() += 1;
            self.held.lock().unwrap().clone()
        }

        fn write(&self, text: &str) {
            *self.held.lock().unwrap() = Some(text.to_string());
            *self.writes.lock().unwrap() += 1;
        }
    }

    /// A frame in `mode` whose clipboard was last written by `writer`.
    fn frame_in(mode: &str, generation: u64, writer: Option<&str>) -> GuiSnapshot {
        let mut snapshot = super::super::snapshot(&Editor::with_content("fn main() {}\n"), 1, 0);
        snapshot.mode = mode.to_string();
        snapshot.clipboard = GuiClipboard {
            generation,
            writer: writer.map(str::to_string),
        };
        snapshot
    }

    fn frame(generation: u64, writer: Option<&str>) -> GuiSnapshot {
        frame_in("NORMAL", generation, writer)
    }

    /// A remote bridge whose editor answers `ReadClipboard` with `text`.
    fn bridged(
        text: &str,
        generation: u64,
        writer: Option<&str>,
    ) -> (GuiBridge, Arc<RecordingTransport>) {
        let answer = GuiClipboardText {
            generation,
            text: text.to_string(),
            writer: writer.map(str::to_string),
        };
        let transport = Arc::new(RecordingTransport::remote_answering(move |command| {
            matches!(command, GuiCommand::ReadClipboard)
                .then(|| GuiReply::Clipboard(Ok(answer.clone())))
        }));
        (GuiBridge::new(transport.clone()), transport)
    }

    fn key(name: &str) -> GuiKeyInput {
        GuiKeyInput {
            key: name.to_string(),
            shift: false,
            control: false,
            alt: false,
            meta: false,
        }
    }

    fn ctrl(letter: &str) -> GuiKeyInput {
        GuiKeyInput {
            control: true,
            ..key(letter)
        }
    }

    /// Attach, then let this window send one command, as a user typing `y`
    /// would before the yank's frame arrives.
    async fn attached_and_acted(clipboard: &ClipboardBridge, generation: u64) {
        clipboard.mirror(&frame(generation, None)).await;
        clipboard.bridge.select_tab(0).await.unwrap();
    }

    fn writes_sent(transport: &RecordingTransport) -> Vec<GuiCommand> {
        transport
            .received()
            .into_iter()
            .filter(|command| {
                matches!(
                    command,
                    GuiCommand::WriteClipboard { .. } | GuiCommand::Key { .. }
                )
            })
            .collect()
    }

    // ---- editor to this machine ----------------------------------------

    #[tokio::test]
    async fn a_yank_this_window_made_after_attaching_reaches_this_machine() {
        let local = Arc::new(FakeClipboard::default());
        let (bridge, _transport) = bridged("yanked line\n", 2, Some(THIS_WINDOW));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        attached_and_acted(&clipboard, 1).await;

        clipboard.mirror(&frame(2, Some(THIS_WINDOW))).await;

        assert_eq!(local.contents().as_deref(), Some("yanked line\n"));
    }

    #[tokio::test]
    async fn attaching_to_a_session_that_has_yanked_leaves_this_clipboard_alone() {
        // The session's last write predates this window, so copying it here
        // would replace whatever the user has copied since -- even when the
        // session says this very window wrote it.
        let local = Arc::new(FakeClipboard::holding("copied this morning"));
        let (bridge, transport) = bridged("yesterday's yank", 7, Some(THIS_WINDOW));
        let clipboard = ClipboardBridge::new(bridge, local.clone());

        clipboard.mirror(&frame(7, Some(THIS_WINDOW))).await;

        assert_eq!(local.contents().as_deref(), Some("copied this morning"));
        assert!(transport.received().is_empty(), "nothing is even fetched");
    }

    #[tokio::test]
    async fn the_first_frame_after_a_reconnect_is_only_a_baseline() {
        let local = Arc::new(FakeClipboard::holding("copied during the outage"));
        let (bridge, transport) = bridged("written while away", 9, Some(THIS_WINDOW));
        let (clipboard, follow) =
            start(&bridge, local.clone()).expect("a remote editor needs the bridge");
        let following = tokio::spawn(follow);
        transport.show(frame(3, None));
        tokio::task::yield_now().await;
        clipboard.bridge.select_tab(0).await.unwrap();

        transport.set_connection(GuiConnection::Reconnecting {
            attempt: 1,
            retry_in_ms: 500,
            detail: "the connection dropped".to_string(),
        });
        transport.set_connection(GuiConnection::Connected);
        transport.show(frame(9, Some(THIS_WINDOW)));
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        following.abort();

        assert_eq!(
            local.contents().as_deref(),
            Some("copied during the outage")
        );
        assert_eq!(local.writes(), 0);
    }

    #[tokio::test]
    async fn a_yank_another_client_made_is_not_copied_here() {
        let local = Arc::new(FakeClipboard::holding("mine"));
        let (bridge, _transport) = bridged("theirs", 2, Some("another-window"));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        attached_and_acted(&clipboard, 1).await;

        clipboard.mirror(&frame(2, Some("another-window"))).await;
        clipboard.mirror(&frame(3, None)).await;

        assert_eq!(local.contents().as_deref(), Some("mine"));
    }

    #[tokio::test]
    async fn a_write_claimed_for_this_window_while_it_did_nothing_is_not_copied() {
        // A session can name this window as the writer whenever it likes; it
        // cannot make the window send a command.
        let local = Arc::new(FakeClipboard::holding("mine"));
        let (bridge, transport) = bridged("injected", 2, Some(THIS_WINDOW));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        clipboard.mirror(&frame(1, None)).await;

        clipboard.mirror(&frame(2, Some(THIS_WINDOW))).await;

        assert_eq!(local.contents().as_deref(), Some("mine"));
        assert!(transport.received().is_empty());
    }

    #[tokio::test]
    async fn an_answer_that_names_another_writer_is_not_copied() {
        // The frame and the fetch are two moments; another client can yank
        // between them.
        let local = Arc::new(FakeClipboard::holding("mine"));
        let (bridge, _transport) = bridged("theirs", 3, Some("another-window"));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        attached_and_acted(&clipboard, 1).await;

        clipboard.mirror(&frame(2, Some(THIS_WINDOW))).await;

        assert_eq!(local.contents().as_deref(), Some("mine"));
    }

    #[tokio::test]
    async fn a_remote_clipboard_past_the_limit_is_refused_here_too() {
        let oversized = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
        let local = Arc::new(FakeClipboard::holding("mine"));
        let (bridge, _transport) = bridged(&oversized, 2, Some(THIS_WINDOW));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        attached_and_acted(&clipboard, 1).await;

        clipboard.mirror(&frame(2, Some(THIS_WINDOW))).await;

        assert_eq!(local.contents().as_deref(), Some("mine"));
    }

    #[tokio::test]
    async fn a_frame_that_yanked_nothing_new_costs_no_round_trip() {
        let local = Arc::new(FakeClipboard::default());
        let (bridge, transport) = bridged("yanked line\n", 2, Some(THIS_WINDOW));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        attached_and_acted(&clipboard, 1).await;

        for _ in 0..20 {
            clipboard.mirror(&frame(2, Some(THIS_WINDOW))).await;
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
    async fn a_clipboard_that_already_holds_the_text_is_left_alone() {
        // A paste from this window raises the editor's generation under this
        // window's name; the mirror that follows finds its own text.
        let local = Arc::new(FakeClipboard::holding("already here"));
        let (bridge, _transport) = bridged("already here", 4, Some(THIS_WINDOW));
        let clipboard = ClipboardBridge::new(bridge, local.clone());
        attached_and_acted(&clipboard, 3).await;

        clipboard.mirror(&frame(4, Some(THIS_WINDOW))).await;

        assert_eq!(local.writes(), 0);
    }

    // ---- this machine to the editor ------------------------------------

    #[tokio::test]
    async fn a_put_from_the_clipboard_register_sends_the_clipboard_just_ahead_of_it() {
        let local = Arc::new(FakeClipboard::holding("copied in a browser"));
        let (bridge, transport) = bridged("", 0, None);
        transport.show(frame(0, None));
        let clipboard = ClipboardBridge::new(bridge, local);

        for name in ["\"", "+", "p"] {
            clipboard.key(key(name)).await.unwrap();
        }

        assert_eq!(
            writes_sent(&transport),
            vec![
                GuiCommand::Key { input: key("\"") },
                GuiCommand::Key { input: key("+") },
                GuiCommand::WriteClipboard {
                    text: "copied in a browser".to_string()
                },
                GuiCommand::Key { input: key("p") },
            ]
        );
    }

    #[tokio::test]
    async fn ordinary_keys_never_read_or_send_this_clipboard() {
        // Including a plain `p`: `clipboard=unnamedplus` is the remote's
        // option, and the remote does not get to decide when this machine's
        // clipboard leaves it.
        let local = Arc::new(FakeClipboard::holding("a password, for all this knows"));
        let (bridge, transport) = bridged("", 0, None);
        transport.show(frame(0, None));
        let clipboard = ClipboardBridge::new(bridge, local.clone());

        for name in ["y", "y", "p", "P", "\"", "a", "p", "g", "p", "+", "*"] {
            clipboard.key(key(name)).await.unwrap();
        }
        transport.show(frame_in("INSERT", 0, None));
        for name in ["\"", "+", "p", "+"] {
            clipboard.key(key(name)).await.unwrap();
        }

        assert!(!transport
            .received()
            .iter()
            .any(|command| matches!(command, GuiCommand::WriteClipboard { .. })));
        assert_eq!(local.reads(), 0);
    }

    #[test]
    fn every_put_spelling_from_the_clipboard_registers_is_a_paste() {
        let normal = |keys: &[&str]| {
            let mut detector = PasteDetector::default();
            keys.iter()
                .map(|name| detector.observe(&key(name), "NORMAL"))
                .collect::<Vec<_>>()
        };
        assert_eq!(normal(&["\"", "+", "p"]), [false, false, true]);
        assert_eq!(normal(&["\"", "*", "P"]), [false, false, true]);
        assert_eq!(normal(&["\"", "+", "3", "p"]), [false, false, false, true]);
        assert_eq!(normal(&["\"", "+", "g", "p"]), [false, false, false, true]);
        assert_eq!(normal(&["\"", "+", "]", "p"]), [false, false, false, true]);
        assert_eq!(normal(&["\"", "+", "y", "p"]), [false; 4]);
        assert_eq!(normal(&["\"", "a", "p"]), [false; 3]);

        let mut visual = PasteDetector::default();
        assert!(!visual.observe(&key("\""), "VISUAL_LINE"));
        assert!(!visual.observe(&key("*"), "VISUAL_LINE"));
        assert!(visual.observe(&key("p"), "VISUAL_LINE"));
    }

    #[test]
    fn ctrl_r_with_a_clipboard_register_is_a_paste_in_text_entry_modes_only() {
        for mode in ["INSERT", "COMMAND", "SEARCH", "REPLACE"] {
            let mut detector = PasteDetector::default();
            assert!(!detector.observe(&ctrl("r"), mode));
            assert!(detector.observe(&key("+"), mode), "{mode}");

            let mut literally = PasteDetector::default();
            assert!(!literally.observe(&ctrl("r"), mode));
            assert!(!literally.observe(&ctrl("r"), mode));
            assert!(literally.observe(&key("*"), mode), "{mode}");
        }
        // In Normal mode `Ctrl-R` is redo and `+` moves down a line.
        let mut normal = PasteDetector::default();
        assert!(!normal.observe(&ctrl("r"), "NORMAL"));
        assert!(!normal.observe(&key("+"), "NORMAL"));
    }

    #[test]
    fn put_ex_commands_naming_the_clipboard_are_a_paste_on_enter() {
        let typed = |line: &str| {
            let mut detector = PasteDetector::default();
            assert!(!detector.observe(&key(":"), "NORMAL"));
            for character in line.chars() {
                assert!(!detector.observe(&key(&character.to_string()), "COMMAND"));
            }
            detector.observe(&key("Enter"), "COMMAND")
        };
        assert!(typed("put +"));
        assert!(typed("pu *"));
        assert!(typed("3put! +"));
        assert!(typed("'<,'>pu +"));
        assert!(!typed("put"));
        assert!(!typed("put a"));
        assert!(!typed("put + x"));
        assert!(!typed("s/+/plus/"));

        // A line this window cannot follow (recalled from history) is not
        // guessed at.
        let mut recalled = PasteDetector::default();
        recalled.observe(&key(":"), "NORMAL");
        recalled.observe(&key("ArrowUp"), "COMMAND");
        assert!(!recalled.observe(&key("Enter"), "COMMAND"));
    }

    #[tokio::test]
    async fn a_paste_is_one_extra_command_and_nothing_else() {
        let local = Arc::new(FakeClipboard::holding("copied"));
        let (bridge, transport) = bridged("", 0, None);
        transport.show(frame_in("INSERT", 0, None));
        let clipboard = ClipboardBridge::new(bridge, local);

        clipboard.key(ctrl("r")).await.unwrap();
        clipboard.key(key("+")).await.unwrap();

        assert_eq!(
            writes_sent(&transport),
            vec![
                GuiCommand::Key { input: ctrl("r") },
                GuiCommand::WriteClipboard {
                    text: "copied".to_string()
                },
                GuiCommand::Key { input: key("+") },
            ]
        );
    }

    #[tokio::test]
    async fn a_clipboard_too_large_to_carry_is_not_sent_and_the_key_still_is() {
        let oversized = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
        let local = Arc::new(FakeClipboard::holding(&oversized));
        let (bridge, transport) = bridged("", 0, None);
        transport.show(frame(0, None));
        let clipboard = ClipboardBridge::new(bridge, local);

        for name in ["\"", "+", "p"] {
            clipboard.key(key(name)).await.unwrap();
        }

        assert_eq!(writes_sent(&transport).len(), 3, "three keys, no write");
    }

    #[test]
    fn an_editor_running_in_this_process_gets_no_bridge_at_all() {
        // It writes the very clipboard this process would write, through the
        // same `arboard` handle. A second writer could only get in its way.
        let bridge = GuiBridge::new(RecordingTransport::new());

        assert!(start(&bridge, Arc::new(FakeClipboard::default())).is_none());
    }

    #[test]
    fn a_yank_past_the_limit_is_refused_where_the_text_is() {
        assert!(within_limit(&"y".repeat(MAX_CLIPBOARD_BYTES)).is_ok());

        let error = within_limit(&"y".repeat(MAX_CLIPBOARD_BYTES + 1)).unwrap_err();

        assert!(error.contains("limit"), "{error}");
    }

    #[test]
    fn what_an_editor_hands_over_is_what_it_wrote_not_what_its_host_holds() {
        // An in-memory clipboard, so the test leaves the machine's own alone.
        let mut editor = Editor::with_content("fn main() {}\n");
        editor.registers_mut().use_memory_clipboard();
        editor.registers_mut().set_clipboard("yanked".to_string());

        let handed = clipboard_text(&editor).unwrap();

        assert_eq!(handed.text, "yanked");
        assert_eq!(handed.generation, 1);
    }
}
