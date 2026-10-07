//! Compiling the patterns typed after `/`, `?`, `:s` and `:g` and in ex
//! ranges (`:/pat/d`) with vim's case rules, in one place.
//!
//! A pattern ignores case when it says so (`\c`; `\C` forbids it), else when
//! a `:s` flag says so (`i` / `I`), else when 'ignorecase' is set and either
//! 'smartcase' is off or the pattern has no upper-case letter. The patterns
//! are still Rust regexes, not vim regexes; only the case escapes are vim's.

use regex::{Regex, RegexBuilder};

/// The 'ignorecase' and 'smartcase' options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaseOptions {
    pub ignorecase: bool,
    pub smartcase: bool,
}

impl CaseOptions {
    pub fn new(ignorecase: bool, smartcase: bool) -> Self {
        Self {
            ignorecase,
            smartcase,
        }
    }

    pub fn of(options: &crate::editor::EditorOptions) -> Self {
        Self::new(options.ignorecase, options.smartcase)
    }
}

/// Compile `pattern`. `forced` is the `:s` flag: `Some(true)` for `i`,
/// `Some(false)` for `I`; `\c` and `\C` in the pattern still win.
pub fn compile(
    pattern: &str,
    options: CaseOptions,
    forced: Option<bool>,
) -> Result<Regex, regex::Error> {
    let (source, escaped) = strip_case_escapes(pattern);
    let ignore_case = escaped
        .or(forced)
        .unwrap_or(options.ignorecase && !(options.smartcase && has_uppercase(&source)));
    RegexBuilder::new(&source)
        .case_insensitive(ignore_case)
        .build()
}

/// Remove `\c` and `\C` from `pattern` (a regex has no such escapes) and
/// report what they asked for: `\c` beats `\C`.
fn strip_case_escapes(pattern: &str) -> (String, Option<bool>) {
    let mut source = String::with_capacity(pattern.len());
    let mut requested = None;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            source.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => requested = Some(true),
            Some('C') => requested = requested.or(Some(false)),
            Some(escaped) => {
                source.push('\\');
                source.push(escaped);
            }
            None => source.push('\\'),
        }
    }
    (source, requested)
}

/// Whether 'smartcase' sees an upper-case letter. Escapes such as `\S` or
/// `\W` name a class, not a letter, so they do not count.
fn has_uppercase(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            c if c.is_uppercase() => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const IGNORE: CaseOptions = CaseOptions {
        ignorecase: true,
        smartcase: false,
    };
    const SMART: CaseOptions = CaseOptions {
        ignorecase: true,
        smartcase: true,
    };

    fn matches(pattern: &str, options: CaseOptions, forced: Option<bool>, text: &str) -> bool {
        compile(pattern, options, forced).unwrap().is_match(text)
    }

    #[test]
    fn case_matters_unless_ignorecase_is_set() {
        assert!(!matches("foo", CaseOptions::default(), None, "Foo"));
        assert!(matches("foo", IGNORE, None, "Foo"));
        // smartcase: an upper-case letter in the pattern makes it exact.
        assert!(matches("foo", SMART, None, "Foo"));
        assert!(!matches("Foo", SMART, None, "foo"));
        // smartcase without ignorecase does nothing.
        assert!(!matches("foo", CaseOptions::new(false, true), None, "Foo"));
    }

    #[test]
    fn substitute_flags_override_the_options() {
        assert!(matches("foo", CaseOptions::default(), Some(true), "Foo"));
        assert!(!matches("foo", IGNORE, Some(false), "Foo"));
        assert!(matches("Foo", SMART, Some(true), "foo"));
    }

    #[test]
    fn case_escapes_beat_flags_and_options() {
        assert!(matches(r"foo\c", CaseOptions::default(), None, "FOO"));
        assert!(matches(
            r"\cfoo",
            CaseOptions::default(),
            Some(false),
            "FOO"
        ));
        assert!(!matches(r"foo\C", IGNORE, None, "Foo"));
        assert!(!matches(r"foo\C", IGNORE, Some(true), "Foo"));
        // `\c` wins over `\C`, wherever they sit.
        assert!(matches(r"\Cfoo\c", CaseOptions::default(), None, "FOO"));
    }

    #[test]
    fn escaped_backslash_before_c_is_not_an_escape() {
        let regex = compile(r"a\\c", IGNORE, None).unwrap();
        assert!(regex.is_match(r"A\C"));
        assert!(!regex.is_match("ac"));
    }

    #[test]
    fn smartcase_ignores_class_escapes() {
        // vim: `\S` is not an upper-case letter.
        assert!(matches(r"\Sfoo", SMART, None, "xFOO"));
        assert!(!matches(r"\Sfoo", CaseOptions::default(), None, "xFOO"));
    }
}
