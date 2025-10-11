# Java LSP Status Line Updates - Final Fix

## Problem

Java LSP initialization was logging to **stderr/terminal** instead of the **status line**:

```
[Java] Detected Java 17 project
[Java] Downloading jdtls...
[Java] JVM found, launching jdtls...
```

This cluttered the terminal and made it hard to see progress in the editor.

## Solution ✅

Created a **channel-based status update system** that displays all Java LSP progress on the status line.

### Implementation

#### 1. Global Status Channel

```rust
// In src/main.rs
use std::sync::OnceLock;

static JAVA_STATUS_SENDER: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();
```

#### 2. Helper Function

```rust
fn send_java_status(msg: String) {
    if let Some(tx) = JAVA_STATUS_SENDER.get() {
        let _ = tx.send(format!("Java: {}", msg));
    }
}
```

#### 3. Initialize Channel in main()

```rust
// Create channel for Java LSP status updates
let (java_status_tx, java_status_rx) = mpsc::unbounded_channel();

// Store the sender in a static for background tasks to use
JAVA_STATUS_SENDER.set(java_status_tx).ok();

// Pass receiver to event loop
run_event_loop(&mut ui, &mut editor, api_rx, java_status_rx).await?;
```

#### 4. Poll Channel in Event Loop

```rust
async fn run_event_loop(
    ui: &mut UI,
    editor: &mut Editor,
    mut api_rx: Option<mpsc::UnboundedReceiver<ApiRequest>>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    while !editor.should_quit() {
        // Check for Java LSP status updates
        while let Ok(status) = java_status_rx.try_recv() {
            editor.set_lsp_status(status);
        }

        // ... rest of event loop
    }
}
```

#### 5. Use in Background Task

```rust
async fn initialize_java_lsp_background(...) {
    send_java_status("Detecting project configuration...".to_string());
    // ... detect Java version
    send_java_status(format!("Detected Java {} project", version));

    send_java_status("Downloading jdtls... (first time setup)".to_string());
    // ... download
    send_java_status("Download complete!".to_string());

    send_java_status("Finding JVM...".to_string());
    // ... find JVM
    send_java_status("JVM found, launching jdtls...".to_string());

    send_java_status("Starting LSP server...".to_string());
    // ... start server
    send_java_status("Ready ✓".to_string());
}
```

## Status Line Flow

Now when you open a Java file, the **status line** shows:

```
Java: Detecting project configuration...
Java: Detected Java 17 project
Java: Downloading jdtls... (first time setup)
Java: Downloading jdtls...
Java: Attempt 1/3: https://download.eclipse.org/...
Java: Downloaded 98326674 bytes
Java: jdtls installed successfully!
Java: Download complete!
Java: Configuring launcher...
Java: Finding JVM...
Java: JVM found, launching jdtls...
Java: Starting LSP server...
Java: Initializing LSP connection...
Java: Opening file...
Java: Ready ✓
```

All visible in the editor's status line at the bottom!

## Benefits

### Before (stderr logging)
- ❌ Clutters terminal
- ❌ Mixed with other logs
- ❌ Not visible in TUI
- ❌ Confusing for users

```
 NORMAL  [No Name]                                                         1:1
[Java] Detected Java 17 project
                               [Java] Downloading jdtls...
import org.apache.commons.lang3.StringUtils;
                                                [Java] JVM found...
public class UrlTools {
```

### After (status line)
- ✅ Clean editor display
- ✅ Clear progress tracking
- ✅ Always visible in status bar
- ✅ Professional UX

```
 NORMAL  UrlTools.java                     Java: JVM found, launching jdtls...  1:1

import org.apache.commons.lang3.StringUtils;

public class UrlTools {
    public static String makeRedirUrl(String redirCode) {
        if (SpondEnvironment.isRunningOnProduction()) {
```

## Architecture

### Message Flow

```
Background Task                Channel              Event Loop           Editor
──────────────────────────────────────────────────────────────────────────────
send_java_status()
    ↓
JAVA_STATUS_SENDER.send()  →  java_status_rx
                                    ↓
                              try_recv()  →  editor.set_lsp_status()
                                                    ↓
                                                Status Line UI
```

