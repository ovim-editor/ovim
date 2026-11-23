# Round 2 hands-on findings: run / debug / test (2026-09-29)

Tester: Claude (Sonnet 5.5), driving release builds of ovim + hyperion-lsp/-dap from /tmp/ide-bin, headless (`ovim send`, `ovim exec`, `/v1/render`).
Env: OpenJDK 26, Gradle 9.3.1, Maven 3.9.9 (from /tmp/icp), network available. Scratch projects in /tmp/r2-run/:
- `shop` (Gradle: `lib` + `app` Java with JUnit5/Mockito/AssertJ, `kapp` Kotlin+coroutines+kotlin.test, `web` Spring Boot 3.5.6)
- `mvnmulti` (Maven parent + core + app, guava), `plain` (no build tool), `ktproj` (Java+Kotlin multi-module), `stripe` (resolveLaunch only)
- extras: attach target `ext.Ticker` on jdwp 5006, `.ovim/debug.toml` with launch/attach/gradle configs, `.run/*.xml`.

Repos: [ovim] = `<repo>`, [hyperion] = `<hyperion-ls checkout>`. Reads of `ovim send` keys used below: `<F9>` breakpoint, `<F5>` debug/continue,
`<Space>rr` run, `<Space>rf` focus console, `<Space>tn/tf/ta/tl/td` tests, `<Space>df` focus debug panel, `<Space>cl/cL` lens.
Note: I sometimes mis-picked breakpoint lines (a `}` line has no code) and re-did those runs; only findings that reproduced are listed.

## P0

### P0-1 Maven multi-module: LSP root is the nearest `pom.xml` (the module), so Run/Debug get a classpath without sibling modules or dependencies, and no warning is shown [ovim, hyperion]
Repro: `ovim /tmp/r2-run/mvnmulti/app/src/main/java/demo/app/Main.java --headless --session s` (app depends on module `core` which uses guava), wait for LSP, cursor in `main`, `<Space>rr`.
Observed: `$ java -cp /tmp/r2-run/mvnmulti/app/target/classes demo.app.Main` then `NoClassDefFoundError: demo/core/Names`. The classpath has only `app/target/classes`; the resolveLaunch result carried no warnings.
The build step itself was right (`mvn -q -pl app -am compile` from the reactor root), so only the classpath is wrong.
Expected: `app/target/classes:core/target/classes:guava...` (what Hyperion returns when started on the reactor root: I confirmed with a standalone client, `rootDir=/tmp/r2-run/mvnmulti`, correct 9-jar classpath, `warnings: []`).
Root cause: `ovim-core/languages.toml:172-265` `root_markers` picks the nearest ancestor with any marker; for Gradle `settings.gradle*` comes first in the list so it works, for Maven the module pom wins (lsp.log: `root=/tmp/r2-run/mvnmulti/app`). Fix in ovim (prefer topmost pom that has `<modules>` / walk up while a parent pom lists this dir), and/or in hyperion (when the root pom has `<parent>` with a relativePath that is a reactor aggregator, climb).
Same root also means Maven tests classpath/debug for sibling modules are incomplete. Maven `TestNearest` itself worked (mvn ran, `Tests: 1 passed`).

