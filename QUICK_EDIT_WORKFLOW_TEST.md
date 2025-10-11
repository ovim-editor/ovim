# Quick Edit Workflow - Performance Testing

## User Profile: Terminal-Native Neovim Enthusiast

**Workflow:**
- Works entirely in terminal
- Opens editor, makes quick fix, closes
- Repeats many times per day
- Doesn't want to configure LSP plugins
- Expects vim keybindings + LSP features to "just work"

**Critical Metric: Time from `ovim File.java` to first keystroke**

---

## The Problem with Current Implementation

```bash
$ cd ~/my-spring-project
$ ovim src/main/Controller.java
[Wait 60-120 seconds for jdtls...]  # 😱 UNACCEPTABLE
... make 10-second edit ...
:wq

$ ovim src/main/Service.java
[Wait ANOTHER 60-120 seconds!]      # 😱😱😱 TERRIBLE
... make 5-second edit ...
:wq
```

**For quick edits, spending 120 seconds waiting vs 10 seconds editing is a dealbreaker.**

---

## Test 1: Rapid Open/Close Cycle

```bash
cd ~/ovim-test/gradle-app

# First open (expected to be slow)
echo "=== Open 1: Controller.java ==="
time cargo run --release -- app/src/main/java/com/example/Controller.java
# Type :q immediately

# Second open - SAME PROJECT (should this be faster?)
echo "=== Open 2: Service.java ==="
time cargo run --release -- app/src/main/java/com/example/Service.java
# Type :q immediately

# Third open
echo "=== Open 3: Repository.java ==="
time cargo run --release -- app/src/main/java/com/example/Repository.java
# Type :q immediately

# Check for zombie processes
ps aux | grep jdtls
```

**Questions:**
- ⚠️ Does jdtls stay running after :q?
- ⚠️ Do subsequent opens reuse the jdtls instance?
- ⚠️ How long does each open take?

**Ideal behavior:**
- 1st open: 60s (jdtls init) - acceptable
- 2nd open: <5s (reuse jdtls) - ✅ GOOD
- 3rd open: <5s (reuse jdtls) - ✅ GOOD

**Current behavior:** Likely 60s every time ❌

---

## Test 2: Edit Before LSP Ready

```bash
ovim HelloWorld.java
# Immediately start typing - don't wait for LSP!
```

**Can I:**
- [ ] Start typing immediately (before "Java: Ready ✓")?
- [ ] See characters appear without lag?
- [ ] Get basic syntax highlighting (without LSP)?
- [ ] Save file before LSP is ready?
- [ ] Use basic vim motions (hjkl, w, b, etc.)?

**This tests if ovim is usable for quick edits without waiting.**

---

## Test 3: LSP Daemon Pattern (Feature Request)

**Ideal workflow:**
```bash
# Start LSP daemon once per project (or globally)
$ ovim-daemon start ~/my-spring-project

# Now all opens in that project are instant
$ ovim src/Controller.java  # <1 second to start editing
$ ovim src/Service.java     # <1 second
$ ovim src/Repository.java  # <1 second

# When done for the day
$ ovim-daemon stop
```

**Does ovim support this?** Probably not yet.

**Alternative: Background server mode?**
```bash
$ ovim --server start  # Keep jdtls alive
$ ovim --remote Controller.java  # Connect to existing instance
```

---

## Test 4: Instant Start + Background LSP

**Ideal behavior:**
```bash
$ ovim Controller.java
# Editor opens INSTANTLY (<1 second)
# Can start typing immediately
# Syntax highlighting works (from tree-sitter, not LSP)

# Status line shows:
# "Java: Initializing LSP..." (in background)

# After 60 seconds:
# "Java: Ready ✓"
# Now K, gd, completion work
```

**Test this:**
```bash
time ovim Controller.java < <(echo "i" && sleep 0.5 && echo "Hello" && sleep 0.5 && echo -e "\x1b:q\n")
```

**Measure:**
- Time until "i" is processed (instant?)
- Time until "Hello" appears (instant?)
- Total time to :q

---

## Test 5: Compare with Pure Vim (Baseline)

```bash
# How fast is vim without LSP?
time vim Controller.java -c "qa"

# How fast is nvim without LSP?
time nvim Controller.java -c "qa"

# How fast is ovim without waiting for LSP?
time ovim Controller.java < <(echo ":q")
```

