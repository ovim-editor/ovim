# Java LSP Performance Testing Guide

## Real-World Test Scenarios

### Test 1: Hello World (Baseline)

```bash
# Create simple standalone file
mkdir -p ~/ovim-test/simple
cat > ~/ovim-test/simple/HelloWorld.java << 'EOF'
public class HelloWorld {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}
EOF

cd ~/ovim-test/simple

# Measure first-time startup (with download)
rm -rf ~/.cache/ovim/java
time cargo run --release -- HelloWorld.java

# Measure subsequent startup (cached)
time cargo run --release -- HelloWorld.java
```

**What to test:**
- [ ] Opens without errors
- [ ] Syntax highlighting appears immediately
- [ ] Status line shows progress updates
- [ ] UI stays responsive during init
- [ ] Press `K` on "System" → hover info appears
- [ ] Type `gd` on "println" → jumps to definition
- [ ] Press `i`, type "System." → completions appear
- [ ] Can type/move during initialization

**Expected times:**
- First open: ~90 seconds (download + init)
- Subsequent: ~60 seconds (jdtls init only)
- LSP ready: Within 2 minutes total

---

### Test 2: Gradle Project (Build Tool Integration)

```bash
# Create Gradle project
mkdir -p ~/ovim-test/gradle-app
cd ~/ovim-test/gradle-app
gradle init --type java-application --dsl groovy --test-framework junit --project-name myapp --package com.example

# Open the generated app
cargo run --release -- app/src/main/java/com/example/App.java
```

**What to test:**
- [ ] Detects Java version from build.gradle
- [ ] Status shows: "Java: Detected Java XX project"
- [ ] Hover on `import` statements shows documentation
- [ ] Completion includes JUnit classes
- [ ] `gd` on JUnit annotations jumps to definition
- [ ] Diagnostics show if there are errors
- [ ] Format command works (if exposed)

**Check in status line:**
```
Java: Detecting project configuration...
Java: Detected Java 17 project
Java: Downloading jdtls... (if first time)
Java: Starting LSP server...
Java: Ready ✓
```

---

### Test 3: Spring Boot Project (Real-World Complexity)

```bash
# Clone Spring Boot sample
cd ~/ovim-test
git clone https://github.com/spring-guides/gs-spring-boot
cd gs-spring-boot/complete

# Open main application class
cargo run --release -- src/main/java/com/example/springboot/Application.java
```

**What to test:**
- [ ] Handles multi-module project structure
- [ ] Detects Java version from Maven pom.xml
- [ ] Hover on `@SpringBootApplication` shows info
- [ ] `gd` on `SpringApplication.run` jumps to Spring class
- [ ] Completion shows Spring Boot classes
- [ ] Import statements are recognized
- [ ] Diagnostics appear if project has errors

**Advanced LSP features:**
- [ ] `gd` on controller method navigates correctly
- [ ] Hover on autowired fields shows bean info
- [ ] Completion includes Spring annotations
- [ ] Can navigate between files in project

---

### Test 4: Multi-File Navigation

```bash
# Create project with multiple related classes
mkdir -p ~/ovim-test/multifile/src/com/example
cd ~/ovim-test/multifile

# Create interface
cat > src/com/example/UserService.java << 'EOF'
package com.example;

public interface UserService {
    String getUserName(int id);
}
EOF

# Create implementation
cat > src/com/example/UserServiceImpl.java << 'EOF'
package com.example;

public class UserServiceImpl implements UserService {
    @Override
    public String getUserName(int id) {
        return "User " + id;
    }
}
EOF

# Create main class that uses the service
cat > src/com/example/Main.java << 'EOF'
package com.example;

public class Main {
    public static void main(String[] args) {
        UserService service = new UserServiceImpl();
        System.out.println(service.getUserName(42));
    }
}
EOF

# Open main class
cargo run --release -- src/com/example/Main.java
```

**What to test:**
- [ ] Hover on `UserService` shows interface definition
- [ ] `gd` on `UserService` jumps to interface file (if supported)
- [ ] `gd` on `UserServiceImpl` jumps to impl file (if supported)
- [ ] Completion on `service.` shows `getUserName`
- [ ] No errors shown for correct imports

