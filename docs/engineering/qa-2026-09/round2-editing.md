# Round 2 hands-on findings: editing / navigation / refactoring / diagnostics (2026-09-29)

Tester: Java-side by me, Kotlin + Java/Kotlin interop by an Opus subagent (section K). Binaries were
/tmp/ide-bin/{ovim,hyperion-lsp} (release builds of main). NOTE: someone ran `cargo build --release -p hyperion-lsp`
during the session, so a binary may have been swapped mid-run (Kotlin part especially). Projects (all scratch under
/tmp/r2-edit): `stripe` (copy of /tmp/gap/stripe = Spring Boot 3.5.6 + Lombok, Gradle; the example-projects copy pins
Boot 3.2.1 whose jars are not in ~/.gradle so it resolves nothing), `mini` (plain Java+JUnit), `jackson` (unzipped
jackson-databind-2.19.2 sources = 481 files / 135k lines, the "medium real-world" project; no Kafka checkout exists on
this machine), `kt/kmix` (+ new `billing` module). Helpers: /tmp/r2-edit/{h.sh,cp.py,pq.py,ca.py,refac.py,alldiag.py}.
O = observed. C = from reading code. "Regr" = regression vs round 1.

Headline: the big round-1 P0s are genuinely fixed (libraries indexed, didOpen at startup, crash recovery, class rename,
move-to-package, extract interface, external-change pickup, type/call hierarchy). What now blocks daily use is
**diagnostic correctness** (huge false-positive floods on real code + most real compile errors still missed),
**completion** (chained calls, `System.out.`, JDK class names) and **ovim's LSP UX gaps** (no signature help, useless
workspace-symbol picker, rename edits lost after `:e relpath`).

---------------------------------------------------------------------------------------------------------------------
## P0

### P0-1 Diagnostics: massive false positives on real, compiling code  [hyperion, semantics/type checker; not a regression (round 1 had no jars), newly visible]
Repro (jackson-databind sources, deps jackson-core/annotations resolved, "Indexed 2 dependency JARs"):
`ovim /tmp/r2-edit/jackson/src/main/java/com/fasterxml/jackson/databind/ObjectMapper.java --headless --session x`, wait ~14s,
`ovim lsp diagnostics -s x` -> 196 errors (file compiles under javac). Sampled 10 files by `:e`: ObjectNode 78 errors,
BeanSerializerBase 106, StdDateFormat 18, ClassUtil 21, StdSerializer 13, JsonNode 12, StringDeserializer 4; only
TypeFactory/BeanPropertyDefinition were clean. Examples (message = exact):
- ObjectNode.java:76 `JsonPointer ptr = ...` -> "incompatible types: ...databind.node.JsonPointer cannot be converted to ...core.JsonPointer"
  (type from `import com.fasterxml.jackson.core.*` is resolved as a same-package type).
- ObjectNode.java:59 `ObjectNode ret = objectNode();` -> "ObjectNode cannot be converted to ObjectNode.ObjectNode".
- ObjectNode.java:156 "cannot find symbol: OverwriteMode" (nested enum inherited from a superclass).
- ObjectMapper.java:922.. "cannot find symbol: ObjectMapper" inside ObjectMapper.java itself; :126 "cannot find symbol:
  java.io.Serializable // as of 2.1" (trailing comment swallowed into the type name).
- BeanSerializerBase.java:757: 20+ parser errors ("Expected SEMICOLON, found FINAL_KW") on valid syntax.
Expected: zero errors on code javac accepts. Suspect: wildcard-import / same-package-first resolution and inherited nested
types in `hyperion-semantics/src/type_checker/mod.rs` (check_method_call ~:452-560, is_type_assignable) and the resolver.
Impact: red squiggles everywhere on non-trivial code, which trains the user to ignore diagnostics.

