# Testing Vim Semantics: Derive from the Reference, Not the Implementation

## The failure mode this prevents

In the 2026-08 bug hunt, **seven existing tests asserted buggy behavior**
and passed for months:

- four dot-repeat/undo flows asserted that backward `dF`/`dT` include the
  cursor character (vim: exclusive — OV-00288)
- one repeat test asserted `3J` performs three joins (vim: count lines,
  i.e. two joins — OV-00290)
- two visual-block tests asserted that paste rows past the last buffer
  line are silently dropped (vim: appended as padded lines — OV-00291)

Each test was written by running the ovim implementation and encoding
what it happened to do. A test derived from the implementation can only
detect *change*, never *wrongness* — for emulation code, that's most of
the value gone.

## The convention

For any test asserting vim-compatible behavior (motions, operators,
registers, text objects, counts, undo grouping, mode transitions):

1. **Verify the expectation in vim/nvim first.** Run the exact keystrokes
   against the exact buffer content in a clean instance:

   ```bash
   nvim --clean
   # or: vim -N -u NONE
   ```

   Set the buffer, run the keys, observe the result — buffer content,
   cursor position, and register contents (`:reg`) as applicable.

2. **Record the derivation in the test.** A one-line comment stating the
   reference behavior is enough:

   ```rust
   // vim: "axbc" cursor on 'c', dFx → "ac" (backward F is exclusive)
   ```

   An existing example of this style already in-tree:
   `dot_repeat_test.rs` cites `vim -N -u NONE` for the visual-line-change
   flow.

3. **If ovim intentionally diverges from vim, say so in the test** and
   name the reason. Divergence is allowed; *silent* divergence is not.

4. **When a semantics test fails after a change, do not update the
   expectation without re-checking vim.** The old expectation may be the
   bug (as in the seven cases above) — or the new behavior may be. The
   reference decides, not whichever assertion is convenient.

## Comparing with nvim mechanically

`tools/nvim_parity.py` types the same keys into `nvim --clean` and into a
headless ovim and compares the text, the cursor and the mode:

```bash
cargo build -p ovim
printf 'A\twd2$\nB\tjf"da"\n' > /tmp/cases.txt
tools/nvim_parity.py compare /tmp/cases.txt   # prints the cases that differ
tools/nvim_parity.py rows /tmp/cases.txt      # prints rows for a Rust test
```

`ovim/tests/nvim_parity_test.rs` is built from such rows: each one is what
nvim did, so the test can only pass when ovim does the same. Its cases are
in `tools/nvim_parity_cases.txt`. `tools/nvim_parity_sweep.txt` is a wider
sample of about 800 everyday commands for finding differences; the ones
still open are listed in OV-00514.

The script's header lists what a difference can mean other than a bug (nvim
leaving Insert mode when the keys end, the `\e` / `\c` / `\n` escapes of
`ovim send`, ovim's cursor position after undo).

## Where the reference matters most

Priority areas, in order of past defect density:

- operator + motion combinations (`d`/`c`/`y` × `w`/`f`/`}`/text objects)
- exclusive/inclusive/linewise classification (see
  `ovim-core/src/motion_range.rs`, which encodes `:help exclusive`)
- register types (charwise vs linewise vs blockwise) and numbered-register
  rotation
- counts (`3J`, `d2w`, `2aw`)
- cursor position after operations (paste, join, undo/redo)

## Non-vim surfaces

CLI/API/headless behavior has no external reference — those tests define
the contract themselves. Keep the two categories distinguishable: a vim
semantics test cites vim; a contract test documents the intended contract
in its comment.
