# Phase 2: Unified Slot Architecture

**Goal:** Every LSP feature uses the same `Slot<T>` pattern. No keystroke is ever lost. No action blocks the event loop. Same-type cancels, different-type coexists.

**Fixes:** Actions lost during fast input. Format/code-actions/rename blocking the event loop for 100-500ms. Ad-hoc polling code for completion, inlay hints, diagnostics, hover, goto.

**Risk:** Medium. Replaces the action dispatch system, but each conversion is independent and shippable. The `Slot<T>` abstraction is simple enough to be correct by inspection.

## The Insight

The codebase already has the right pattern — it's just implemented four different ways:

| Feature | Slot-like struct | Sequence tracking | Cancel-on-replace | Poll method |
|---------|-----------------|-------------------|-------------------|-------------|
| Goto definition | `PendingLspResponses.definition` | No | `task.abort()` | `poll_definition_slot()` |
| Hover | `PendingLspResponses.hover` | No | `task.abort()` | `poll_hover_slot()` |
| Completion | `PendingCompletionRequest` | `completion_request_seq` | Stale rejection | `poll_pending_completion_response()` |
| Inlay hints | `PendingInlayHintRequest` | `inlay_hint_request_seq` | Stale rejection | `poll_pending_inlay_hint_response()` |
| Diagnostics | `PendingDiagnosticRefresh` | `diagnostic_refresh_seq` | Stale rejection | `poll_pending_diagnostic_refresh_response()` |
| Format | None (inline await) | No | N/A | N/A (blocks) |
| Code actions | None (inline await) | No | N/A | N/A (blocks) |
| References | None (inline await) | No | N/A | N/A (blocks) |
| Rename | None (inline await) | No | N/A | N/A (blocks) |

Five ad-hoc implementations of the same idea, plus four features that don't use it at all (and block the event loop instead). One generic abstraction replaces all of them.

## The Abstraction

```rust
/// A single in-flight request with its expected result type.
/// The only abstraction needed for async LSP features.
pub struct Slot<T> {
    inflight: Option<Inflight<T>>,
}

struct Inflight<T> {
    task: JoinHandle<()>,
    rx: oneshot::Receiver<Result<T>>,
    started: Instant,
    buffer_version: u64,
}

impl<T> Slot<T> {
    pub fn new() -> Self { Self { inflight: None } }

    /// Fire a new request. If one is already in flight, cancel it.
    pub fn fire(&mut self, task: JoinHandle<()>, rx: oneshot::Receiver<Result<T>>,
                buffer_version: u64) {
        if let Some(old) = self.inflight.take() {
            old.task.abort();
        }
        self.inflight = Some(Inflight { task, rx, started: Instant::now(), buffer_version });
    }

    /// Check if the result has arrived. Non-blocking.
    pub fn poll(&mut self) -> Option<Result<T>> {
        let inflight = self.inflight.as_mut()?;
        match inflight.rx.try_recv() {
            Ok(result) => { self.inflight.take(); Some(result) }
            Err(TryRecvError::Empty) => {
                // Timeout safety: cancel requests that have been in flight too long
                if inflight.started.elapsed() > Duration::from_secs(15) {
                    self.inflight.take().unwrap().task.abort();
                }
                None
            }
            Err(TryRecvError::Closed) => { self.inflight.take(); None }
        }
    }

    pub fn is_pending(&self) -> bool { self.inflight.is_some() }
    pub fn cancel(&mut self) { if let Some(old) = self.inflight.take() { old.task.abort(); } }
    pub fn buffer_version(&self) -> Option<u64> {
        self.inflight.as_ref().map(|i| i.buffer_version)
    }
}
```

Three methods. One struct. The `fire` / `poll` / `cancel` lifecycle covers every LSP feature uniformly.

### Key properties

**Same-type cancels.** `fire()` aborts the old task before starting the new one. Press `gd` twice: second cancels first. No sequence numbers needed — cancellation is structural.

**Different-type coexists.** Each feature has its own `Slot<T>`. Goto and hover are separate slots — they can't interfere. No gate, no queue, no dispatcher.

**Nothing blocks.** Every `fire_*` method spawns a task and returns immediately. The event loop polls each slot on each tick. The result is applied when it arrives.

**Timeouts are built in.** The `poll()` method checks elapsed time and aborts stale requests. No separate cleanup task needed.

## The Slots

