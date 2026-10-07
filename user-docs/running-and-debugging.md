# Running and Debugging

ovim runs and debugs JVM programs (Java, Kotlin) from inside the editor. The
language server ([Hyperion](LANGUAGE_SUPPORT.md)) tells ovim what is runnable at
the cursor and how to build and launch it; ovim builds, starts the program,
and shows everything in the **run console**.

Java/Kotlin/Scala/Groovy language and debug support use hyperion-lsp, which is not yet publicly available.

## Keys and commands

| Keys | Command | Does |
|------|---------|------|
| `Space r r` / `Ctrl-F5` | `:Run` | run the code at the cursor (no debugger) |
| `Space r d` / `F5` / `Space d c` | `:Debug`, `:debug start` | debug the code at the cursor |
| `Space r l` / `Space d r` | `:RunLast`, `:debug last` | rerun the last run or debug (replaces a running one) |
| `Space r c` | `:RunConfig` | pick a configuration to run |
| `Space r C` / `Space d C` | `:DebugConfig` | pick a configuration to debug |
| `Space r s` / `Space d s` / `Shift-F5` | `:RunStop`, `:debug stop` | stop everything (build, program, debugger) |
| `Space r t` | `:RunConsole` | show / hide the run console |
| `Space r f` | `:RunFocus` | focus the console to scroll and jump |
| `Space r x` | `:RunClear` | drop finished runs from the console |
| `Space c l` / `Space c L` | `:CodeLens`, `:CodeLensDebug` | run / debug the code lens on this line |

