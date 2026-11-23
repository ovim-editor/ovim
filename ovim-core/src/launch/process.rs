//! Asynchronous child-process runner for builds and program runs.
//!
//! The editor tick must never block on a child, so a spawned process is fully
//! owned by background tasks that stream output lines through a channel the
//! tick polls with `try_recv`. The channel is bounded: a program that writes
//! faster than the editor draws is slowed down (its pipe fills up) like it
//! would be by a terminal that cannot keep up, instead of piling its output
//! up in memory. How the process ended travels on its own channel, so it is
//! never stuck behind a backlog of output. Children are placed in their own
//! process group so stopping a run also stops everything it started (Gradle
//! client -> test worker, `java` launched by a wrapper script, ...).
//! Dropping the handle kills the group too, so nothing outlives the editor
//! session that started it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, watch};

use super::process_groups::{register_group, terminate_group};

/// Output lines buffered between the readers and the tick. When the queue is
/// full the readers wait, which stops the child once its pipe fills up.
const OUTPUT_QUEUE_LINES: usize = 8192;
/// Longest output line kept; the rest of a longer line is dropped.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
pub(super) const TRUNCATED_MARKER: &str = " ... [line truncated]";
/// How long a pipe may stay quiet after the child exited before its reader
/// gives up (a grandchild can hold the pipe open indefinitely).
const READER_GRACE: Duration = Duration::from_millis(500);
/// How much more a reader takes from a pipe once the child has exited: what
/// the pipe still held (kernel pipes are at most 1 MiB). A grandchild that
/// keeps writing does not hold the run open.
const MAX_BYTES_AFTER_EXIT: usize = 4 << 20;

/// Which pipe a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