```rust
pub struct LspSlots {
    // Navigation (spawn-and-poll, currently working)
    pub goto_definition: Slot<GotoResult>,
    pub goto_implementation: Slot<GotoResult>,
    pub goto_type: Slot<GotoResult>,
    pub hover: Slot<HoverResult>,

    // Query (spawn-and-poll, currently working via ad-hoc structs)
    pub completion: Slot<CompletionResult>,
    pub inlay_hints: Slot<InlayHintResult>,
    pub diagnostics: Slot<DiagnosticResult>,

    // Query (currently inline-await, need conversion)
    pub references: Slot<ReferencesResult>,
    pub document_symbols: Slot<DocumentSymbolsResult>,
    pub workspace_symbols: Slot<WorkspaceSymbolsResult>,
    pub call_hierarchy: Slot<CallHierarchyResult>,
    pub type_hierarchy: Slot<TypeHierarchyResult>,
    pub code_actions: Slot<CodeActionsResult>,

    // Mutate (currently inline-await, need conversion)
    pub format: Slot<FormatResult>,
    pub rename: Slot<RenameResult>,
    pub organize_imports: Slot<OrganizeImportsResult>,

    // Tokens
    pub semantic_tokens: Slot<SemanticTokensResult>,
}
```

Each result type is a simple struct containing what the poll handler needs to apply the result.

## The Intent Bridge (sync → async)

Key handlers are synchronous. Spawning a tokio task requires async context. The bridge is a per-type intent flag:

```rust
pub struct LspIntents {
    pub goto_definition: Option<LspRequestParams>,
    pub goto_definition_new_tab: Option<LspRequestParams>,
    pub hover: Option<LspRequestParams>,
    pub format: bool,
    pub code_actions: bool,
    pub references: bool,
    // ... one field per action type
}

pub struct LspRequestParams {
    pub line: u32,
    pub character: u32,
    pub uri: Uri,
    pub language_id: String,
    pub file_path: String,
}
```

The sync key handler records the intent:

```rust
// In normal mode key handler (synchronous):
('g', 'd') => {
    editor.lsp_intents.goto_definition = Some(editor.current_lsp_params());
}
'K' => {
    editor.lsp_intents.hover = Some(editor.current_lsp_params());
}
('g', 'q') => {
    editor.lsp_intents.format = true;
}
```

The async event loop dispatches all intents after the input batch:

```rust
// After process_input_events() returns:
if let Some(params) = editor.lsp_intents.goto_definition.take() {
    editor.dispatch_goto_definition(params).await;
}
if let Some(params) = editor.lsp_intents.hover.take() {
    editor.dispatch_hover(params).await;
}
if editor.lsp_intents.format.take() {
    editor.dispatch_format().await;
}
// ...
```

Each `dispatch_*` method calls `ensure_lsp_document_synced()`, spawns the task, and fires into the slot. This is the rewritten `_impl()` method — always spawn-and-poll, never inline-await.

### Why this solves the input batch problem

```
User types gd then K in the same 16ms frame:

process_input_events():
  g → pending_command = 'g'
  d → lsp_intents.goto_definition = Some(params)
  K → lsp_intents.hover = Some(params)

After batch:
  goto_definition intent → dispatch_goto_definition → fires into goto slot
  hover intent → dispatch_hover → fires into hover slot
  Both in flight simultaneously. Neither lost.
```

Compare with the old single-slot design:

```
process_input_events():
  g → pending_command = 'g'
  d → pending_lsp_action = Some(GoToDefinition)
  K → pending_lsp_action = Some(ShowHover)  ← OVERWRITES goto

process_pending_lsp_actions():
  Only ShowHover fires. GoToDefinition lost.
```

## The Polling Tick

```rust
fn poll_lsp_slots(editor: &mut Editor) -> bool {
    let mut changed = false;

    // Navigation results
    if let Some(Ok(result)) = editor.lsp_slots.goto_definition.poll() {
        editor.jump_to_location(result.location, result.new_tab);
        changed = true;
    }
    if let Some(Ok(result)) = editor.lsp_slots.hover.poll() {
        editor.show_hover_popup(result.text, result.line, result.col);
        changed = true;
    }

    // Mutation results
    if let Some(Ok(result)) = editor.lsp_slots.format.poll() {
        editor.apply_lsp_edits(result.edits);
        editor.set_status("Formatted");
        changed = true;
    }
    if let Some(Ok(result)) = editor.lsp_slots.rename.poll() {
        editor.apply_workspace_edit(result.edit);
        editor.set_status(format!("Renamed to '{}'", result.new_name));
        changed = true;
    }

    // Query results (open pickers)
    if let Some(Ok(result)) = editor.lsp_slots.references.poll() {
        editor.open_location_picker(result.locations, "References");
        changed = true;
    }

    // ... each is 3-5 lines, independent of the others

    changed
}
```

Called once per tick. Each poll is independent. Each result handler is focused on one thing.

## What Gets Deleted

