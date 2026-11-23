//! Characterization of Normal/Visual-mode multi-key input (OV-00488).
//!
//! Feeds every short operator / prefix sequence (operators with motions and
//! text objects, the g/z/Z/[/]/"/q/@/<C-w> prefixes, counts, registers,
//! Esc cancel, mode switches while something is pending) one key at a time
//! and records, after every key, the pending input the editor reports plus
//! the final buffer, cursor, mode and registers. The snapshot pins the
//! behaviour of the input state machine across refactors of how pending
//! input is stored; it does not claim every recorded outcome is vim-exact
//! (vim semantics are pinned by the dedicated operator/motion tests).

mod helpers;
use helpers::EditorTest;
use ovim::editor::InputState;
use std::fmt::Write;

const CONTENT: &str =
    "foo bar(baz, qux) \"str\" end\n  alpha beta gamma\nxdelta\n\nlast line here\n";

/// Split key notation into single keys: `<Esc>`, `<C-w>` or one char.
fn tokens(keys: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = keys.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' && chars.peek().is_some_and(|n| n.is_ascii_uppercase()) {
            let mut token = String::from("<");
            for n in chars.by_ref() {
                token.push(n);
                if n == '>' {
                    break;
                }
            }
            out.push(token);
        } else {
            out.push(c.to_string());
        }
    }
    out
}

fn pending(test: &EditorTest) -> String {
    let editor = &test.editor;
    let mut parts = Vec::new();
    // A count typed before the operator lives in the pending operator; the
    // count a motion would repeat is the product of both.
    let operator_count = match editor.input_state() {
        InputState::OperatorPending { count, .. } => *count,
        _ => None,
    };
    if let Some(count) = match (operator_count, editor.count()) {
        (Some(before), Some(after)) => Some(before * after),
        (before, after) => before.or(after),
    } {
        parts.push(format!("n={count}"));
    }
    if let Some(op) = editor.input_state().pending_operator() {
        parts.push(format!("op={op:?}"));
    }
    if let Some(cmd) = editor.input_state().prefix_key() {
        parts.push(format!("cmd={cmd}"));
    }
    if let Some(reg) = editor.pending_register() {
        parts.push(format!("reg={reg}"));
    }
    match editor.input_state() {
        InputState::AwaitingChar { motion, operator } => {
            parts.push(format!("char={motion:?}/{operator:?}"))
        }
        InputState::Leader { keys } => parts.push(format!("leader={keys:?}")),
        _ => {}
    }
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(",")
    }
}

fn run_case(out: &mut String, keys: &str) {
    let mut test = EditorTest::new(CONTENT);
    test.set_cursor(1, 8);
    let _ = write!(out, "{keys:<14}|");
    for token in tokens(keys) {
        test.keys(&token);
        let _ = write!(out, " {token}[{}]", pending(&test));
    }
    let (line, col) = test.cursor();
    let _ = write!(out, " => {:?} ({line},{col})", test.mode());
    let text = test.editor.buffer().rope().to_string();
    if text != CONTENT {
        let _ = write!(out, " text={text:?}");
    }
    for reg in ['"', 'a'] {
        if let Some(content) = test.get_register_content(reg) {
            let _ = write!(out, " @{reg}={content:?}");
        }
    }
    if test.editor.is_recording_macro() {
        let _ = write!(out, " recording");
    }
    out.push('\n');
}

fn operator_cases() -> Vec<String> {
    let operators = [
        "d", "c", "y", "<", ">", "=", "gu", "gU", "g~", "g?", "gq", "zf",
    ];
    let prefixes = ["", "2", "\"a"];
    let targets = [
        "w", "e", "b", "$", "0", "j", "k", "G", "gg", "gn", "iw", "aw", "i(", "a\"", "ip", "fa",
        "ta", "Fa", "l", "}", "2w", "d", "c", "y", "<", ">", "u", "U", "~", "<Esc>", "x", "v",
        "i<Esc>", "K", "z", "\"", "g<Esc>", "gx", "q", ":",
    ];
    let mut cases = Vec::new();
    for prefix in prefixes {
        for op in operators {
            for target in targets {
                cases.push(format!("{prefix}{op}{target}"));
            }
        }
    }
    cases
}

fn prefix_cases() -> Vec<String> {
    let prefixes = [
        "g", "z", "Z", "[", "]", "\"", "q", "@", "<C-w>", "2g", "\"ag",
    ];
    let seconds = [
        "g", "e", "E", "_", "J", "u", "U", "~", "i", "I", "v", "'", "r", "z", "t", "f", "a", "b",
        "Q", "[", "]", "m", "{", "(", "c", "\"", "q", "@", "<Esc>", "x", "d", "y", "p", "0", "$",
        "<Enter>", ":",
    ];
    let mut cases = Vec::new();
    for prefix in prefixes {
        for second in seconds {
            cases.push(format!("{prefix}{second}"));
        }
    }
    // Three-key prefix sequences and pending state after a third key.
    cases.extend(
        [
            "grn<Esc>",
            "grx",
            "gr<Esc>",
            "gu<Esc>w",
            "guw.",
            "gUiw",
            "zfj",
            "zfjzo",
            "qaxq",
            "qaxq@a",
            "\"ayy\"ap",
            "\"a2yy",
            "2\"ayy",
            "\"<Esc>x",
            "\"_dd",
            "\"Ayw",
            "d\"ayw",
            "m<Esc>x",
            "ma'a",
            "fa;",
            "ra",
            "r<Esc>",
            "<Space><Esc>x",
            "d<Space>",
            "c<Esc>x",
            "diw.",
            "dw.u",
            "ciwX<Esc>.",
            "cwY<Esc>j.",
            "3dd",
            "d3d",
            "2d2w",
            "dfa.",
            "dta;",
            "yiwP",
            "2yyP",
            "v<Esc>",
            "gv",
            "ZZ",
        ]
        .map(String::from),
    );
    cases
}

fn visual_cases() -> Vec<String> {
    let starts = ["v", "V", "<C-v>"];
    let seconds = [
        "iw",
        "aw",
        "i(",
        "a\"",
        "ip",
        "gg",
        "gn",
        "g<Esc>",
        "gx",
        "\"ay",
        "\"<Esc>",
        "i<Esc>",
        "fa",
        "ta",
        "ra",
        "d",
        "y",
        "c<Esc>",
        "<Esc>",
        "jd",
        "u",
        "U",
        "~",
        ">",
        "<Space><Esc>",
        "2iw",
        "IX<Esc>",
        "AX<Esc>",
        "$AX<Esc>",
        "jIX<Esc>",
        "jAX<Esc>",
        "jcX<Esc>",
        "j$AX<Esc>",
    ];
    let mut cases = Vec::new();
    for start in starts {
        for second in seconds {
            cases.push(format!("{start}{second}"));
        }
    }
    cases
}

fn snapshot(cases: Vec<String>) -> String {
    let mut out = String::new();
    for case in cases {
        run_case(&mut out, &case);
    }
    out
}

#[test]
fn operator_sequences() {
    insta::assert_snapshot!(snapshot(operator_cases()));
}

#[test]
fn prefix_sequences() {
    insta::assert_snapshot!(snapshot(prefix_cases()));
}

#[test]
fn visual_sequences() {
    insta::assert_snapshot!(snapshot(visual_cases()));
}
