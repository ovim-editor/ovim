//! OV-00250: bracketed paste must normalize CR variants to LF before
//! inserting into the rope. Without this, pasted text from Windows
//! clipboards / browser scrollback / Mac-classic sources leaves literal
//! `\r` in the buffer and renders as `^M`.

mod helpers;
use helpers::EditorTest;

#[test]
fn paste_in_insert_strips_crlf() {
    let mut test = EditorTest::new("");
    test.press('i'); // Enter insert mode through the proper input path
    test.editor
        .handle_paste_event("first\r\nsecond\r\nthird")
        .unwrap();

    let content = test.editor.buffer().rope().to_string();
    assert!(
        !content.contains('\r'),
        "paste should strip \\r, got: {content:?}"
    );
    assert!(content.starts_with("first\nsecond\nthird"));
}

#[test]
fn paste_in_insert_strips_bare_cr() {
    // Mac-classic content / pasted terminal scrollback can contain bare CRs.
    // Vim and VS Code treat these as line breaks; we do the same.
    let mut test = EditorTest::new("");
    test.press('i');
    test.editor.handle_paste_event("a\rb\rc").unwrap();

    let content = test.editor.buffer().rope().to_string();
    assert!(!content.contains('\r'));
    assert!(content.starts_with("a\nb\nc"));
}

#[test]
fn paste_in_normal_strips_crlf() {
    // Normal-mode paste runs through the unnamed-register path (different
    // branch in handle_paste_event); verify the same normalization applies.
    let mut test = EditorTest::new("seed\n");
    test.editor.handle_paste_event("p1\r\np2").unwrap();

    let content = test.editor.buffer().rope().to_string();
    assert!(
        !content.contains('\r'),
        "paste should strip \\r, got: {content:?}"
    );
}

#[test]
fn paste_lf_only_is_unchanged() {
    let mut test = EditorTest::new("");
    test.press('i');
    test.editor
        .handle_paste_event("alpha\nbeta\ngamma")
        .unwrap();

    let content = test.editor.buffer().rope().to_string();
    assert!(content.starts_with("alpha\nbeta\ngamma"));
}

#[test]
fn paste_preserves_unicode_around_crlf() {
    // CR is ASCII and never inside a UTF-8 multi-byte sequence, but verify
    // the normalization didn't accidentally byte-iterate and corrupt
    // non-ASCII text neighboring the line break.
    let mut test = EditorTest::new("");
    test.press('i');
    test.editor.handle_paste_event("café\r\ndéjà vu").unwrap();

    let content = test.editor.buffer().rope().to_string();
    assert!(content.starts_with("café\ndéjà vu"));
    assert!(!content.contains('\r'));
}
