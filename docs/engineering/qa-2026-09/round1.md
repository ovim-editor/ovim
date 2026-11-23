# Round 1 hands-on findings (2026-09-29)

Tested with release builds of hyperion-ls aa7261d4-ish and ovim origin/main, driving ovim headless
on scratch copies of hyperion-ls/example-projects (stripe-clone = Spring Boot+Lombok+Gradle,
a Java+Kotlin two-module project, a plain Java project). O = observed, C = inferred from code.

## Run/debug

1. O `:debug start` and `<Space>dc` never launch: pass `run_config: None`
   (ovim-core/src/commands.rs:2127-2152, editor/input/leader.rs:134-160); Initialized handler only syncs
   breakpoints (ovim-core/src/dap/mod.rs:354-361) -> "configurationDone failed: not connected to debuggee".
   No-config F5 gives the same cryptic error (ovim/src/frontend/tick.rs:422-437). [ovim]
2. O Hyperion `.run/*.xml` Application config launches with no classpath -> "Could not find or load main
   class", silently (hyperion-lsp/src/run_config.rs:93-118). `known_runtime_dependency_jars` exists at
   hyperion-lsp/src/project_classpath.rs:1265. [hyperion]
3. O Output/launch errors vanish when the program terminates: panels_visible=false on Terminated
   (dap/mod.rs:348-353); renderer draws only while session_active (ovim/src/ui/renderer/core.rs:115-132).
   6-line panel, no scrolling. [ovim]
4. O No build before launch: edited+saved code, relaunch ran stale classes. No hot code replace. [both]
5. O Gradle debug config freezes editor ~60s then fails: spawn_gradle_and_wait (tick.rs:647-698) reads
   stderr but Gradle prints "Listening..." on stdout; awaited inside tick loop; leaks suspended JVM on
   5005; prefers ./gradlew even if wrapper jar missing; hardcoded 5005. [ovim]
6. O Kotlin breakpoints and Java nested-class breakpoints never hit: source_path_to_jni_signature maps a
   file to one class (hyperion-dap/src/session/presentation.rs:4-26, used session.rs:306). Must match
   loaded classes by SourceFile attribute + line tables. [hyperion]
7. O Evaluate only handles variable/field paths; `users.size()`, `total * 2`, string concat fail
   (hyperion-dap/src/session.rs:1390-1600, session/evaluation.rs). Variables show declared type, not
   runtime type / toString. [hyperion]
8. O No Java/Kotlin test runner: runners only rust/js/python/go
   (ovim-core/src/editor/test_runner/runners.rs:66-76, nearest.rs:56-61). [both]
9. O Session lifecycle: failed attach leaves session "active"; adapter lingers after Terminated;
   execution marker not cleared; stale status errors; `hyperion-lsp dap` writes no log; adapter sends
   `terminated` twice and no `exited` event. [ovim mostly, adapter events/logging hyperion]
10. C No run command; ovim has no codeLens support; Hyperion's Run lens executes in the bytecode VM, not
    a real JVM (hyperion-lsp/src/run_executor.rs). `:make` only parses rustc/gcc/tsc. [ovim]
11. C GUI debugger partial: no variables, evaluate, breakpoint/execution gutter, start button
    (ovim/src/gui/mod.rs:792-798,3630-3662; ovim/gui/src/App.tsx:2864+). [ovim]
12. C Project root = current_dir() not LSP root (tick.rs:327-330,414); relative paths resolve vs ovim cwd;
    run_configurations() hardcodes the "java" server (ovim-core/src/lsp/requests.rs:1384). [ovim]
Works: attach flow, breakpoints/stack/variables/step in Java top-level classes, :DebugExpand, frame nav.

## General IDE

