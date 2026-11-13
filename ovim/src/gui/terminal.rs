//! A single native terminal session for the GUI.
//!
//! The PTY reader and writer run on dedicated threads because portable-pty
//! exposes blocking handles. Bounded queues keep a slow webview or shell from
//! accumulating unlimited data in the editor process.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::oneshot;

use super::GuiBridge;

const OUTPUT_CHUNK_SIZE: usize = 4096;
const OUTPUT_QUEUE_CAPACITY: usize = 16;
const INPUT_QUEUE_CAPACITY: usize = 16;
const MAX_INPUT_SIZE: usize = 64 * 1024;
const OUTPUT_CREDIT: usize = 64 * 1024;
const MAX_COLUMNS: u16 = 500;
const MAX_ROWS: u16 = 300;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalEvent {
    Data { id: u64, data: Vec<u8> },
    Exit { id: u64, code: u32 },
    Error { id: u64, message: String },
}

#[derive(Clone, Default)]
pub struct TerminalHost {
    inner: Arc<Mutex<HostState>>,
}

#[derive(Default)]
struct HostState {
    next_id: u64,
    session: Option<Session>,
}

struct Session {
    id: u64,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    input: mpsc::SyncSender<InputRequest>,
    flow: Arc<FlowControl>,
}

struct InputRequest {
    bytes: Vec<u8>,
    completed: oneshot::Sender<Result<(), String>>,
}

struct FlowControl {
    state: Mutex<FlowState>,
    changed: Condvar,
}

struct FlowState {
    credit: usize,
    closed: bool,
}

impl FlowControl {
    fn new() -> Self {
        Self {
            state: Mutex::new(FlowState {
                credit: OUTPUT_CREDIT,
                closed: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn reserve(&self) -> Option<(usize, bool)> {
        let mut state = self.state.lock().ok()?;
        while state.credit == 0 && !state.closed {
            state = self.changed.wait(state).ok()?;
        }
        if state.closed {
            // Keep draining the PTY after close. A process with pending output
            // can remain in kernel exit until its terminal buffer is drained.
            return Some((OUTPUT_CHUNK_SIZE, true));
        }
        let amount = state.credit.min(OUTPUT_CHUNK_SIZE);
        state.credit -= amount;
        Some((amount, false))
    }

    fn refund(&self, bytes: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.credit = state.credit.saturating_add(bytes).min(OUTPUT_CREDIT);
            self.changed.notify_one();
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.changed.notify_all();
        }
    }
}

impl TerminalHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(
        &self,
        directory: &Path,
        columns: u16,
        rows: u16,
        on_event: Channel<TerminalEvent>,
        command: Option<&str>,
    ) -> Result<u64, String> {
        self.open_with_sink(
            directory,
            columns,
            rows,
            shell_command(command),
            move |event| on_event.send(event).is_ok(),
        )
    }

    fn open_with_sink(
        &self,
        directory: &Path,
        columns: u16,
        rows: u16,
        mut command: CommandBuilder,
        on_event: impl Fn(TerminalEvent) -> bool + Send + 'static,
    ) -> Result<u64, String> {
        let size = terminal_size(columns, rows)?;
        let mut state = self.inner.lock().map_err(|_| "Terminal lock poisoned")?;
        if state.session.is_some() {
            return Err("A terminal is already open".into());
        }

        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| format!("Failed to open terminal: {error}"))?;
        command.cwd(directory);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Failed to start shell: {error}"))?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().map_err(|error| {
            let _ = child.kill();
            let _ = child.wait();
            format!("Failed to read terminal: {error}")
        })?;
        let writer = pair.master.take_writer().map_err(|error| {
            let _ = child.kill();
            let _ = child.wait();
            format!("Failed to write terminal: {error}")
        })?;

