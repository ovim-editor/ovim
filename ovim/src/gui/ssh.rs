//! Bringing a remote editor up with one command.
//!
//! [`RemoteTransport`](super::RemoteTransport) can drive an editor on another
//! host, but only once somebody has started a headless session there, learned
//! its port and capability, and forwarded the port. This module does that
//! automatically, so the whole feature is
//!
//! ```text
//! ovim gui --remote user@host /path/to/project
//! ```
//!
//! Four properties shape everything below.
//!
//! * **One authentication per window.** The bootstrap and the port forward are
//!   two SSH invocations, and on a host with 2FA or a passphrase-protected key
//!   that would be two prompts. The first invocation opens a multiplexing
//!   master ([`CONTROL_PERSIST`]) and the second rides on it, so the user is
//!   asked exactly once.
//! * **The capability is never an argument.** It comes back on the bootstrap's
//!   stdout, inside the SSH channel, and lives only in memory. Nothing here
//!   writes it to a file or puts it on a command line, where the process table
//!   and the shell history would keep it.
//! * **The session outlives the window.** Closing the GUI tears down the
//!   tunnel and the control socket and stops there. Reconnecting later finds
//!   warm language servers and intact undo history, which is the entire reason
//!   the editor runs remotely rather than the files being mounted.
//! * **The remote session stays on loopback.** The forward binds
//!   `127.0.0.1` at both ends; no port is ever exposed on either machine.

use super::remote::RemoteEndpoint;
use anyhow::{Context, Result};
use ovim_core::session::SessionInfo;
use rand::{rngs::OsRng, RngCore};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long the multiplexing master lingers after its last client leaves.
///
/// It is a safety net rather than a feature: teardown asks the master to exit.
/// It only matters when this process is killed outright, and then it bounds
/// how long a forgotten master can sit on the machine.
const CONTROL_PERSIST: &str = "60";
/// How long the remote session is given to write its descriptor.
///
/// Generous because the first start on a cold host pays for reading the
/// project and spawning language servers.
const SESSION_READY_SECONDS: u32 = 30;
/// How long the forward is given to start listening locally.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the bootstrap conversation may take in total.
///
/// Long enough to cover an interactive password or a 2FA prompt.
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(180);
/// How many local ports are tried before giving up.
///
/// A port is chosen by binding zero and reading back what the kernel gave, so
/// a collision needs another process to take it in the instant between that
/// and `ssh` binding it. Rare, and a retry lands on a different port.
const FORWARD_ATTEMPTS: u32 = 3;

/// The banner the remote bootstrap script prints before anything else.
const REPORT_BANNER: &str = "ovim-remote-bootstrap 1";
/// The line after which the rest of the output is the session descriptor.
const REPORT_DESCRIPTOR: &str = "descriptor";
/// The remote script found no `ovim` to run.
const EXIT_NO_BINARY: i32 = 10;
/// The requested project path does not exist on the remote host.
const EXIT_NO_PATH: i32 = 11;
/// A session was started but never wrote a descriptor.
const EXIT_NO_SESSION: i32 = 12;

/// What to open, and where.
#[derive(Clone, Debug)]
pub struct RemoteTarget {
    /// `[user@]host`, or a `Host` alias from the user's SSH config.
    pub destination: String,
    /// The project path *on the remote host*. Resolved there, not here.
    pub path: String,
    /// Replace any existing session for this project instead of reattaching.
    pub fresh: bool,
    /// Connect even when the two Ovim versions could speak different protocols.
    pub allow_version_mismatch: bool,
}

/// A reachable remote editor, and whatever has to stay alive to reach it.
#[derive(Debug)]
pub struct RemoteLaunch {
    /// Where to dial, which port the session thinks it is on, and its capability.
    pub endpoint: RemoteEndpoint,
    /// The tunnel, when this process built one. Dropping it closes the tunnel
    /// and leaves the remote session running.
    pub tunnel: Option<SshTunnel>,
}

impl RemoteLaunch {
    /// A launch against an endpoint somebody else arranged to be reachable.
    pub fn preconnected(endpoint: RemoteEndpoint) -> Self {
        Self {
            endpoint,
            tunnel: None,
        }
    }
}

/// The remote flags as they arrive from a command line, before they mean
/// anything.
///
/// Both binaries that can open a window fill this in -- `ovim gui` through
/// clap and `ovim-gui` through its own small parser -- so the rules about
/// which combinations make sense live in one place instead of two.
#[derive(Clone, Debug, Default)]
pub struct RemoteOptions {
    /// `--remote`: the host to bootstrap on.
    pub destination: Option<String>,
    /// The positional path, which belongs to the remote host under `--remote`.
    pub path: Option<String>,
    /// `--fresh`.
    pub fresh: bool,
    /// `--allow-version-mismatch`.
    pub allow_version_mismatch: bool,
    /// `--remote-session`: a descriptor for a session reached some other way.
    pub session_file: Option<PathBuf>,
    /// `--remote-endpoint`: where that session is reachable.
    pub endpoint: Option<String>,
}

impl RemoteOptions {
    /// Turn the flags into a reachable editor, or into `None` for a local one.
    ///
    /// Everything that can fail here fails before a window exists, so a wrong
    /// host or an unreadable descriptor reads as a startup error rather than
    /// as a window that never draws.
    pub fn resolve(self) -> Result<Option<RemoteLaunch>> {
        anyhow::ensure!(
            self.destination.is_none() || self.session_file.is_none(),
            "--remote and --remote-session both say where the editor is; use one.\n\
             --remote brings a session up over SSH, --remote-session drives one that is \
             already reachable."
        );
        anyhow::ensure!(
            self.session_file.is_some() || self.endpoint.is_none(),
            "--remote-endpoint needs --remote-session: without a descriptor there is no \
             capability to authenticate with."
        );
        anyhow::ensure!(
            self.destination.is_some() || !(self.fresh || self.allow_version_mismatch),
            "--fresh and --allow-version-mismatch only mean something with --remote."
        );

        if let Some(destination) = self.destination {
            let path = self.path.context(
                "--remote needs a path on the remote host, as in \
                 'ovim gui --remote user@host /path/to/project'.",
            )?;
            return launch(&RemoteTarget {
                destination,
                path,
                fresh: self.fresh,
                allow_version_mismatch: self.allow_version_mismatch,
            })
            .map(Some);
        }
        self.session_file
            .map(|path| {
                RemoteEndpoint::from_session_file(&path, self.endpoint.as_deref())
                    .map(RemoteLaunch::preconnected)
            })
            .transpose()
    }
}

/// Start or reattach a remote session and forward its port to this machine.
pub fn launch(target: &RemoteTarget) -> Result<RemoteLaunch> {
    validate_destination(&target.destination)?;
    let program = ssh_program()?;
    let socket = ControlSocket::create()?;

    let report = bootstrap(&program, &socket, target)?;
    check_versions(target, &report)?;
    ovim_core::log_info!(
        "gui",
        "Remote session '{}' on {} ({:?}) for {}",
        report.session.session_name,
        target.destination,
        report.origin,
        report.project
    );

    let tunnel = SshTunnel::open(&program, socket, &target.destination, report.session.port)?;
    let endpoint = RemoteEndpoint::from_session(&report.session, Some(&tunnel.local_address()))
        .context("The remote session descriptor cannot be used to reach it")?;
    Ok(RemoteLaunch {
        endpoint,
        tunnel: Some(tunnel),
    })
}