P0
1. Library jars never indexed on real Gradle projects. After a real build populated ~/.gradle, zero jars
   indexed: one "POM metadata is incomplete" (guava, jackson, logback...) marks the model incomplete and
   indexes nothing (hyperion-lsp/src/gradle_integration.rs:591,630). Consequences: no hover/definition/
   completion into Spring/Guava/JUnit, false `cannot find symbol: assertEquals`, false
   `cannot find symbol: log` on every @Slf4j class, rename refused project-wide ("workspace source and
   dependency indexing is incomplete"). Also: fresh clone has no jars at all (never runs Gradle). [hyperion]
2. ovim never sends didOpen for Java/Kotlin: Java startup path (ovim/src/frontend/tick.rs:13-19
   apply_java_status) sets Ready but never syncs the document, unlike generic path
   (ovim/src/lsp_init/background.rs:191). Diagnostics appear only after a hover/goto/completion. [ovim]
3. Rename mostly unusable: class rename fails whenever the class is imported anywhere ("contains an
   unresolved refactoring candidate 'Circle'"), even in a dependency-free project. Package rename
   unsupported; safe delete returned applied:false. ovim shows only "LSP request 'textDocument/rename'
   failed", dropping the server's message. [hyperion M, ovim S]
4. Missed compile errors: call to missing method, wrong ctor arity, missing return, unhandled checked
   exception, private field access, class not implementing interface method. Noisy defaults: "Magic
   number '2.0'", "Parameter 'args' is never used" on main. [hyperion]

P1
5. Completion: nothing for `names.stream().`, `Customer.` (static), `Customer.builder().` (Lombok),
   `customer.getEmail().` (HY-000266 chains through library return types). `Str` offers internal JDK
   classes (Stratum, StreamBarrier) not String; labels say "from workspace" without package; inside
   `names.forEach(n -> n.` offers List methods. Auto-import on accept works. [hyperion]
6. Import quick fix for `List` offers com.sun.tools.javac.util.List and java.awt.List but not
   java.util.List; offers constructors as imports ("Import 'java.util.ArrayList.ArrayList(int)'"), private
   nested Arrays.ArrayList, duplicates. No "import all missing"; organize imports doesn't add. [hyperion]
7. Extract interface emits `public public interface IProbe`. "Move 'Probe' to package" CRASHED ovim
   (ropey "Line index out of bounds") and Hyperion's edit targets a not-yet-existing file without a
   CreateFile op and blanks the original instead of renaming/deleting it. Generate ctor/getters ignore
   Lombok. [hyperion S, ovim S]
8. No JUnit/Gradle test runner (see run/debug 8). Code lens `className` bare ("App") not FQN. [both]
9. No recovery when Hyperion crashes: after kill -9, state stuck "failed", zombie process, no restart
   command. [ovim]
10. External changes not picked up: method renamed in unopened file via git; diagnostics in open caller
    never updated even after editing it. ovim sends no didChangeWatchedFiles. [both]
11. Diagnostics only for open files; no workspace diagnostics (workspace/diagnostic "Method not found").
    :make with gradle: column 0, duplicate entries, "symbol:" detail lost. [both]
12. Type hierarchy unreachable: Hyperion registers dynamically only; ovim hard-codes capability off
    (OV-00132). Call hierarchy flat one-level picker labelled file:line. Goto-implementation on an
    interface METHOD returns the method itself. [both]
13. No navigation into JDK sources when no src.zip (no decompiler / stub fallback). Workspace symbols
    don't include JDK/library classes, no CamelHump (`PIS` -> 0), fields ranked above classes. [hyperion]

P2 Kotlin
14. Java->Kotlin: `SquareKt` facade false "cannot find symbol"; `sq.getSide()`, `c.radius` unresolved;
    references to getRadius null. Kotlin completion offers `getRadius` not `radius`; nothing for `"x".`
    or `shapes.first().`. Kotlin type mismatch not reported. No Kotlin formatting/folding/code actions.
    ovim highlights Kotlin with the Java grammar (ovim-core/languages.toml).

P3 project-level
15. ovim lacks replace-in-files, recent files picker, breadcrumbs, git stage/commit/log/conflict UI.
16. No Spring awareness, DB tools, HTTP client.
17. Maven resolution cache-only too.

Wrong docs: ovim says Hyperion is auto-downloaded (only `which`, ovim/src/lsp_init/java.rs:50);
Hyperion README rename "Yes", Lombok "Yes", type hierarchy, code lens rows overstate; ovim headless.md
`lsp hover FILE:LINE:COL` can't combine with -s.

Works well: warm startup ~3.3s, 480-690MB; Java references/incoming calls; goto-impl on types incl.
Kotlin implementors; Java formatting; signature help; auto-import on completion; Lombok getters in
completion; generate equals/toString; :make+quickfix; live grep; blame.
