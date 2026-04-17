# 16: Event Loop Phase Grouping (DONE)

> **Shipped in `443ffb4`** — `process_editor_tick()` is now a ~50-line outline
> that delegates to `process_lsp_notifications`, `process_lsp_init`,
> `process_lsp_sync_and_inlay_hints`, `process_dap_events`,
> `process_pending_debug_action`, `spawn_syntax_highlighting`,
> `drain_syntax_results`, `poll_background_tasks`, and `process_picker_tick`.
> The 279-line DAP match block is fully extracted. Kept below for the
> original phase inventory that guided the extraction.

---

**Goal:** Make `process_editor_tick()` scannable by grouping its ~20 phases into named functions, so a reader can understand the loop's rhythm without reading 480 lines.

**Fixes:** Readability. Someone adding a new phase can see where it belongs without understanding every existing phase.

**Risk:** None. Pure extract-function refactor — no behavior change.

## Current state

`process_editor_tick()` in `ovim/src/event_loop.rs` (lines 35–513) runs ~20 phases as a flat list. The phases fall into six natural groups:

### Group 1: LSP Lifecycle (lines 43–89)
```
Java status channel drain          [43–45]    independent
LSP notification processing        [47–60]    must precede workspace edits
LSP workspace edits polling        [62–83]    must precede sync
LSP initialization check           [85–89]    must precede sync
```

### Group 2: LSP Sync + Query Cycle (lines 91–104)
```
Sync edits + refresh diagnostics   [91–96]    must follow init
Poll inlay hint response           [98–100]   after sync
Request inlay hints refresh        [101–104]  after sync
```

### Group 3: Debug / DAP (lines 106–398)
```
Poll DAP events                    [106–118]  must precede pending actions
Process pending debug actions      [120–398]  279 lines of match arms
```

### Group 4: Syntax Highlighting (lines 400–439)
```
Spawn background syntax init       [402–422]  must precede result drain
Drain completed syntax results     [424–439]  after spawn
```

### Group 5: LSP Response Polling + Dispatch (lines 441–484)
```
Poll all LSP response slots        [441–444]  independent
Poll AI jobs                       [446–448]  independent
Poll make                          [450–452]  independent
Poll git refresh                   [454–457]  independent
Check approved LSP install         [459–463]  independent
Poll AI chat jobs                  [465–467]  independent
Poll workflow jobs                 [469–471]  independent
Dispatch pending LSP intents       [473–476]  independent (each _impl syncs)
Process Lua commands               [478]      independent
Spawn pending LSP installs         [481–484]  independent
```

### Group 6: Picker (lines 486–511)
```
Picker tick + grep drain           [486–499]  within picker
Apply debounced filter             [502–504]  after tick
Spawn preview/file loading         [505–506]  after tick
Rapid scroll detection             [508–510]  after tick
```

## Ordering dependencies

**Hard dependencies** (must preserve order):
1. LSP init → LSP sync (server must exist before syncing)
2. LSP sync → inlay hints (server must have latest content)
3. DAP events → DAP pending actions (events set pending_action flags)
4. Syntax spawn → syntax drain (results come from spawned tasks)

**Soft dependencies** (current order is sensible but not required):
- Workspace edits before sync (edits modify buffer; sync sends latest)
- LSP notifications before workspace edits (notifications may affect server state)

**Fully independent** (any order works):
- All Group 5 pollers (AI, make, git, Lua, workflow, installs)
- Group 3 vs Group 4 vs Group 5 vs Group 6

## The fix

Extract each group into a named async function:

```rust
async fn process_editor_tick(editor, ...) {
    // === LSP lifecycle ===
    process_lsp_lifecycle(editor, java_status_rx).await;

    // === LSP sync + query cycle ===
    process_lsp_sync_cycle(editor).await;

    // === Debug adapter ===
    process_debug_events(editor).await;

    // === Syntax highlighting ===
    process_syntax_highlighting(editor, syntax_tx, syntax_rx).await;

    // === Background task polling ===
    poll_background_tasks(editor);

    // === LSP intent dispatch ===
    editor.dispatch_pending_intents().await;
    let _ = editor.process_lua_commands();
    spawn_pending_installs(editor);
    if editor.poll_install_progress() {
        editor.mark_dirty();
    }

    // === Picker ===
    if editor.mode() == Mode::Picker {
        process_picker_tick(editor, preview_tx, file_tx);
    }
}
```

The biggest win is extracting the DAP match block (279 lines) into `process_debug_events()`. After that, the tick body is ~50 lines of function calls with section comments — the structure is visible at a glance.

## What about the headless loop?

Both `run_headless_loop()` and `run_event_loop()` already call `process_editor_tick()` — no duplication. The extraction happens inside the shared function, so both loops benefit.

## Files

- `ovim/src/event_loop.rs` — all changes here
