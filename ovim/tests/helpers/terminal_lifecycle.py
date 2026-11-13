"""Real Unix PTY regression for terminal handoff, repaint, and chat dismissal.

Invoked by shell_commands_test.rs with an Ovim binary and isolated workspace.
Only terminal capability replies are emulated; input goes through the real TUI
and real interactive shell. No AI turn is submitted.
"""

import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import time

binary = str(Path(sys.argv[1]).resolve())
root = Path(sys.argv[2]).resolve()
subprocess.run(["git", "init", "-q", str(root)], check=True)
source = root / "terminal-qa.txt"
source.write_text("keep this buffer\n")
preference = root / "chat-preference.json"
preference.write_text(json.dumps({
    "version": 1,
    "selection": {"profile": "claude_code", "provider": "claude_code", "model": None},
}))

pid, fd = pty.fork()
if pid == 0:
    for kind in ["CONFIG", "DATA", "STATE", "CACHE"]:
        os.environ[f"XDG_{kind}_HOME"] = str(root / kind.lower())
    os.environ["OVIM_CHAT_PREFERENCE_FILE"] = str(preference)
    os.environ["SHELL"] = "/bin/sh"
    os.environ["PS1"] = "ovim-test$ "
    os.environ["TERM"] = "xterm-256color"
    os.chdir(root)
    os.execv(binary, [binary, "--dimension", "100x30", str(source)])

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
transcript = bytearray()
pending = bytearray()
reaped = False


def wait_for(marker, timeout=15):
    deadline = time.monotonic() + timeout
    while marker not in pending:
        remaining = deadline - time.monotonic()
        assert remaining > 0, f"Timed out waiting for {marker!r}; tail={pending[-1500:]!r}"
        ready, _, _ = select.select([fd], [], [], remaining)
        if not ready:
            continue
        data = os.read(fd, 65536)
        assert data, f"PTY closed waiting for {marker!r}"
        # Crossterm keyboard negotiation and graphics capability probes must
        # finish before the application's event reader owns stdin.
        if b"\x1b[?u" in data:
            os.write(fd, b"\x1b[?0u\x1b[?1;2c")
        if b"\x1b[5n" in data:
            os.write(fd, b"\x1b[0n")
        if b"\x1b[6n" in data:
            os.write(fd, b"\x1b[1;1R")
        transcript.extend(data)
        pending.extend(data)
    del pending[:pending.index(marker) + len(marker)]


def send(data):
    os.write(fd, data)


try:
    wait_for(b"NORMAL")
    pending.clear()
    send(b":term\r")
    wait_for(b"\x1b[?1049l")
    wait_for(b"$ ")
    send(b"stty -echo; printf '\\033]0;child-title\\007\\033[5 qTUI-shell-ready\\n'\n")
    wait_for(b"TUI-shell-ready\r\n")
    send(b"exit\n")
    wait_for(b"\x1b[?1049h")
    wait_for(b"\x1b]0;terminal-qa.txt")
    # Re-emitting unchanged source and status proves the renderer invalidated
    # its back buffer, instead of painting only the changed command message.
    wait_for(b"keep")
    wait_for(b"NORMAL")
    print("PASS interactive :term input, exit, and full editor repaint", flush=True)

    pending.clear()
    send(b":term printf 'pipeline check' | tr a-z A-Z\r")
    wait_for(b"\x1b[?1049l")
    wait_for(b"PIPELINE CHECK")
    wait_for(b"\x1b[?1049h")
    wait_for(b"NORMAL")
    print("PASS quoted :term command and pipeline", flush=True)

    pending.clear()
    send(b":term exit 7\r")
    wait_for(b"\x1b[?1049l")
    wait_for(b"\x1b[?1049h")
    wait_for(b"shell returned")
    print("PASS nonzero exit restores editor and reports failure", flush=True)

    pending.clear()
    send(b":!printf 'bang check'\r")
    wait_for(b"\x1b[?1049l")
    wait_for(b"bang check")
    wait_for(b"Press ENTER to continue")
    send(b"\r")
    wait_for(b"\x1b[?1049h")
    wait_for(b"NORMAL")
    print("PASS :! acknowledgement owns stdin and repaints", flush=True)

    pending.clear()
    send(b"  ")
    wait_for(b"AI")
    send(b"\x1b")
    wait_for(b"NORMAL")
    send(b"iAFTER-CHAT")
    wait_for(b"INSERT")
    # Wait for each mode transition, so Escape is not interpreted as an Alt
    # prefix on the following colon (real terminals make that distinction).
    send(b"\x1b")
    wait_for(b"NORMAL")
    send(b":w\r")
    expected = "AFTER-CHATkeep this buffer\n"
    deadline = time.monotonic() + 15
    # Ratatui may split the written status across cursor-position sequences.
    # Synchronize on the saved file, the actual downstream result of :w.
    while source.read_text() != expected:
        remaining = deadline - time.monotonic()
        assert remaining > 0, f"Save did not complete: {source.read_text()!r}"
        ready, _, _ = select.select([fd], [], [], min(remaining, 0.05))
        if ready:
            transcript.extend(os.read(fd, 65536))
    print("PASS chat dismissal returns to source; edit and save work", flush=True)

    send(b":qa!\r")
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        result, status = os.waitpid(pid, os.WNOHANG)
        if result:
            reaped = True
            assert os.waitstatus_to_exitcode(status) == 0, status
            break
        select.select([], [], [], 0.05)
    assert reaped, "Ovim did not exit"
finally:
    (root / "pty-output.bin").write_bytes(transcript)
    if not reaped:
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(pid, 0)
    os.close(fd)