### P0-2 Spring Boot app whose Gradle build uses `platform(SpringBootPlugin.BOM_COORDINATES)` runs with an incomplete classpath -> `ClassNotFoundException` at startup [hyperion]
Repro: `web/build.gradle.kts` = `plugins { java; id("org.springframework.boot") }` + `implementation(platform(SpringBootPlugin.BOM_COORDINATES))` + `spring-boot-starter-web` (Boot 3.5.6, all jars in ~/.gradle cache). Open `WebApp.java`, `<Space>rr`.
Observed: exit 1 after 1.9 s, `ClassNotFoundException: io.micrometer.observation.transport.RequestReplyReceiverContext`. Status bar had said "Hyperion: 5 dependency problem(s) ... io.micrometer:micrometer-observation". `resolveLaunch` warning: "classpath incomplete: 5 dependencies not in the local cache". Truth from `gradle :web:dependencies`: `micrometer-observation:1.14.11 -> 1.15.4` (constraint from the imported BOM).
Expected: BOM version wins for transitives (as Gradle does) and `micrometer-observation/commons:1.15.4` (present in cache) are on the classpath.
Observed diff vs Gradle: only `micrometer-commons` and `micrometer-observation` were missing; direct starter jars were resolved (BOM was applied to direct deps only).
With the `io.spring.dependency-management` plugin (1.1.7) the same app resolved perfectly and ran. Suspect: platform() constraints are not applied to transitive edges in the cache-only mediation (hyperion-lsp/src/gradle_integration.rs / project_classpath.rs).
Note the failure is only visible as a stack trace; the run console does not print the `warnings` when they are "N dependencies not in the local cache" for the Gradle case (it did print the Kotlin warning), so the user is not told why.

## P1

### P1-1 `:eval <expr>` rejects every method call ("method calls are not run for hovers") [ovim]
Repro: stop at a breakpoint in `Cart.sum` lambda, `:eval it.line() * 2`, `:eval it.name().toUpperCase()`.
Observed status: `Eval error: DAP error: method calls are not run for hovers; evaluate the expression in the debug console`. Field/arith `:eval it.price + 1` works (`= 2.5`).
Expected: run the call (docs promise "Evaluate method calls"; hyperion supports it for `repl`).
Root cause: `ovim/src/frontend/tick.rs:344-348` `PendingDebugAction::Evaluate` calls `evaluate(..., Some("hover"))`; should be `Some("repl")`. Workarounds that DO work: `:DebugWatch it.line() * 2` (= 12) and conditional breakpoints with calls (`:DebugCondition it.qty() == 3` stopped only on "pear"). `K` hover-eval works for fields (`cart` shows children), not calls (by design).

### P1-2 Stale JDWP step request: after a step-over that was interrupted by a breakpoint, a later Continue stops again with reason "step" [hyperion]
Repro (Main.java from shop): breakpoints on line 15 (lambda in `Cart.sum`) and 33 (`double s = cart.sum();`). Debug -> stops at 33. `:debug next` (F10) -> stops in lambda at 15 (breakpoint interrupts the step). `<F5>` -> stops at 15 again (pear; correct). `<F5>` again.
Observed: stops at `main Main.java:34` with reason `step` (an unrelated stale step-over finished). Expected: no stop at 34; runs on to the next breakpoint (22).
Root cause: `hyperion-dap/src/session.rs:447-457` `handle_continue` never calls `clear_active_step_request()`, and the Breakpoint branch in `server.rs` (~L300-320) does not clear it either (only the SingleStep branch does). Clear it on continue and on any stop.

### P1-3 Stopping in a different file than the open buffer does not open that file, and the ▶ marker is drawn on the same line number of the wrong buffer [ovim]
Repro: shop; breakpoint at `lib/.../Pricing.java:17` (label()); open `app/.../Main.java`, cursor in `main`, `<F5>`.
Observed: stack panel says `label Pricing.java:17`, but the editor stays on Main.java (cursor unchanged at 29) and shows `▶` on Main.java line 17 (unrelated code). Only pressing Enter on the frame in the debug panel navigates to Pricing.java (that works, cursor line 16).
Root cause: `ovim/src/ui/renderer/buffer.rs:665-669` compares only `exec_line == line`, ignoring the file part of `execution_position()`; and no jump-to-file on stopped event (only frame selection does it). Same issue likely in `ovim/src/gui/mod.rs:2700`. Very visible in multi-module debugging and any step-into a user class in another file.