| Current | Replaced by |
|---------|------------|
| `LspAction` enum (16 variants) | Gone. Each slot has its own concrete result type. |
| `pending_lsp_action: Option<LspAction>` | `LspIntents` struct with per-type flags. |
| `queue_lsp_action()` | Each key handler sets its own intent flag. |
| `process_pending_lsp_actions()` (60-line match) | Per-intent dispatch loop (each branch is 1 line). |
| `PendingLspResponses` (4 named fields) | Subsumed by `LspSlots`. |
| `PendingCompletionRequest` + `completion_request_seq` | `Slot<CompletionResult>` — cancellation replaces sequence tracking. |
| `PendingInlayHintRequest` + `inlay_hint_request_seq` | `Slot<InlayHintResult>`. |
| `PendingDiagnosticRefresh` + `diagnostic_refresh_seq` | `Slot<DiagnosticResult>`. |
| `has_pending_lsp_response()` | Gone. No gate needed. |
| `lsp_action_retry_count` | Gone. Retry is the user pressing the key again. |
| `prepare_lsp_request()` + its sleeps | `ensure_lsp_document_synced()` called in each `dispatch_*`. |
| 5 separate `poll_*` methods | One `poll_lsp_slots()` function. |

## Migration Steps

Each step is independently shippable. The old and new systems can coexist during migration.

### Step 1: Introduce `Slot<T>` and `LspSlots`

Add the generic `Slot<T>` struct. Add `LspSlots` to the editor with all fields initialized as empty. No behavior change yet — the old system still runs.

**File:** `ovim-core/src/editor/lsp_slot.rs` (new)

### Step 2: Convert goto-definition to Slot

Rewrite `goto_definition_impl()` to fire into `lsp_slots.goto_definition` instead of the ad-hoc `pending_lsp_responses.definition` field. Add the poll handler. Remove the old field.

This is the template conversion — get it right once, then repeat for each feature.

**Files:** `ovim-core/src/editor/lsp_modules/goto.rs`, `ovim-core/src/editor/lsp_state.rs`

### Step 3: Convert hover, implementation, type-definition

Same pattern as step 2. After this, `PendingLspResponses` is empty and can be deleted.

### Step 4: Convert completion, inlay hints, diagnostics

Replace `PendingCompletionRequest`, `PendingInlayHintRequest`, `PendingDiagnosticRefresh` with their `Slot<T>` equivalents. Delete `completion_request_seq`, `inlay_hint_request_seq`, `diagnostic_refresh_seq`.

### Step 5: Convert inline-await actions to spawn-and-poll

Convert `format_document_impl`, `code_actions_impl`, `find_references_impl`, `document_symbols_impl`, `workspace_symbols_impl`, `rename_impl`, `organize_imports_impl`, `call_hierarchy_*_impl`, `type_hierarchy_impl`, `semantic_tokens_impl`. Each becomes a `dispatch_*` that fires into its slot.

Priority order:
1. `format_document_impl` — most visible blocking
2. `find_references_impl` / `document_symbols_impl` — open pickers, straightforward
3. `code_actions_impl` — complex (fallback logic, multi-server)
4. `rename_impl` — interactive (user input first)

### Step 6: Introduce `LspIntents` and remove the single slot

Add `LspIntents` struct. Rewrite key handlers to set intent flags instead of calling `queue_lsp_action()`. Add the per-intent dispatch loop to the event loop. Delete `pending_lsp_action`, `queue_lsp_action()`, `process_pending_lsp_actions()`, and the `LspAction` enum.

### Step 7: Consolidate polling

Replace the scattered `poll_pending_*` calls in `process_editor_tick` with a single `poll_lsp_slots()` function.

## Adding a New LSP Feature After Migration

1. Define the result type: `pub struct FooResult { ... }`
2. Add the slot: `foo: Slot<FooResult>` to `LspSlots`
3. Add the intent: `foo: Option<FooParams>` to `LspIntents` (or `foo: bool`)
4. Write `dispatch_foo()` — spawn task, fire into slot (~10 lines)
5. Write the poll handler — apply result (~5 lines)
6. Wire the key binding — set the intent flag (1 line)

No enum variant. No match arm in a dispatcher. No thinking about blocking vs non-blocking.

## Files Changed

| File | Change |
|------|--------|
| `ovim-core/src/editor/lsp_slot.rs` | New: `Slot<T>`, `Inflight<T>` |
| `ovim-core/src/editor/lsp_state.rs` | `LspSlots`, `LspIntents`, delete old structs |
| `ovim-core/src/editor/lsp_integration.rs` | Delete `queue_lsp_action`, `process_pending_lsp_actions`, add dispatch loop |
| `ovim-core/src/editor/lsp_modules/*.rs` | Each `_impl()` becomes `dispatch_*()` firing into slot |
| `ovim/src/event_loop.rs` | Intent dispatch after input, `poll_lsp_slots()` in tick |
| `ovim-core/src/editor/input/normal/pending_commands.rs` | Set intent flags instead of calling queue_lsp_action |

## Verification

1. **Batch input test:** Send `gd` + `K` in same input batch via API. Both should produce results.
2. **Same-type cancel test:** Send `gd` + `gd` + `gd` rapidly. Only last result should apply.
3. **Format non-blocking test:** Trigger `gq`. Verify input is still accepted during format (type characters, verify they appear immediately).
4. **All existing tests pass.** The slot architecture is a structural change, not a behavioral one.