        let id = state.next_id.wrapping_add(1).max(1);
        state.next_id = id;
        let (input_tx, input_rx) = mpsc::sync_channel(INPUT_QUEUE_CAPACITY);
        let (output_tx, output_rx) = mpsc::sync_channel(OUTPUT_QUEUE_CAPACITY);
        let flow = Arc::new(FlowControl::new());
        state.session = Some(Session {
            id,
            master: pair.master,
            killer: child.clone_killer(),
            input: input_tx,
            flow: flow.clone(),
        });
        drop(state);

        let event_host = self.clone();
        if let Err(error) = thread::Builder::new()
            .name("ovim-terminal-events".into())
            .spawn(move || {
                for event in output_rx {
                    if !on_event(event) {
                        let _ = event_host.close(id);
                        break;
                    }
                }
            })
        {
            let _ = self.close(id);
            let _ = child.wait();
            return Err(format!("Failed to start terminal event thread: {error}"));
        }
        let input_host = self.clone();
        let input_events = output_tx.clone();
        if let Err(error) = thread::Builder::new()
            .name("ovim-terminal-input".into())
            .spawn(move || write_input(writer, input_rx, input_events, input_host, id))
        {
            let _ = self.close(id);
            let _ = child.wait();
            return Err(format!("Failed to start terminal input thread: {error}"));
        }

        let host = self.clone();
        let child_slot = Arc::new(Mutex::new(Some(child)));
        let worker_child = child_slot.clone();
        if let Err(error) = thread::Builder::new()
            .name("ovim-terminal-output".into())
            .spawn(move || {
                let child = worker_child.lock().ok().and_then(|mut slot| slot.take());
                if let Some(child) = child {
                    read_output(reader, child, output_tx, flow, host, id);
                }
            })
        {
            let _ = self.close(id);
            if let Ok(mut slot) = child_slot.lock() {
                if let Some(mut child) = slot.take() {
                    let _ = child.wait();
                }
            }
            return Err(format!("Failed to start terminal output thread: {error}"));
        }
        Ok(id)
    }

    pub async fn write(&self, id: u64, data: String) -> Result<(), String> {
        if data.len() > MAX_INPUT_SIZE {
            return Err("Terminal input is too large".into());
        }
        let (completed, completion) = oneshot::channel();
        {
            let state = self.inner.lock().map_err(|_| "Terminal lock poisoned")?;
            let session = current_session(&state, id)?;
            session
                .input
                .try_send(InputRequest {
                    bytes: data.into_bytes(),
                    completed,
                })
                .map_err(|error| match error {
                    mpsc::TrySendError::Full(_) => "Terminal input is busy".to_string(),
                    mpsc::TrySendError::Disconnected(_) => "Terminal has exited".to_string(),
                })?;
        }
        completion
            .await
            .map_err(|_| "Terminal input worker stopped".to_string())?
    }

    pub fn resize(&self, id: u64, columns: u16, rows: u16) -> Result<(), String> {
        let size = terminal_size(columns, rows)?;
        let state = self.inner.lock().map_err(|_| "Terminal lock poisoned")?;
        current_session(&state, id)?
            .master
            .resize(size)
            .map_err(|error| format!("Failed to resize terminal: {error}"))
    }

    pub fn acknowledge(&self, id: u64, bytes: usize) -> Result<(), String> {
        let state = self.inner.lock().map_err(|_| "Terminal lock poisoned")?;
        if let Ok(session) = current_session(&state, id) {
            session.flow.refund(bytes);
        }
        Ok(())
    }

    pub fn close(&self, id: u64) -> Result<(), String> {
        let session = {
            let mut state = self.inner.lock().map_err(|_| "Terminal lock poisoned")?;
            current_session(&state, id)?;
            state.session.take()
        };
        terminate(session);
        Ok(())
    }

    pub fn shutdown(&self) {
        let session = self
            .inner
            .lock()
            .ok()
            .and_then(|mut state| state.session.take());
        terminate(session);
    }

    fn finish(&self, id: u64) {
        if let Ok(mut state) = self.inner.lock() {
            if state
                .session
                .as_ref()
                .is_some_and(|session| session.id == id)
            {
                state.session.take();
            }
        }
    }
}