// ── SSH invocations ──────────────────────────────────────────────────────
//
// Every invocation is built as data by one of the functions below and only
// then handed to a process, so the arguments can be asserted on without a
// host to connect to.

/// Where the `ssh` client is, or why remote editing cannot work without it.
fn ssh_program() -> Result<PathBuf> {
    which::which("ssh").map_err(|_| {
        anyhow::anyhow!(
            "Remote editing needs an OpenSSH client, and no 'ssh' was found on PATH.\n\
         Install OpenSSH (the package is usually called openssh-client or openssh) and try again."
        )
    })
}

/// A destination this module is willing to hand to `ssh`.
///
/// `ssh` takes the destination positionally, so one that begins with `-` would
/// be read as options. Rejecting it here is clearer than relying on `--`,
/// which not every client on every host parses the same way.
fn validate_destination(destination: &str) -> Result<()> {
    anyhow::ensure!(
        !destination.is_empty(),
        "--remote needs a destination, for example user@host"
    );
    anyhow::ensure!(
        !destination.starts_with('-'),
        "'{destination}' cannot be an SSH destination: it would be read as an option.\n\
         Write it as user@host, or as a Host alias from your SSH config."
    );
    anyhow::ensure!(
        !destination
            .chars()
            .any(|character| character.is_whitespace() || character.is_control()),
        "'{destination}' is not a usable SSH destination: it contains whitespace.\n\
         Write it as user@host, or as a Host alias from your SSH config."
    );
    Ok(())
}

/// Arguments for the phase that starts or finds the remote session.
///
/// This is the invocation that authenticates. `ControlMaster=auto` makes it
/// leave a multiplexing master behind, which the forward then reuses without
/// asking the user for anything.
///
/// The script arrives on stdin rather than as an argument: `ssh host <script>`
/// is interpreted by the *login* shell, which may be fish or csh, whereas
/// `/bin/sh -s` is a bare word every shell passes through untouched.
fn bootstrap_arguments(destination: &str, socket: &Path) -> Vec<String> {
    let mut arguments = control_arguments(socket, ControlRole::Master);
    arguments.push(destination.to_string());
    arguments.push("/bin/sh".to_string());
    arguments.push("-s".to_string());
    arguments
}

/// Arguments for the forward that carries the GUI conversation.
///
/// `ExitOnForwardFailure=yes` turns a port that could not be bound into an
/// immediate non-zero exit with a message, instead of a live connection whose
/// tunnel silently does not exist.
///
/// The remote command is `cat`, not `-N`, on purpose: it holds this process's
/// stdin pipe open, so if the GUI dies in any way at all -- including a kill
/// that runs no teardown -- the pipe closes, `cat` sees end of file, and the
/// tunnel takes itself down. That is what keeps forwards from accumulating.
fn forward_arguments(
    destination: &str,
    socket: &Path,
    local_port: u16,
    remote_port: u16,
) -> Vec<String> {
    let mut arguments = control_arguments(socket, ControlRole::Client);
    arguments.push("-o".to_string());
    arguments.push("ExitOnForwardFailure=yes".to_string());
    // A dropped link should surface as a closed tunnel rather than as an
    // editor that has become permanently slow.
    arguments.push("-o".to_string());
    arguments.push("ServerAliveInterval=15".to_string());
    arguments.push("-o".to_string());
    arguments.push("ServerAliveCountMax=3".to_string());
    arguments.push("-L".to_string());
    // Both ends are spelled out. The remote session asserts a loopback bind,
    // and a forward that defaulted to a wildcard local bind would put a
    // capability-protected editor on the laptop's network interfaces.
    arguments.push(format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"));
    arguments.push(destination.to_string());
    arguments.push("cat".to_string());
    arguments
}

/// Arguments that ask the multiplexing master to exit.
fn control_exit_arguments(destination: &str, socket: &Path) -> Vec<String> {
    let mut arguments = vec![
        "-o".to_string(),
        format!("ControlPath={}", socket.display()),
        "-O".to_string(),
        "exit".to_string(),
    ];
    arguments.push(destination.to_string());
    arguments
}

/// Which side of the multiplexed connection an invocation is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ControlRole {
    /// May create the shared connection.
    Master,
    /// Must reuse it, and must never authenticate on its own.
    Client,
}

fn control_arguments(socket: &Path, role: ControlRole) -> Vec<String> {
    let mut arguments = Vec::new();
    arguments.push("-o".to_string());
    arguments.push(match role {
        ControlRole::Master => "ControlMaster=auto".to_string(),
        ControlRole::Client => "ControlMaster=no".to_string(),
    });
    arguments.push("-o".to_string());
    arguments.push(format!("ControlPath={}", socket.display()));
    if role == ControlRole::Master {
        arguments.push("-o".to_string());
        arguments.push(format!("ControlPersist={CONTROL_PERSIST}"));
    }
    arguments
}

// ── The control socket ───────────────────────────────────────────────────

/// The multiplexing socket for one launch.
///
/// Per launch rather than per host: a shared path would let one window's
/// teardown pull the connection out from under another's, and a socket left by
/// a killed process would make `ssh` quietly disable multiplexing -- which
/// looks exactly like the double password prompt this module exists to avoid.
#[derive(Debug)]
struct ControlSocket {
    path: PathBuf,
}

impl ControlSocket {
    /// A private, short path for the socket.
    ///
    /// Short matters: a unix socket path is capped near 104 bytes, and `ssh`
    /// spends nine of them on the temporary name it binds before renaming. A
    /// control path under a deep home directory overruns that and fails with
    /// an error that names neither the limit nor the path.
    fn create() -> Result<Self> {
        let mut bases = Vec::new();
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            bases.push(PathBuf::from(runtime));
        }
        bases.push(std::env::temp_dir());
        bases.push(PathBuf::from("/tmp"));

        let mut token = [0_u8; 4];
        OsRng.fill_bytes(&mut token);
        let token = token
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let mut longest = 0;
        for base in &bases {
            let directory = base.join("ovim-ssh");
            let path = directory.join(&token);
            let length = path.as_os_str().len();
            longest = longest.max(length);
            // Nine bytes of headroom for the ".XXXXXXXX" suffix ssh binds
            // before renaming the socket into place.
            if length + 9 > 104 {
                continue;
            }
            if ensure_private_directory(&directory).is_err() {
                continue;
            }
            return Ok(Self { path });
        }
        anyhow::bail!(
            "No short enough directory was found for the SSH control socket \
             (the shortest candidate was {longest} bytes, and the limit is near 104).\n\
             Set TMPDIR to a short path such as /tmp and try again."
        )
    }
}

fn ensure_private_directory(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// ── Phase one: start or find the session ─────────────────────────────────

/// Where a session came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SessionOrigin {
    /// Nothing was running for this project, so a session was started.
    Started,
    /// A session for this project was already running and was reused.
    Reattached,
}

/// What the remote bootstrap reported about itself.
struct BootstrapReport {
    /// The `ovim` that was run there.
    binary: String,
    /// Whatever `ovim --version` printed, unparsed.
    version: String,
    origin: SessionOrigin,
    /// The project path as the remote host resolved it.
    project: String,
    session: SessionInfo,
}