**Note:** Multi-file navigation requires buffer management, which may not be implemented yet.

---

### Test 5: Performance Benchmarks

```bash
# Create benchmark script
cat > ~/ovim-test/benchmark.sh << 'EOF'
#!/bin/bash

echo "=== ovim Java LSP Performance Benchmark ==="
echo ""

# Test file
TEST_FILE="$HOME/ovim-test/simple/HelloWorld.java"

# Test 1: Cold start (no cache)
echo "Test 1: Cold start (with download)"
rm -rf ~/.cache/ovim/java
/usr/bin/time -f "Time: %E\nMemory: %M KB" \
  timeout 180 cargo run --release -- "$TEST_FILE" < <(sleep 2 && echo ":q")
echo ""

# Test 2: Warm start (cached jdtls)
echo "Test 2: Warm start (cached jdtls)"
/usr/bin/time -f "Time: %E\nMemory: %M KB" \
  timeout 120 cargo run --release -- "$TEST_FILE" < <(sleep 2 && echo ":q")
echo ""

# Test 3: Memory usage after initialization
echo "Test 3: Memory usage check"
cargo run --release -- "$TEST_FILE" &
OVIM_PID=$!
sleep 90  # Wait for LSP to initialize
ps aux | grep -E "PID|$OVIM_PID|jdtls" | grep -v grep
kill $OVIM_PID 2>/dev/null
echo ""

echo "=== Benchmark Complete ==="
EOF

chmod +x ~/ovim-test/benchmark.sh
~/ovim-test/benchmark.sh
```

**Metrics to collect:**
- Startup time (cold/warm)
- Memory usage (ovim + jdtls)
- Time to first LSP response
- CPU usage during init
- Responsiveness during init

---

### Test 6: Comparison with Other Editors

```bash
# Compare with nvim + coc.nvim
cd ~/ovim-test/simple
time nvim +":sleep 60" +":q" HelloWorld.java

# Compare with VSCode (if available)
time code --wait HelloWorld.java

# Compare with ovim
time cargo run --release -- HelloWorld.java < <(sleep 60 && echo ":q")
```

**What to compare:**
- Startup time
- Memory usage
- Responsiveness
- LSP feature parity

---

### Test 7: Stress Test - Large File

```bash
# Generate large Java file
mkdir -p ~/ovim-test/large
cat > ~/ovim-test/large/LargeClass.java << 'EOF'
public class LargeClass {
EOF

# Add 1000 methods
for i in {1..1000}; do
  cat >> ~/ovim-test/large/LargeClass.java << EOF
    public int method$i(int x) {
        return x + $i;
    }
EOF
done

echo "}" >> ~/ovim-test/large/LargeClass.java

# Open large file
cargo run --release -- ~/ovim-test/large/LargeClass.java
```

**What to test:**
- [ ] Opens without crashing
- [ ] Scrolling is smooth
- [ ] Syntax highlighting works
- [ ] Can edit without lag
- [ ] LSP features still work (hover, completion)
- [ ] Memory usage stays reasonable

---

### Test 8: Error Handling

```bash
# Create file with errors
mkdir -p ~/ovim-test/errors
cat > ~/ovim-test/errors/BuggyCode.java << 'EOF'
public class BuggyCode {
    public static void main(String[] args) {
        String x = 123;  // Type error
        System.out.println(undefinedVariable);  // Undefined variable
        missingMethod();  // Missing method
    }
}
EOF

cargo run --release -- ~/ovim-test/errors/BuggyCode.java
```

**What to test:**
- [ ] Diagnostics appear in status line
- [ ] Error count shown
- [ ] Hover on errors shows diagnostic message
- [ ] Can still edit despite errors
- [ ] No crashes

---

### Test 9: Real Project - Spring PetClinic

```bash
# Clone a real-world Spring Boot application
cd ~/ovim-test
git clone https://github.com/spring-projects/spring-petclinic
cd spring-petclinic

# Open a controller
cargo run --release -- src/main/java/org/springframework/samples/petclinic/owner/OwnerController.java
```