Tests have their own keys (see [getting-started](getting-started.md#running-tests)):
`Space t n` / `:TestNearest`, `Space t f` / `:TestFile`, `Space t a` / `:TestSuite`,
`Space t l` / `:TestLast` and `Space t d` / `:TestDebug` (debug the nearest test) work
for Java and Kotlin and fill the test panel with a result per test.

While a debug session is stopped: `F5` continue, `F10` step over, `F11` step
in, `Shift-F11` step out, `F9` toggle breakpoint (also while running),
`Shift-F9` conditional breakpoint, `:eval expr` (explicit evaluation, method
calls run). The function keys work from the buffer, the debug panel and the run
console alike. When the debuggee stops in another file, that file is opened and
the marker is drawn there.

For Java and Kotlin `main`s the language server's root is the outermost build
root: the `settings.gradle(.kts)` above the file, or the Maven reactor (the
topmost `pom.xml` whose `<modules>` lead down to the module). All modules of a
build share one server, so Run/Debug see the sibling modules and dependencies.

"The code at the cursor" is decided by the language server
(`hyperion.resolveLaunch`): inside a test method or class it is the test,
otherwise the `main` entry point of the enclosing class or file. If the server
finds nothing (or is too old to know), ovim falls back to the configurations
below: one configuration runs directly, several open a picker, none gives a
message telling you what to add.

## What happens on Run / Debug

1. **Save.** Modified buffers are written so the build sees what is on screen.
2. **Resolve.** The server returns the build command, classpath, main class
   (or test task), working directory and arguments.
3. **Build.** If the plan has a build step (`./gradlew :app:classes`,
   `mvn -q compile`, `javac -d ...`) it runs in the background; its output
   streams into the console and the editor stays responsive. Compiler errors
   from javac, kotlinc, Gradle and Maven land in the quickfix list with the
   right file, line and column (and javac's `symbol:`/`location:` details),
   the first error is opened, and the launch is **aborted**.

4. **Launch.** Run starts `java -cp ...` directly. Debug starts the JVM
   itself, suspended and listening on a free port (so it has the same input,
   output and process-group handling as Run), and the debug adapter attaches
   to it. Tests use the build tool: Run executes the
   test task and reads the JUnit XML reports afterwards (failures go to
   quickfix); Debug starts `--debug-jvm`, waits for
   `Listening for transport dt_socket at address: N` on stdout or stderr, and
   attaches to that port. It gives up after three minutes and shows the last
   output.

`:make` and the test keys for other languages (`cargo test`, `vitest`, `pytest`,
`go test`, ...) run through the same machinery: their output streams into the run
console, `Space r s` / `:RunStop` stops them (the whole process group), and
starting a new run stops the one still going. `:make` puts diagnostics in the
quickfix list and opens the first *error*; failing tests fill the quickfix list
silently and show in the test panel. Both save modified buffers first.

### Tests

`<Space>tn`, `:TestFile`, `:TestSuite` and `:TestDebug` ask the server the same
way (`hyperion.resolveLaunch`, target `test`). If the server cannot answer
(older Hyperion, still indexing, no server, or no test found by the server), ovim
composes the command itself: tree-sitter finds the `@Test`, `@ParameterizedTest`,
`@RepeatedTest`, `@TestFactory` and `@TestTemplate` methods and the classes
around them (`Outer$Inner` for `@Nested`), the nearest `build.gradle(.kts)` /
`pom.xml` picks the module, and the build tool's filter selects the tests:

| Build tool | Run | Debug |
|------------|-----|-------|
| Gradle | `gradle :app:cleanTest :app:test --tests pkg.Class.method` | same plus `--debug-jvm` |
| Maven | `mvn -Dtest=pkg.Class#method [-pl module -am] test` | same plus `-Dmaven.surefire.debug=-agentlib:jdwp=...address=127.0.0.1:<free port>` |

`<Space>tn` with the cursor on a class declaration line runs that class (and
its nested classes). Parameterized invocations are listed as
`method [1] arg`. Surefire swallows the JVM's `Listening for transport` line,
so for Maven ovim gives the JVM a free port itself and attaches as soon as it
listens; Stop takes the whole process group down.

`cleanTest` runs first so an unchanged rerun is not skipped as UP-TO-DATE.
`./gradlew` / `./mvnw` are used only when the wrapper is really there. The
Gradle project path is derived from the module's directory relative to the
`settings.gradle(.kts)` (custom `projectDir` mappings are not followed).

After the run the JUnit XML reports are read into the **test panel**: one
line per test (`✓`/`✗`/`○`), failure messages and their stack frames without
JUnit/Gradle internals, a pass/fail summary. Failing frames in your code go to
the quickfix list (`:cfirst`, `:cn`). `Space t l` (`:TestLast`) repeats the last test
in the same mode (run or debug).

## The run console

A panel at the bottom of the editor (a "Run" tab in the GUI) keeps the output
of every run, with stdout, stderr, build output and editor notes in separate
colours. It stays after the process exits and shows the exit code and how long
it ran. It keeps the last eight runs.

Programs that read `System.in` work with Run and with Debug (ovim starts the
JVM itself, suspended on a free JDWP port, and the debugger attaches): in the
console `i` (or `:RunInput text`) sends a line to the program (echoed as
`» text`), `D` (or `:RunEof`) ends the input. Test tasks do not take input.

`Space r f` focuses it; `+` / `-` make it taller / shorter (`:PanelSize console +3`).
The test panel and debug panel are resized with `:PanelSize test +6` /
`:PanelSize debug -4` (`reset` restores the default; the debug panel also
takes `<` / `>` when focused). When both side panels are open they share the
width so neither is squeezed and the editor keeps at least 40 columns.

`Space r f` focuses it: `j`/`k` move, `Ctrl-d`/`Ctrl-u` page, `g`/`G` top/bottom
(`G` follows live output again), `[` / `]` switch between runs, `r` rerun,
`s` stop, `x` clear, `q` back to the buffer. On a stack-trace line
(`at com.foo.Bar.baz(Bar.java:42)`) or a compiler error, `Enter` opens the
source. Frames are looked up under the project root (`src/main/java`,
`src/test/kotlin`, ...); when the file is not there (other module, library
with sources, JDK sources the server materialised) ovim asks the language
server through `workspace/symbol`.

## `.ovim/debug.toml`

Project-local configurations, in the workspace root:

```toml
[[config]]
name = "App (build first)"
type = "launch"
main_class = "com.example.App"
classpath = "build/classes/java/main:build/resources/main"
args = ["--port", "8080"]
jvm_args = ["-Xmx512m"]
cwd = "."
build = ["gradle", "classes", "--console=plain"]   # optional; failure aborts

[[config]]
name = "Integration tests"
type = "gradle"                 # Run: gradle <task>; Debug: gradle <task> --debug-jvm
task = ":app:integrationTest"
args = ["--tests", "com.example.SlowIT"]

[[config]]
name = "Attach to server"
type = "attach"                 # debug only
host = "127.0.0.1"
port = 5005
```

Relative paths resolve against the workspace root of the current file (the
language server's root), not the directory ovim was started from. `./gradlew`
is used only when `gradle/wrapper/gradle-wrapper.jar` exists, otherwise
`gradle` from `PATH`. Configurations from `hyperion.runConfigurations` (for
example `.run/*.xml`) show up in the same picker. A malformed entry is
reported in the console instead of being skipped silently.

## Code lenses

Servers that provide `textDocument/codeLens` (Hyperion shows `▶ Run` on `main`
methods) get their lenses drawn at the end of the line as `▶ Run │ ▶ Debug`,
refreshed once edits settle and on `workspace/codeLens/refresh`. `Space c l`
on that line runs it. A `hyperion.run` lens goes through the same flow as
`Space r r` (a real JVM), not through Hyperion's built-in interpreter.

## Debug panel

While a session is active (and pinned with `Space d v` even without one) the
right side shows one list with the call stack, the variables of the selected
frame, watch expressions, all breakpoints, and the exception-breakpoint filters
the adapter offers. `Space d f` (or `:DebugPanel`) focuses it:

| Key | Does |
|-----|------|
| `j` `k` `Ctrl-d` `Ctrl-u` `g` `G` | move (the list scrolls with the cursor) |
| `Enter` `l` `Space` | select a frame, expand/collapse a variable or watch value, jump to a breakpoint, toggle an exception filter |
| `h` | collapse, or go to the parent variable |
| `d` `x` | delete the breakpoint or watch under the cursor |
| `e` `t` | enable/disable the breakpoint (or filter) |
| `a` | add a watch (`:DebugWatch`) |
| `E` | toggle "break on exceptions" (first filter) |
| `F5` `F9` `F10` `F11` | continue, breakpoint, step over/in (`Shift-F11` out) |
| `<` `>` | narrower / wider panel |
| `c` `n` `i` `o` `s` | continue, step over/in/out, stop |
| `q` `Esc` | back to the buffer |

When the debuggee stops on an exception, the panel, the status line and the
run console show its type and message (from the adapter's `exceptionInfo`).
With more than one thread a *Threads* section lists them (JVM housekeeping
threads are hidden); `Enter` on one shows its stack and variables, and
stepping then steps that thread.

`:DebugLogpoint <message>` (log `{expr}`s instead of stopping) and
`:DebugHitCount <n|>n|%n>` attach to the breakpoint at the cursor (empty
argument removes them). They are sent only when the adapter advertises
`supportsLogPoints` / `supportsHitConditionalBreakpoints`; otherwise the
breakpoint is not set (a bare line would stop on every hit) and the console
says so. Hyperion does not advertise them yet.

Commands: `:DebugWatch <expr>` (re-evaluated at every stop; a result with
children can be expanded), `:DebugUnwatch [n|expr]`, `:DebugBreakpoints
[list|on|off|clear]`, `:DebugException [name]`, `:DebugExpand name`,
`:eval expr`. `Space d w` watches the expression under the cursor. `K` while
stopped evaluates the expression under the cursor (`user.address.city` when on
`city`) and shows it, with its children, in the hover popup.

Disabled breakpoints stay listed and are drawn hollow (`○`) in the gutter but
are not sent to the adapter; conditional ones are `◆`. Exception filters come
from the adapter when a session starts, and your choice is kept for the next
session. In the GUI the debugger panel has the same variable/watch/breakpoint
list (click to expand, buttons to disable or remove), a click on a line number
toggles a breakpoint, and breakpoint and execution-line markers appear in the
gutter.

Debuggee output goes to the run console, and it is still there after the
program ends. When the session ends the execution marker is cleared and the
adapter process is stopped.

## Server commands

`:LspExec <command> [json args...]` runs a `workspace/executeCommand` on the
language server that owns the current file; `:LspReloadProject` is
`:LspExec hyperion.reloadProject`. Options passed to language servers
(for example `hyperion.buildToolClasspath`) are described in
[configuration.md](configuration.md#language-server-options-lsp_settings--ovimlspconfigure).