fn bootstrap(
    program: &Path,
    socket: &ControlSocket,
    target: &RemoteTarget,
) -> Result<BootstrapReport> {
    let script = bootstrap_script(&target.path, target.fresh, SESSION_READY_SECONDS);
    let mut child = Command::new(program)
        .args(bootstrap_arguments(&target.destination, &socket.path))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to run {}", program.display()))?;

    // Written and closed before the output is read: the script only starts
    // running once sh has it, and sh only finishes reading at end of file.
    child
        .stdin
        .take()
        .context("The ssh client accepted no input")?
        .write_all(script.as_bytes())
        .context("Failed to send the bootstrap script to the remote host")?;

    let output = wait_with_deadline(&mut child, BOOTSTRAP_TIMEOUT).map_err(|error| {
        anyhow::anyhow!(
            "{error} while starting Ovim on {}.\n\
             Run 'ssh {}' by hand to see what the connection is waiting for.",
            target.destination,
            target.destination
        )
    })?;

    if !output.status.success() {
        return Err(bootstrap_failure(target, output.code, &output.stderr));
    }
    parse_report(&output.stdout).with_context(|| {
        format!(
            "The bootstrap on {} answered in a form this Ovim cannot read",
            target.destination
        )
    })
}

/// The POSIX shell script that runs on the remote host.
///
/// It is deliberately one script rather than several round trips: each round
/// trip is a latency the user waits through, and the decisions here (is there
/// a session, is it alive, does the path exist) all depend on remote state.
///
/// Reattaching beats starting: a session that is already running holds warm
/// language servers, undo history, and in-flight work, and finding it again is
/// the whole reason for putting the editor on the far side of the link.
fn bootstrap_script(project: &str, fresh: bool, ready_seconds: u32) -> String {
    let project = shell_quote(project);
    let fresh = u8::from(fresh);
    format!(
        r#"set -u
project={project}
fresh={fresh}
ready={ready_seconds}

# A tilde arrives literally, because the path is quoted all the way here.
case "$project" in
  '~') project="$HOME" ;;
  '~/'*) project="$HOME/${{project#\~/}}" ;;
esac

bin=$(command -v ovim 2>/dev/null) || bin=
if [ -z "$bin" ]; then
  for candidate in "$HOME/.cargo/bin/ovim" "$HOME/.local/bin/ovim" \
                   /usr/local/bin/ovim /opt/homebrew/bin/ovim; do
    if [ -x "$candidate" ]; then bin=$candidate; break; fi
  done
fi
if [ -z "$bin" ]; then
  echo "no ovim executable" >&2
  exit {EXIT_NO_BINARY}
fi

if [ -d "$project" ]; then
  resolved=$(cd "$project" 2>/dev/null && pwd -P) || resolved=
elif [ -f "$project" ]; then
  parent=$(cd "$(dirname "$project")" 2>/dev/null && pwd -P) || parent=
  if [ -n "$parent" ]; then resolved="$parent/$(basename "$project")"; else resolved=; fi
else
  resolved=
fi
if [ -z "$resolved" ]; then
  echo "$project does not exist" >&2
  exit {EXIT_NO_PATH}
fi

# The session name is derived from the resolved path so that a later launch
# for the same project finds the same session. cksum is POSIX and everywhere;
# the readable stem is what makes a collision obvious rather than baffling.
stem=$(printf '%s' "$(basename "$resolved")" | tr -c 'A-Za-z0-9_-' '-' | cut -c1-24)
sum=$(printf '%s' "$resolved" | cksum | cut -d' ' -f1)
name="gui-$stem-$sum"

if [ -n "${{OVIM_SESSION_DIR:-}}" ]; then
  dir=$OVIM_SESSION_DIR
elif [ "$(uname -s)" = Darwin ]; then
  dir="$HOME/Library/Caches/ovim/sessions"
else
  dir="${{XDG_CACHE_HOME:-$HOME/.cache}}/ovim/sessions"
fi
descriptor="$dir/$name.json"

pid=
if [ -f "$descriptor" ]; then
  # Tolerant of layout: the descriptor is pretty-printed today, but a session
  # file that ever gets written compactly should still be found.
  pid=$(sed -n 's/.*"pid": *\([0-9][0-9]*\).*/\1/p' "$descriptor" | head -n 1)
fi
alive=0
if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
  # A descriptor outlives the process that wrote it, so a number that answers
  # to signal 0 is not yet evidence that this session is the thing answering.
  # Ovim's own liveness check guards against a recycled pid by comparing start
  # times; the process's arguments are the portable equivalent from here.
  # Getting it wrong is not academic: --fresh would kill whatever inherited the
  # number, and a reattach would tunnel to a port serving somebody else.
  args=$(ps -ww -p "$pid" -o args= 2>/dev/null) || args=$(ps -p "$pid" -o args= 2>/dev/null) || args=
  case "$args" in
    *"--session $name"*) alive=1 ;;
    # No usable ps leaves the bare pid, which is all there ever was.
    '') alive=1 ;;
    *) alive=0 ;;
  esac
fi

if [ "$alive" = 1 ] && [ "$fresh" = 1 ]; then
  kill "$pid" 2>/dev/null
  waited=0
  while [ "$waited" -lt 5 ] && kill -0 "$pid" 2>/dev/null; do
    sleep 1
    waited=$((waited + 1))
  done
  kill -9 "$pid" 2>/dev/null
  alive=0
fi

if [ "$alive" = 1 ]; then
  state=reattached
else
  state=started
  rm -f "$descriptor"
  # Detached from this shell in every direction: nohup survives the hangup
  # when ssh closes the channel, and the redirections keep the session from
  # holding the channel open or reading what is left of this script.
  nohup "$bin" "$resolved" --headless --session "$name" </dev/null >/dev/null 2>&1 &
  waited=0
  while [ "$waited" -lt "$ready" ] && [ ! -f "$descriptor" ]; do
    sleep 1
    waited=$((waited + 1))
  done
  if [ ! -f "$descriptor" ]; then
    echo "$name wrote no descriptor within ${{ready}}s" >&2
    exit {EXIT_NO_SESSION}
  fi
fi

version=$("$bin" --version 2>/dev/null | head -n 1) || version=
echo "{REPORT_BANNER}"
echo "binary=$bin"
echo "version=$version"
echo "state=$state"
echo "project=$resolved"
echo "{REPORT_DESCRIPTOR}"
cat "$descriptor"
"#
    )
}

/// Wrap a string so a POSIX shell sees exactly its bytes.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Split the bootstrap's output into its report lines and the descriptor.
fn parse_report(stdout: &str) -> Result<BootstrapReport> {
    let mut lines = stdout.lines();
    anyhow::ensure!(
        lines.next().map(str::trim) == Some(REPORT_BANNER),
        "the output does not start with the bootstrap banner"
    );

    let mut binary = String::new();
    let mut version = String::new();
    let mut state = String::new();
    let mut project = String::new();
    let mut descriptor = String::new();
    let mut in_descriptor = false;
    for line in lines {
        if in_descriptor {
            descriptor.push_str(line);
            descriptor.push('\n');
            continue;
        }
        if line.trim() == REPORT_DESCRIPTOR {
            in_descriptor = true;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "binary" => binary = value.to_string(),
            "version" => version = value.to_string(),
            "state" => state = value.to_string(),
            "project" => project = value.to_string(),
            _ => {}
        }
    }
    anyhow::ensure!(in_descriptor, "the output carries no session descriptor");

    let origin = match state.as_str() {
        "reattached" => SessionOrigin::Reattached,
        "started" => SessionOrigin::Started,
        other => anyhow::bail!("'{other}' is not a session state this Ovim knows"),
    };
    let session: SessionInfo = serde_json::from_str(descriptor.trim())
        .context("the session descriptor is not readable as JSON")?;
    Ok(BootstrapReport {
        binary,
        version,
        origin,
        project,
        session,
    })
}