**What to test:**
- [ ] Handles large project (50+ classes)
- [ ] Detects Maven project structure
- [ ] LSP understands dependencies
- [ ] Hover on Spring annotations works
- [ ] Completion includes Spring classes
- [ ] Can navigate to other project classes
- [ ] Performance remains acceptable

**This is the ultimate real-world test!**

---

### Test 10: Workflow Simulation

```bash
# Simulate a real development workflow
cd ~/ovim-test/gradle-app/app/src/main/java/com/example

# Open App.java and perform common tasks
cargo run --release -- App.java
```

**Manual workflow test:**

1. **Open file** - Check startup time
2. **Wait for LSP** - Note how long until "Ready ✓"
3. **Navigate** - Press `K` on a symbol, see hover info
4. **Jump to definition** - `gd` on imported class
5. **Get completion** - Type `System.` and wait for suggestions
6. **Edit code** - Add a new method
7. **See diagnostics** - Introduce an error, see if it's caught
8. **Format** - Use format command (if available)
9. **Save** - `:w` and verify file saved

**Time each step and note any freezing or lag**

---

## Success Criteria

### Must Work
- ✅ UI never freezes (even during init)
- ✅ Basic LSP features (hover, goto-def, completion)
- ✅ Syntax highlighting
- ✅ Can edit while LSP initializes
- ✅ Error-free operation

### Should Work
- 🎯 Goto-definition across project files
- 🎯 Completion includes project dependencies
- 🎯 Diagnostics show compilation errors
- 🎯 Reasonable performance (< 2 min to LSP ready)
- 🎯 Memory usage < 1GB total (ovim + jdtls)

### Nice to Have
- 💡 Sub-60s initialization
- 💡 Organize imports
- 💡 Rename refactoring
- 💡 Code actions
- 💡 Multi-file editing

---

## Known Limitations (Document These)

1. **Single file editing** - Can't switch between files yet
2. **No buffer management** - One file at a time
3. **Limited LSP features exposed** - Have hover/goto-def/completion, but no rename/organize imports
4. **No build integration** - Can't run gradle/maven from editor
5. **No test runner** - Manual test execution
6. **First-time download slow** - 90s for jdtls download (one-time)
7. **Initialization time** - 60-120s for jdtls to be ready

---

## Recommended Next Features

Based on testing results, prioritize:

### P0 - Critical for Real Use
1. Buffer/file management (switch between files)
2. Project-aware file opener (fuzzy finder for project files)
3. Expose more LSP features (organize imports, rename)

### P1 - Major Quality of Life
1. Faster initialization (daemon mode for jdtls?)
2. Better diagnostics display (quickfix list)
3. Multi-file refactoring support

### P2 - Nice to Have
1. Build integration (:Gradle test)
2. Test runner
3. Git integration
4. Project tree view

---

## Reporting Template

After testing, document:

```markdown
# ovim Java LSP Test Results

## Environment
- OS: [Linux/Mac/Windows]
- Java version: [17/21/etc]
- CPU: [specs]
- RAM: [amount]

## Performance Metrics
- Cold start: [time]
- Warm start: [time]
- Time to LSP ready: [time]
- Memory usage (ovim): [MB]
- Memory usage (jdtls): [MB]
- Total memory: [MB]

## Feature Test Results
- [✅/❌] Hover (K)
- [✅/❌] Goto definition (gd)
- [✅/❌] Completion (Ctrl-Space)
- [✅/❌] Diagnostics
- [✅/❌] Format
- [✅/❌] Multi-file navigation
- [✅/❌] Gradle project support
- [✅/❌] Maven project support
- [✅/❌] Spring Boot project support

## Issues Found
1. [Description]
2. [Description]

## Comparison to Alternatives
- vs nvim: [faster/slower/similar]
- vs VSCode: [faster/slower/similar]
- vs IntelliJ: [faster/slower/similar]

## Recommendation
[Would you recommend ovim for Java development? Why/why not?]
```