fn current_session(state: &HostState, id: u64) -> Result<&Session, String> {
    state
        .session
        .as_ref()
        .filter(|session| session.id == id)
        .ok_or_else(|| "Terminal session is no longer active".into())
}

fn terminate(session: Option<Session>) {
    if let Some(mut session) = session {
        session.flow.close();
        #[cfg(unix)]
        if let Some(group) = session
            .master
            .process_group_leader()
            .filter(|group| *group > 0)
        {
            // Interactive shells place the foreground job in its own group.
            // Signal that job before terminating the shell itself.
            unsafe { libc::kill(-group, libc::SIGHUP) };
        }
        let _ = session.killer.kill();
        // Dropping the master and input sender closes the PTY and wakes its
        // worker threads. The output worker owns and reaps the child.
    }
}

fn terminal_size(columns: u16, rows: u16) -> Result<PtySize, String> {
    if !(1..=MAX_COLUMNS).contains(&columns) || !(1..=MAX_ROWS).contains(&rows) {
        return Err("Terminal dimensions are out of range".into());
    }
    Ok(PtySize {
        cols: columns,
        rows,
        ..PtySize::default()
    })
}

fn shell_command(command: Option<&str>) -> CommandBuilder {
    #[cfg(windows)]
    let shell = std::env::var_os("COMSPEC")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "cmd.exe".into());
    #[cfg(not(windows))]
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut builder = CommandBuilder::new(shell);
    if let Some(command) = command {
        builder.args([if cfg!(windows) { "/C" } else { "-c" }, command]);
    }
    builder
}

fn write_input(
    mut writer: Box<dyn Write + Send>,
    input: mpsc::Receiver<InputRequest>,
    events: mpsc::SyncSender<TerminalEvent>,
    host: TerminalHost,
    id: u64,
) {
    for request in input {
        if let Err(error) = writer.write_all(&request.bytes) {
            let message = format!("Terminal input failed: {error}");
            let _ = request.completed.send(Err(message.clone()));
            let _ = events.try_send(TerminalEvent::Error { id, message });
            let _ = host.close(id);
            break;
        }
        let _ = request.completed.send(Ok(()));
    }
}

