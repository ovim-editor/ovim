# Claude runtime

`sdk.mjs.gz` is the unmodified `sdk.mjs` from the official
`@anthropic-ai/claude-agent-sdk` npm package, version **0.3.278**.
Uncompressed SHA-256:
`d768bb75542ea853a66c570bd82b010a7b843f4949ab3ebb20e9d1f2a27af881`.

Copyright Anthropic PBC. All rights reserved. SDK use is subject to
<https://code.claude.com/docs/en/legal-and-compliance>.
The SDK's copyright and terms notice remains in the module.
Ovim does not include or modify the Claude Code executable.

To reproduce the compressed asset:

```sh
npm ci --prefix ovim/claude-runtime --omit=optional --ignore-scripts
npm run vendor --prefix ovim/claude-runtime
npm test --prefix ovim/claude-runtime
```

The helper and SDK are extracted to a private temporary directory for each
turn. It requires Node 18.18+ and `claude` on PATH. No npm installation happens
at runtime. Claude Code owns authentication and uses its normal environment
and settings. Ovim never reads its credential files.