**Acceptable startup time for vim-like editor: <1 second**

---

## Test 6: Project Context Awareness

```bash
cd ~/my-spring-project

# Open file from project root
ovim src/main/Controller.java

# Open another file - does it know it's the same project?
ovim src/test/ControllerTest.java

# Open file from different project
cd ~/different-project
ovim src/main/App.java  # Should this start new jdtls?
```

**Questions:**
- How does ovim detect project boundaries?
- Does it share jdtls for same project?
- Does it isolate different projects?

---

## Test 7: Resource Cleanup

```bash
# Open and close repeatedly
for i in {1..5}; do
    ovim File$i.java < <(sleep 1 && echo ":q")
done

# Check for process leaks
ps aux | grep -E "ovim|jdtls" | wc -l

# Check for port leaks
lsof -i | grep -E "ovim|jdtls"

# Check memory usage
ps aux | grep -E "ovim|jdtls" | awk '{sum+=$6} END {print sum " KB"}'
```

**Should see:**
- No zombie processes
- No leaked ports
- No memory leaks

---

## Test 8: The "Real Day" Simulation

Simulate a real day of development:

```bash
#!/bin/bash
# Simulate 20 quick edits across 5 files

cd ~/my-spring-project
FILES=(
    "src/main/Controller.java"
    "src/main/Service.java"
    "src/main/Repository.java"
    "src/test/ControllerTest.java"
    "src/test/ServiceTest.java"
)

echo "=== Simulating 20 quick edits ==="
START_TIME=$(date +%s)

for i in {1..20}; do
    FILE=${FILES[$RANDOM % ${#FILES[@]}]}
    echo "Edit $i: $FILE"

    # Open, wait 2 seconds (simulating quick edit), close
    timeout 90 ovim "$FILE" < <(sleep 2 && echo ":q") 2>&1 | grep -E "Java:|Error" &

    sleep 3  # Wait for editor to fully close
done

END_TIME=$(date +%s)
TOTAL_TIME=$((END_TIME - START_TIME))

echo "=== Total time for 20 edits: ${TOTAL_TIME}s ==="
echo "=== Average per edit: $((TOTAL_TIME / 20))s ==="
```

**Acceptable:** <10s average per edit
**Good:** <5s average per edit
**Excellent:** <2s average per edit

**Current (probably):** 60-90s per edit ❌

---

## Test 9: Comparison with Configured Neovim

```bash
# How long does a Neovim user wait with LSP?
# (Assuming they have nvim-lspconfig set up)

cd ~/my-spring-project

# Neovim with LSP
time nvim src/main/Controller.java < <(sleep 60 && echo ":q")

# ovim
time ovim src/main/Controller.java < <(sleep 60 && echo ":q")
```

**Key question:** Is ovim faster/slower than configured nvim?

**ovim's value proposition:**
- Zero config (just works)
- Auto-downloads jdtls
- Auto-detects Java version
- Same speed or faster than manually configured nvim

---

## Test 10: The Ultimate Test - Would You Use It?

**Scenario:**
```bash
# You have a bug to fix
# You found the file in your IDE
# You could either:
# A) Fix it in IntelliJ (already open, instant LSP)
# B) Exit to terminal and use ovim

# With current ovim (60s wait):
time ovim src/main/Controller.java  # 60 seconds
# ... make 10 second edit ...
:wq

# Total time: 70 seconds
# vs just clicking in IntelliJ: 10 seconds

# Would you use ovim? NO ❌
```

**With improved ovim (instant start, background LSP):**
```bash
time ovim src/main/Controller.java  # 0.5 seconds
# ... make 10 second edit with vim bindings ...
:wq
# (LSP is ready in background, used K to check type)

# Total time: 11 seconds
# vs clicking and mousing in IntelliJ: 15 seconds

# Would you use ovim? YES ✅
```

---

## Critical Features Needed for This Workflow

### P0 - Must Have (or ovim is unusable)
1. **Instant editor startup** (<1s to first keystroke)
   - Syntax highlighting without LSP (tree-sitter only)
   - Basic editing works immediately
   - LSP initializes in background

