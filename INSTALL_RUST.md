# Install Rust and Cargo

## Quick Installation

### Linux/macOS/WSL
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Verify Installation
```bash
rustc --version
cargo --version
```

## Then Run Tests

```bash
cd /workspace

# Run all tests
cargo test

# Review snapshots
cargo insta review

# Fix bugs as they appear!
```

## What to Expect

The tests will find bugs! Be ready to:
1. See test failures (that's good!)
2. Review snapshot diffs
3. Fix the bugs in src/
4. Re-run tests
5. Accept snapshots when correct

Let's make ovim great! 🚀
