# Phase 3: TypeScript Auto-Install - Manual Testing Guide

**Purpose**: Verify TypeScript LSP auto-install works correctly in real-world scenarios
**Date**: 2026-01-04
**Tester**: _______
**Platform**: macOS / Linux / Windows (circle one)

---

## Prerequisites

Before starting tests, ensure:
- [ ] ovim is built: `cargo build --release`
- [ ] Test files exist (create if needed):
  ```bash
  echo "const x: number = 42;" > test.ts
  echo "function hello() { return 'world'; }" > test.js
  ```

---

## Test 1: Happy Path (Auto-Install Works)

**Goal**: Verify auto-install works when npm is available and LSP is not installed

### Setup
```bash
# Remove typescript-language-server if installed
npm uninstall -g typescript-language-server

# Verify it's gone
which typescript-language-server
# Should output nothing
```

### Test Steps
1. Open TypeScript file:
   ```bash
   ./target/release/ovim test.ts
   ```

2. Observe status line (bottom of screen)

### Expected Behavior
- **Initial**: Status shows "LSP: Installing typescript-language-server..."
- **30 seconds later**: Status shows "LSP: typescript-language-server installed successfully!"
- **Shortly after**: LSP status shows "LSP: TypeScript ready"

### Verification
```bash
# Check if typescript-language-server was installed
which typescript-language-server
# Should output path (e.g., /usr/local/bin/typescript-language-server or ~/.npm-global/bin/...)

# Try LSP features in ovim
# 1. Open test.ts again
# 2. Press K (hover) on 'number' keyword
# Expected: Hover window shows type information
```

### Results
- [ ] Status showed "Installing..." message
- [ ] Installation completed within 60 seconds
- [ ] Status showed "installed successfully!" message
- [ ] LSP started and shows "ready" status
- [ ] Hover (K) works and shows type info
- [ ] No errors in stderr

**Notes**:
```
_____________________________________________________________________
_____________________________________________________________________
```

---

## Test 2: Already Installed (No Auto-Install)

**Goal**: Verify auto-install is skipped when LSP is already available

### Setup
```bash
# Ensure typescript-language-server is installed
npm install -g typescript-language-server

# Verify it's available
which typescript-language-server
# Should output path
```

### Test Steps
1. Open TypeScript file:
   ```bash
   ./target/release/ovim test.ts
   ```

2. Observe status line

### Expected Behavior
- **No** "Installing..." message (should skip straight to "ready")
- **Within 2 seconds**: Status shows "LSP: TypeScript ready"

### Verification
```bash
# LSP should work immediately
# Press K on a keyword -> hover should appear
```

### Results
- [ ] No installation message
- [ ] LSP started within 2 seconds
- [ ] Hover (K) works
- [ ] No errors

**Notes**:
```
_____________________________________________________________________
```

---

## Test 3: npm Not Installed (Prerequisites Missing)

**Goal**: Verify helpful error when npm is not available

### Setup
```bash
# Temporarily remove npm from PATH
export PATH_BACKUP=$PATH
export PATH=/usr/bin:/bin

# Verify npm is not found
npm --version
# Should output "command not found"

# Remove typescript-language-server if installed
# (can't use npm, so manually remove or skip if not installed)
```

### Test Steps
1. Open TypeScript file:
   ```bash
   ./target/release/ovim test.ts
   ```

2. Observe status line

### Expected Behavior
- **Status shows error**:
  ```
  LSP: npm not found. Install Node.js first:
    - macOS: brew install node
    - Linux: sudo apt install nodejs npm
    - Windows: Download from https://nodejs.org
  ```

### Cleanup
```bash
# Restore PATH
export PATH=$PATH_BACKUP
```

### Results
- [ ] Error message appeared
- [ ] Message includes OS-specific install instructions
- [ ] No crash or hang
- [ ] Can still edit file (LSP just not available)

**Notes**:
```
_____________________________________________________________________
```

---

## Test 4: Permission Denied (Global Install)

**Goal**: Verify helpful error when npm global install requires sudo

**Note**: This test is platform-specific and might not trigger on all systems.
If npm doesn't require sudo on your system, you can skip this test.

