# Code Signing Guide for ovim on macOS

## The Problem

macOS (especially Apple Silicon) requires binaries to be code-signed. Without proper signing, you may see:
- **SIGKILL (Code Signature Invalid)** errors
- Exit code 137
- "Taskgated Invalid Signature" in crash reports

This happens because:
1. macOS validates signatures at **runtime**, not just at build time
2. Dynamic library dependencies (LuaJIT, OpenSSL from Homebrew) trigger stricter checks
3. Adhoc signatures from the linker may become stale

## Solutions by Use Case

### 1. Local Development (Current Setup)

**Automatic Signing via Cargo** (`.cargo/config.toml`):
```toml
[build]
rustflags = ["-C", "link-arg=-Wl,-adhoc_codesign"]
```

This enables **linker-level adhoc signing** for every build. Just use:
```bash
cargo build --release
./target/release/ovim  # Should work now!
```

**Why this works:**
- The linker creates a fresh signature every time
- No stale signatures from previous builds
- Completely transparent to your workflow

### 2. Manual Signing (Fallback)

If the automatic approach fails, use the helper script:
```bash
./build-and-sign.sh --release
```

Or manually:
```bash
cargo build --release
codesign --sign - --force ./target/release/ovim
./target/release/ovim
```

### 3. Distribution (Homebrew, cargo install, releases)

For **public distribution**, you need a **Developer ID signature**:

**Option A: GitHub Actions with Developer ID**
```yaml
- name: Sign binary
  env:
    MACOS_CERTIFICATE: ${{ secrets.MACOS_CERTIFICATE }}
    MACOS_CERTIFICATE_PWD: ${{ secrets.MACOS_CERTIFICATE_PWD }}
  run: |
    echo $MACOS_CERTIFICATE | base64 --decode > certificate.p12
    security create-keychain -p actions build.keychain
    security default-keychain -s build.keychain
    security unlock-keychain -p actions build.keychain
    security import certificate.p12 -k build.keychain -P $MACOS_CERTIFICATE_PWD -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k actions build.keychain
    codesign --sign "Developer ID Application: Your Name (TEAM_ID)" --force ./target/release/ovim
```

**Option B: Notarization (for .dmg/.pkg)**
```bash
# Sign
codesign --sign "Developer ID Application: Your Name" --options runtime --timestamp ./target/release/ovim

# Notarize (requires Apple Developer account)
xcrun notarytool submit ovim.zip --apple-id you@example.com --password APP_SPECIFIC_PASSWORD --team-id TEAM_ID

# Staple
xcrun stapler staple ./target/release/ovim
```

**Option C: Homebrew (adhoc is OK)**
For Homebrew formulae, adhoc signing is acceptable because Homebrew builds from source on the user's machine. The `.cargo/config.toml` approach is sufficient.

### 4. cargo install (Users Installing from crates.io)

When users run `cargo install ovim`, Cargo will automatically adhoc-sign the binary if:
1. `.cargo/config.toml` is included in the published crate
2. OR the user has their own rustflags configured

**Best practice**: Include `.cargo/config.toml` in your published crate so macOS users get automatic signing.

## Verification

Check if signing worked:
```bash
# Quick check
codesign --verify --verbose ./target/release/ovim

# Detailed info
codesign -dvv ./target/release/ovim

# Test execution
./target/release/ovim --version
```

Expected output for adhoc signing:
```
Signature=adhoc
CodeDirectory v=20400 ... flags=0x20002(adhoc,linker-signed)
```

## Troubleshooting

**"Code Signature Invalid" still happens:**
1. Verify rustflags are active: `cargo clean && cargo build --release -vv | grep adhoc`
2. Force re-sign: `codesign --sign - --force ./target/release/ovim`
3. Check for file modifications post-build (strip, install_name_tool, etc.)

**Build fails with signing error:**
- Ensure Xcode Command Line Tools are installed: `xcode-select --install`
- Check macOS version (adhoc signing requires macOS 10.14+)

**Distribution users report signature issues:**
- Use Developer ID signature for distributed binaries
- Consider notarization for .app bundles or installers
- For Homebrew, adhoc is fine (builds locally)

## References

- [Apple Code Signing Guide](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/)
- [Rust on macOS - Signing Binaries](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html)
- [Homebrew Formulae - Code Signing](https://docs.brew.sh/Formula-Cookbook#code-signing)