### P0-2 Diagnostics: overload resolution false errors on ubiquitous library calls  [hyperion]
Repro (stripe, any file): with `Logger LOG = LoggerFactory.getLogger(..)`:
`LOG.info("a {}", map);` -> ERROR "method info() argument 1: incompatible types: String cannot be converted to Marker" +
"argument 2: Map<String,Object> cannot be converted to String". Same for List/Customer/int/Object args;
`LOG.info("g {} {}", s, i)`, `LOG.error("h", e)` (Exception arg) and 3-arg `LOG.debug(".. {} {} {}", a,b,c)` ->
"expects 2 arguments but 4 were provided" (varargs ignored). `sb.append(1).append("a").append(c)` ->
"append() argument 1: String cannot be converted to AbstractStringBuilder". In src/test of stripe: 64 false errors
`objectMapper.readValue(String, JavaType)` -> "argument 1: String cannot be converted to byte[]" and
"Object cannot be converted to List<ConnectedAccountResponse>" (generic `<T> T readValue`). Signature help agrees with the
bug: for `LOG.info("a {}", m)` it activates signature #5 `info(String)` instead of `info(String,Object)`.
Suspect: `find_method_check_target` (hyperion-semantics/src/type_checker/mod.rs ~:470) returns a non-applicable
"representative" overload; Object/generic/varargs applicability from bytecode-indexed signatures is wrong.

### P0-3 Diagnostics: most real compile errors are still NOT reported  [hyperion; round-1 #4 still open, tracker has no row]
Repro: /tmp/r2-edit/stripe/src/main/java/com/paystream/qa/Bugs.java and Bugs2.java. `ovim lsp diagnostics` reports none of:
call to missing method (`o.missingMethod()`, `s.foo()` on String, `.length;` without parens, `new Bugs2().nothing`);
wrong constructor arity (`new Other()` for `Other(int)`); `return "x"` in an `int` method; missing return statement;
unhandled checked exception (`Thread.sleep(10)`, `new FileReader("x")`, call to a `throws Exception` method);
private member access from another class; class not implementing an interface method (`Impl implements Greeter` lacking
`count()`); `if (s)` / `for (int k : 5)` / `Integer.parseInt(1)` type errors; `final` reassignment; duplicate variable;
static-context access (`secret++` in static method, `this` in static); `List<int>`; `new Greeter()` on interface;
import of a nonexistent third-party package (`import com.nonexistent.Foo;` + use `Foo f;` -> no error; a missing JDK class
`java.util.Lisst` IS flagged). What IS caught: undefined variable, undefined class, declaration type mismatch, generic
mismatch, wrong arg count for a known method, unused import/variable, unreachable code, syntax errors, and (correctly) stale
tests calling `getCustomer360(1 arg)` where 4 are required.
Also noisy/cascading: unresolved `Foo` reported twice + `ArrayList` unresolved gives 3 errors ("cannot find symbol",
"com.paystream.qa.ArrayList<String> cannot be converted to List<String>"); when an external edit makes `Shape` unresolved every
`shapes.add(x)` gets "add() expects 2 arguments but 1 were provided". Syntax-error position: missing `;` is reported at the next
line's first token, not at the end of the incomplete line. Defaults still noisy: "Parameter 'args' is never used" on `main`,
"Magic number" hints (30 in one test file), "Parameter 'names' is never used" on every unused param.

### P0-4 Completion: member completion after ANY call result returns nothing; `System.out.` too  [hyperion; round-1 #5 unfixed]
Repro (raw + in ovim): in a valid file put the cursor after `b.name().`, `"abc".trim().`, `names.stream().`,
`customer.getEmail().`, `Customer.builder().`, `List.of(1).`, `Optional.of("a").get().`, `new StringBuilder().append("x").`,
`Objects.requireNonNull(customer).`, `System.out.` -> 0 items (even for workspace-declared methods, so this is not only
HY-000266). In ovim: `8Gcc` `System.out.pr` -> no popup at all; `customer.getEmail().t` -> nothing.
Static access `Customer.` -> 0 items (`Customer.builder()` from Lombok not offered); `Box.` (own class) lists instance methods
and no statics. Lambda parameter `names.stream().map(x -> x.` offers `Stream` methods (should be String); inside a call
argument `names.stream().collect(Collectors.` offers `Stream` methods instead of `Collectors` statics.
Expected: IntelliJ-grade chain completion. Owner: hyperion completion (hyperion-lsp/src/completion/java_completion.rs, provider.rs).

