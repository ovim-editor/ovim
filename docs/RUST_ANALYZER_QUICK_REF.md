# Rust-Analyzer LSP Quick Reference

## TL;DR

1. **Wait 15 seconds** after `didOpen` before hover works
2. **Use UTF-16** for position encoding (not UTF-8 chars)
3. **Use `\r\n`** for protocol headers (not `\n`)

## Message Format

```
Content-Length: {byte_count}\r\n
\r\n
{json_message}
```

## Initialization Sequence

```
1. initialize (id:1) → wait for response
2. initialized (notification) → wait 1s
3. textDocument/didOpen (notification) → wait 15s
4. Now hover/goto work!
```

## Position Encoding

```rust
// ASCII: 1 char = 1 UTF-16 unit
"hello" → position of 'e' = 1 ✓

// Emoji: 1 char = 2 UTF-16 units
"😀hi" → position of 'h' = 2 (not 1!) ⚠️

// Conversion function
fn char_to_utf16(line: &str, char_idx: usize) -> usize {
    line.chars().take(char_idx).map(|c| c.len_utf16()).sum()
}
```

## Hover Response

```json
{
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "```rust\nstruct Foo\n```\nDocs here"
    },
    "range": {...}
  }
}
```

## Common Issues

| Issue | Cause | Fix |
|-------|-------|-----|
| Hover returns `null` | Workspace indexing not done | Wait 15s after didOpen |
| Wrong hover position | Using char offset, not UTF-16 | Convert to UTF-16 |
| "Malformed header" error | Using `\n` not `\r\n` | Use CRLF line endings |
| Shutdown fails | Using `{}` for params | Use `null` |

## Testing Command

```bash
/workspace/test_ra_workspace.sh
python3 /workspace/parse_lsp_output.py /tmp/ra_workspace_output.log
```

## Key Timing

- Initialize response: 500ms
- Diagnostics: 500ms after didOpen
- **Workspace indexing: 10-15s** ⚠️
- Hover response: 100-500ms (after indexing)

## Files

- Full report: `docs/rust_analyzer_manual_test_report.md`
- Message examples: `docs/rust_analyzer_working_messages.json`
- Integration guide: `docs/RUST_ANALYZER_INTEGRATION_GUIDE.md`
