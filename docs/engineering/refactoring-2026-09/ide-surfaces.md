# Scout report: ovim IDE surfaces (base 9d1aaba9 -> main; PRs #29-#36)

Scope read: 70 commits, +37.5k/-2.9k, 208 files. Read-only; no cargo run. All paths are under
`<repo>/` (`core/` below = `ovim-core/src/`, `bin/` = `ovim/src/`).
Line numbers are from current main (c2168b8f).

Headline: the launch pipeline (PR #30/#33: `launch/*`, `editor/launch_flow.rs`) is the good, tokio-based,
process-group-safe design, but the older runners (`:make`, non-JVM `<Space>t*`, shell filters) were not
migrated onto it, and JVM root detection now exists in four incompatible copies. Several pieces of PR #30/#33 are
dead after the "ovim spawns the JVM, DAP only attaches" change.

Confirmed NOT duplicated (checked, do not spend a PR on it): JUnit XML parsing (only `launch/junit.rs`;
`editor/test_panel.rs::parse_test_failures` parses cargo/jest/pytest text, a different job); WorkspaceEdit
application (search_replace correctly goes through `apply_lsp_edits_to_buffer_index` +
`write_through_workspace_edit_buffer`); `git/ops.rs` (libgit2 write ops) vs `git/mod.rs` (read-only status/blame)
do not overlap in function, only in repo-opening (see #5).

---------------------------------------------------------------------------------------------------

## Ranking (value / risk)

1. One process runner: retire `spawn_test_job`, `:make` blocking thread, ad-hoc killpg (HIGH value, MED risk)
2. One JVM/project root resolver (+ delete `find_module`'s second Maven algorithm) (HIGH, LOW-MED)
3. Delete the dead pre-#33 DAP launch path and stale debug plumbing (MED, LOW)
4. Split the god files: `gui/mod.rs` 5370, `lsp_integration.rs` 3203, `launch_flow.rs` 2183 (HIGH, LOW)
5. Small shared seams: repo-root/`picker_base_dir` dupes, picker-opening helper, LSP request boilerplate (MED, LOW)

---------------------------------------------------------------------------------------------------

## 1. One process runner for build / test / make / run

### Evidence (parallel paths doing one job: "run a command, stream lines, collect exit, feed quickfix")

| Path | Site | Mechanism | Kill? | Process group? |
|---|---|---|---|---|
| Launch pipeline (JVM run/debug/tests) | `core/launch/process.rs:78-260` `ProcessHandle` | tokio, `kill_on_drop`, `process_group(0)`, SIGTERM then SIGKILL after 3s, reader timeout, stdin channel | yes | yes |
| Non-JVM test runner | `core/editor/test_runner/mod.rs:268-340` `spawn_test_job` | `std::thread` + `sh -c` + 2 reader threads + `std::sync::mpsc`; polled by `test_panel.rs:188 poll_pending_test` | **none** | **no** |
| `:make` | `core/commands.rs:1751-1795` `execute_make_command` | `std::thread` + `Command::output()` (buffers everything, no streaming); polled by `editor/ui_features.rs:221 poll_pending_make` | none | no |
| `:!`-style shell filters | `core/editor/input/commands/shell.rs:55,142,292` and `commands.rs:2051` | blocking `.output()` | n/a (sync, deliberate) | no |
| AI shell tool | `core/editor/ai_tool_execution.rs:120-171` + `ai_chat_state.rs:271 kill_shell_process_group` (nix `killpg`) | own process-group kill, elaborate reaping | yes | yes |

Concrete bug in the gap: `spawn_test_job` supersedes an in-flight run only by replacing `build.pending_test`
(test_runner/mod.rs:294). The old `sh -c cargo test ...` child is never signalled; dropping the receiver only makes
the reader threads exit on the next line. `launch_stop` (launch_flow.rs:532) stops only `launch.job`, so a
non-JVM test run cannot be stopped at all, and `:make` cannot either. Two group-kill implementations exist
(`libc::kill(-pid)` in `launch/process.rs:~300` and nix `killpg` in `ai_chat_state.rs:253-276`).

Three copies of "turn finished output into quickfix", with **different behaviour**:
- `ui_features.rs:233` make: `parse_compiler_output`, always jumps to entry 0 (`first()`) even if it is a warning, opens window.
- `launch_flow.rs:1608 report_build_problems`: `parse_compiler_output_in(log, cwd)` (via `LaunchJob::diagnostics`, line 171), selects first *error*, jump optional, resolves relative paths against the build cwd.
- `test_panel.rs:269 failure_quickfix_entry` / `finish_test_run:235`: silent, no jump, title `test <cmd>`.
`make` and the generic test path call `parse_compiler_output` **without** a base dir, so relative paths in
Gradle/rustc output from a non-cwd project resolve against the process cwd (the launch path fixed this with
`_in`; the older two did not).

Two output stores for one run: `RunRecord` (launch/console.rs) and `TestRun` (test_panel.rs). For JVM tests both are
written (launch_flow.rs:1641 `start_test_panel_run`, `finish_test_panel_run:1774`). Non-JVM tests only fill `TestRun`.

### Target design
Keep `launch::process::ProcessHandle` as the only spawner. Add a thin, editor-independent seam:

```rust
// core/launch/job.rs (new)
pub enum JobSink { Console(u64 /*run id*/), TestPanel, Quickfix { title, jump: JumpPolicy } }
pub struct ShellJob { spec: CommandSpec /* argv = ["sh","-c",cmd] */, base_dir: PathBuf, sink: .. }
impl Editor { pub(crate) fn spawn_shell_job(&mut self, job: ShellJob) }
```
- Non-JVM tests: `LaunchPlan { kind: PlanKind::Task, task: TaskPlan{argv:["sh","-c",cmd], cwd, reports_dir: None}, build_tool:"shell" }`
  goes through `begin_request(LaunchRequest{ source: LaunchSource::Plan{..} })` exactly like `run_jvm_test` already does
  (test_runner/mod.rs:186-262). `finish_test_reports` gets a `reports_dir == None` branch that calls the existing
  `parse_test_failures` / `extract_summary` on `job.log` (they already are pure functions). The `TestRun` panel record
  stays (it is the shared test surface), fed by `drain_process`.
- `:make`: same, `PlanKind::Task`, sink = quickfix; policy = `jump to first error` (pick the launch path's behaviour;
  document the change).
- One `quickfix_from_output(output, base_dir, policy)` on Editor replacing the three copies.
- One `terminate_group` (keep `launch/process.rs`), have `ai_chat_state::kill_shell_process_group` call it
  (AI shell keeps its own reaping semantics, just not its own signal code; if the semantics differ leave AI alone and only
  dedupe the signal call).

### Migration steps (each green)
1. Characterization tests first: (a) `spawn_test_job` streams stderr+stdout lines and Finished{success} (ovim-core
   test_runner tests exist at `test_runner/tests.rs`; add: superseding a run leaves the first child running today - assert
   the *desired* behaviour with `#[ignore]` until step 3); (b) `parse_compiler_output` relative-path behaviour with and
   without base dir; (c) make jump-to-first behaviour.
2. Extract `quickfix_from_output` + JumpPolicy; switch the three callers (pure refactor, keep each caller's current policy).
3. Route non-JVM `spawn_test_job` through the launch pipeline (`begin_test_shell`): delete `spawn_test_job`, `PendingTest`,
   `poll_pending_test`, `TestEvent`, `build.pending_test`, and the `poll_pending_test` call in `bin/src/frontend/tick.rs:468`.
   `run_test_last` (mod.rs:69) collapses to always replaying `LastTest.request` (`command`/`cwd` fields go away).
4. Route `:make` the same way; delete `PendingMake`, `MakeResult`, `poll_pending_make`, `set_pending_make`, tick.rs:465.
5. Unify group-kill.

### Size / risk
Delete ~330 (spawn_test_job 75 + poll/finish 120 + make 45+60 + PendingTest/Make plumbing 30), add ~120. Net -200 and, more
important, `launch_stop`/`:LaunchStop` now stops every run.
Risk MED: `TestRun`'s live line semantics (test_panel expects `String` lines, console splits stdout/stderr and strips CR),
`build.last_make_output` for `:TestOutput`/`:MakeOutput` must keep being written (`launch_flow` already keeps `job.log`;
have finish set `last_make_output`). Tick polling: `poll_launch` already forces a repaint while any run is active.
Tokio requirement: `ProcessHandle::spawn` needs a runtime; `run_jvm_test` already relies on it, headless/TUI both have one.
Unblocks: Stop for tests, run history in the console for all runs, GUI shows one run surface (see #4), kills the
"test panel vs console" double bookkeeping.

---------------------------------------------------------------------------------------------------

## 2. One JVM/project root resolver

### Evidence: four+ roots for the same file, three of them Maven/Gradle-aware, disagreeing
1. `core/language_config.rs:850 find_project_root_with_outermost` + `:880 maven_reactor_root` (PR #29/#31 "outermost Maven reactor", commit 7be8f425): used by LSP (`LspConfig::find_root`, :215) - "outermost `settings.gradle*`, or the reactor aggregator whose `<modules>` lists the module".
2. `core/editor/launch_flow.rs:718 launch_project_root`: asks the LSP manager for the server root first (`server_root`), else a hard-coded marker list (`settings.gradle*, pom.xml, build.gradle*, .ovim, .git`) fed to (1). Marker list is a literal in the function, duplicating the Java `root_markers`/`outermost_root_markers` in `languages.toml`.
3. `core/editor/test_runner/jvm.rs:37-75 find_module` (the local test fallback): its own walk, its own Maven rule ("poms must be contiguous", `has_gradle_settings`) - **not** the reactor `<modules>` rule of (1). For a Maven project whose parent pom does not list the module, or non-contiguous nesting, `find_module` and the LSP disagree on `root`, so `-pl <rel> -am` is computed against a different root than the one the server (and `launch_project_root`, used at test_runner/mod.rs:196) uses. Also hard-codes `build/test-results/test` and `target/surefire-reports` (jvm.rs:~323,~349) while Hyperion's plan supplies `reports_dir`.
4. Non-JVM "git root" copies: `core/editor/project_nav.rs:33 project_root_of` and `core/editor/picker_manager.rs:315-343 picker_base_dir` are line-for-line the same loop (walk up for `.git`, else parent dir); `core/editor/ui_features.rs:84 open_file_tree` (git2 `discover`, else Cargo.toml/package.json markers); `core/native_diff.rs:23 worktree_root`; `core/git/ops.rs:38 workdir_of`; `core/editor/ai_tool_path.rs:411 discover_repo_root_from_start`. Call-site counts: `project_root_of` 4 (all in project_nav, recent-files), `picker_dirs().0`/`project_root_for_pickers` 8 (search_replace.rs:384, problems.rs:216, cmd_project.rs:302, project_nav x4, outline.rs:315), `launch_project_root` 3.

Consequence of divergence: `:grep`, replace-in-files and Problems use the **git root** (picker_dirs), while Run/Test/Debug use the **build root** and the LSP uses the **reactor root**. In a Gradle/Maven monorepo where the git root is above the build root, "replace in files" and "test" address different trees. Not necessarily wrong, but it is unspecified and unshared.

Also `local_test_plan` re-derives `cleanTest` (jvm.rs:297-301 hard-coded `{project_path}:cleanTest`) instead of calling `launch::plan::with_clean_test` (plan.rs:398, the copy used for Hyperion plans; plan.rs tests exist at :620-637; jvm tests assert at test_runner/tests.rs:862). Two implementations of "prepend clean so reruns aren't UP-TO-DATE", one of which (with_clean_test) is idempotent (`!argv.contains(&clean)`).

### Target design
New `core/project_root.rs` (or extend `language_config`):
```rust
pub enum RootKind { Vcs, Build, Lsp }
pub fn vcs_root(path: &Path) -> Option<PathBuf>            // one .git/gitfile walk (replaces project_root_of, picker_base_dir loop, worktree_root, discover_repo_root_from_start body)
pub fn build_root(path: &Path) -> BuildRoot { root: PathBuf, tool: BuildTool, module_dir: PathBuf }   // gradle settings / maven reactor via find_project_root_with_outermost
```
`find_module` becomes `build_root(file)` (Maven reactor rule, shared). `launch_project_root` keeps "LSP root first" but takes markers from the language registry, not a literal. `local_test_plan` builds its argv through `with_clean_test`. `ai_tool_path` keeps its own *policy* (canonicalisation/approval) but calls `vcs_root` for discovery. `picker_dirs()`/`open_file_tree` call `vcs_root`.

### Migration steps
1. Characterization tests: table test with 6 layouts (single gradle, multi-module gradle w/ settings, maven aggregator listing module, maven parent NOT listing module, nested non-contiguous poms, git root above build root) asserting the result of each of the current functions **as-is** (record the divergences; the Maven-not-listing and non-contiguous cases will show `find_module` != `maven_reactor_root`). Decide the winning behaviour per row (recommend the reactor rule; it has the unrelated-`~/pom.xml` guard the `find_module` walk lacks: `find_module` walks up *any* ancestor `pom.xml` as long as contiguous, but the contiguity rule stops at a gap so it is safe; the reactor rule additionally requires `<modules>` membership).
2. Add `vcs_root`; replace `project_root_of` + `picker_base_dir`'s loop (pure dedupe, identical semantics; keep `.git` file-or-dir `exists()`).
3. Replace `find_module` with `build_root`; make `local_test_plan` call `with_clean_test`; take `reports_dir` from a shared helper (`gradle_reports_dir(module)`, `maven_reports_dir`), one place.
4. Replace the literal marker array in `launch_project_root` with the registry's Java markers.
5. (optional) `open_file_tree`, `worktree_root`, `workdir_of` onto `vcs_root`.

### Size / risk
Delete ~110 (find_module 45, project_root_of 12, picker_base_dir 25, cleanTest copy 8, misc), add ~60. Risk LOW-MED: behaviour change only in the Maven edge rows, which the step-1 table pins. Kills the class "test runs against wrong `-pl`/root" (cf. commit 7be8f425 and c2168b8f/dfdbc1e9-style root fixes on the Hyperion side).

---------------------------------------------------------------------------------------------------

## 3. Delete the dead pre-#33 DAP launch path and adjacent stale code

PR #33 (commit ebc68ce7, c8e4d47d) changed debug to "ovim spawns the JVM, DAP only *attaches*". Now only `DapLaunchRequest::Attach`
is ever constructed (launch_flow.rs:1161, :1488; dap/mod.rs:663 test). Dead or vestigial (each verified by grep, count = definition only unless noted):

- `DapLaunchRequest::Launch(..)` (dap/mod.rs:65) - constructed nowhere; match arm at `bin/src/frontend/tick.rs:382`.
- `DapManager::launch` (dap/mod.rs:298) + `DebugAdapterClient::launch` (dap/client.rs:204) - only caller is that dead arm.
- `PendingDebugAction::Stop` (dap/mod.rs:81) - constructed nowhere (stop now uses `request_stop()`/`stop_requested` flag); handled at tick.rs:228.
- `PendingDebugAction::SyncBreakpoints` is real (set at tick.rs:387), but its body (tick.rs:206-211 and 258-270) duplicates the "sync all breakpoint paths + exception breakpoints" loop that also runs at tick.rs:203-211 for `take_breakpoint_sync_request`. Extract `sync_all_breakpoints(editor)`.
- `Editor::dap_config_for_current_file` (debug_integration.rs:838) - no callers. `debug_sync_breakpoints` only from tick.
- `DapManager.launch_request` field (dap/mod.rs:126) duplicates `PendingDebugAction::Start{launch}`; written at `debug_integration.rs:127` and read once at `tick.rs:373`; cleared at launch_flow.rs:1375. Two homes for one value; keep it as the single home and drop the `launch` payload from `Start` or vice-versa.
- `core/debug_config.rs:4-5` module doc claims "imports IntelliJ run configurations from `.idea/runConfigurations/` and `.run/`": no such code exists (grep for `idea`/`.run` in the file = only the doc lines). Also `DebugRunKind::Launch{ classpath, jvm_args, .. }` is turned into a plan in `plan.rs:503`, so it is live; only the doc is stale.
- `core/editor/search_replace.rs` ~ line 640: `let was_clean = ...; let _ = was_clean;` dead local.
- `open_location_picker(items, _title)` (lsp_modules/references.rs:176) ignores `title`; callers pass "Document Symbols" etc. that are silently dropped (bug-ish, see #5).
- `Editor::request_document_symbols` (lsp_integration.rs:1246): 1 reference (definition). The whole `:LspDocumentSymbols` flat picker (poll at lsp_integration.rs:650-690; slot `slots.document_symbols`; `dispatch_pending_intents:2269 document_symbols_impl`) is superseded by the full-tree outline picker (`editor/outline.rs:300`, own `documentSymbol` request with tree-sitter fallback at outline.rs:239). It lists only top-level symbols and ignores nesting. Two independent documentSymbol pipelines + two caches (`lsp.state.available_document_symbols` as `Vec<DocumentSymbol>` for AI tools/ai_subagents.rs:2892/ai_tool_execution.rs:830, and `OutlineState.symbols` as `OutlineSymbol`). Recommend: outline is the only requester; AI tools read from the outline cache (needs `OutlineSymbol` to carry range/kind, or keep raw `DocumentSymbol` alongside); delete slot/intent/picker branch.

### Steps
1. Delete `Launch` variant + `launch()` x2 + `Stop` variant + `dap_config_for_current_file` + stale docs (pure deletion, compiler guides; -60 lines).
2. `sync_all_breakpoints` helper; single `launch_request` home (-25).
3. Document-symbol unification (separate PR; touches AI tool code - needs the AI tool tests `ai_chat_tools_tests.rs` as characterization; -120 / +30).
Risk LOW for 1-2. Before deleting `DapManager::launch`, confirm `ovim/tests/run_launch_test.rs` + `helpers/fake_dap.py` do not exercise a `launch` request (grep shows none in src; check tests dir).

---------------------------------------------------------------------------------------------------

## 4. Split the god files (mechanical, zero behaviour change)

Over the CLAUDE.md "~3k" rule or close to it, all grown today:

| File | Lines | Non-test | What is in it | Proposed split |
|---|---|---|---|---|
| `bin/src/gui/mod.rs` | **5370** | ~5300 | snapshot structs (Gui* x40, lines 123-980), `GuiRequest` handling (983-1568), snapshot assembly, ~30 per-feature projection fns (`test_panel` 3915, `problem_list` 3949, `search_replace` 3991, `run_console` 4206, `debug_panel` 4171, `completion` 3582, `ai_chat` 3642 ...), line/segment layout (2641-3470), theme (4278-4398) | `gui/snapshot/{types,lines,panes,layout,segments}.rs`, `gui/projection/{ai_chat,run_console,test_panel,problems,search_replace,debug,completion,picker}.rs`, `gui/requests.rs`, `gui/theme.rs`. Keep `mod.rs` = `GuiBridge` + re-exports |
| `core/editor/lsp_integration.rs` | **3203** | 2455 (+750 tests at 2455-3203) | status toasts (18-107), init/sync (108-350), **poll_action_slots 601-844 (~245 lines)**, completion/inlay poll (845-1050), register/unregister (1074-1140), `dispatch_pending_intents` 2218-2455 (~240), doc-sync (1265-2200) | (a) move tests to `lsp_integration/tests.rs` (-750 from the file, free); (b) `lsp_integration/{sync,poll_slots,intents,servers}.rs`. `poll_action_slots` per-slot arms should each live beside their feature in `lsp_modules/*` (format/references/actions/symbols) |
| `core/editor/launch_flow.rs` | **2183** | ~2060 | request types, job state machine, LSP `executeCommand` plumbing (312-376), console UI actions (546-660), LSP frame lookup (625-670), config picker (1040-1125), test reporting (1641-1830 + 2062-2158: `junit_panel_lines`, `parameterized_methods_in_source`), debug polling (1835-1965) | `launch_flow/{mod(request+job),resolve(on_resolved,fallback,config picker),process(spawn_step,drain,exit),debug(begin_debugger,poll_debug_state,ingest_debug_output),test_report(finish_test_reports,junit_panel_lines),console_ui}.rs`. `launch_project_root` moves with #2 |
| `core/editor/mod.rs` | 3025 | - | pre-existing | not today's; leave |
| `core/lsp/{server,notifications,requests}.rs` | 2412 / 2382 / 2157 | - | `requests.rs` has 41 methods, +379/-222 today | see #5c |

Also test binaries: `ovim/tests/run_launch_test.rs` 2020 lines (81 tests) is fine as one scenario file but carries its own
`Session` harness (dir + fake python LSP + registration, lines 22-90); `lsp_startup_test.rs:27-50` and
`lsp_completion_menu_test.rs:40-60` each repeat the same "write server.py, write initialize-response.json, register
DynamicLanguageSpec" setup around `helpers/controlled_lsp.py` / `completion_lsp.py`. Move it into `tests/helpers/fake_lsp.rs`
(`FakeLspProject::new(script, caps)`). Separately, `ovim/tests/` holds **164 `*_test.rs` + others = 184 files, each its own
integration-test binary** linking the full `ovim` crate (link-time/disk cost; commit a44b15fb already had to "shrink debug info so
CI test binaries fit on the runner disk"). A `tests/it/main.rs` aggregation of the pure editor tests (most use only
`helpers::EditorTest`) would cut linking ~10x; do it in groups (motions/operators/render) not all at once. Low priority but the
largest test-cost lever. Note stray `tests/change_operations_test.rs.backup`.

### Steps (one PR each, `cargo check` + moved tests only)
1. gui/mod.rs: peel `theme`, `Gui*` types, per-feature projection fns into modules (5 PRs of ~1k moved lines each, or one). `pub use` keeps
   external paths stable (`crate::gui::GuiSnapshot` etc.).
2. lsp_integration tests out, then `poll_action_slots`/`dispatch_pending_intents` moved.
3. launch_flow split along the seams above; do it *after* #1/#3 so fewer lines move.
Risk LOW (compile-checked moves); conflict-prone with the follow-ups agent working in a worktree on folding/signature help
(`gui/mod.rs`, `lsp_modules/signature_help.rs`, `fold.rs`): schedule after that lands or split `gui/mod.rs` first and let them rebase.

### GUI/TUI projection duplication (in the same vein)
`bin/src/gui/mod.rs` recomputes what `bin/src/ui/renderer/*` also computes from the same core state, with independent
copies of the presentation rules:
- Run status class: `gui/mod.rs:4206 run_console` maps `RunStatus/RunOutcome` -> "running/succeeded/stopped/failed/error" and `LineKind` -> class strings; `ui/renderer/run_console.rs:46 status_style`/`:65 line_style` do the same match again.
- Replace-in-files row state: GUI `checked/unchecked/partial` (gui/mod.rs:4010-4030) vs TUI `all/none` from `checked_count()` (`ui/renderer/search_replace.rs:244-245`).
- `truncate_panel_text` / `centered_window_start` windowing exists only on the GUI side; TUI does its own scroll windowing per panel.
Target: core view-model methods (`RunRecord::status_class() -> StatusClass`, `FileMatches::check_state() -> CheckState`) that both frontends map to
colour/CSS, like `project_nav::symbol_row_columns` already does (that one is the right pattern: "derived here so the TUI and GUI agree").
~-60 lines, tiny risk; do inside the gui split PRs.

---------------------------------------------------------------------------------------------------

## 5. Smaller shared seams (each one PR)

### 5a. One "open a picker with results" helper
`Picker::new_with_results(..); set_picker; set_mode(Mode::Picker); mark_picker_selection_changed()` is spelled out in
10 sites (20 `mark_picker_selection_changed` calls total): `project_nav.rs:184 show_location_picker`, `lsp_modules/references.rs:187`,
`outline.rs:318`, `problems.rs:231`, hierarchy.rs, workspace symbol / git status / history pickers. They already **disagree on
base_dir**: cwd (`open_location_picker_keeping_hierarchy`), `picker_dirs().0` (outline, problems, project_nav), so a result's
relative path display root differs between "references" and "outline" opened from the same file. And `open_location_picker`'s
title parameter is ignored. Add `Editor::present_picker(picker: Picker)` (does the three calls, clears `lsp.state.hierarchy` unless
told otherwise) and route `open_location_picker` through `project_nav::show_location_picker`'s `picker_dirs().0` base. -50 lines.
De-risk: render test of `display` for a reference result from a subdirectory (ovim/tests/*picker*).

### 5b. Two fuzzy matchers
`core/editor/fuzzy.rs` (picker: `fuzzy_score`, `fuzzy_match_with_positions`, used at picker/mod.rs:278,580) and
`core/editor/completion_match.rs:48 fuzzy_match` (PR #36: tiers prefix/hump/subsequence, positions). Different scoring on purpose
(completion needs tiers + case rules; picker wants filename-weighted score) so **not** a merge candidate now; but expose one
`match_positions -> highlight` helper. Only worth a PR if a third consumer appears. Note here for awareness.

### 5c. LSP request boilerplate in `core/lsp/requests.rs`
30 copies of
```rust
let server = self.servers.get(language_id).ok_or_else(|| anyhow!("No server for language: {}", language_id))?;
if !server.supports_X().await { return Ok(None); }
let result = server.request("method", serde_json::to_value(params)?).await?;
parse_lsp_response(result, "method")
```
(lines 149, 245, 339, 405, 453, 510, ... 1313, plus the `_multi` variants at 1674-1955 that use `servers_for_document`). Two server-selection
strategies coexist: language-keyed primary (`self.servers.get(language_id)`, 34 sites) vs document-routed multi-server
(`servers_for_document`, used by `code_lenses` (1366), `_multi` fns): call/type hierarchy (added today, 1026-1250) picked the
legacy one, code lens the new one. Introduce
```rust
async fn request_primary<P: Serialize, R: DeserializeOwned>(&self, language_id:&str, method:&'static str, supported: impl Fn(&Server)->..., params:P) -> Result<Option<R>>
```
and migrate the 11 hierarchy/rename/symbol/folding/selection/highlight methods first (largest, newest, no `_multi`
twin), leave hover/completion/code-action for a second PR. -300 / +80. De-risk: existing `lsp_operations_test` +
`ovim/tests/lsp_*` fake-server tests; add a test that an unsupported capability yields `Ok(None)` for each migrated method
(the branch is identical today but easy to break).

### 5d. Git: repo discovery repeated
`Repository::discover` appears 6x in `git/mod.rs` (85, 249, 331, 361, 392) + `git/ops.rs:17 open` (the good wrapper: returns
repo + workdir-relative path) + `native_diff.rs` x5 + `git_tools.rs:539` + `ui_features.rs:93` + `ai_tool_path.rs:418`. `git_tools.rs:539
git_show_commit` opens a repo just to read `parent_count()` then calls `crate::git::commit_diff(&anchor, oid)` which discovers again;
`git/mod.rs:349-386 commit_info/commit_diff` are read helpers that belong in `git/ops.rs` next to `LogEntry`/`file_history`
(ops.rs:568-660), which they overlap in purpose (commit metadata for history rows). Move `commit_info`/`commit_diff`/`branch_name`
into `git/ops.rs` (or split `git/` into `read.rs`/`write.rs`), give `ops::open` a `pub(crate)` `open_repo(path)` used by native_diff too.
`diff_review/mod.rs:1498` shells out to `git fetch` (only remaining subprocess git in the editor path besides agent_runtime/ai tools; libgit2 fetch
needs credentials handling, so keep, but note it is the lone place that requires a `git` binary although `git/ops.rs` doc says "needs no git binary").
~-70 lines. Low value; do after #2.

### 5e. Workspace watcher is single-purpose
`core/editor/workspace_watch.rs` (433 lines, notify + `ignore` walker + SKIP_DIRS) and `lsp/watchers.rs` (333) feed only
`didChangeWatchedFiles`. Nothing else that would benefit uses them: the file tree (`filetree.rs`) has no external-change refresh, buffers
detect external edits only by mtime at write time (`buffer/file_io.rs:410-422`). Not duplication today; flagging that the watcher is the
seam to reuse (filetree refresh, "file changed on disk" prompt) instead of a second notify user later. `SKIP_DIRS` (workspace_watch.rs:35)
duplicates the skip lists in `run_log/workspace_capture.rs:125` and grep/walker builders (`editor/grep.rs:75 build_walker`, `project_search.rs:152` - the last two
differ: project_search sets `require_git(false)`, grep.rs does not, so `.gitignore` is honoured in a non-repo dir by replace-in-files but not by the
grep picker; and `grep.rs` uses smart-case + literal-fallback query semantics vs project_search's explicit regex/case/word options: both engines walk with
`ignore::WalkBuilder` and have their own binary/size/line caps: 5000/500 chars vs 20000/2000 chars/4 MiB). Unify only the *walker builder*
(`build_walker(root)` with `require_git(false)`) - one-line, one PR, tiny; leave the two matchers (picker streaming grep vs review-list search) alone, they
differ on purpose.

---------------------------------------------------------------------------------------------------

## Short list of the rest (not worth a full design)
- `spawn_test_job` etc: covered in #1. `set_quickfix_list` has 6 callers (cmd_project.rs:328, ui_features.rs:236, launch_flow.rs:1594/1596/1631, test_panel.rs:235); #1 step 2 covers it.
- `lsp/supervisor.rs` `TaskSupervisor` (385 lines, generic restart policy) is used only by `lsp/server.rs:430` for the reader task; `lsp/recovery.rs` (349) reimplements restart with backoff for whole servers (separate policy struct `RestartState`, own `backoff()`). Two restart-with-backoff engines; merging is speculative (different subjects) - just make `recovery::backoff` use `RestartPolicy`'s backoff if cheap.
- `lsp_execute_command`/`poll_server_command` (launch_flow.rs:312-376) is generic LSP plumbing living in the launch file; moves with the #4 split to `lsp_modules/commands.rs`.
- `lookup_frame_via_lsp`/`poll_frame_lookup` (launch_flow.rs:625-670) does workspace/symbol for stack frames; `project_nav` has the workspace-symbol picker; share the request wrapper (`search_symbols`, lsp_modules/navigation.rs:60) rather than a second call.
- `outline.rs` vs breadcrumbs: NOT duplicated (breadcrumbs *are* `enclosing_chain` over the outline cache). But `ai_tool_execution.rs:860 find_enclosing_symbol` re-implements enclosing-symbol lookup over raw `DocumentSymbol`s (different tie-break: `<` span vs outline's `min_by_key`, and it does not skip non-crumb kinds). Falls out of #3's symbol-store unification.
- `core/debug_config.rs` `parse_lsp_run_configs` vs `RawConfig::into_run_config` (lines 89-115 vs 176-224) parse the same three kinds (`gradle/attach/launch`) from two shapes (TOML vs JSON `Value`): convert TOML to `Value` (or serde-derive both onto one struct) and delete one; tests at 240-369 cover the JSON side only, add TOML equivalents first. -50 lines.
- `test_panel.rs` `TestRun` + `RunRecord` unification (long-term): after #1 the panel could be a *view* over `RunRecord` + parsed failures; do only if the GUI/TUI panels get merged.
- `dap/panel.rs` (464) vs GUI `gui_debug_rows` (gui/mod.rs:4115): GUI maps `PanelRow` to its own rows; check there is one mapping, not two (it is via `debug_panel_rows()`, fine).
- `ovim/tests/helpers/fake_dap.py`, `controlled_lsp.py`, `completion_lsp.py` each hand-roll `Content-Length` framing (3 copies, 81/77/133 lines): one `helpers/jsonrpc.py` module.