### P0-5 Completion: type-name completion is broken for JDK classes  [hyperion]
Repro: type/probe these prefixes in a method body (stripe): `Arr` -> 13x `Array`, no ArrayList/Arrays; `Arra` -> ArrayList
missing; `Str`/`Stri` -> Strategy..., String at #20 (`Str`: absent, incl. Stream/StringBuilder); `Opt`/`Optio` -> OptBoolean...,
Optional at #22; `Coll`/`Colle`/`Collec` -> Collectors/Collections MISSING (only CollectedHeap...); `Loc` -> LocalDate MISSING;
`Hash` -> 0 items; `Sys` -> `sysout, syserr, SysexMessage, SysInfo...` with System at #12; `Instan` -> only keyword
`instanceof`; `Objec` -> no Objects. `new Arr` in `List<String> x = new Arr` shows only `Array`. Results are capped/truncated
before ranking and internal JDK/JVM classes (SystemButton, StrategyCreator, ObjArrayKlass, CollectedHeap, CollapsedStringAdapter,
Xalan/vavr classes) outrank java.lang/java.util. `ARRAYLENGTH` (a bytecode constant class) is offered first for `new ArrayL`.
A demanding user cannot autocomplete `ArrayList`, `Optional`, `Collectors` by prefix. (Round 1: `Str` gave Stratum -> unchanged.)
Suspect: candidate cap + missing "java.* first / used-in-project first" ranking (hyperion-lsp/src/completion/ranker.rs).

### P0-6 Kotlin diagnostics are mostly wrong (see section K)  [hyperion]
Clean Kotlin gets many false errors; real Kotlin type errors are not reported. Details in K1-K3.

---------------------------------------------------------------------------------------------------------------------
## P1

### P1-1 ovim: rename/refactor edits go to a stale buffer after `:e <relative path>`  [ovim, high]
Repro (deterministic): start `ovim src/main/java/com/paystream/service/CustomerService.java --headless --session c` in the
project dir. `:e src/main/java/com/paystream/controller/CustomerController.java`, then `:e src/main/java/com/paystream/service/CustomerService.java`
(relative paths), `ggO// UNSAVED EDIT<Esc>`, cursor on a local (`26G17|`), `grn`, type new name, Enter. Status says
"Renamed to 'yyy'" but the visible buffer is unchanged and nothing is on disk (edit went to another/duplicate buffer).
Variants seen in the same session lineage: with a CLEAN buffer the edit is written straight to disk and the buffer is
"File reloaded after external change" (so `u` cannot undo the rename); with a dirty buffer the file on disk was written with
edits computed for the unsaved text at shifted offsets (`.emayyy.getEmail())`, corrupted) and `:w` then fails with E211.
The identical sequence with ABSOLUTE paths (`:e /tmp/.../CustomerService.java`) or with gd/`<C-t>` navigation works correctly,
undoable, in-buffer. Suspect: `find_buffer_by_path`/`find_or_load_buffer_index_by_uri` (ovim-core/src/editor/buffer_manager.rs:350-389)
+ write-through in workspace_edits.rs:200-216 when the current buffer is not matched. Silent wrong-target edits + a false success
message is the worst combination. (Not in round 1.)

### P1-2 ovim: no signature help / parameter hints at all  [ovim]
`ovim-core/src/lsp/requests.rs:617 signature_help` exists but nothing in the editor calls it (no key, no popup). Typing
`Strings.join(` shows nothing. Hyperion's signature help works at protocol level (round-1 "works": true only server-side).

### P1-3 ovim: `<Space>S` workspace symbols unusable  [ovim + hyperion]
Opens with 100/100 rows labelled only `File.java:16:14` (no symbol names); query is empty (TODO at
ovim-core/src/editor/lsp_modules/references.rs:68-70, per Kotlin tester). `ovim lsp symbol` server side: no CamelHump (`CT`, `PIS`,
`StrTit`, `ArrL`, `HashM` -> 0), no JDK/library classes (`ArrayList`, `Coll` -> 0), `Circ` returns members (area, name, radius) ranked
among classes, `Circ*`/`Shpe` -> 0. Round-1 #13 unchanged.

### P1-4 Rename is refused far too often  [hyperion]
Class rename now works (mini: Circle->Ring updates ctor, imports, tests; compiles) - big improvement. But:
- Any string/javadoc containing the method name blocks it: `rename Shape.area` -> "Cannot rename atomically ... Shape.java contains
  a possible name-based refactoring use of 'area' in a string or documentation link" (the string is `" with area "`). Log messages
  containing a method/field word are common.
- In stripe every class/method rename is refused because ONE unrelated test uses `getMethod(..)` with a non-constant
  ("DemoFeatureTests.java contains dynamic symbol lookup through 'getMethod'"), or because a test file has an "unresolved
  refactoring candidate 'IdGenerator'" (PaymentIntentFlowTest line 40 `IdGenerator.generateMerchantId()`; file has
  `import static ...MockMvcRequestBuilders.*` / assertj wildcards).
