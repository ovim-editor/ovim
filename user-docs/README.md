# ovim User Documentation

This folder is the user-facing documentation for running and configuring ovim.

## Getting Started

```bash
ovim file.txt                               # Open a file
ovim file.rs --headless --session dev       # Headless mode with named session
```

## Docs

- [Getting Started](getting-started.md) - Build/install, open files, basic workflow
- [Configuration](configuration.md) - `init.lua`, `languages.toml`, and common tweaks
- [Headless & Automation](headless.md) - Sessions, REST API, `ovim` subcommands, `ovim-ctl`
- [Language Support](LANGUAGE_SUPPORT.md) - LSP + syntax support and adding languages
- [Options](options.md) - `:set` options reference (scrolling, wrap, clipboard, etc.)
- [Troubleshooting](troubleshooting.md) - Common issues (sessions, LSP, dependencies)