fn read_output(
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    events: mpsc::SyncSender<TerminalEvent>,
    flow: Arc<FlowControl>,
    host: TerminalHost,
    id: u64,
) {
    let mut buffer = [0; OUTPUT_CHUNK_SIZE];
    while let Some((capacity, discard)) = flow.reserve() {
        match reader.read(&mut buffer[..capacity]) {
            Ok(0) => {
                flow.refund(capacity);
                break;
            }
            Ok(length) => {
                if discard {
                    continue;
                }
                flow.refund(capacity - length);
                if events
                    .send(TerminalEvent::Data {
                        id,
                        data: buffer[..length].to_vec(),
                    })
                    .is_err()
                {
                    let _ = host.close(id);
                    continue;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                flow.refund(capacity);
                continue;
            }
            Err(error) if is_pty_eof(&error) => {
                flow.refund(capacity);
                break;
            }
            Err(error) => {
                let _ = events.send(TerminalEvent::Error {
                    id,
                    message: format!("Terminal output failed: {error}"),
                });
                let _ = host.close(id);
                break;
            }
        }
    }
    let result = child.wait();
    host.finish(id);
    match result {
        Ok(status) => {
            let _ = events.send(TerminalEvent::Exit {
                id,
                code: status.exit_code(),
            });
        }
        Err(error) => {
            let _ = events.send(TerminalEvent::Error {
                id,
                message: format!("Failed to wait for shell: {error}"),
            });
        }
    }
}

fn is_pty_eof(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::EIO)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

#[tauri::command]
pub async fn gui_terminal_open(
    bridge: State<'_, GuiBridge>,
    host: State<'_, TerminalHost>,
    columns: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
    command: Option<String>,
) -> Result<u64, String> {
    let directory = bridge.workspace_directory().await?;
    host.open(&directory, columns, rows, on_event, command.as_deref())
}

#[tauri::command]
pub async fn gui_terminal_write(
    host: State<'_, TerminalHost>,
    id: u64,
    data: String,
) -> Result<(), String> {
    host.write(id, data).await
}

#[tauri::command]
pub fn gui_terminal_resize(
    host: State<'_, TerminalHost>,
    id: u64,
    columns: u16,
    rows: u16,
) -> Result<(), String> {
    host.resize(id, columns, rows)
}

#[tauri::command]
pub fn gui_terminal_close(host: State<'_, TerminalHost>, id: u64) -> Result<(), String> {
    host.close(id)
}

#[tauri::command]
pub fn gui_terminal_ack(
    host: State<'_, TerminalHost>,
    id: u64,
    bytes: usize,
) -> Result<(), String> {
    host.acknowledge(id, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn input_completion_waits_for_the_pty_writer() {
        struct GatedWriter {
            started: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }

        impl Write for GatedWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.started.send(()).unwrap();
                self.release.recv().unwrap();
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (input_tx, input_rx) = mpsc::sync_channel(1);
        let (events_tx, _events_rx) = mpsc::sync_channel(1);
        let (completed_tx, mut completed_rx) = oneshot::channel();
        let writer_thread = thread::spawn(move || {
            write_input(
                Box::new(GatedWriter {
                    started: started_tx,
                    release: release_rx,
                }),
                input_rx,
                events_tx,
                TerminalHost::new(),
                1,
            );
        });

        input_tx
            .send(InputRequest {
                bytes: b"hello".to_vec(),
                completed: completed_tx,
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            completed_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release_tx.send(()).unwrap();
        assert_eq!(completed_rx.await.unwrap(), Ok(()));
        drop(input_tx);
        writer_thread.join().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_round_trip_resize_and_exit() {
        let host = TerminalHost::new();
        let (tx, rx) = mpsc::channel();
        let id = host
            .open_with_sink(
                Path::new("/tmp"),
                80,
                24,
                CommandBuilder::new("/bin/sh"),
                move |event| tx.send(event).is_ok(),
            )
            .unwrap();
        assert_eq!(host.resize(id, 100, 35), Ok(()));
        assert_eq!(
            host.write(
                id,
                "stty -echo; stty size; printf '\\nterminal-ready\\n'; exit 7\n".into()
            )
            .await,
            Ok(())
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        let mut exit = None;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(TerminalEvent::Data { data, .. }) => {
                    host.acknowledge(id, data.len()).unwrap();
                    output.extend(data);
                }
                Ok(TerminalEvent::Exit { code, .. }) => {
                    exit = Some(code);
                    break;
                }
                Ok(TerminalEvent::Error { message, .. }) => panic!("{message}"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => panic!("{error}"),
            }
        }
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("35 100"), "{text}");
        assert!(text.contains("\r\nterminal-ready\r\n"), "{text}");
        assert_eq!(exit, Some(7));
        assert!(host.write(id, "stale".into()).await.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn requested_command_preserves_quotes_pipelines_directory_and_exit_status() {
        let host = TerminalHost::new();
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("input file"), "hello world\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let id = host
            .open_with_sink(
                directory.path(),
                80,
                24,
                shell_command(Some("cat 'input file' | tr a-z A-Z; exit 7")),
                move |event| tx.send(event).is_ok(),
            )
            .unwrap();
        let mut output = Vec::new();
        loop {
            match rx
                .recv_timeout(Duration::from_secs(5))
                .expect("terminal command timed out")
            {
                TerminalEvent::Data { data, .. } => {
                    host.acknowledge(id, data.len()).unwrap();
                    output.extend(data);
                }
                TerminalEvent::Exit { code, .. } => {
                    assert_eq!(code, 7);
                    break;
                }
                TerminalEvent::Error { message, .. } => panic!("{message}"),
            }
        }
        assert!(String::from_utf8_lossy(&output).contains("HELLO WORLD"));
        assert!(host.resize(id, 80, 24).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn close_rejects_stale_session_and_allows_reopen() {
        let host = TerminalHost::new();
        let id = host
            .open_with_sink(
                Path::new("/tmp"),
                80,
                24,
                CommandBuilder::new("/bin/sh"),
                |_| true,
            )
            .unwrap();
        assert!(host
            .open_with_sink(
                Path::new("/tmp"),
                80,
                24,
                CommandBuilder::new("/bin/sh"),
                |_| true
            )
            .is_err());
        host.close(id).unwrap();
        let next = host
            .open_with_sink(
                Path::new("/tmp"),
                80,
                24,
                CommandBuilder::new("/bin/sh"),
                |_| true,
            )
            .unwrap();
        assert_ne!(id, next);
        assert!(host.close(id).is_err());
        assert!(host.resize(id, 80, 24).is_err());
        host.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn acknowledged_output_can_exceed_initial_credit() {
        let host = TerminalHost::new();
        let (tx, rx) = mpsc::channel();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "awk 'BEGIN {for (i=0; i<140000; i++) printf \"x\"}'"]);
        let id = host
            .open_with_sink(Path::new("/tmp"), 80, 24, command, move |event| {
                tx.send(event).is_ok()
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut received = 0;
        let mut exited = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(TerminalEvent::Data { data, .. }) => {
                    received += data.iter().filter(|&&byte| byte == b'x').count();
                    host.acknowledge(id, data.len()).unwrap();
                }
                Ok(TerminalEvent::Exit { code, .. }) => {
                    assert_eq!(code, 0);
                    exited = true;
                    break;
                }
                Ok(TerminalEvent::Error { message, .. }) => panic!("{message}"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => panic!("{error}"),
            }
        }
        assert!(exited, "shell did not exit after receiving credit");
        assert_eq!(received, 140000);
    }

    #[cfg(unix)]
    #[test]
    fn close_wakes_reader_waiting_for_credit() {
        let host = TerminalHost::new();
        let (tx, rx) = mpsc::channel();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "yes x"]);
        let id = host
            .open_with_sink(Path::new("/tmp"), 80, 24, command, move |event| {
                tx.send(event).is_ok()
            })
            .unwrap();
        let mut received = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while received < OUTPUT_CREDIT && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(TerminalEvent::Data { data, .. }) => received += data.len(),
                Ok(TerminalEvent::Error { message, .. }) => panic!("{message}"),
                Ok(TerminalEvent::Exit { .. }) => panic!("shell exited early"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => panic!("{error}"),
            }
        }
        assert_eq!(received, OUTPUT_CREDIT);
        host.close(id).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(
                rx.recv_timeout(Duration::from_millis(100)),
                Ok(TerminalEvent::Exit { .. })
            ) {
                return;
            }
        }
        panic!("terminal process was not reaped after close");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_event_delivery_drains_and_reaps_a_flooding_child() {
        let host = TerminalHost::new();
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let mut command = CommandBuilder::new("/bin/sh");
        command.args([
            "-c",
            "echo $$ > \"$1\"; yes x",
            "sh",
            pid_file.to_str().unwrap(),
        ]);
        let (tx, rx) = mpsc::channel();
        let id = host
            .open_with_sink(directory.path(), 80, 24, command, move |event| {
                let _ = tx.send(event);
                false
            })
            .unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(TerminalEvent::Data { .. })
        ));
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let gone = unsafe { libc::kill(pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if gone {
                assert!(host.write(id, String::new()).await.is_err());
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("terminal process survived failed event delivery");
    }
}