- Package rename: mini `demo.util` -> `utils` refused: "Cannot move: reference discovery is incomplete ... App.java contains an
  unresolved refactoring candidate 'Strings'". Kotlin agent: package rename worked in the kotlin project.
Works: local, parameter, private field, static method, class, and (Kotlin agent) top-level fn/Java class from Kotlin.
IntelliJ would rename and offer to also touch strings; refusing project-wide is a daily blocker. Server errors are now shown in
ovim (fix verified). Rename prompt: Ctrl-U/Ctrl-W/Ctrl-C insert a literal `u`/`w`/`c` (ovim-core/src/editor/input/rename_input_mode.rs:8-12
only special-cases Ctrl-A); no way to clear the prefilled name except many Delete presses.

### P1-5 Extract method produces broken code; inline variable wrongly applies  [hyperion, refactor]
mini App.java: select `System.out.println(total(shapes));` (or `t += s.area();`) -> "Extract method 'extractedMethod'". Edit inserts the
new method at (line 18, char 4) = BEFORE the enclosing method's closing `}` (inside the method), params are every identifier
including `out`, `println`, `total`, `area` typed `Object`, no `static`, no return of assigned local. Result does not compile.
`Inline variable 't'` on `double t = 0;` (t is later `+=`) yields `0 += s.area();` (invalid). "Extract to variable/constant/field/parameter"
worked. Verified with a tiny WorkspaceEdit applier + javac (/tmp/r2-edit/refac.py).

### P1-6 Performance cliffs on large files / memory  [hyperion]
Synthetic single class: 4,200 lines (600 methods) -> goto-definition 5-18s per request, find-references 10-11s, semanticTokens 4.4s;
2,800 lines -> definition 2-4s; 10,500 lines -> definition timed out at 60s, semanticTokens 25s, references 23s, hover 0.02s,
edit->error diagnostic 4.5s. ovim `lsp hover` on a big file blocked 11s. Stripe's biggest real file (564 lines) is instant.
Memory: stripe (147 files, 96 jars) 1.1 GB; mini 0.5 GB; jackson (135k lines) 0.65 GB; after opening/creating ~15k lines of synthetic files
in a mini session the server sat at 3.2 GB / ~50% CPU and did NOT release memory after the files were deleted (HY-000309 only
mentions the jar case). Each library goto-definition also spawns a SECOND hyperion-lsp (`java@<hash>`, root
/tmp/hyperion-materialized-sources-v1/...) for the session (`:LspInfo`), extra ~130-480 MB.

### P1-7 Go-to-definition has no JDK fallback; typeDefinition unsupported  [hyperion]
No src.zip in this JDK 26 install: `gd` on ArrayList/Collectors/Map/Math -> "No definition found" (round-1 #13 unchanged; no
decompiler/stub view). `gy` (typeDefinition), `declaration`, `selectionRange`, `onTypeFormatting`, `linkedEditingRange` all
"Method not found". Library jumps with -sources jars work (Spring @Service/@GetMapping, ObjectMapper, LoggerFactory.getLogger,
Guava ImmutableList) but see P2-1. Navigating from a library source file to another symbol: unreliable ("No definition found").

### P1-8 `<C-o>` does not return after LSP goto-definition  [ovim]
`gd` from CustomerService.java onto `@Service` opens the library file; `<C-o>` stays; `<C-t>` returns. Vim users expect the jumplist to
contain LSP jumps. (Same `C-o` after `gd` inside the same file not verified.)

### P1-9 Quick-fix set is thin / noisy  [hyperion]
Good: "Import 'java.util.ArrayList'" first, "Add all missing imports", "Organize imports" (via `<Space>i` also ADDS the missing import
and removes unused: verified), round-1 constructor/private/duplicate junk gone. Gaps: no "Create class/method/variable" quick fix for
unresolved symbols; "Implement missing methods" not offered (only "Override N method(s)") for a class missing an abstract method; no
"Add throws / surround with try-catch" tied to diagnostics (none exist). Noise: on any position the raw action list has 9 "Surround with ..."
/ "Extract to variable" / "Introduce Constant/Field/Parameter" entries, plus "Add all missing imports" and "Organize imports" even when nothing to import.
"Create test class 'SquareTest'" writes `src/test/java/SquareTest.java` (default-package location) with `package demo.core;` - wrong directory.
Move class only offers existing packages (`demo`, `demo.app`, `demo.util`), cannot type a new package. `gra` took ~5 s to show actions in a small project.

