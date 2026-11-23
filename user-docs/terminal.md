# Terminal sessions

## Native GUI

Click the Terminal icon in the left activity bar, choose **View → Terminal**, or
press **Cmd+Shift+T** on macOS or **Ctrl+Shift+T** elsewhere to open or close the
integrated terminal. The terminal opens an interactive shell in the current
workspace directory. Typing, paste, and terminal shortcuts go directly to the
shell while it has focus.

Use `:term`, `:terminal`, or `:shell` to open the same terminal. A bare command
reveals an existing session; `:term <command>` starts a fresh session running that
command (including shell pipelines).

The Terminal action and shortcut close the terminal when it is selected. The
**Close terminal** button also ends the shell and removes its panel. Exiting the
shell (`exit` or Ctrl-D on Unix) closes the panel automatically. Opening it again
starts a fresh shell.

Switching to AI chat or another context tab preserves a running shell and its
scrollback. Closing chat returns focus to the editor without revealing the
terminal; select Terminal explicitly to return to it. Closing Ovim ends the
terminal session.

The integrated terminal is available in the native GUI; the browser development
preview does not start local processes.

## Terminal frontend

In the terminal frontend, use `:terminal` or `:term` to leave Ovim temporarily
and start your configured interactive shell:

```vim
:terminal
```

Ovim uses `SHELL` on Unix and `COMSPEC` on Windows, with `/bin/sh` and
`cmd.exe` as fallbacks. Exit the shell normally (`exit` or Ctrl-D on Unix) to
return to the same editor session.

You can also run a specific program or command:

```vim
:terminal lazygit
:term cargo test
```

`:shell` is an alias for opening the interactive shell. For a non-interactive
command whose output you want to inspect, continue to use `:!command`; Ovim
shows that output and waits for Enter before returning.

## Command scope

In the TUI, terminal sessions temporarily use the terminal that launched Ovim.
They are not PTY-backed Ovim buffers, so there is no terminal scrollback buffer, split-window
terminal, or terminal-mode keymap yet. The TUI event loop is paused until the
child exits. The GUI handles the same commands through its integrated PTY panel.
The headless command API rejects interactive sessions.