### P1-4 Kotlin breakpoint on a line inside an inline lambda (`orders.forEach { o -> ... }` in `runBlocking`) stops at the wrong line, twice, and with the wrong locals [hyperion]
Repro (kapp `Coroutines.kt`): breakpoint on line 20 (`val x = o.amount * 2` inside `orders.forEach {}`), debug `main`.
Observed: 2 stops, `invokeSuspend Coroutines.kt:23` and `main Coroutines.kt:23` (the closing `}`), no `o`/`x` locals; the 3 real iterations at line 20 never stop. `javap` confirms line 20 exists in `CoroutinesKt$main$1.invokeSuspend` (code index 355).
Likely cause: `hyperion-dap/src/session/breakpoints.rs` `bind_entry`: `slide_line = entry.actual_line.unwrap_or(requested)` - the entry slides to 23 on the first-loaded class (the `CoroutinesKt` facade has no code on line 20) and `actual_line` is then reused when `CoroutinesKt$main$1` loads later. Should keep `requested` for classes that have exact code, and slide only when no loaded class has it.
Positive: breakpoints in Kotlin top-level functions (`fetch`, line 10) and in Kotlin tests (bp at line 17) hit correctly.

### P1-5 Stopping on an exception shows no exception type or message [ovim, small hyperion part]
Repro: `Boom.java` (`a[5]` on a 2-element array), `<F5>` with default "Uncaught" filter.
Observed: stops at `Boom.java:12`, panel says `Call Stack (exception)`; nothing names `ArrayIndexOutOfBoundsException: Index 5 out of bounds for length 2`, no `$exception` variable. Hyperion implements DAP `exceptionInfo` (`hyperion-dap/src/session.rs:670`) but ovim never sends it (no `exceptionInfo` in ovim-core). Also `StoppedEventBody.description/text` are `None` (`hyperion-dap/src/server.rs:359`).
Exception breakpoints themselves work: Caught + Uncaught toggles stopped at the caught NPE (line 7) then the uncaught AIOOBE.

