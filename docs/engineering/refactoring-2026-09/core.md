# Scout report: ovim editor core + frontends (read-only)

Repo: `<repo>` (main @ 41243785). Area: ovim-core editor/commands/input/change tracking, frontend tick/loops, GUI projection,
TUI renderer, AI subsystems (structural). All line numbers are from that commit. Nothing was executed (no cargo).

## Stale docs found first (fix in passing; each is cheap and stops agents chasing ghosts)

- ISSUE_TRACKER OV-00062 ("Pattern A `Change::insert/delete` + `add_change()`, 73 call sites") is stale. `Change` now has only `Recorded` and
  `ResourceOp` (ovim-core/src/change.rs:230-258). `add_change` has 0 callers (definitions only: change.rs:709, editor/mod.rs:2247). Real
  residue is in candidate 5 below.
- OV-00061 sizes are stale: editor/mod.rs is 3025 lines (not 1952; ~1000 of that is tests at 2560-3025), commands.rs 2805 (not 1920).
  The listed "large files" are not the real top of the list: gui/mod.rs 5370, ai_subagents.rs 4397, dispatch.rs 4072, ai_chat.rs (renderer) 3832,
  renderer/buffer.rs 3743, lsp_integration.rs 3203, builtins.rs 3014, ai_code_explanation.rs 2972.
- OV-00298 (file-mode vs session-mode CLI edit engines, HIGH) is Pending in the tracker but DONE in code: ovim/src/edit_engine.rs (756 lines)
  is called from api_dispatch.rs:683/760/809 and subcommands.rs:528/563/582. Mark Done.