### P1-10 Folding is a no-op in ovim; no inlay hints; hover quality  [ovim mostly]
`zc/zM/zf` do nothing (`folding_range` never called; per Kotlin tester ovim-core/src/lsp/requests.rs:899). Hyperion returns foldingRanges for Java.
inlayHint returns null for Java (no parameter-name hints). Hover for library calls: `LOG.info(..)` -> "No applicable overload for the inferred
argument types" (false, P0-2); `customers.stream().map(Customer::getEmail).collect(toList())` hovers show `Stream<Object>` /
`List<Object>` (lambda/method-ref generics not inferred); hover on a Lombok getter method-ref = bare "Identifier". Hovers end with
"*Hyperion Evaluation - Activate at hyperion-ls.com*" (nag appended to every hover popup, incl. `K` in ovim).

---------------------------------------------------------------------------------------------------------------------
## P2

1. Library-source buffers (materialized, hash file names like `.../files/65f9c7...4ee.java`) are editable, and show 46-85 false errors (second server has no classpath).
2. ovim status line stays "Requesting completions..." forever after an empty completion result.
3. Completion ranking: own members after `Object` methods (`customer.g` lists getClass first); accepted method completion inserts `getEmail` without `()`; no snippet/postfix visible in ovim; `Sys` offers `sysout`/`syserr` templates (nice) but System at #12.
4. `prepareCallHierarchy` on a call-site (`total(shapes)` inside main) returns the enclosing method, not the callee (incoming calls then empty). Incoming/outgoing UI otherwise good.
5. Code lens: only "Run" (no reference/implementation counts). Code-action popup/documentSymbol fine.
6. References returned column 0 for some test-file hits (MultiCurrencyAndFraudDetectionTest 121:1).
7. `:LspInfo` scratch buffer shows "External file change: Failed to read file: .../[LspInfo]" in the status.
8. rangeFormatting returns `[]`; Java full formatting is good (google-java-format-like, wraps stream chains, keeps text blocks).
9. Test files in stripe: 30 "Magic number" hints, `unchecked_nullable_chain` warnings - tolerable but constant.
10. Kotlin/mixed: two hyperion-lsp processes per Java+Kotlin session (see K12).

---------------------------------------------------------------------------------------------------------------------
## K. Kotlin + Java/Kotlin interop (subagent, project /tmp/r2-edit/kt/kmix incl. `billing`; verified vs `./gradlew --offline` compiling clean)