### P1-6 Step-into dives into JDK internals (no stepping filters) and step-over out of a lambda lands on a synthetic frame [hyperion]
Repro A (Spring `WebApp.price`, bp on `double t = Pricing.total(List.of(p, p));`): `<F11>` x3 -> `valueOf Double.java:9` -> `<init> Double.java:10` -> `<init> Number.java:59`. IntelliJ skips `java.*`/`jdk.*`/`sun.*` by default and goes to `Pricing.total`.
Repro B (Main lambda at line 15/16): step over on the last line of the lambda ends in `apply ?:0` (hidden-class lambda proxy, no source, line 0); `<F10>` again stays in JDK stream frames.
Root cause: `session.rs:474-511` `do_step` sets only a `Step` modifier, no `ClassExclude` filters and no auto step-out when the landing frame has no source/line.
Also: the JDK here has no `src.zip` (Arch package), so those frames are source-less; a stub/decompile fallback would help (round 1 #13 still open).

### P1-7 Maven test debug hangs 3 minutes and `Stop` leaves the surefire JVM alive holding port 5005 [ovim + hyperion contract]
Repro: mvnmulti, open `MainTest.java`, `<Space>td`. Console shows `[INFO] T E S T S` then nothing for the 3 min timeout.
Observed: the forked surefire JVM IS listening (`-agentlib:jdwp=...suspend=y,address=localhost:5005`, `ss` confirms), but the `Listening for transport dt_socket at address: 5005` text never reaches Maven's output (I reproduced with plain `mvn -B -Dtest=... -Dmaven.surefire.debug ... test`: only "T E S T S"). ovim waits for that line, so it never attaches. After `<Space>rs` the jdwp JVM survived (had to `kill -9`).
Fix ideas: poll-connect port 5005 instead of scraping the log; kill the process tree (ovim) on stop. Gradle test debug works and stop cleans up (verified: no worker left, port freed).

### P1-8 Debug sessions get no stdin (program sees EOF immediately) [ovim / hyperion-dap]
Repro: `Echo.java` (Scanner on System.in), `<F5>`. Prints `ready`, `bye`, `Debug session ended (exit code 0)`; `:RunInput hi` does nothing. Documented limitation, but a real gap vs IntelliJ (stdin in the run/debug console). Run mode (`<Space>rr`, `i`/`:RunInput`, `:RunEof`) works fine.

### P1-9 Hyperion memory: one full server per language, each ~2 GB after 25 min on a 4-module Gradle project [hyperion / ovim]
`ps`: session with shop opened (Java + a Kotlin file opened) had two `hyperion-lsp` at RSS 2.48 GB and 2.02 GB after 25 min (fresh: ~480-550 MB each). Plain project with a .kt file also spawns a second server (2 x ~500 MB). Consider sharing one server between Java and Kotlin buffers of the same root, and investigate growth.

### P1-10 False compile errors on inner-class instantiation `Cart.Audit a = cart.new Audit();` [hyperion]
Main.java has `static class Cart { class Audit {...} }` and `Cart.Audit a = cart.new Audit();` -> 3 errors ("cannot find symbol: Cart.Audit", "cannot find symbol: Audit", "incompatible types: shop.app.Audit cannot be converted to shop.app.Main.Cart.Audit"). javac accepts it and the program runs. Permanent red squiggles in a running project.

## P2

- F5 / F10 / F11 / Shift-F11 do nothing while the debug panel is focused (mode `DEBUG`), contradicting the docs ("F5 continue, F10 step over ... while stopped"). Panel letters `c n i o s` work. `:DebugBreakpoints list` (and `<Space>df`) enter that mode, so F5 pressed afterwards is silently ignored. [ovim, input/normal/mode_transitions.rs:198-250 is only reached in NORMAL]
- No thread information/switching: with two threads hitting the same breakpoint (`Threads.java`) the panel shows a single stack, no thread name, no way to pick a thread. All-stop only. [ovim, hyperion]
- No logpoints, no hit-count/ignore-count breakpoints (`:DebugCondition` only). [ovim]
- Toggling an exception filter while a session is stopped does not reach the adapter (turned "Caught" off mid-session, session kept stopping on JDK-internal caught exceptions, e.g. `URLClassPath.getJarFile`, `Class.forName0`); and the filter choice persists across sessions/projects, so "Caught Exceptions" left on makes the next Kotlin debug stop inside the class loader before user code. [ovim]
- Adapter crash is reported as success: `kill -9` of `hyperion-lsp dap` mid-session prints `Main (app): session ended (exit 0) after 16.2s` / `Debug session ended`. Debuggee JVM was cleaned up. [ovim]
- Kotlin file without any build tool: warning printed but launch proceeds and fails with `Could not find or load main class KKt`; should abort or use `kotlinc`. [ovim/hyperion]
- `:TestNearest` with the cursor on the class declaration line runs only the first test method (`stockDoubles`, "1 passed") instead of the whole class. Docs say a test class under the position wins. [hyperion resolveLaunch / ovim]
- Parameterized tests show as `OrdersTest.[1] 1` (method name lost, from JUnit XML testcase name). [ovim junit.rs]
- Test panel, debug panel and run console all open at once leave the test panel ~47 columns wide (140-col terminal); stale debug panel with old watches (`<not evaluated>`) stays after the session ends. [ovim UX]
- Hover on an identifier shows the line `Hyperion Evaluation - Activate at hyperion-ls.com` inside the LSP hover popup. [hyperion]
- `:cclose` clears the quickfix list ("Quickfix list cleared") instead of only closing the window. [ovim]
- Stack frames from lambdas show `apply ?:0` / `run ?:0` (synthetic hidden-class frames) and JDK/JUnit frames are not collapsed. [hyperion / ovim]
- `/tmp/hyperion-dap.log` was last modified 12:53 despite adapter runs at 14:xx (log writing not observed; docs claim `$LOG_DIR/hyperion-dap.log`). Unverified: may be LOG_DIR. [hyperion]
- Run-config picker: after `:RunClear` + `<Space>rr` in `WebApp.java` (after a failed run) the run silently did nothing three times (console stayed "nothing has run yet"); worked after `:e` to another file and back. Could not reproduce reliably afterwards. [ovim, flaky]
- Debug/attach port is hardcoded to 5005 for Gradle/Maven test debug: two simultaneous sessions (or anything already on 5005) collide.
- `Run` with unsaved edit in another hidden buffer: `:e other` is refused (normal Vim), so I could not test multi-buffer save; single-buffer save-before-run works.
- Kotlin file `K.kt` (no build tool) etc. spawn a second hyperion (see P1-9).

## Works well (verified)

- Run main under cursor (Gradle): 5.6 s cold (daemon start), 0.5-0.8 s warm; auto save + rebuild; edit -> rerun shows new output; multi-module: editing `lib` and running `app` rebuilds lib (`lib-v1` -> `lib-v2`).
- Compile error: launch aborted, javac error lands in quickfix with right line/col and `symbol:`/`location:` (`:cfirst` jumped to Main.java:40:31); no-build-tool project uses `javac -g -d out/production` and works (run and debug, debug with locals).
- stdin: `<Space>rf` `i` text `<CR>` and `:RunInput`, `:RunEof` all worked (`» hello world`, `echo:`, `bye`).
- Spring Boot server (with dep-management plugin): output streams live, `curl :18080/price` answered, `<Space>rs` graceful shutdown (Tomcat shutdown hook ran), port freed within 0.5 s, rerun while running replaces the process, no leaked java.
- Stop during the Gradle build phase works; double `<Space>rr` replaces cleanly; 300k lines of output (8 s) keep the editor responsive.
- Stack trace `Enter` in the console jumps to source; `.ovim/debug.toml` launch/attach/gradle configs and `.run/*.xml` Application configs (with VM/program args) appear in the picker and run; code lens `<Space>cl`/`cL` runs/debugs a real JVM.
- Debug: line, conditional (`it.qty() == 3` with a method call) breakpoints; add/remove breakpoint while running and while stopped; watches with method calls, arithmetic, string concat (`it.line() * 2 = 12`); variables show runtime `toString`, collections expand to elements (`items = size=2 [...]`, `[0]` ...); `K` hover-eval with children; frame selection navigates across files; step over/in/out; breakpoints in lambdas, inner classes (`Audit.note`), anonymous/nested (`Main$Cart`), Kotlin top-level and suspend functions; exception breakpoints; `:DebugConfig` attach to an external JVM (attach ~2 s, later added breakpoint hit, `debug stop` detaches and leaves the target running).
- Gradle `bootRun` via `type = "gradle"` debug config attaches (5005 detected in 2 s) and a breakpoint in a `@GetMapping` handler stopped on a real HTTP request.
- Tests: nearest / file / suite / last / debug for JUnit 5 (+Mockito, AssertJ), `@Nested` (`OrdersTest$WhenEmpty.zero`), `@ParameterizedTest`, Kotlin `kotlin.test`; failing assertion shows message (`[stock of b] expected: 3 but was: 2`) and `:cfirst`/`:cn` land on the exact failing line (33, 27); debugging a failing test with a breakpoint stopped correctly (Java and Kotlin) and the panel updated with the failure afterwards; Maven `TestNearest` works.
- Resilience: killing `hyperion-lsp` mid-debug leaves the debug session working, `:LspRestart` recovers (state `failed: Writer failed: Broken pipe` -> `ready`); Run works without LSP via re-resolve; killing an ovim session while a program runs leaves no orphan java and no orphan hyperion; closing the buffer during a debug session keeps the session alive; "Nothing to run here: no main method or test" and "No Gradle or Maven project found" messages are clear.
- `resolveLaunch` is solid for Gradle: FQN main classes, nested/Kotlin facades, module outputs of dependent modules, IDEA configs; stripe-clone resolves 71 classpath entries with an honest warning.

## Timings (warm daemon, 140x45 headless)
Run Java main 0.5-0.8 s; Spring Boot ready ~2 s after build; Debug launch to first stop 6-8 s; TestNearest 1.1-4.4 s; TestDebug to first stop ~20 s; attach ~2 s; bootRun --debug-jvm to attach 2 s.