2. **LSP reuse between sessions**
   - First open: start jdtls (slow, acceptable)
   - Subsequent opens in same project: reuse jdtls (fast, <1s)
   - Daemon mode or background server

3. **Clean resource management**
   - No zombie processes
   - No memory leaks
   - Proper cleanup on exit

### P1 - Should Have (nice but not critical)
1. **Quick project detection**
   - Know which files belong to same project
   - Share LSP instance intelligently

2. **Visual feedback for LSP state**
   - "LSP initializing..." (can still edit)
   - "LSP ready ✓" (features available)
   - Don't block waiting

3. **Buffer management** (maybe)
   - Might want to open multiple files
   - But for quick edits, maybe not needed

---

## Success Criteria for "Quick Edit" User

**Must work:**
- ✅ Editor starts in <1 second
- ✅ Can type immediately (before LSP ready)
- ✅ Basic syntax highlighting works instantly
- ✅ Second file in same project opens fast (<5s)
- ✅ No process/memory leaks
- ✅ Vim keybindings work perfectly

**Should work:**
- 🎯 LSP features available within 60s of first open
- 🎯 Hover (K), goto-def (gd), completion work when LSP ready
- 🎯 Clear status showing LSP state
- 🎯 Can :q before LSP is ready (no hang)

**Would be amazing:**
- 💡 Daemon mode (start once, use all day)
- 💡 LSP persists between editor sessions
- 💡 Sub-5s startup for all opens after first
- 💡 Smart project detection

---

## Current Status Assessment

Based on what we've built:

**✅ What works well:**
- UI never freezes (can type during LSP init)
- Async/non-blocking everything
- Auto-downloads jdtls
- Auto-detects Java version
- Progress updates in status line

**❌ What doesn't work for quick edits:**
- Probably starts jdtls on every open (60s each time)
- No LSP daemon/reuse mode
- Might not be usable before LSP ready
- Unknown if resources cleanup properly

**🔍 What needs testing:**
- Can I actually type before "Java: Ready ✓"?
- Does jdtls persist between ovim sessions?
- How long does 2nd/3rd file open take?
- Are there zombie processes?

---

## Recommended Next Steps

1. **Test the rapid open/close cycle** (Test 1)
   - This will reveal if current implementation is usable

2. **Test instant start without LSP** (Test 2)
   - Can I edit immediately?

3. **Check resource cleanup** (Test 7)
   - Zombie processes?

4. **If tests show problems, implement:**
   - Daemon mode for jdtls (reuse across sessions)
   - Instant editor start (don't wait for LSP)
   - Proper jdtls lifecycle management

5. **Make it usable for quick edits:**
   - Target: <1s start, <5s for subsequent opens
   - Clear LSP status feedback
   - Don't block user's workflow

---

## Comparison with Alternatives

**For "quick edit" workflow:**

| Editor | Startup | LSP Ready | After First | Config |
|--------|---------|-----------|-------------|--------|
| vim | 0.1s | N/A | 0.1s | None |
| nvim (no LSP) | 0.2s | N/A | 0.2s | None |
| nvim (with LSP) | 0.5s | 60s | 60s ❓ | Complex |
| IntelliJ IDEA | 10s | Instant | 0s | None |
| VSCode | 3s | 30s | 30s ❓ | Some |
| **ovim (current)** | ???s | 60-120s | 60-120s ❓ | **None** |
| **ovim (ideal)** | <1s | 60s | <5s ✅ | **None** |

**ovim's niche:**
- Zero config (like IntelliJ) ✅
- Vim keybindings (unlike IntelliJ) ✅
- Fast startup (like vim) - need to achieve ⚠️
- LSP features (like VSCode) ✅
- Reuse LSP (need to implement) ⚠️

---

## Bottom Line

**For the quick edit workflow to work, ovim MUST:**
1. Start editing in <1 second
2. Reuse jdtls across sessions (after first init)
3. Let user work while LSP initializes in background

**Without these, the user will just use:**
- Plain vim (for truly quick edits, no LSP)
- IntelliJ (if they need LSP features)

**With these, ovim becomes the perfect tool:**
- "I want vim but with LSP that just works"
- "I don't want to configure coc.nvim or nvim-lspconfig"
- "I want quick edits with IDE features"
