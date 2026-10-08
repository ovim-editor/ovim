# Claude runtime

The Claude Agent SDK (`@anthropic-ai/claude-agent-sdk`) is not open source, so
Ovim does not redistribute it. `sdk_install.rs` installs the pinned version
(`SDK_VERSION` in `claude_code.rs`) with npm into
`~/.cache/ovim/claude-agent-sdk/<version>/` on the first Claude Agent turn,
skipping optional dependencies (bundled Claude executables), peer dependencies
and install scripts. The installed `sdk.mjs` must match `SDK_SHA256`.
Ovim does not include or modify the Claude Code executable.

To bump the SDK, update `SDK_VERSION` and `SDK_SHA256`, then confirm the pin:

```sh
cargo test -p ovim-core --lib sdk_install -- --include-ignored
npm ci --prefix ovim/claude-runtime --omit=optional --ignore-scripts
npm test --prefix ovim/claude-runtime
```

The helper and SDK are copied to a private temporary directory for each turn.
It requires Node 18.18+ with npm, and `claude` on PATH. Claude Code owns
authentication and uses its normal environment and settings. Ovim never reads
its credential files.