/// A command to run: argv[0] plus arguments, working directory, extra env.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandSpec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    /// Shell-quoted one-line rendering for the console header.
    pub fn display(&self) -> String {
        self.argv
            .iter()
            .map(|a| {
                if a.is_empty()
                    || a.chars()
                        .any(|c| c.is_whitespace() || "\"'$&|;<>()".contains(c))
                {
                    format!("'{}'", a.replace('\'', "'\\''"))
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    /// Exit code; `None` when killed by a signal or the wait failed.
    pub code: Option<i32>,
    /// True when the exit was requested through [`ProcessHandle::kill`]
    /// (or the handle was dropped).
    pub killed: bool,
}

/// Event streamed from a running process. `Exit` is always the last one,
/// delivered once every line that was read before the process ended (none,
/// after [`ProcessHandle::kill`]) has been handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcEvent {
    Line { stream: StreamKind, text: String },
    Exit(ExitInfo),
}

/// Owner of a running child process.
pub struct ProcessHandle {
    lines: mpsc::Receiver<(StreamKind, String)>,
    exit_rx: oneshot::Receiver<ExitInfo>,
    /// The exit, once the waiter task reported it and until it is handed out.
    exit: Option<ExitInfo>,
    kill_tx: Option<oneshot::Sender<()>>,
    /// Output is discarded from the moment the process is told to die.
    killed: bool,
    pid: Option<u32>,
    /// Text for the child's stdin; `None` when stdin is closed (or was never
    /// piped). Dropping the sender closes the pipe.
    stdin_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

impl ProcessHandle {
    /// Starts `spec` with stdin closed (builds). Must be called inside a
    /// tokio runtime.
    pub fn spawn(spec: &CommandSpec) -> Result<Self, String> {
        Self::spawn_with(spec, false)
    }

    /// Starts `spec` with a pipe on stdin, fed through [`send_stdin`](Self::send_stdin)
    /// (programs that read `System.in`).
    pub fn spawn_interactive(spec: &CommandSpec) -> Result<Self, String> {
        Self::spawn_with(spec, true)
    }

    fn spawn_with(spec: &CommandSpec, interactive: bool) -> Result<Self, String> {
        let Some(program) = spec.argv.first() else {
            return Err("empty command".to_string());
        };
        let mut command = Command::new(program);
        command
            .args(&spec.argv[1..])
            .current_dir(&spec.cwd)
            .envs(&spec.env)
            .stdin(if interactive {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(target_os = "linux")]
        die_with_parent(&mut command);
        let mut child = command.spawn().map_err(|e| {
            format!(
                "failed to run '{}' in {}: {}",
                program,
                spec.cwd.display(),
                e
            )
        })?;
        let pid = child.id();
        // Registered until the group is done, so that the editor can stop it
        // on its way out (see `process_groups`).
        let group = pid.map(register_group);
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin_tx = child.stdin.take().map(|mut stdin| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
            tokio::spawn(async move {
                while let Some(bytes) = rx.recv().await {
                    // A program that stopped reading (or exited) closes the pipe.
                    if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                        break;
                    }
                }
                // Channel closed: dropping `stdin` here signals EOF.
            });
            tx
        });

        let (tx, lines) = mpsc::channel(OUTPUT_QUEUE_LINES);
        let (exit_tx, exit_rx) = oneshot::channel();
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let (exited_tx, exited_rx) = watch::channel(false);

        let readers: Vec<tokio::task::JoinHandle<()>> = [
            stdout.map(|s| spawn_reader(s, StreamKind::Stdout, tx.clone(), exited_rx.clone())),
            stderr.map(|s| spawn_reader(s, StreamKind::Stderr, tx.clone(), exited_rx.clone())),
        ]
        .into_iter()
        .flatten()
        .collect();

        tokio::spawn(async move {
            let mut killed = false;
            let status = tokio::select! {
                status = child.wait() => status,
                _ = kill_rx => {
                    killed = true;
                    terminate_group(pid, false);
                    match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                        Ok(status) => status,
                        Err(_) => {
                            terminate_group(pid, true);
                            let _ = child.start_kill();
                            child.wait().await
                        }
                    }
                }
            };
            if killed {
                // Nobody wants the output of a stopped process.
                for reader in &readers {
                    reader.abort();
                }
                // Whatever the group still holds goes with it. After a normal
                // exit leftovers are not touched: build tools leave daemons
                // behind on purpose.
                terminate_group(pid, true);
            }
            // The leader is gone; the editor no longer answers for the group.
            drop(group);
            let _ = exited_tx.send(true);
            if !killed {
                // Readers end at the end of the output, or when it dries up
                // (see `spawn_reader`); one blocked on a full queue is waited
                // for, so that no output is lost.
                for reader in readers {
                    let _ = reader.await;
                }
            }
            let code = status.ok().and_then(|s| s.code());
            let _ = exit_tx.send(ExitInfo { code, killed });
        });

        Ok(Self {
            lines,
            exit_rx,
            exit: None,
            killed: false,
            kill_tx: Some(kill_tx),
            pid,
            stdin_tx,
        })
    }

    /// Whether the child's stdin is still open for [`send_stdin`](Self::send_stdin).
    pub fn accepts_stdin(&self) -> bool {
        self.stdin_tx.as_ref().is_some_and(|tx| !tx.is_closed())
    }

    /// Writes `text` to the child's stdin. Returns false when stdin is closed.
    pub fn send_stdin(&self, text: &str) -> bool {
        self.stdin_tx
            .as_ref()
            .is_some_and(|tx| tx.send(text.as_bytes().to_vec()).is_ok())
    }

    /// Closes the child's stdin (EOF for `System.in`).
    pub fn close_stdin(&mut self) {
        self.stdin_tx = None;
    }

    /// Next pending event without blocking.
    pub fn try_recv(&mut self) -> Option<ProcEvent> {
        // Look at the exit before the lines: everything the readers queued
        // before the process was reported gone is then already visible.
        if self.exit.is_none() {
            self.exit = self.exit_rx.try_recv().ok();
        }
        if self.killed {
            while self.lines.try_recv().is_ok() {}
        } else if let Ok((stream, text)) = self.lines.try_recv() {
            return Some(ProcEvent::Line { stream, text });
        }
        self.exit.take().map(ProcEvent::Exit)
    }

    /// Process id of the direct child, when known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Asks the process group to terminate (SIGTERM, then SIGKILL after 3s).
    /// Output not handed out yet is discarded; the `Exit` event still
    /// arrives through `try_recv`.
    pub fn kill(&mut self) {
        self.killed = true;
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Has the kernel kill the child when the editor dies without a chance to
/// clean up (SIGKILL, the OOM killer). Only the child itself: the rest of its
/// group is beyond the kernel's reach.
#[cfg(target_os = "linux")]
fn die_with_parent(command: &mut Command) {
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: only async-signal-safe calls (prctl, getppid) run between fork
    // and exec.
    unsafe {
        command.pre_exec(move || {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            // The editor may have died before the flag was set.
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

type OutputLine = (StreamKind, String);

/// Assembles one output line from pipe chunks: bounded in length, with
/// carriage-return progress output reduced to what a terminal would finally
/// show.
#[derive(Default)]
struct LineBuilder {
    buf: Vec<u8>,
    /// Bytes beyond [`MAX_LINE_BYTES`] were dropped.
    truncated: bool,
    /// A `\r` was seen: it replaces what came before if more text follows,
    /// and is dropped at the end of the line (`\r\n`).
    carriage_return: bool,
    /// Any byte has arrived since the last line was taken.
    started: bool,
}

impl LineBuilder {
    fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.started = true;
        let mut parts = bytes.split(|&b| b == b'\r');
        self.append(parts.next().unwrap_or_default());
        for part in parts {
            self.carriage_return = true;
            self.append(part);
        }
    }

    fn append(&mut self, part: &[u8]) {
        if part.is_empty() {
            return;
        }
        if std::mem::take(&mut self.carriage_return) {
            self.buf.clear();
            self.truncated = false;
        }
        let room = MAX_LINE_BYTES - self.buf.len();
        self.truncated |= part.len() > room;
        self.buf.extend_from_slice(&part[..part.len().min(room)]);
    }

    /// The finished line; the builder starts over.
    fn take(&mut self) -> String {
        let mut text = String::from_utf8_lossy(&self.buf).into_owned();
        if self.truncated {
            // The cut may have split a multi-byte character.
            text.truncate(text.trim_end_matches('\u{FFFD}').len());
            text.push_str(TRUNCATED_MARKER);
        }
        self.buf.clear();
        self.truncated = false;
        self.carriage_return = false;
        self.started = false;
        text
    }
}

/// Reads one pipe line by line into the queue. Once the child has exited it
/// stops at the end of the pipe, when the pipe goes quiet, or after taking
/// [`MAX_BYTES_AFTER_EXIT`] more, whichever comes first.
fn spawn_reader<R>(
    stream: R,
    kind: StreamKind,
    tx: mpsc::Sender<OutputLine>,
    mut exited: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut line = LineBuilder::default();
        let mut bytes_after_exit = 0;
        loop {
            let quiet_after_exit = async {
                let _ = exited.clone().wait_for(|&gone| gone).await;
                tokio::time::sleep(READER_GRACE).await;
            };
            let chunk = tokio::select! {
                chunk = reader.fill_buf() => match chunk {
                    Ok(chunk) if !chunk.is_empty() => chunk,
                    _ => break,
                },
                _ = quiet_after_exit => break,
            };
            let (used, complete) = match chunk.iter().position(|&b| b == b'\n') {
                Some(end) => {
                    line.push(&chunk[..end]);
                    (end + 1, true)
                }
                None => {
                    line.push(chunk);
                    (chunk.len(), false)
                }
            };
            reader.consume(used);
            // A full queue parks the reader here until the tick catches up.
            if complete && tx.send((kind, line.take())).await.is_err() {
                return;
            }
            if *exited.borrow_and_update() {
                bytes_after_exit += used;
                if bytes_after_exit > MAX_BYTES_AFTER_EXIT {
                    break;
                }
            }
        }
        if line.started {
            let _ = tx.send((kind, line.take())).await;
        }
    })
}

/// A free local TCP port (best effort: it is released before use).
pub fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    Some(listener.local_addr().ok()?.port())
}

/// Whether some process listens on local TCP `port`. On Linux this reads
/// `/proc/net/tcp*` and never connects: a stray connection would look like a
/// debugger handshake to a JDWP agent and could end its listening. Elsewhere
/// a connection attempt is the only portable probe.
pub fn port_is_listening(port: u16) -> bool {
    #[cfg(target_os = "linux")]
    {
        let want = format!(":{port:04X}");
        ["/proc/net/tcp", "/proc/net/tcp6"].iter().any(|table| {
            std::fs::read_to_string(table).is_ok_and(|text| {
                text.lines().skip(1).any(|line| {
                    let mut cols = line.split_whitespace().skip(1);
                    let local = cols.next().unwrap_or("");
                    let _remote = cols.next();
                    // st == 0A is LISTEN.
                    cols.next() == Some("0A") && local.ends_with(&want)
                })
            })
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(200),
        )
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(script: &str) -> CommandSpec {
        CommandSpec {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
        }
    }

    async fn drain(handle: &mut ProcessHandle) -> (Vec<(StreamKind, String)>, ExitInfo) {
        let mut lines = Vec::new();
        loop {
            match handle.try_recv() {
                Some(ProcEvent::Line { stream, text }) => lines.push((stream, text)),
                Some(ProcEvent::Exit(info)) => return (lines, info),
                None => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    }

    #[tokio::test]
    async fn streams_both_pipes_and_reports_exit_code() {
        let mut handle = ProcessHandle::spawn(&spec("echo out; echo err >&2; exit 3")).unwrap();
        let (lines, exit) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        assert!(lines.contains(&(StreamKind::Stdout, "out".into())));
        assert!(lines.contains(&(StreamKind::Stderr, "err".into())));
        assert_eq!(
            exit,
            ExitInfo {
                code: Some(3),
                killed: false
            }
        );
    }

    #[tokio::test]
    async fn interactive_children_read_lines_from_stdin_until_eof() {
        let mut handle = ProcessHandle::spawn_interactive(&spec(
            "while read line; do echo \"got:$line\"; done; echo eof",
        ))
        .unwrap();
        assert!(handle.accepts_stdin());
        assert!(handle.send_stdin("one\n"));
        assert!(handle.send_stdin("two words\n"));
        handle.close_stdin();
        assert!(!handle.accepts_stdin());
        let (lines, exit) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        let out: Vec<String> = lines.into_iter().map(|(_, t)| t).collect();
        assert_eq!(out, vec!["got:one", "got:two words", "eof"]);
        assert_eq!(exit.code, Some(0));
    }

    #[tokio::test]
    async fn a_listening_port_is_seen_without_connecting_to_it() {
        // Keep ownership of the port throughout: after dropping a listener,
        // a concurrent process can reuse its port before the negative probe.
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(!port_is_listening(port));
        let listener = socket.listen(1).unwrap();
        assert!(port_is_listening(port));
        #[cfg(target_os = "linux")]
        {
            let listener = listener.into_std().unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        #[cfg(not(target_os = "linux"))]
        drop(listener);
    }

    #[tokio::test]
    async fn plain_spawn_gives_the_child_an_empty_stdin() {
        let mut handle = ProcessHandle::spawn(&spec("read x; echo \"[$x]\"")).unwrap();
        assert!(!handle.accepts_stdin());
        assert!(!handle.send_stdin("ignored\n"));
        let (lines, _) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        assert_eq!(lines, vec![(StreamKind::Stdout, "[]".to_string())]);
    }

    #[tokio::test]
    async fn final_line_without_newline_is_flushed() {
        let mut handle = ProcessHandle::spawn(&spec("printf tail")).unwrap();
        let (lines, _) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        assert_eq!(lines, vec![(StreamKind::Stdout, "tail".to_string())]);
    }

    #[tokio::test]
    async fn carriage_return_progress_output_keeps_what_the_terminal_shows() {
        let mut handle =
            ProcessHandle::spawn(&spec("printf 'a\\rb\\r\\nc\\n10%%\\r20%%\\r30%%'")).unwrap();
        let (lines, _) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        let out: Vec<&str> = lines.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(out, vec!["b", "c", "30%"]);
    }

    #[tokio::test]
    async fn a_very_long_line_is_cut_with_a_marker() {
        // No newline at all: the reader must not buffer the whole stream.
        let mut handle =
            ProcessHandle::spawn(&spec("head -c 300000 /dev/zero | tr '\\0' a")).unwrap();
        let (lines, _) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        assert_eq!(lines.len(), 1);
        let text = &lines[0].1;
        assert!(text.ends_with(TRUNCATED_MARKER), "{}", text.len());
        assert_eq!(text.len(), MAX_LINE_BYTES + TRUNCATED_MARKER.len());
    }

    #[tokio::test]
    async fn a_cut_inside_a_multibyte_character_leaves_no_replacement_character() {
        // 3-byte characters; 64 KiB is not a multiple of 3.
        let mut handle =
            ProcessHandle::spawn(&spec("yes '\u{20ac}' | tr -d '\\n' | head -c 200000")).unwrap();
        let (lines, _) = tokio::time::timeout(Duration::from_secs(5), drain(&mut handle))
            .await
            .unwrap();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].1.contains('\u{FFFD}'), "{}", lines[0].1.len());
        assert!(lines[0].1.ends_with(TRUNCATED_MARKER));
    }

    #[tokio::test]
    async fn a_slow_consumer_loses_no_output_and_sees_the_exit_last() {
        // More lines than the queue holds, all written before the child
        // exits: the readers wait for the consumer instead of giving up.
        let total = OUTPUT_QUEUE_LINES + 3000;
        let mut handle = ProcessHandle::spawn(&spec(&format!("yes x | head -n {total}"))).unwrap();
        tokio::time::sleep(READER_GRACE * 3).await;
        let (lines, exit) = tokio::time::timeout(Duration::from_secs(10), drain(&mut handle))
            .await
            .unwrap();
        assert_eq!(lines.len(), total);
        assert_eq!(exit.code, Some(0));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_grandchild_holding_the_pipe_open_does_not_hold_the_run_open() {
        let mut handle = ProcessHandle::spawn(&spec("sleep 300 & echo $!; echo hi")).unwrap();
        let (lines, exit) = tokio::time::timeout(Duration::from_secs(10), drain(&mut handle))
            .await
            .expect("the run waited for a process that merely inherited its pipes");
        assert_eq!(exit.code, Some(0));
        assert_eq!(lines.len(), 2);
        let sleeper: i32 = lines[0].1.trim().parse().unwrap();
        // SAFETY: tidying up the sleeper the child left behind on purpose.
        unsafe { libc::kill(sleeper, libc::SIGKILL) };
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_grandchild_that_keeps_writing_is_cut_off_after_the_exit() {
        let mut handle = ProcessHandle::spawn(&spec(
            "yes \"$(head -c 1000 /dev/zero | tr '\\0' x)\" & echo $!",
        ))
        .unwrap();
        let (lines, _) = tokio::time::timeout(Duration::from_secs(20), drain(&mut handle))
            .await
            .expect("an endless writer held the run open");
        let flooder: i32 = lines[0].1.trim().parse().unwrap();
        // SAFETY: tidying up the writer the child left behind on purpose.
        unsafe { libc::kill(flooder, libc::SIGKILL) };
        assert!(
            lines.len() < 2 * MAX_BYTES_AFTER_EXIT / 1000,
            "{} lines",
            lines.len()
        );
    }

    #[tokio::test]
    async fn an_unread_flood_stays_bounded_and_stop_ends_it_promptly() {
        let mut handle = ProcessHandle::spawn(&spec("yes")).unwrap();
        // Nobody reads: the queue fills up and the child is held back.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(handle.lines.len(), OUTPUT_QUEUE_LINES);
        handle.kill();
        let (lines, exit) = tokio::time::timeout(Duration::from_secs(10), drain(&mut handle))
            .await
            .expect("the exit was stuck behind the output backlog");
        assert!(exit.killed);
        assert!(lines.is_empty(), "output of a stopped process is dropped");
    }

    #[tokio::test]
    async fn missing_program_is_a_spawn_error_naming_the_program() {
        let bad = CommandSpec {
            argv: vec!["definitely-not-a-real-program-xyz".into()],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
        };
        let err = ProcessHandle::spawn(&bad).err().unwrap();
        assert!(err.contains("definitely-not-a-real-program-xyz"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_takes_down_the_whole_process_group() {
        // The shell backgrounds a sleeper; killing the run must reap it too.
        let mut handle = ProcessHandle::spawn(&spec("sleep 300 & echo $! ; wait")).unwrap();
        let sleeper = loop {
            if let Some(ProcEvent::Line { text, .. }) = handle.try_recv() {
                break text.trim().parse::<i32>().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        handle.kill();
        let (_, exit) = tokio::time::timeout(Duration::from_secs(10), drain(&mut handle))
            .await
            .unwrap();
        assert!(exit.killed);
        let mut alive = true;
        for _ in 0..100 {
            // SAFETY: probing for existence only.
            alive = unsafe { libc::kill(sleeper, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!alive, "background child {sleeper} survived Stop");
    }

    #[tokio::test]
    async fn dropping_the_handle_kills_the_process() {
        let handle = ProcessHandle::spawn(&spec("sleep 300")).unwrap();
        let pid = handle.pid().unwrap() as i32;
        drop(handle);
        tokio::time::sleep(Duration::from_millis(300)).await;
        #[cfg(unix)]
        {
            let mut alive = true;
            for _ in 0..100 {
                // SAFETY: probing for existence only. The zombie is reaped by the waiter task.
                alive = unsafe { libc::kill(pid, 0) } == 0;
                if !alive {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(!alive);
        }
    }
}
