//! LSP Daemon Mode
//!
//! Keeps jdtls (or other LSP servers) alive between editor sessions for instant startup.
//!
//! ## Architecture
//!
//! - **Client**: ovim process that connects to daemon
//! - **Daemon**: Background process that manages LSP server
//! - **Protocol**: Request/response over Unix domain socket
//!
//! Daemon processes use stderr for debugging output.
#![allow(clippy::print_stderr)]

//! ## Safety Features
//!
//! - **PID Verification**: Prevents PID reuse attacks (start time + cmdline hash)
//! - **Process Killing**: Robust termination with escalation (SIGTERM → SIGKILL)
//! - **File Locking**: Prevents race conditions during daemon startup
//! - **Stale Detection**: Identifies and cleans up dead daemons
//!
//! ## Usage
//!
//! ```text
//! # Pseudocode (API subject to change)
//! # Start or connect to daemon for a project
//! let daemon = connect_or_start_daemon(project_root);
//!
//! # Send LSP requests
//! let response = daemon.send_request(Hover { uri, position });
//!
//! # Daemon stays alive after client disconnects
//! ```

pub mod lock;
pub mod pid;
pub mod process;
pub mod protocol;

pub use lock::DaemonLock;
pub use pid::{get_process_cmdline, get_process_start_time, process_exists, DaemonPidInfo};
pub use process::{
    get_process_state, kill_process_forcefully, ProcessKillStatus, RogueProcess,
    RogueProcessTracker,
};
pub use protocol::{DaemonRequest, DaemonResponse, LspPosition, LspRange};
