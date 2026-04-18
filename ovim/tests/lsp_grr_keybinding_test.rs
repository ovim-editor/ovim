//! OV-00007: pin the keybinding the `ovim lsp references` CLI relies on.
//!
//! The CLI's `cmd_find_references` sends keystrokes to a running session
//! and reads the picker that appears. Sending the wrong prefix produces
//! an empty list silently. Earlier the subcommand sent `gr` — which only
//! arms the `gr*` prefix and never fires — and 100 % of CLI calls returned
//! `references: []` regardless of cursor position. The keymap path is
//! `g r r` → `R` prefix → `R r` → `request_find_references`.
//!
//! These tests pin both halves of the contract:
//! 1. `gr` arms the prefix without firing the request.
//! 2. `grr` raises the `find_references` intent — the same flag the LSP
//!    dispatcher polls. If anyone re-routes the keymap, the CLI breaks
//!    silently again; this test surfaces the change first.

mod helpers;

use helpers::EditorTest;

#[test]
fn gr_alone_arms_prefix_but_does_not_fire_references_intent() {
    let mut test = EditorTest::new("fn main() {}\n");

    test.keys("gr");

    assert!(
        !test.editor.pending_intents().find_references,
        "`gr` alone must NOT raise the find_references intent — it only arms the prefix"
    );
    assert_eq!(
        test.editor.pending_command(),
        Some('R'),
        "`gr` must leave the LSP-prefix `R` as a pending command awaiting the next key"
    );
}

#[test]
fn grr_raises_find_references_intent() {
    let mut test = EditorTest::new("fn main() {}\n");

    test.keys("grr");

    assert!(
        test.editor.pending_intents().find_references,
        "`grr` must raise the find_references intent — this is the keystroke the \
         `ovim lsp references` CLI sends and depends on"
    );
}
