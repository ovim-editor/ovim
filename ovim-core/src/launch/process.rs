//! Asynchronous child-process runner for builds and program runs.
//!
//! The editor tick must never block on a child, so a spawned process is fully
//! owned by background tasks that stream output lines through a channel the
//! tick polls with `try_recv`. Children are placed in their own process group
//! so stopping a run also stops everything it started (Gradle client -> test
//! worker, `java` launched by a wrapper script, ...). Dropping the handle
//! kills the group too, so nothing outlives the editor session that
//! started it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

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

/// Event streamed from a running process. `Exit` is always the last one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcEvent {
    Line { stream: StreamKind, text: String },
    Exit(ExitInfo),
}

/// Owner of a running child process.
pub struct ProcessHandle {
    rx: mpsc::UnboundedReceiver<ProcEvent>,
    kill_tx: Option<oneshot::Sender<()>>,
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
        let mut child = command.spawn().map_err(|e| {
            format!(
                "failed to run '{}' in {}: {}",
                program,
                spec.cwd.display(),
                e
            )
        })?;
        let pid = child.id();
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

        let (tx, rx) = mpsc::unbounded_channel();
        let (kill_tx, kill_rx) = oneshot::channel::<()>();

        let readers: Vec<tokio::task::JoinHandle<()>> = [
            stdout.map(|s| spawn_reader(s, StreamKind::Stdout, tx.clone())),
            stderr.map(|s| spawn_reader(s, StreamKind::Stderr, tx.clone())),
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
            // Grandchildren that inherited the pipes can keep them open long
            // after the direct child is gone; do not wait for them forever.
            for reader in readers {
                let _ = tokio::time::timeout(Duration::from_millis(500), reader).await;
            }
            // Make sure nothing in the group survives a normal exit either
            // (e.g. a wrapper script that backgrounded the real program).
            if killed {
                terminate_group(pid, true);
            }
            let code = status.ok().and_then(|s| s.code());
            let _ = tx.send(ProcEvent::Exit(ExitInfo { code, killed }));
        });

        Ok(Self {
            rx,
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
        self.rx.try_recv().ok()
    }

    /// Process id of the direct child, when known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Asks the process group to terminate (SIGTERM, then SIGKILL after 3s).
    /// The `Exit` event still arrives through the channel.
    pub fn kill(&mut self) {
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn spawn_reader<R>(
    stream: R,
    kind: StreamKind,
    tx: mpsc::UnboundedSender<ProcEvent>,
) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    while matches!(buf.last(), Some(b'\n' | b'\r')) {
                        buf.pop();
                    }
                    let text = String::from_utf8_lossy(&buf);
                    // Carriage-return progress output: keep what the
                    // terminal would finally show.
                    let text = text.rsplit('\r').next().unwrap_or("").to_string();
                    if tx.send(ProcEvent::Line { stream: kind, text }).is_err() {
                        break;
                    }
                }
            }
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

#[cfg(unix)]
fn terminate_group(pid: Option<u32>, force: bool) {
    let Some(pid) = pid else { return };
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: plain signal delivery to a process group we created.
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}

#[cfg(not(unix))]
fn terminate_group(_pid: Option<u32>, _force: bool) {}

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