P0: K1 any method call on typed param/local is flagged "not in scope" (16 false errors on a clean file: forEach, filter, first, sumOf, isEmpty,
trim, toInt, `m.rounded()`, `c.getName()`, `sb.append()`; lazy/buildString/runCatching "unresolved reference") - `check_extension_function_scope`,
hyperion-semantics/src/kotlin_type_checker.rs:826-906 ignores receiver members + default imports. K2 more false errors: `_lines += item` on `val`
MutableList -> "val cannot be reassigned"; `copy(...)` in a data class; same-package top-level fn from another file; `is InvoiceStatus.Draft` in
`when` in another file; Java `import demo.core.SquareKt` / `MoneyKt.total(...)` "cannot find symbol" (round-1 #14, NOT fixed). K3 real Kotlin errors
mostly not reported (`val s: String = 42`, wrong return type, missing return, wrong arg count, non-exhaustive when, `List<Int> = listOf("a")`,
private set, unimported types); caught: undefined name, `String?.length`, val reassign; Java calling Kotlin unchecked. K4 Kotlin auto-import inserts
`import x.Y;` ABOVE `package` (hyperion-lsp/src/import_manager.rs:154,177-221). K5 Java sees Kotlin property names not getters (completion on `inv.`),
Kotlin sees Java getters not properties (`cust.` offers getName). K6 rename can silently break code: param rename misses `$customerId` in string
template; renaming Kotlin property `Invoice.note` misses Java `inv.getNote()`; many renames refused.
P1: K7 Kotlin completion: stdlib receivers give 0 items, `items.` offers java.util.List + vavr-like list methods, `s?.` offers Xalan members, project extensions
never offered, companion lists instance members; `items.fi`/`s.subs` no popup. K8 navigation: Java->Kotlin facade/@JvmStatic/@JvmField/getters/enum entries null;
Kotlin property access, stdlib, Java methods null; `it.total()` jumps to wrong extension; implementation of `Shape.area` returns only itself.
K9 references incomplete across languages (Money, Money.of, ZERO, isZero, total, toMoney, InvoiceStatus; `getRadius`/`getSide`/`SquareKt.allShapes` still miss).
K10 no Kotlin code actions/formatting/folding/inlay/code lens; `<Space>i` "No organize imports action". K11 (see P1-3). K12 two hyperion-lsp processes per
mixed session (java + kotlin, ~630 MB each; unsaved edits do not cross between them; only saved files via watcher). K13 a parse error in any file blocks moves; moved
Kotlin file misses imports of top-level fns/extensions. K14 stale state after `git checkout` of a Kotlin file (null definition 25s+, recovers on open/`:LspReloadProject`).
K15 ovim: cannot `:e` other file with a modified buffer (no `hidden`, no `:split FILE`); hover thin/wrong; signature help only for top-level Kotlin functions.
P2: hierarchy supertypes omit Comparable/Any, outgoing calls empty; import quick-fix duplicates; no decompiler for jars without sources; no Gradle DSL completion
in build.gradle.kts; after cross-file rename the renamed file stays unsaved `[+]` while others are written; Kotlin tree-sitter nits.

---------------------------------------------------------------------------------------------------------------------
## Works well (verified this round)

- Startup: session created -> first diagnostics ~0.6-1 s (didOpen at startup fixed, OV-00400); full Gradle jar indexing done ~5 s (stripe, 96 jars), jackson 135k lines first diagnostics 13.5 s cold; RSS 0.5-1.1 GB.
- Library indexing: Spring/Jackson/slf4j/Lombok jars indexed; `@Slf4j` `log` resolves; static-import overloads (assertEquals/assertThat) no longer false errors; per-source-set scope; hover on library classes; goto-definition into -sources jars (Spring, Jackson, slf4j, Guava) in ovim.
- Adding a dependency to build.gradle.kts is picked up automatically within seconds (guava ImmutableList resolved without reload); `:LspReloadProject` reports done. `[[lsp_settings]]` + `hyperion.buildToolClasspath` works (Kotlin agent).
- Crash recovery: `kill -9` of hyperion-lsp -> ovim shows "restarting java...", ready again in ~2 s, diagnostics/hover work, `:LspInfo` shows `restarts: 1`; `:LspRestart` works (also restarts the materialized-sources server).
- External changes: file edited/reverted via shell or `git checkout -- .` updates diagnostics of open files in ~1 s; new/deleted files handled (didChangeWatchedFiles).
- Refactors that work: class rename with imports/tests (compiles), local/param/private-field/static-method rename, Move class to an existing package (CreateFile/RenameFile ok, compiles), Extract interface (own file, compiles), Extract variable, generate getters/setters, Lombok-aware (no generate offered on @Data), safe delete command, organize imports (adds + removes), Import quick fix (correct java.util candidate first) and add-all-missing-imports.
- Navigation: references complete and fast in normal-size files (7/7 for a static method incl. tests), goto-implementation on types and on interface methods, type hierarchy (sub/super), call hierarchy incoming through overridden declarations and Tab drill-down in ovim, resource-op WorkspaceEdits no longer crash ovim.
- Java formatting (full document) is good; unused import/var/param + unreachable-code lints; stale-test detection (`getCustomer360` arity) is correct.
- Stability: 150 edits + 150 undos + 150 redos in ~7 s, buffer byte-identical, diagnostics consistent; server errors shown in ovim (rename refusal message now visible).

## Docs that overclaim / wrong
- Hyperion README rename/Lombok/hierarchy/code-action rows: rename is still refused whenever any string/javadoc mentions the name, any test uses reflection, or a candidate is "unresolved" (P1-4); code actions/formatting/organize-imports/folding are Java-only.
- ovim user-docs: workspace symbols/`<Space>S` and signature help are advertised implicitly by LSP support but are unusable/absent in the editor (P1-2, P1-3); Kotlin support rows in LANGUAGE_SUPPORT.md (grammar only is true).
- headless.md `ovim lsp hover FILE:LINE:COL` note is accurate; `ovim lsp status` intermittently says "authorization required" from some cwd (Kotlin agent).
- ISSUE_TRACKER: HY-000320..329 "Done" are true for Java-only simple cases; HY-000322 (external change) only partially for Kotlin; no tracker row exists for missing-compile-error checks (P0-3) or for the false-positive floods (P0-1/P0-2).

## Cleanup
All `r2ed-*` sessions and my hyperion-lsp processes were killed (`ovim session list` empty at end; the running `cargo build` belongs to someone else). Scratch left in /tmp/r2-edit (no repo edits, no commits, no branches).