### Setup
```bash
# Check if npm requires sudo
npm install -g test-package-that-does-not-exist 2>&1 | grep -i "EACCES"
# If you see "EACCES", this test is applicable

# Remove typescript-language-server
npm uninstall -g typescript-language-server
```

### Test Steps
1. Open TypeScript file:
   ```bash
   ./target/release/ovim test.ts
   ```

2. If permission error occurs, observe status line

### Expected Behavior
- **If permission error occurs**, status should show:
  ```
  LSP: Auto-install failed: Permission denied. Try one of these:
    1. Run with sudo: sudo npm install -g typescript-language-server
    2. Configure npm to use local directory: ...
    3. Use a version manager like nvm
  ```

### Results
- [ ] If applicable: Permission error was caught
- [ ] If applicable: Helpful suggestions shown
- [ ] If not applicable: Installed successfully (Test 1 behavior)

**Notes**:
```
_____________________________________________________________________
```

---

## Test 5: Network Failure

**Goal**: Verify error handling when network is unavailable

### Setup
```bash
# Remove typescript-language-server
npm uninstall -g typescript-language-server

# Disconnect from internet
# (varies by system - disable Wi-Fi, disconnect ethernet, etc.)
```

### Test Steps
1. Open TypeScript file:
   ```bash
   ./target/release/ovim test.ts
   ```

2. Observe status line

### Expected Behavior
- **Status shows error**:
  ```
  LSP: Auto-install failed: Network error. Check internet connection and try again.
  ```
- **OR** (if npm detects network issue differently):
  ```
  LSP: Auto-install failed: [stderr from npm with network-related error]
  ```

### Cleanup
```bash
# Reconnect to internet
```

### Results
- [ ] Error message appeared
- [ ] Message mentions network/connection
- [ ] No crash or hang

**Notes**:
```
_____________________________________________________________________
```

---

## Test 6: JavaScript File (Reuses TypeScript LSP)

**Goal**: Verify JavaScript files also trigger TypeScript LSP auto-install

### Setup
```bash
# Remove typescript-language-server
npm uninstall -g typescript-language-server
```

### Test Steps
1. Open JavaScript file:
   ```bash
   ./target/release/ovim test.js
   ```

2. Observe status line

### Expected Behavior
- **Same as Test 1**: Auto-install should run
- Status shows "LSP: Installing typescript-language-server..."
- After install: "LSP: JavaScript ready" (note: language ID is "javascript")

### Results
- [ ] Auto-install triggered for .js file
- [ ] LSP started after install
- [ ] Hover (K) works on JavaScript code
- [ ] No errors

**Notes**:
```
_____________________________________________________________________
```

---

## Test 7: Fallback Locations (node_modules/.bin)

**Goal**: Verify ovim checks fallback locations before auto-installing

### Setup
```bash
# Create local node_modules with typescript-language-server
# (This simulates project-local install)
mkdir -p node_modules/.bin
npm install typescript-language-server
# This installs locally, not globally

# Verify it's in node_modules
ls node_modules/.bin/typescript-language-server
```

### Test Steps
1. Open TypeScript file from this directory:
   ```bash
   ./target/release/ovim test.ts
   ```

2. Observe status line

### Expected Behavior
- **No auto-install** (should find local installation)
- Status shows "LSP: TypeScript ready"
- LSP uses `node_modules/.bin/typescript-language-server`

### Verification
```bash
# Check LSP status endpoint (if running headless with --session)
# curl http://localhost:PORT/lsp/status | jq '.servers.typescript.command'
# Should show local path
```

### Results
- [ ] No auto-install triggered
- [ ] LSP started using local installation
- [ ] Hover (K) works

**Notes**:
```
_____________________________________________________________________
```

---

## Test 8: Multiple Files (Install Once)

**Goal**: Verify auto-install only runs once, even when opening multiple files

### Setup
```bash
# Remove typescript-language-server
npm uninstall -g typescript-language-server

# Create multiple TypeScript files
echo "const a = 1;" > file1.ts
echo "const b = 2;" > file2.ts
```

### Test Steps
1. Open first file:
   ```bash
   ./target/release/ovim file1.ts
   ```
2. Wait for auto-install to complete
3. Open second file (in same session):
   ```bash
   # In ovim, use :e file2.ts or open new instance
   ./target/release/ovim file2.ts
   ```