// ── Version matching ─────────────────────────────────────────────────────

/// What a local and a remote Ovim version imply about talking to each other.
#[derive(Clone, PartialEq, Eq, Debug)]
enum VersionVerdict {
    /// The same build on both ends.
    Same,
    /// Different patch releases, which do not change the GUI protocol.
    PatchDrift,
    /// Different feature releases, which may.
    Incompatible,
    /// The remote version could not be read at all.
    Unreadable,
}

/// Read the version out of whatever `ovim --version` printed.
///
/// clap prints `ovim 1.2.7`, but a wrapper script or a distribution patch can
/// add a prefix, so the last whitespace-separated token wins.
fn parse_version(reported: &str) -> Option<(u64, u64, u64)> {
    let token = reported.split_whitespace().next_back()?;
    let token = token.trim_start_matches('v');
    // A pre-release or build suffix is not part of the comparison.
    let core = token.split_once(['-', '+']).map_or(token, |(core, _)| core);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn compare_versions(local: &str, remote: &str) -> VersionVerdict {
    let (Some(local), Some(remote)) = (parse_version(local), parse_version(remote)) else {
        return VersionVerdict::Unreadable;
    };
    if local == remote {
        VersionVerdict::Same
    } else if (local.0, local.1) == (remote.0, remote.1) {
        VersionVerdict::PatchDrift
    } else {
        VersionVerdict::Incompatible
    }
}

/// Refuse, or warn, on a version pair that could speak different protocols.
///
/// The GUI protocol carries no version of its own, so a mismatched pair fails
/// as a missing field or an unknown command variant rather than as anything
/// that names the cause. That argues for refusing. Against it: a laptop
/// running a source build against a released host is the normal way this
/// feature gets used, and a hard stop on every patch difference would make it
/// unusable. The split follows what the repository actually changes -- patch
/// releases do not add command variants, feature releases do -- so a patch
/// difference warns and a feature difference stops, with an override for the
/// user who knows their two builds agree.
fn check_versions(target: &RemoteTarget, report: &BootstrapReport) -> Result<()> {
    let local = env!("CARGO_PKG_VERSION");
    match compare_versions(local, &report.version) {
        VersionVerdict::Same => Ok(()),
        VersionVerdict::PatchDrift => {
            ovim_core::log_warn!(
                "gui",
                "Ovim {} here and {} on {}; patch releases share the GUI protocol, so continuing",
                local,
                report.version.trim(),
                target.destination
            );
            Ok(())
        }
        VersionVerdict::Unreadable if target.allow_version_mismatch => Ok(()),
        VersionVerdict::Unreadable => anyhow::bail!(
            "{} on {} did not report a version this Ovim could read (it printed {:?}).\n\
             That usually means it is not an Ovim binary. Check the path, or pass \
             --allow-version-mismatch to connect anyway.",
            report.binary,
            target.destination,
            report.version.trim()
        ),
        VersionVerdict::Incompatible if target.allow_version_mismatch => {
            ovim_core::log_warn!(
                "gui",
                "Connecting Ovim {} to {} on {} on request; the GUI protocol is not versioned",
                local,
                report.version.trim(),
                target.destination
            );
            Ok(())
        }
        VersionVerdict::Incompatible => anyhow::bail!(
            "Ovim {local} here and {} on {}.\n\
             The GUI protocol is not versioned yet, so a mismatched pair fails in ways that do \
             not name the cause. Upgrade whichever side is older, or pass \
             --allow-version-mismatch to connect anyway.",
            report.version.trim(),
            target.destination
        ),
    }
}

// ── Phase two: the tunnel ────────────────────────────────────────────────

/// A forwarded port, and the multiplexed connection carrying it.
///
/// Dropping this closes both and stops there. The remote session keeps
/// running: a window closing is not a reason to throw away warm language
/// servers, and reconnecting to them later is the point of the architecture.
#[derive(Debug)]
pub struct SshTunnel {
    program: PathBuf,
    destination: String,
    socket: ControlSocket,
    /// The forwarding client. Its stdin pipe is load-bearing, not incidental:
    /// it is what makes the remote end notice this process dying.
    forward: Child,
    local_port: u16,
}

impl SshTunnel {
    fn open(
        program: &Path,
        socket: ControlSocket,
        destination: &str,
        remote_port: u16,
    ) -> Result<Self> {
        let mut last: Option<anyhow::Error> = None;
        for _ in 0..FORWARD_ATTEMPTS {
            let local_port = free_local_port()?;
            let mut forward = Command::new(program)
                .args(forward_arguments(
                    destination,
                    &socket.path,
                    local_port,
                    remote_port,
                ))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .with_context(|| format!("Failed to run {}", program.display()))?;

            match wait_for_forward(&mut forward, local_port) {
                Ok(()) => {
                    drain_stderr(&mut forward);
                    return Ok(Self {
                        program: program.to_path_buf(),
                        destination: destination.to_string(),
                        socket,
                        forward,
                        local_port,
                    });
                }
                Err(error) => {
                    let _ = forward.kill();
                    let _ = forward.wait();
                    last = Some(forward_failure(
                        destination,
                        local_port,
                        remote_port,
                        &error,
                    ));
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("The SSH port forward could not be started")))
    }

    /// Where this process should dial to reach the remote session.
    fn local_address(&self) -> String {
        format!("127.0.0.1:{}", self.local_port)
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        // The forward first: killing it closes its stdin, which is also how a
        // GUI that never got here at all releases the remote end.
        let _ = self.forward.kill();
        let _ = self.forward.wait();
        // Then the master, so nothing is left holding an authenticated
        // connection. ssh unlinks the control socket as it exits.
        let _ = Command::new(&self.program)
            .args(control_exit_arguments(&self.destination, &self.socket.path))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// A local port the kernel has just confirmed is free.
///
/// Bound and released rather than guessed: a hardcoded port collides with the
/// second window, and with whatever else on the machine likes round numbers.
fn free_local_port() -> Result<u16> {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .context("Failed to reserve a local port for the SSH tunnel")?;
    let port = listener
        .local_addr()
        .context("Failed to read back the reserved local port")?
        .port();
    drop(listener);
    Ok(port)
}

/// Wait until the forward is listening, or until ssh says it will not be.
fn wait_for_forward(forward: &mut Child, local_port: u16) -> Result<()> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
    let deadline = Instant::now() + FORWARD_TIMEOUT;
    loop {
        if let Some(status) = forward
            .try_wait()
            .context("Failed to check on the SSH port forward")?
        {
            let stderr = take_stderr(forward);
            anyhow::bail!(
                "ssh exited with {} {}",
                status.code().unwrap_or(-1),
                first_meaningful_line(&stderr)
            );
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("it was not listening within {}s", FORWARD_TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Keep reading the forward's stderr so a chatty ssh cannot block on a full
/// pipe, and so a link that drops later leaves a trace in the log.
fn drain_stderr(forward: &mut Child) {
    let Some(mut stderr) = forward.stderr.take() else {
        return;
    };
    std::thread::spawn(move || {
        let mut text = String::new();
        if stderr.read_to_string(&mut text).is_ok() && !text.trim().is_empty() {
            ovim_core::log_warn!("gui", "SSH tunnel: {}", text.trim());
        }
    });
}

fn take_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut text = String::new();
    let _ = stderr.read_to_string(&mut text);
    text
}

// ── Running a child with a deadline ──────────────────────────────────────

struct CapturedOutput {
    status: std::process::ExitStatus,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Collect a child's output, giving up if it never finishes.
///
/// The bootstrap can legitimately sit for a while at a password or a 2FA
/// prompt, but a connection that hangs forever must not leave the GUI with no
/// window and no explanation.
fn wait_with_deadline(child: &mut Child, timeout: Duration) -> Result<CapturedOutput> {
    // Read on threads so neither pipe can fill and deadlock the other.
    let stdout = child.stdout.take().map(reader_thread);
    let stderr = child.stderr.take().map(reader_thread);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .context("Failed to check on the ssh client")?
        {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("ssh did not answer within {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = stdout
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    Ok(CapturedOutput {
        code: status.code(),
        status,
        stdout,
        stderr,
    })
}

fn reader_thread(mut source: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = source.read_to_string(&mut text);
        text
    })
}

// ── Turning failures into instructions ───────────────────────────────────

/// The first line of ssh's complaint that is worth showing.
///
/// ssh precedes the real message with banners and with its own reminder about
/// the authenticity of hosts, none of which identifies the problem.
fn first_meaningful_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("Warning: Permanently added"))
        .unwrap_or("ssh said nothing")
        .to_string()
}

/// Map a failed bootstrap onto something the user can act on.
///
/// Every arm names the next step. A remote feature that reports only what
/// failed leaves the user guessing between six machines' worth of causes.
fn bootstrap_failure(target: &RemoteTarget, code: Option<i32>, stderr: &str) -> anyhow::Error {
    let destination = &target.destination;
    let detail = first_meaningful_line(stderr);
    match code {
        Some(EXIT_NO_BINARY) => anyhow::anyhow!(
            "No 'ovim' was found on {destination}.\n\
             Install Ovim there and make sure a non-interactive SSH shell can see it: \
             'ssh {destination} command -v ovim' must print a path. Ovim does not upload \
             itself to the remote host."
        ),
        Some(EXIT_NO_PATH) => anyhow::anyhow!(
            "'{}' does not exist on {destination}.\n\
             The path is resolved on the remote host, not on this one, so give an absolute \
             remote path (a leading ~ is expanded there).",
            target.path
        ),
        Some(EXIT_NO_SESSION) => anyhow::anyhow!(
            "Ovim on {destination} did not finish starting a session for '{}' in time.\n\
             Run 'ssh {destination}' and then 'ovim {} --headless --session probe' by hand to \
             see what it is failing on.",
            target.path,
            target.path
        ),
        // 255 is ssh's own failure, as opposed to any exit status the remote
        // command produced, so the cause is in the transport rather than in
        // anything Ovim did.
        Some(255) | None => ssh_transport_failure(destination, &detail),
        Some(other) => anyhow::anyhow!(
            "The Ovim bootstrap on {destination} failed with exit status {other}: {detail}"
        ),
    }
}

fn ssh_transport_failure(destination: &str, detail: &str) -> anyhow::Error {
    let lowered = detail.to_ascii_lowercase();
    if lowered.contains("could not resolve")
        || lowered.contains("name or service not known")
        || lowered.contains("nodename nor servname")
        || lowered.contains("no address associated")
    {
        return anyhow::anyhow!(
            "The host in '{destination}' could not be resolved: {detail}\n\
             Check the spelling, or give it a Host entry in your SSH config if it is an alias."
        );
    }
    if lowered.contains("host key verification failed")
        || lowered.contains("remote host identification has changed")
    {
        return anyhow::anyhow!(
            "{destination} did not pass host key verification: {detail}\n\
             Run 'ssh {destination}' once by hand to see the key and decide whether to accept it. \
             Ovim will not answer that question for you."
        );
    }
    if lowered.contains("permission denied")
        || lowered.contains("too many authentication failures")
        || lowered.contains("no supported authentication")
        || lowered.contains("authentication failed")
    {
        return anyhow::anyhow!(
            "{destination} refused the SSH authentication: {detail}\n\
             Check that 'ssh {destination}' works on its own. If it needs a key, add it with \
             'ssh-add' or 'ssh-copy-id {destination}'."
        );
    }
    if lowered.contains("connection refused")
        || lowered.contains("no route to host")
        || lowered.contains("network is unreachable")
        || lowered.contains("timed out")
        || lowered.contains("connection closed")
        || lowered.contains("connection reset")
    {
        return anyhow::anyhow!(
            "{destination} could not be reached over SSH: {detail}\n\
             Check that the host is up and that it accepts SSH. If it listens on a \
             non-standard port, put the Port in your SSH config rather than in --remote."
        );
    }
    anyhow::anyhow!(
        "The SSH connection to {destination} failed: {detail}\n\
         Run 'ssh {destination}' by hand; whatever stops it there stops Ovim here."
    )
}

fn forward_failure(
    destination: &str,
    local_port: u16,
    remote_port: u16,
    error: &anyhow::Error,
) -> anyhow::Error {
    anyhow::anyhow!(
        "The tunnel from local port {local_port} to the Ovim session's port {remote_port} on \
         {destination} did not come up: {error}\n\
         Another program may have taken the local port between it being reserved and ssh binding \
         it; running the command again picks a different one. The remote session is still \
         running, so nothing was lost."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOCKET: &str = "/run/user/1000/ovim-ssh/0a1b2c3d";

    fn target() -> RemoteTarget {
        RemoteTarget {
            destination: "user@host".to_string(),
            path: "/srv/project".to_string(),
            fresh: false,
            allow_version_mismatch: false,
        }
    }

    fn descriptor(capability: &str) -> String {
        format!(
            r#"{{
  "pid": 4242,
  "port": 51829,
  "file": "/srv/project",
  "started_at": 1700000000,
  "session_name": "gui-project-1234567890",
  "capability": "{capability}",
  "lsp_ready": true,
  "start_time": 9876
}}"#
        )
    }

    fn report(capability: &str, state: &str, version: &str) -> String {
        format!(
            "{REPORT_BANNER}\n\
             binary=/usr/local/bin/ovim\n\
             version={version}\n\
             state={state}\n\
             project=/srv/project\n\
             {REPORT_DESCRIPTOR}\n{}\n",
            descriptor(capability)
        )
    }

    #[test]
    fn the_bootstrap_multiplexes_and_takes_its_script_on_standard_input() {
        let arguments = bootstrap_arguments("user@host", Path::new(SOCKET));

        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-o".to_string(), "ControlMaster=auto".to_string()]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-o".to_string(), format!("ControlPath={SOCKET}")]));
        assert!(arguments.windows(2).any(|pair| pair
            == [
                "-o".to_string(),
                format!("ControlPersist={CONTROL_PERSIST}")
            ]));
        // The script is not an argument, so it is neither in the remote
        // process table nor mangled by an exotic login shell.
        assert_eq!(
            &arguments[arguments.len() - 3..],
            ["user@host", "/bin/sh", "-s"]
        );
    }

    #[test]
    fn the_forward_reuses_the_master_and_refuses_to_run_without_its_port() {
        let arguments = forward_arguments("user@host", Path::new(SOCKET), 40001, 51829);

        // ControlMaster=no is what keeps this invocation from authenticating
        // on its own, which is the whole point of multiplexing.
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-o".to_string(), "ControlMaster=no".to_string()]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-o".to_string(), "ExitOnForwardFailure=yes".to_string()]));
        assert!(arguments.windows(2).any(|pair| pair
            == [
                "-L".to_string(),
                "127.0.0.1:40001:127.0.0.1:51829".to_string()
            ]));
        assert_eq!(&arguments[arguments.len() - 2..], ["user@host", "cat"]);
    }

    #[test]
    fn the_forward_binds_loopback_at_both_ends() {
        let arguments = forward_arguments("user@host", Path::new(SOCKET), 40001, 51829);
        let forward = arguments
            .iter()
            .position(|argument| argument == "-L")
            .map(|index| arguments[index + 1].clone())
            .expect("the forward invocation should carry a -L specification");

        // A wildcard local bind would put a capability-protected editor on the
        // laptop's network interfaces, and the session asserts a loopback bind
        // on its own side.
        assert!(forward.starts_with("127.0.0.1:"));
        assert!(forward.contains(":127.0.0.1:"));
    }

    #[test]
    fn teardown_asks_the_master_to_exit_by_its_own_socket() {
        let arguments = control_exit_arguments("user@host", Path::new(SOCKET));

        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-O".to_string(), "exit".to_string()]));
        assert!(arguments.contains(&format!("ControlPath={SOCKET}")));
        assert_eq!(arguments.last().map(String::as_str), Some("user@host"));
    }

    #[test]
    fn no_ssh_invocation_carries_the_session_capability() {
        let capability = "b3a1f00dcafeb3a1f00dcafeb3a1f00dcafeb3a1f00dcafeb3a1f00dcafeb3a1";
        let parsed = parse_report(&report(capability, "reattached", "ovim 1.2.7"))
            .expect("the report should parse");
        let vectors = [
            bootstrap_arguments("user@host", Path::new(SOCKET)),
            forward_arguments("user@host", Path::new(SOCKET), 40001, parsed.session.port),
            control_exit_arguments("user@host", Path::new(SOCKET)),
        ];

        for arguments in vectors {
            for argument in arguments {
                assert!(
                    !argument.contains(capability),
                    "a capability reached the process table in {argument}"
                );
            }
        }
        assert!(!bootstrap_script("/srv/project", false, 30).contains(capability));
        // And the capability really did survive the trip, so the assertion
        // above is not passing because nothing was parsed.
        assert_eq!(parsed.session.capability.expose_secret(), capability);
    }

    #[test]
    fn the_bootstrap_script_quotes_a_path_that_would_otherwise_run_a_command() {
        let script = bootstrap_script("/srv/it's here; rm -rf /", false, 30);

        assert!(script.contains(r"project='/srv/it'\''s here; rm -rf /'"));
        // Nothing outside the quoted assignment reintroduces the path.
        assert!(!script.contains("rm -rf /\n"));
    }

    #[test]
    fn the_bootstrap_script_reattaches_before_it_starts_anything() {
        let script = bootstrap_script("/srv/project", false, 30);

        assert!(script.contains("fresh=0"));
        assert!(script.contains("state=reattached"));
        assert!(script.contains("--headless --session"));
        // The session name is derived from the resolved path, which is what
        // makes the next launch find this same session.
        assert!(script.contains(r#"name="gui-$stem-$sum""#));
    }

    #[test]
    fn a_fresh_launch_replaces_the_session_instead_of_reattaching() {
        let script = bootstrap_script("/srv/project", true, 30);

        assert!(script.contains("fresh=1"));
        assert!(script.contains(r#"kill -9 "$pid""#));
    }

    #[cfg(unix)]
    #[test]
    fn a_forward_counts_as_up_once_something_answers_on_the_local_port() {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("a loopback listener should be bindable");
        let port = listener.local_addr().unwrap().port();
        // Stands in for the ssh client: alive, and not the thing being probed.
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("a shell should be runnable");

        let outcome = wait_for_forward(&mut child, port);

        let _ = child.kill();
        let _ = child.wait();
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_client_that_exits_instead_of_forwarding_reports_what_it_said() {
        // ExitOnForwardFailure=yes is what turns a port that could not be
        // bound into this, rather than into a connection with no tunnel.
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "echo 'bind [127.0.0.1]:40001: Address already in use' >&2; exit 255",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("a shell should be runnable");

        let error = wait_for_forward(&mut child, 1)
            .expect_err("a client that exited cannot have forwarded anything")
            .to_string();

        assert!(error.contains("255"));
        assert!(error.contains("Address already in use"));
    }

    #[cfg(unix)]
    #[test]
    fn a_forward_that_can_never_bind_gives_up_instead_of_retrying_forever() {
        let workspace = tempfile::tempdir().expect("a temporary directory should be creatable");
        let program = workspace.path().join("ssh");
        std::fs::write(
            &program,
            "#!/bin/sh\necho 'bind: Address already in use' >&2\nexit 255\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let socket = ControlSocket::create().expect("a control socket path should be available");

        let error = SshTunnel::open(&program, socket, "user@host", 51829)
            .expect_err("a client that always fails should not look like a tunnel")
            .to_string();

        assert!(error.contains("51829"));
        assert!(error.contains("still running"));
    }

    #[test]
    fn a_reattached_session_is_reported_with_its_descriptor() {
        let parsed = parse_report(&report(
            "ab".repeat(32).as_str(),
            "reattached",
            "ovim 1.2.7",
        ))
        .expect("the report should parse");

        assert_eq!(parsed.origin, SessionOrigin::Reattached);
        assert_eq!(parsed.binary, "/usr/local/bin/ovim");
        assert_eq!(parsed.version, "ovim 1.2.7");
        assert_eq!(parsed.project, "/srv/project");
        assert_eq!(parsed.session.port, 51829);
        assert_eq!(parsed.session.session_name, "gui-project-1234567890");
    }

    #[test]
    fn a_started_session_is_told_apart_from_a_reattached_one() {
        let parsed = parse_report(&report("cd".repeat(32).as_str(), "started", "ovim 1.2.7"))
            .expect("the report should parse");

        assert_eq!(parsed.origin, SessionOrigin::Started);
    }

    #[test]
    fn output_without_the_banner_is_refused_rather_than_guessed_at() {
        // A login shell that prints a message of the day is the ordinary way
        // this happens, and reading past it would mean parsing anything.
        let noise = format!(
            "Welcome to host\n{}",
            report(&"ef".repeat(32), "started", "ovim 1.2.7")
        );

        assert!(parse_report(&noise).is_err());
        assert!(parse_report(&format!("{REPORT_BANNER}\nstate=started\n")).is_err());
    }

    #[test]
    fn identical_versions_agree_and_a_patch_difference_only_drifts() {
        assert_eq!(
            compare_versions("1.2.7", "ovim 1.2.7"),
            VersionVerdict::Same
        );
        assert_eq!(
            compare_versions("1.2.7", "ovim 1.2.9"),
            VersionVerdict::PatchDrift
        );
    }

    #[test]
    fn a_feature_release_apart_is_incompatible_in_either_direction() {
        assert_eq!(
            compare_versions("1.2.7", "ovim 1.3.0"),
            VersionVerdict::Incompatible
        );
        assert_eq!(
            compare_versions("1.3.0", "ovim 1.2.7"),
            VersionVerdict::Incompatible
        );
        assert_eq!(
            compare_versions("1.2.7", "ovim 2.0.0"),
            VersionVerdict::Incompatible
        );
    }

    #[test]
    fn a_version_that_cannot_be_read_is_not_silently_treated_as_a_match() {
        assert_eq!(compare_versions("1.2.7", ""), VersionVerdict::Unreadable);
        assert_eq!(
            compare_versions("1.2.7", "sh: ovim: not found"),
            VersionVerdict::Unreadable
        );
    }

    #[test]
    fn a_prerelease_suffix_does_not_make_two_equal_versions_differ() {
        assert_eq!(
            compare_versions("1.2.7", "ovim v1.2.7-dev"),
            VersionVerdict::Same
        );
    }

    #[test]
    fn a_feature_mismatch_stops_the_launch_and_names_the_override() {
        let parsed = parse_report(&report(&"11".repeat(32), "started", "ovim 9.9.9"))
            .expect("the report should parse");

        let refusal = check_versions(&target(), &parsed)
            .expect_err("a feature release apart should not connect silently");
        let message = refusal.to_string();
        assert!(message.contains("9.9.9"));
        assert!(message.contains("--allow-version-mismatch"));

        let allowed = RemoteTarget {
            allow_version_mismatch: true,
            ..target()
        };
        assert!(check_versions(&allowed, &parsed).is_ok());
    }

    #[test]
    fn a_missing_remote_ovim_says_how_to_check_the_path_it_looked_on() {
        let message = bootstrap_failure(&target(), Some(EXIT_NO_BINARY), "").to_string();

        assert!(message.contains("command -v ovim"));
        assert!(message.contains("does not upload"));
    }

    #[test]
    fn a_missing_remote_path_says_which_host_resolves_it() {
        let message = bootstrap_failure(&target(), Some(EXIT_NO_PATH), "").to_string();

        assert!(message.contains("/srv/project"));
        assert!(message.contains("resolved on the remote host"));
    }

    #[test]
    fn a_session_that_never_appears_says_how_to_watch_it_fail() {
        let message = bootstrap_failure(&target(), Some(EXIT_NO_SESSION), "").to_string();

        assert!(message.contains("--headless --session"));
    }

    #[test]
    fn an_unresolvable_host_is_told_apart_from_a_refused_one() {
        let dns = bootstrap_failure(
            &target(),
            Some(255),
            "ssh: Could not resolve hostname host: Name or service not known",
        )
        .to_string();
        assert!(dns.contains("could not be resolved"));
        assert!(dns.contains("SSH config"));

        let refused = bootstrap_failure(
            &target(),
            Some(255),
            "ssh: connect to host port 22: Connection refused",
        )
        .to_string();
        assert!(refused.contains("could not be reached"));
        assert!(refused.contains("non-standard port"));
    }

    #[test]
    fn rejected_authentication_points_at_the_key_rather_than_at_the_network() {
        let message = bootstrap_failure(
            &target(),
            Some(255),
            "user@host: Permission denied (publickey).",
        )
        .to_string();

        assert!(message.contains("refused the SSH authentication"));
        assert!(message.contains("ssh-copy-id user@host"));
    }

    #[test]
    fn a_changed_host_key_is_a_question_for_the_user_not_for_ovim() {
        let message =
            bootstrap_failure(&target(), Some(255), "Host key verification failed.").to_string();

        assert!(message.contains("host key verification"));
        assert!(message.contains("will not answer that question"));
    }

    #[test]
    fn a_forward_that_never_listens_says_the_remote_session_survived_it() {
        let message = forward_failure(
            "user@host",
            40001,
            51829,
            &anyhow::anyhow!("ssh exited with 255 bind: Address already in use"),
        )
        .to_string();

        assert!(message.contains("40001"));
        assert!(message.contains("51829"));
        assert!(message.contains("still running"));
    }

    #[test]
    fn ssh_banners_are_skipped_when_quoting_the_reason_a_connection_failed() {
        let stderr = "\nWarning: Permanently added 'host' (ED25519) to the list of known hosts.\n\
                      user@host: Permission denied (publickey).\n";

        assert_eq!(
            first_meaningful_line(stderr),
            "user@host: Permission denied (publickey)."
        );
    }

    #[test]
    fn no_remote_flags_at_all_means_the_editor_runs_here() {
        assert!(RemoteOptions::default()
            .resolve()
            .expect("a local launch should resolve")
            .is_none());
    }

    #[test]
    fn the_two_ways_of_saying_where_the_editor_is_cannot_be_combined() {
        let both = RemoteOptions {
            destination: Some("user@host".to_string()),
            session_file: Some(PathBuf::from("dev.json")),
            ..RemoteOptions::default()
        };

        let message = both
            .resolve()
            .expect_err("both at once is ambiguous")
            .to_string();
        assert!(message.contains("use one"));
    }

    #[test]
    fn a_remote_launch_without_a_path_says_what_the_command_should_look_like() {
        let pathless = RemoteOptions {
            destination: Some("user@host".to_string()),
            ..RemoteOptions::default()
        };

        let message = pathless
            .resolve()
            .expect_err("there is nothing to open")
            .to_string();
        assert!(message.contains("--remote user@host /path/to/project"));
    }

    #[test]
    fn a_destination_that_would_be_read_as_an_option_is_refused() {
        assert!(validate_destination("user@host").is_ok());
        assert!(validate_destination("build-box").is_ok());
        assert!(validate_destination("-oProxyCommand=touch /tmp/pwned").is_err());
        assert!(validate_destination("user@host --oops").is_err());
        assert!(validate_destination("").is_err());
    }

    #[test]
    fn a_reserved_local_port_is_free_when_it_is_handed_over() {
        let port = free_local_port().expect("the kernel should hand out a loopback port");

        assert!(port > 0);
        // Free again immediately, which is what lets ssh bind it.
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
            .expect("the reserved port should have been released");
    }

    #[test]
    fn the_control_socket_stays_well_inside_the_unix_path_limit() {
        let socket = ControlSocket::create().expect("a control socket path should be available");

        // ssh binds the path plus a nine byte suffix before renaming it, and
        // the kernel truncates rather than complaining.
        assert!(socket.path.as_os_str().len() + 9 <= 104);
        assert!(socket.path.parent().is_some_and(Path::exists));
    }

    /// A stand-in for `ovim` on the far side.
    ///
    /// It answers `--version` and, when asked for a headless session, writes a
    /// descriptor and stays alive -- which is exactly the state the reattach
    /// branch looks for.
    #[cfg(unix)]
    fn stub_ovim(directory: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("ovim");
        std::fs::write(
            &path,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "ovim VERSION"; exit 0; fi
project=$1
shift
name=
while [ $# -gt 0 ]; do
  if [ "$1" = "--session" ]; then shift; name=$1; fi
  shift
done
# Laid out the way serde_json::to_string_pretty writes a real descriptor.
cat > "$OVIM_SESSION_DIR/$name.json" <<EOF
{
  "pid": $$,
  "port": 51829,
  "file": "$project",
  "started_at": 1,
  "session_name": "$name",
  "capability": "cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000",
  "lsp_ready": false,
  "start_time": null
}
EOF
# No exec: a real Ovim keeps the arguments it was started with, and the
# launcher reads them back to tell this session from a recycled pid.
trap 'kill $waiting 2>/dev/null; exit 0' TERM
sleep 30 &
waiting=$!
wait
"#
            .replace("VERSION", env!("CARGO_PKG_VERSION")),
        )
        .expect("the stub should be writable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the stub should be executable");
        path
    }

    /// Run the bootstrap script the way `ssh` would: `/bin/sh -s`, script on
    /// stdin, and nothing about it as an argument.
    #[cfg(unix)]
    fn run_script(script: &str, bin_directory: &Path, session_directory: &Path) -> CapturedOutput {
        let path = format!(
            "{}:{}",
            bin_directory.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = Command::new("/bin/sh")
            .arg("-s")
            .env("PATH", path)
            .env("OVIM_SESSION_DIR", session_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("a POSIX shell should be runnable");
        child
            .stdin
            .take()
            .expect("the shell should accept input")
            .write_all(script.as_bytes())
            .expect("the script should be writable to the shell");
        wait_with_deadline(&mut child, Duration::from_secs(60))
            .expect("the bootstrap script should finish")
    }

    #[cfg(unix)]
    #[test]
    fn the_bootstrap_script_starts_a_session_and_then_finds_it_again() {
        let workspace = tempfile::tempdir().expect("a temporary directory should be creatable");
        let sessions = workspace.path().join("sessions");
        let binaries = workspace.path().join("bin");
        let project = workspace.path().join("project");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        stub_ovim(&binaries);
        let script = bootstrap_script(&project.to_string_lossy(), false, 20);

        let first = run_script(&script, &binaries, &sessions);
        assert!(
            first.status.success(),
            "the first launch should start a session: {}",
            first.stderr
        );
        let started = parse_report(&first.stdout).expect("the first report should parse");
        assert_eq!(started.origin, SessionOrigin::Started);
        assert_eq!(started.session.port, 51829);

        // The second launch must find the first one rather than start a rival:
        // that is what makes closing the laptop and coming back cheap.
        let second = run_script(&script, &binaries, &sessions);
        let reattached = parse_report(&second.stdout).expect("the second report should parse");
        assert_eq!(reattached.origin, SessionOrigin::Reattached);
        assert_eq!(
            reattached.session.session_name, started.session.session_name,
            "the same project must resolve to the same session name"
        );
        assert!(reattached.session.session_name.starts_with("gui-project-"));

        // And the versions agree, because the stub reports this build's.
        assert_eq!(
            compare_versions(env!("CARGO_PKG_VERSION"), &started.version),
            VersionVerdict::Same
        );

        let _ = Command::new("kill")
            .arg(started.session.pid.to_string())
            .status();
    }

    #[cfg(unix)]
    #[test]
    fn a_recycled_pid_is_not_mistaken_for_the_session_that_wrote_the_descriptor() {
        let workspace = tempfile::tempdir().expect("a temporary directory should be creatable");
        let sessions = workspace.path().join("sessions");
        let binaries = workspace.path().join("bin");
        let project = workspace.path().join("project");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        stub_ovim(&binaries);
        let project = project.to_string_lossy().into_owned();

        // Something that is emphatically not an Ovim session, standing in for
        // whatever inherits the number after the session it named is gone.
        let mut bystander = Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a shell should be runnable");

        // Kill the session outright -- so the descriptor survives, as it does
        // after a crash -- and leave its pid pointing at the bystander.
        let orphan = |session: &SessionInfo| {
            let _ = Command::new("kill")
                .args(["-9", &session.pid.to_string()])
                .status();
            let path = sessions.join(format!("{}.json", session.session_name));
            let rewritten = std::fs::read_to_string(&path)
                .expect("the descriptor should outlive the process that wrote it")
                .replace(
                    &format!("\"pid\": {}", session.pid),
                    &format!("\"pid\": {}", bystander.id()),
                );
            std::fs::write(&path, rewritten).unwrap();
        };

        let first = run_script(&bootstrap_script(&project, false, 20), &binaries, &sessions);
        orphan(&parse_report(&first.stdout).expect("the first report should parse").session);

        // Reattaching here would tunnel to a port the bystander never served.
        let replaced = parse_report(
            &run_script(&bootstrap_script(&project, false, 20), &binaries, &sessions).stdout,
        )
        .expect("the second report should parse");
        assert_eq!(
            replaced.origin,
            SessionOrigin::Started,
            "a pid that is not this session must not be reattached to"
        );

        // And --fresh is the dangerous direction, because it kills what it finds.
        orphan(&replaced.session);
        let refreshed = parse_report(
            &run_script(&bootstrap_script(&project, true, 20), &binaries, &sessions).stdout,
        )
        .expect("the third report should parse");
        assert!(
            bystander
                .try_wait()
                .expect("the bystander should be waitable")
                .is_none(),
            "--fresh killed a process that was never an Ovim session"
        );

        let _ = bystander.kill();
        let _ = bystander.wait();
        let _ = Command::new("kill")
            .arg(refreshed.session.pid.to_string())
            .status();
    }

    #[cfg(unix)]
    #[test]
    fn the_bootstrap_script_names_the_two_things_it_cannot_do_without() {
        let workspace = tempfile::tempdir().expect("a temporary directory should be creatable");
        let sessions = workspace.path().join("sessions");
        let binaries = workspace.path().join("bin");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&binaries).unwrap();

        // An empty bin directory and a PATH that has no ovim on it. The test
        // binary's PATH might, so the check is on the exit status meaning
        // rather than on it always being reached.
        let missing_path = bootstrap_script("/nonexistent/project/dir", false, 2);
        stub_ovim(&binaries);
        let refused = run_script(&missing_path, &binaries, &sessions);

        assert_eq!(refused.code, Some(EXIT_NO_PATH));
        let message = bootstrap_failure(&target(), refused.code, &refused.stderr).to_string();
        assert!(message.contains("resolved on the remote host"));
    }

    #[test]
    fn an_endpoint_built_from_a_tunnel_dials_locally_but_speaks_the_remote_port() {
        let session: SessionInfo =
            serde_json::from_str(&descriptor(&"22".repeat(32))).expect("the descriptor parses");
        let endpoint = RemoteEndpoint::from_session(&session, Some("127.0.0.1:40001"))
            .expect("a forwarded endpoint should be buildable");

        // The Host guard on the session compares against its own port, so the
        // two ports differing is the normal case rather than a mistake.
        let rendered = format!("{endpoint:?}");
        assert!(rendered.contains("40001"));
        assert!(rendered.contains("51829"));
        // And the capability is redacted even in debug output.
        assert!(!rendered.contains(&"22".repeat(32)));
    }
}