- OV-00300 progress note is accurate (motion_range.rs partially adopted); OV-00276 refers to a "simple wrap path" - re-verify before acting.
- refactor-roadmap/: 00-phase0..08 are historical, 09-11 retired, 12 is "RETIRED" and a duplicate of 17, 13/14/15/16/18/19 shipped. Only 17
  (multi-server sync, LSP scout's area) is active. Roadmap 10 (editor decomposition) has no doc; see "Editor god struct" below. The dir could be
  collapsed to 17 + an ARCHIVE note.

---

## Ranked opportunities (value/risk order)

### 1. Kill the "single pending slot overwritten" bug class (small PR, high value)

Evidence (slots that take `Some(x)` from several producers with no occupancy check, drained one per tick):

- `lsp.state.pending_did_close_file: Option<String>` (editor/lsp_state.rs:453). Six producers:
  buffer_manager.rs:300, :343, :362 (delete_current_buffer), lsp_integration.rs:1496 (path transition), editor/mod.rs:1915 (edit_file switch),
  lsp_modules/workspace_edits.rs:415 (server-driven `ResourceOp::Delete`). Sole consumer `send_lsp_close_if_needed` (lsp_integration.rs:2169)
  does `.take()` once and is called only from the frontend tick (ovim/src/frontend/tick.rs:75). Anything that closes two files between ticks
  loses the first didClose: a WorkspaceEdit with two Delete ops in one apply, `:bufdo bd`, a Lua/API `exec` sequence, or a macro. The server keeps
  a stale open document (wrong diagnostics, stale project view). Same class as the DAP `PendingDebugAction` loss. This is a likely live bug.
- `build.pending_make` (ui_features.rs:218), `build.pending_test` (test_runner/mod.rs:329), `ui_panels.pending_git_fetch`
  (diff_review/mod.rs:1509), `build.pending_shell_command` / `pending_terminal_session` (input/commands.rs:731/752), `lsp.pending_install`
  (mod.rs:895), `pending_file_rename` (file_rename.rs:14): unconditional overwrite. For make/test the old receiver is dropped but the spawned
  thread/child is not killed (test_runner: on `tx.send` error it stops reading but still `child.wait()`s, i.e. an orphan run keeps going).
- `pending_background_tool` has `debug_assert!(is_none())` at ai_background_tools.rs:85 only; release builds silently overwrite (drops the
  oneshot receiver + task, so a tool call vanishes). The other parked-tool slots (candidate 2) have no assert at all.

Design:
- `pending_did_close_file` -> `pending_did_close: Vec<String>` (or `IndexSet`), producers call `self.queue_lsp_did_close(path)`; consumer
  `send_lsp_close_if_needed` drains all, re-running the `still_loaded` check per path. ~25 lines net.
- make/test/git-fetch: `start_*` refuses (status message "already running") or cancels+kills the previous run first. Pick per feature; for
  test runner store a kill handle. shell/terminal pending: `Option` is fine (frontend drains per loop turn) but guard with debug_assert.
- Add a tiny `Slot`-style helper? Not needed; the audit + Vec is the fix. Do NOT generalize.

Migration: (a) characterization test first: two `delete_current_buffer`/two ResourceOp::Delete in one tick, assert two didClose reach a fake
server (there is an LSP test harness in ovim/tests/lsp_*; else assert on the Vec). (b) switch to Vec. (c) make/test guards.
Est: +80 / -20 lines. Risk low. Unblocks candidate 2's occupancy invariant. Behavioural difference to preserve: `still_loaded` early-return.

### 2. Replace 7 parked-tool `Option` slots + `waiting` + `pending_job` with one `ChatTurnState` enum

Evidence (editor/ai_chat_state.rs, `AiChatState` = 163 lines of fields, ~90 fields, lines 653-816):
- Slots: `pending_tool_approval`(:764), `pending_auto_mode_classification`(:765), `pending_shell_execution`(:766), `pending_background_tool`(:774),
  `pending_subagent_control`(:775), `pending_code_explanation`(:777), `pending_no_repo_folder_approval`(:783), plus `external_agent.permission`,
  `pending_job`(:717), `waiting`, `runtime_turn`. Invariant "at most one blocker" lives only in `turn_blocker()` (:864) as a `debug_assert!`.
- The same list is hand-enumerated in three places that already disagree:
  1. `turn_blocker()` (:864-889) covers 5 slots + external permission - it OMITS `pending_subagent_control`, so `activity()` (:895) reports a turn
     parked on a subagent wait/interrupt as `Inference`/`Idle` depending on `waiting`/`pending_job`. GUI/headless `activity` string is therefore wrong there.
  2. Cancel path ai_chat.rs:191-232 (8-tuple of `.take()`s, ~130 lines of per-slot teardown after) includes it.
  3. `activity()` special-cases `pending_no_repo_folder_approval` and code-explanation outside `turn_blocker`.
  Also polled separately: ai_tool_streaming.rs:434 (subagent take), ai_chat.rs poll paths (27 `pending_*` refs in ai_chat.rs, 22 in ai_code_explanation.rs).
- Four of the slots share the shape `{tool_call, continuation: ToolExecutionContinuation, receiver: oneshot, task: JoinHandle}` (PendingShellExecution
  :141, PendingBackgroundTool :421, PendingSubagentControl :429, and a fourth Batch/Dynamic continuation enum in CodeExplanationContinuation :436
  that duplicates `ToolExecutionContinuation::{Dynamic,Batch}` fields nearly verbatim).

Target design:
```rust
pub(crate) enum ParkedTurn {           // what the turn is waiting on; exactly one
    Approval(PendingToolApproval),
    Classifying(PendingAutoModeClassification),
    Shell(PendingShellExecution),
    Background(PendingBackgroundTool),
    SubagentControl(PendingSubagentControl),
    CodeExplanation(PendingCodeExplanation),
}
impl AiChatState {
    fn park(&mut self, p: ParkedTurn) { assert!(self.parked.is_none(), ...); self.parked = Some(p) } // release-mode error, not debug_assert
    fn activity(&self) -> AiChatActivity            // exhaustive match on ParkedTurn, no ad-hoc lists
    fn take_parked(&mut self) -> Option<ParkedTurn> // cancel path = one match with per-variant teardown
}
```
Keep `pending_no_repo_folder_approval` and `external_agent.permission` as separate "decision" prompts folded into `activity()` via the enum's
exhaustive match. `CodeExplanationContinuation` should reuse `ToolExecutionContinuation` (add `EditorMcp` variant there).

Steps (each green): (1) characterization tests: activity() for each parked kind incl. subagent control (currently wrong: write the test as
failing/ignored first); cancel-while-parked for each kind (ai_chat_tools_tests.rs 2.2k lines already has fixtures: 1651-2100 build these slots
by hand, that is the cheapest harness). (2) add `ParkedTurn` + accessor shims with the old field names (`parked_shell()` etc.) and migrate
readers. (3) migrate writers (`chat.pending_x = Some` sites: ai_chat_turn.rs:1171,1620; ai_chat_tools.rs:1630; ai_background_tools.rs:86;
ai_subagents.rs:2205; ai_code_explanation.rs:290; ai_chat_queue.rs:374; ai_chat.rs:830,859). (4) collapse cancel path. (5) unify continuation types.
Est: -350 / +250 lines; most test fixtures need touching (~40 sites in *_tests.rs). Risk medium: ordering in cancel path (kill shell before
abort, fail_tool before synthetic results) must be preserved verbatim per variant. Kills: wrong `activity()` for subagent waits, silent slot
overwrite in release, future "forgot to add slot N to cancel/blocker lists".

### 3. Move background-poll orchestration into ovim-core; give TUI/GUI/headless one loop body

Evidence:
- ovim/src/frontend/tick.rs (636 lines) hand-lists ~25 `if editor.poll_x() { editor.mark_dirty() }` calls (poll_background_tasks :458-497, plus
  launch/code-lens/lsp/dap/outline/git/search-replace/codex-auth/workflow/chat-job) and holds domain logic that is not frontend-specific:
  `process_pending_debug_action` (:193-372, ~180 lines of DAP action handling - DAP scout's area, but it is here), `process_dap_launch_or_attach`,
  `spawn_pending_installs` (:523, LSP auto-install spawn), `open::that_in_background` for pending external URL. There are 50 `fn poll_*` in ovim-core;
  a new async feature is dead in every frontend until someone remembers to add a line to a crate that core tests do not exercise.
- Three hand-written loop bodies compose the tick differently:
  - TUI: `run_tick_work` (event_loop.rs:355) = tick + picker results + external-file check (500ms) ; rehighlight debounced separately (:538, `debounce_delay`).
  - Headless: event_loop.rs:186-196 = tick + external-file + rehighlight only if `last_edit.elapsed() >= 200ms` (no picker results).
  - GUI: gui/mod.rs:1629-1636 = tick + picker results + rehighlight UNCONDITIONALLY (no debounce - runs `process_pending_rehighlight` every
    tick while the user types) + external-file + `update_diff_review_geometry`.
  Behaviour differs between frontends for no documented reason (rehighlight debounce, picker results, `notify_new_agent_attention` only in TUI).
- Pending terminal/shell command handling: TUI executes (event_loop.rs:516-521), GUI silently drops with a status message (gui/mod.rs:1658),
  headless API rejects ad hoc (input/commands.rs:509). Three policies for one queue.

Target: `ovim-core: impl Editor { pub async fn tick(&mut self, ch: &mut FrontendChannels-equivalent, clock: &mut TickClock) -> TickReport }` where
`TickReport { dirty: bool, wants_terminal: Option<TerminalRequest> }`, containing the poll list, rehighlight debounce, external-file cadence and
picker draining. Frontends only provide "what to do with TerminalRequest" and rendering. `FrontendChannels` (frontend/channels.rs, 66 lines) is
already just mpsc endpoints and can move as-is. Keep the LSP-init/syntax two-phase ordering comment/logic intact (tick.rs:13-30, has tests).

Steps: (1) tests: move tick.rs tests (yank-flash defer, yaml paint-before-LSP, working-animation) to run against the new fn; add a test that every
`poll_*` is reachable (grep-based lint is fine). (2) Move `poll_background_tasks`/`spawn_pending_installs`/dap pieces (pure moves). (3) Introduce
TickClock; make TUI and GUI call it; then decide rehighlight policy once (recommend the 200ms debounce, measure typing latency in GUI first).
Est: -150 / +200 lines (mostly moves). Risk medium-low; behaviour change only for GUI rehighlight and headless picker results (both should
be strictly improvements; call them out in the PR). Unblocks: any future frontend (the GUI was the second one; it copied and diverged).

### 4. One ex-command dispatcher (commands.rs / input/commands.rs / cmd_*.rs), registry-driven

Evidence:
- Three layers: `crate::commands::execute_command_inner` (commands.rs:230-1583, ONE 1350-line function with ~86 branches), `cmd_project::try_handle`
  (cmd_project.rs, called at commands.rs:~283), `input/commands.rs::execute_command_single` (:728-1345, ranges, `:s`, `:g`, `:d`, `:y`, `:t`, `:m`, `:w !`, `:!`),
  plus `cmd_buffer.rs`/`cmd_set.rs`. The interactive path calls `commands::execute_command` first and falls through when the ERROR STRING contains
  "Not an editor command" (input/commands.rs:787, :498; produced at commands.rs:1576). `execute_command_string_api` (input/commands.rs:482-530) then uses the
  STATUS LINE as a return channel (clear it, run, read it back, sniff `E\d+:` prefixes via `is_vim_error_status`).
- Tab-completion list `COMMAND_NAMES` (input/commands.rs:174-~296, ~110 names) is hand-maintained and already drifted: missing e.g. Run/Debug/AI/
  Workflow/Git* aliases handled in commands.rs (`handle_debug_command`:2360, `handle_workflow_command`:2216, `handle_session_command`:2265,
  `handle_ai_status`:2102 ...), `:make`, `:Sr`, `:Diagnostics`, `:DocumentSymbols`.
- Behavioural drift already visible: `:!cmd` is intercepted only in the interactive layer (queues `pending_shell_command`), so
  `commands::execute_shell_command` (commands.rs:2028-2100) and the queue are two implementations; the API layer bolts on "Interactive terminal
  sessions require the TUI frontend"; pseudocode/chat-scratch/commit-message buffers each special-case command verbs at the top of
  `execute_command_inner` (:231-300) with their own verb allowlists.

Target design: `struct ExCommand { names: &'static [&'static str], min_abbrev, args: ArgKind, range: RangePolicy, handler: fn(&mut Editor, &ParsedCmd) -> CommandResult, contexts: BufferKindMask }`
in one static table; `parse(&str) -> ParsedCmd { range, name, bang, args }` once; dispatcher does buffer-kind gating (pseudocode / scratch / commit
message) from `contexts`, so allowlists disappear; `COMMAND_NAMES` is derived from the table. `CommandResult` carries the message (already does);
the interactive layer renders it (single-line -> status, multi-line -> hover as now, input/commands.rs:~800). The status-line-as-return hack
goes away.

Steps (green each): (1) characterization: a table-driven test over ~150 command strings asserting (result kind, message, buffer text/cursor) run
through BOTH `execute_command_string` and `execute_command_string_api` (there is already execution_tests.rs and ovim/tests); snapshot current
outputs first. (2) Introduce the table with handlers as thin wrappers over the existing bodies, one group at a time (buffer/window, set, file
IO, project, debug, ai, range commands), deleting the corresponding arms; (3) replace string-sniffed fallthrough with `Option<CommandResult>`/
`Unknown`; (4) derive completion list. Est: -600 / +500 (mostly re-homing; the 1350-line fn disappears). Risk medium-high because ex commands
are user-facing and abbreviations matter (`se`, `sp`, `tabe`); de-risk by asserting the resolved handler for every abbreviation currently
accepted. Kills: completion drift, GUI/headless/TUI parity bugs (input/commands.rs:479 doc comment records one such bug), unwieldy top-level gating.

### 5. Finish the two half-migrated state machines in the input/edit path

5a. `InputContext` (editor/input_context.rs): `pending_operator` and `pending_command` are documented "being phased out in favor of `input_state`" (:24-31)
but are still the working state for Normal mode. `Editor::set_pending_operator` has ~17 callers, `set_pending_command` ~14 (normal/mod.rs:167-245,
pending_commands.rs:104-385/839, operators.rs:72/596-610, motions_input.rs:381, visual_mode.rs:260-391) and ~45 read sites
(`pending_command() == Some('g'|'z'|'Z'|'['|']'|'"'|'i'|'a')`, e.g. operators.rs:32-82, text_objects.rs:20, input/mod.rs:118-311), while `InputState`
(input_state.rs:16, 138 refs) ALSO models `OperatorPending`, `GPrefix`, `ZPrefix`, `BracketPrefix`, `TextObjectPending` and has its own
`pending_operator()` (:165). Two encodings of "d is pending" can disagree; `set_mode` and `reset` clear both by hand (mod.rs:785-786, :1738).
Design: delete the two legacy fields, make `Editor::pending_operator()/pending_command()` thin projections of `InputState` (they already exist
as `InputState::pending_operator()`), migrate each setter to `set_input_state(InputState::X{..})` module by module (normal/mod.rs first: the
`setup_pending_state` function is the single writer for most). Characterization: dot_repeat_test.rs (1856 lines), operator/motion tests, plus a
new generated test that feeds every 2-3 key operator sequence and asserts `input_state`+buffer, run before and after. -150 / +120, risk medium.

5b. Undo/repeat: `ChangeManager` keeps `last_change: Option<Change>` AND `last_repeat_action: Option<RepeatAction>` with hand-maintained mutual
exclusion (change.rs:646-662, :905-906; change_tracking.rs:312-314). But `last_change` now serves four unrelated jobs: dot-repeat fallback
(change_tracking.rs:234-259, replays ABSOLUTE char-offset edits - `Change::repeat` is only correct at the original position, see change.rs:421-433 comment),
the `'.` and `` `^`` marks (mark_jump.rs:37,48), the `".` register (change_tracking.rs:343), and visual-block insert replication
(insert_mode.rs:175-231). Because `set_repeat_action` nulls `last_change` (change_tracking.rs:314), any RepeatAction-based op (dw, cw, p, J, ...)
followed by `'.`/`^`/`".` loses the data those features read. Only producers of `last_change` now are `push_change` (finalize_change_building
mod.rs:2300, insert_mode.rs:122, :248), and each immediately installs a RepeatAction, so the "Change-based repeat fallback" is effectively dead for
new edits.
Design: rename to `last_insert: Option<Change>` (or `last_edit`), drop mutual exclusion, delete the fallback branch and `Change::repeat`/
`set_cursor_before/after` if unused after, delete dead `Editor::add_change`/`ChangeManager::add_change`. Write tests FIRST for `'.` after `dw`,
`".` after `cw`+text, `^` after `A`+text (expect vim behaviour; current may fail => real bug). -120 / +40. Risk low-medium. Closes OV-00062 for real.

---

## Remaining opportunities (short list)

6. **TUI two-engine line rendering (half-migration).** ovim/src/ui/renderer/buffer.rs `render_buffer` (1550-2866, ~1300-line fn) has a line-cache path,
   an indexed-fragment path used only for `len_bytes > 4096 || top_skip > 0` (:1826-2135), and the legacy path (:2200-2700: `expand_tabs_with_mapping`,
   `render_line_with_highlights` :2866, `split_line_into_rows` :1202). Comment at :1286 calls the legacy renderer "the small-line oracle"; a parity test
   exists (:3142). GUI already renders every line from `IndexedLineLayout`. Retiring legacy = -500 lines and removes OV-00276-style disagreements
   (legacy vs wrap-map). Risk high (perf and pixel parity): first extend the oracle test into a corpus/property test (tabs, wide, control chars, conceal,
   inline decorations, wrap widths 1..N) and benchmark typing on a 50k-line file.
7. **Tool metadata as string tables.** The same 8 mutation tool names are hard-coded in: ai_chat_tools.rs:783 (path-scoped), :832 (dispatch),
   builtins.rs:388 (execute_builtin error arm), codex_app_server.rs:969 (`codex_tools_allow_writes`), plus ai_chat_mutations.rs:588 and Codex schema list
   codex_app_server.rs:1377; project-scoped list ai_chat_tools.rs:794; `execute_builtin` (builtins.rs:337-400) is mostly arms that return "must be
   dispatched elsewhere". `ToolDefinition` already has `side_effect`/`required_scope` (ai/tools/mod.rs:242). Add `dispatch: DispatchKind
   {Pure, Editor, Lsp, External, Mutation, Bridge}` and `path_scope: {None, PathArg, Project}` to the definition; derive all lists. New tools then
   cannot be half-registered. -150 / +100, risk low-medium (test: every registered tool has a dispatcher and vice versa; `ai/tools/mod.rs` tests exist).
8. **Subagent control result shaping x3 with drift.** `PreparedHeadlessAgentControl` (ai_subagents.rs:2945) vs `PreparedAsyncSubagentControl`
   (:3029) vs `AiSubagentRun::{wait,followup,interrupt}_nested_agents` (:1085-1230) all build the wait/interrupt/followup JSON separately. Already
   diverged: async followup adds `"state":"queued"`, headless wait adds `agent_id`, nested omits both. Extract `fn wait_json/interrupt_json/followup_json`
   (and a shared `execute` taking a "with agent_id" flag) and add an outcome-shape test per caller. -100 / +60. Same PR can split the 4.4k file:
   service+run (1-1310), Editor UI impl (1311-2960), registries (3144-3336), tests (3338+). Also `dispatch.rs`: restore_* snapshot compat code
   (1834-2210) and projection (2477-2660) into their own modules. Pure moves.
9. **GUI projection leaks/duplicates (gui/mod.rs 5370).** (a) `segments_for_line`, `split_visual_rows` (:3206-3450, `#[cfg(test)]` only) are dead in
   production since `gui_segments_from_layout` replaced them; their tests (:5123-5195) pin dead behaviour - delete ~250 lines. (b) Hand-written enum->string
   mappings (chat role/focus/queued kind :3700-3790, run status/line kind :4206-4258, debug row kinds :4115) duplicate what TUI status_style
   (renderer/run_console.rs:46) does with the same enums; put `as_str()`/`tone()` on the core enums (`RunOutcome`, `LineKind`, `ChatRole`, `ChatFocus`,
   `QueuedChatInputKind`, `RowKind`). (c) Overlay "select item N + activate" logic (gui/mod.rs:2197-2330) reaches into panel internals
   (`panel.selected_index = index`, `file_tree_mut().set_selected_index`, `quickfix_list_mut().set_selected`) and synthesizes Enter key events; move to
   core `Editor::activate_overlay_item(OverlayId, idx, activate)` so TUI mouse (which only handles the picker, input/mouse.rs:940) and GUI share it.
   (d) AI approval resolution duplicated in 3 sites (gui/mod.rs:1966, input/ai_chat_mode.rs:93-130, ai_chat_session.rs:296): add
   `Editor::resolve_ai_approval(Decision)`. (e) Every GUI command costs 4 hand-synced edits (app.rs tauri command, GuiBridge method, GuiRequest
   variant, handle_request arm; ~40 requests) - macro or single `enum GuiRequest` dispatch table. Editor accessor facade for the GUI is 97
   `ai_chat_*/ai_agent_*` methods plus 28 direct `ai_state.chat` reads in ovim/src.
10. **`git status` computed 3 ways:** git/ops.rs:84 (`status`, git2), editor/ai_comprehension.rs:177-212 (own git2 StatusOptions dirty check),
    ai/tools/builtins.rs:148 (`git status --porcelain` subprocess with hand limits). Route through git::ops with a bounded variant. -60 / +40.
11. **Editor size guard is being gamed.** `editor_size_regression` (editor/mod.rs:2757, limit 10_000) - recent commit c331b3d6 says "box signature
    state to keep Editor size in budget". Editor has 85 `impl Editor` blocks, 1843 pub fns under editor/, ~45 top-level fields, and `AiChatState` ~90
    fields. Box `build`, `nav`, `picker_state`, `render_cache` (like `ai_state`, `launch`, `completion_menu` already are) to buy ~1-2KB headroom, and
    treat a further LspSubsystem-style grouping of AiChatState (composer / streaming / parked (cand. 2) / presentation / code-explanation cache) as the
    real decomposition (roadmap 10 has no doc; follow the `LspSubsystem` template noted in refactor-roadmap/00-overview.md).
12. **Big-function splits (mechanical):** `execute_command_inner` (cand. 4), `render_buffer` (cand. 6), `handle_request` in gui (1809-2427, one 620-line
    match), `ai_chat_tools.rs` dispatch (string-match chains at :765-850, :1754-1960).
13. **Parallel builders of "PreparedDelegation"/manifest registries** (ai_subagents.rs:3144 `BaseManifestRegistry`, :3209 `PreparedDelegationRegistry`)
    are near-identical durable-file registries (path/encode/ensure_private_directory); fold into one generic `DurableRegistry<K,V>`. -80 lines.

## Things I checked and found acceptable (do not spend time)
- Undo-stack recording itself (`buffer.record` -> `push_recorded_undo`, 52 + 26 callers) is coherent; `Change` enum is minimal.
- `frontend/tick.rs` ordering (syntax before LSP init, yank flash deferral) is deliberate and tested.
- Single slots that are safe by construction: `pending_tool_approval` (only one tool call parked at a time; sequential batch), `pending_job`
  (owns task, aborts on Drop), `pending_codex_auth` (state-machine re-Some at each phase), `pending_no_repo_folder_approval`.
- Cross-refs for the other scout: DAP/launch surfaces `process_pending_debug_action` in tick.rs, `lsp_integration.rs` 3.2k.