### Expected Behavior
- **First file**: Auto-install runs
- **Second file**: No auto-install (already installed)

### Results
- [ ] Auto-install ran once
- [ ] Second file used already-installed LSP
- [ ] Both files have working LSP

**Notes**:
```
_____________________________________________________________________
```

---

## Test 9: Headless Mode

**Goal**: Verify auto-install works in headless mode

### Setup
```bash
# Remove typescript-language-server
npm uninstall -g typescript-language-server
```

### Test Steps
1. Start headless session:
   ```bash
   ./target/release/ovim --headless --session test test.ts &
   ```
2. Check LSP status:
   ```bash
   ./target/release/ovim lsp-status --session test
   ```
3. Check if auto-install happened:
   ```bash
   which typescript-language-server
   ```

### Expected Behavior
- Headless session should auto-install (same as TUI)
- LSP status should show "ready" after install completes
- No user interaction needed

### Cleanup
```bash
./target/release/ovim kill --session test
```

### Results
- [ ] Auto-install ran in headless mode
- [ ] LSP started after install
- [ ] `lsp-status` shows TypeScript LSP ready

**Notes**:
```
_____________________________________________________________________
```

---

## Test 10: stderr Logging

**Goal**: Verify auto-install logs useful information to stderr

### Setup
```bash
# Remove typescript-language-server
npm uninstall -g typescript-language-server
```

### Test Steps
1. Open TypeScript file with stderr redirection:
   ```bash
   ./target/release/ovim test.ts 2> /tmp/ovim-install.log
   ```
2. After installation, check logs:
   ```bash
   cat /tmp/ovim-install.log
   ```

### Expected Behavior
Logs should include:
- "TypeScript language server not found. Attempting auto-install..."
- "Installing typescript-language-server via npm: npm install -g ..."
- "Successfully installed typescript-language-server at /path/to/binary"

### Results
- [ ] Logs show auto-install started
- [ ] Logs show npm command being run
- [ ] Logs show success message
- [ ] No error messages (unless expected)

**Notes**:
```
_____________________________________________________________________
```

---

## Summary

### Test Results

| Test | Status | Notes |
|------|--------|-------|
| 1. Happy Path | ⬜ Pass / ⬜ Fail | |
| 2. Already Installed | ⬜ Pass / ⬜ Fail | |
| 3. npm Not Installed | ⬜ Pass / ⬜ Fail | |
| 4. Permission Denied | ⬜ Pass / ⬜ Fail / ⬜ N/A | |
| 5. Network Failure | ⬜ Pass / ⬜ Fail | |
| 6. JavaScript File | ⬜ Pass / ⬜ Fail | |
| 7. Fallback Locations | ⬜ Pass / ⬜ Fail | |
| 8. Multiple Files | ⬜ Pass / ⬜ Fail | |
| 9. Headless Mode | ⬜ Pass / ⬜ Fail | |
| 10. stderr Logging | ⬜ Pass / ⬜ Fail | |

### Overall Assessment

**All tests passed?** ⬜ Yes / ⬜ No

**Critical issues found?** ⬜ Yes / ⬜ No

**Description**:
```
_____________________________________________________________________
_____________________________________________________________________
_____________________________________________________________________
```

### Recommendations

⬜ Ready for merge
⬜ Minor issues - can merge with follow-up fixes
⬜ Major issues - needs more work

**Suggested improvements**:
```
_____________________________________________________________________
_____________________________________________________________________
_____________________________________________________________________
```

---

## Platform-Specific Notes

### macOS
- npm usually installed via Homebrew: `brew install node`
- Global npm packages go to: `/usr/local/lib/node_modules/`
- Binaries linked to: `/usr/local/bin/`

### Linux
- npm from package manager: `sudo apt install nodejs npm`
- Global packages: `/usr/lib/node_modules/`
- Binaries: `/usr/bin/` or `~/.npm-global/bin/`

### Windows
- npm from nodejs.org installer
- Global packages: `%APPDATA%\npm\node_modules\`
- Binaries: `%APPDATA%\npm\`

---

**Tester Signature**: _______________________
**Date**: _______________________
**Platform**: _______________________
**ovim version**: _______________________