### Non-Blocking Design

```
User opens Java file
    ↓
tokio::spawn(initialize_java_lsp_background())
    ↓                                   ↓
    ↓                              [Background]
    ↓                              send_java_status("Detecting...")
    ↓                                   ↓
Editor immediately responsive     [Background]
    ↓                              send_java_status("Downloading...")
User can edit file                     ↓
    ↓                              [Background]
Event loop polls channel          send_java_status("Starting...")
    ↓                                   ↓
Updates status line               [Background]
    ↓                              send_java_status("Ready ✓")
Never blocks! ✅
```

## Changes Summary

**Files Modified:**

1. **src/main.rs**
   - Added `JAVA_STATUS_SENDER` static channel
   - Added `send_java_status()` helper function
   - Updated `run_event_loop()` to poll status updates
   - Replaced all `eprintln!()` with `send_java_status()` in background task
   - Created channel in main() and passed to event loop

**Lines Changed:** ~30 lines

**Complexity:** Low - simple channel-based communication

## Testing

### Test 1: First-Time Download

```bash
rm -rf ~/.cache/ovim/java/jdtls
cargo run -- TestJava.java
```

**Expected status line progression:**
```
Java: Detecting project configuration...
Java: Detected Java 17 project
Java: Downloading jdtls... (first time setup)
Java: Downloading jdtls...
Java: Attempt 1/3: https://...
Java: Downloaded 98326674 bytes
Java: jdtls installed successfully!
Java: Download complete!
Java: Configuring launcher...
Java: Finding JVM...
Java: JVM found, launching jdtls...
Java: Starting LSP server...
Java: Initializing LSP connection...
Java: Opening file...
Java: Ready ✓
```

**Terminal:** Clean, no Java logs

### Test 2: Cached JDTLS

```bash
cargo run -- TestJava.java
```

**Expected status line progression:**
```
Java: Detecting project configuration...
Java: Detected Java 17 project
Java: Using cached jdtls
Java: Configuring launcher...
Java: Finding JVM...
Java: JVM found, launching jdtls...
Java: Starting LSP server...
Java: Initializing LSP connection...
Java: Opening file...
Java: Ready ✓
```

**Time:** 2-5 seconds (much faster)

### Test 3: Error Scenarios

**Missing Java:**
```
Java: Failed to find JVM: Could not find Java 17 or higher. Please install Java and set JAVA_HOME.
```

**Download Failed:**
```
Java: Download failed: Failed to download jdtls from all sources
```

All errors visible on status line!

## Comparison to Other Editors

### IntelliJ IDEA
- Shows "Indexing..." in status bar ✅
- Shows percentage progress
- Non-blocking UI ✅

### VS Code
- Shows "Java Language Server: Starting" in status bar ✅
- Shows progress notifications
- Non-blocking UI ✅

### ovim (Before)
- Logged to stderr ❌
- Terminal clutter ❌
- No visible progress in editor ❌

### ovim (After)
- Shows progress in status bar ✅
- Clean terminal ✅
- Non-blocking UI ✅
- **Same UX as IntelliJ/VS Code!** ✅

## Future Enhancements

### Progress Bars
```
Java: Downloading jdtls... [=========>         ] 45% (45MB/98MB)
```

### Spinner Animation
```
Java: Starting LSP server... ⣾
```

### Time Estimates
```
Java: Downloading jdtls... (~15s remaining)
```

### Cancellation
```
Java: Downloading... (Press Esc to cancel)
```

## Summary

**Problem:** Java LSP logging cluttered terminal
**Solution:** Channel-based status line updates
**Result:** Clean, professional UX like IntelliJ/VS Code

**Benefits:**
- ✅ All progress visible in status line
- ✅ Clean terminal output
- ✅ Non-blocking UI
- ✅ Professional user experience
- ✅ Easy to debug (all messages in one place)

**Files Changed:** 1 file (src/main.rs)
**Lines Changed:** ~30 lines
**Complexity:** Low
**Risk:** Minimal

---

**Status:** ✅ COMPLETE
**Date:** 2025-10-07
**Version:** ovim 0.1.0
