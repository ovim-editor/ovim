pub(crate) fn normalize_keys_for_test(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\n' | '\r' => out.push_str("<Enter>"),
            '\t' => out.push_str("<Tab>"),
            '\\' => match chars.peek().copied() {
                Some('n') => {
                    chars.next();
                    out.push_str("<Enter>");
                }
                Some('r') => {
                    chars.next();
                    out.push_str("<Enter>");
                }
                Some('t') => {
                    chars.next();
                    out.push_str("<Tab>");
                }
                Some('"') => {
                    chars.next();
                    out.push('"');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }

    out
}

pub(crate) fn unescape_expected_for_test(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }

        match chars.peek().copied() {
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('r') => {
                chars.next();
                out.push('\n');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('"') => {
                chars.next();
                out.push('"');
            }
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            _ => out.push('\\'),
        }
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }

    out
}

macro_rules! key_test {
    ( $(setup $setup:expr;)* when $when:expr; then $then:expr $(;)?) => {{
        let mut test = $crate::helpers::EditorTest::new("");
        $(
            let setup_keys = $crate::key_test_macro::normalize_keys_for_test($setup);
            test.keys(&setup_keys);
        )*
        let keys = $crate::key_test_macro::normalize_keys_for_test($when);
        test.keys(&keys);
        let expected = $crate::key_test_macro::unescape_expected_for_test($then);
        assert_eq!(test.buffer_content(), expected);
    }};
    (given $given:expr; $(setup $setup:expr;)* when $when:expr; then $then:expr $(;)?) => {{
        let mut test = $crate::helpers::EditorTest::new($given);
        $(
            let setup_keys = $crate::key_test_macro::normalize_keys_for_test($setup);
            test.keys(&setup_keys);
        )*
        let keys = $crate::key_test_macro::normalize_keys_for_test($when);
        test.keys(&keys);
        let expected = $crate::key_test_macro::unescape_expected_for_test($then);
        assert_eq!(test.buffer_content(), expected);
    }};
}
