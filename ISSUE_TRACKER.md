# ovim Issue Tracker

| ID | Status | Priority | Complexity | Description |
|----------|---------|----------|------------|-------------|
| OV-00001 | Fixed | HIGH | Medium | [NAV] Ex command `:N` doesn't update cursor position - Fixed: Added line-number command parsing to src/commands.rs |
| OV-00002 | Fixed | HIGH | Medium | [NAV] Normal mode `gg` doesn't update cursor - Fixed: Code analysis confirmed implementation correct; also fixed client error handling in src/client.rs |
| OV-00003 | Pending | MEDIUM | Low | [MCP] `symbols` command fails with "Unknown tool: get_symbols" - should fall back to standard LSP textDocument/documentSymbol if MCP tool unavailable (cli/symbols.rs) |
| OV-00004 | Pending | MEDIUM | Low | [MCP] `diagnostics` command fails with "Unknown tool: get_diagnostics" - should fall back to standard LSP textDocument/publishDiagnostics cache (cli/diagnostics.rs) |
| OV-00005 | Pending | LOW | Low | [STATE] `snapshot` returns null for cursor_line, cursor_col, file fields - state not being serialized correctly (cli/snapshot.rs) |
| OV-00006 | Pending | TRIAGE | Medium | [LSP] `hover` returns null on valid symbol positions - investigate if this is ovim not forwarding request correctly or LSP response handling issue |
| OV-00007 | Pending | TRIAGE | Medium | [LSP] `find-references` returns empty array - investigate if ovim is correctly forwarding LSP request and parsing response |

## Bugs Filed Against Hyperion (if any)

These may belong in Hyperion's tracker after investigation:

| ID | Status | Priority | Description |
|----|--------|----------|-------------|
| HY-TRIAGE-01 | Triage | UNKNOWN | hover not returning info for method calls - may be unimplemented in hyperion-lsp |
| HY-TRIAGE-02 | Triage | UNKNOWN | find-references returning empty - may be unimplemented in hyperion-lsp |
| HY-TRIAGE-03 | Triage | UNKNOWN | goto-definition fails on DTO type references (AdminSqlRequest) - possibly classpath/dependency resolution |

## Notes

- OV-00001 fixed by adding goto-line parsing to commands.rs; OV-00002 fixed by improving client error handling
- OV-00003 and OV-00004 are MCP vs LSP protocol mismatch - ovim uses MCP, some LSPs don't implement those tools
- HY-TRIAGE items need investigation to determine if they're hyperion-lsp gaps or ovim request/response issues
