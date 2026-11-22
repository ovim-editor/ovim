//! Translating a vim pattern (`:help pattern`) into a `regex` crate pattern.
//!
//! Vim's default is "magic": `*`, `.`, `[` and `~` are special as typed, and
//! everything else needs a backslash (`\(`, `\|`, `\+`, `\{2,3}`, `\<`).
//! `\v` ("very magic") makes all punctuation special as typed, `\M` and `\V`
//! make less of it special. A plain `(` therefore searches for a parenthesis,
//! which is what one wants when searching code.
//!
//! Not translated, because the `regex` crate has no equivalent: look-around
//! (`\@=`, `\@!`, `\@<=`, `\@<!`, `\@>`, `\&`), `\zs` / `\ze`,
//! back-references in the pattern (`\1`), and the position atoms (`\%V`,
//! `\%23l`, `\%^`). These are reported as errors, not silently matched as
//! something else. Classes are ASCII as in vim (`\w` is `[0-9A-Za-z_]`).

/// How much punctuation is special without a backslash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    /// `\V`
    VeryNomagic,
    /// `\M`
    Nomagic,
    /// `\m`, the default
    Magic,
    /// `\v`
    VeryMagic,
}

/// The least magic level at which `c` is special without a backslash. Below
/// that level it is special only with one.
fn bare_special_level(c: char) -> Option<Level> {
    match c {
        '(' | ')' | '|' | '+' | '?' | '=' | '{' | '@' | '%' | '<' | '>' | '&' => {
            Some(Level::VeryMagic)
        }
        '*' | '.' | '[' | '~' => Some(Level::Magic),
        _ => None,
    }
}

/// Translate `pattern` to the syntax of the `regex` crate.
pub(super) fn to_rust_regex(pattern: &str) -> Result<String, String> {
    Translator {
        chars: pattern.chars().collect(),
        at: 0,
        magic: Level::Magic,
        out: String::with_capacity(pattern.len() + 8),
        has_atom: false,
        branch_start: true,
        depth: 0,
    }
    .run()
}

struct Translator {
    chars: Vec<char>,
    at: usize,
    magic: Level,
    out: String,
    /// Whether a multi (`*`, `\+`, ...) has something to repeat.
    has_atom: bool,
    /// At the start of the pattern or just after `\(` or `\|`, where `^` is
    /// an anchor and `*` is a literal star.
    branch_start: bool,
    depth: usize,
}

impl Translator {
    fn run(mut self) -> Result<String, String> {
        while let Some(c) = self.next() {
            if c == '\\' {
                self.backslashed()?;
            } else {
                self.bare(c)?;
            }
        }
        if self.depth > 0 {
            return Err("E54: Unmatched \\(".to_string());
        }
        Ok(self.out)
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.get(self.at).copied();
        if c.is_some() {
            self.at += 1;
        }
        c
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bare(&mut self, c: char) -> Result<(), String> {
        match c {
            '^' if self.branch_start => {
                self.out.push('^');
                Ok(())
            }
            '$' if self.branch_ends_here() => {
                self.out.push('$');
                Ok(())
            }
            _ => match bare_special_level(c) {
                Some(level) if self.magic >= level => self.special(c),
                _ => {
                    self.literal(c);
                    Ok(())
                }
            },
        }
    }

    fn backslashed(&mut self) -> Result<(), String> {
        let Some(c) = self.next() else {
            self.literal('\\');
            return Ok(());
        };
        match c {
            'v' => self.magic = Level::VeryMagic,
            'm' => self.magic = Level::Magic,
            'M' => self.magic = Level::Nomagic,
            'V' => self.magic = Level::VeryNomagic,
            _ => match bare_special_level(c) {
                Some(level) if self.magic >= level => self.literal(c),
                Some(_) => return self.special(c),
                None if c.is_ascii_alphanumeric() || c == '_' => return self.letter_escape(c),
                None => self.literal(c),
            },
        }
        Ok(())
    }

    /// Whether the rest of the pattern is empty or starts with `\|`, `\)` or
    /// `\n`: the places where `$` means end of line.
    fn branch_ends_here(&self) -> bool {
        let rest = &self.chars[self.at..];
        match rest {
            [] => true,
            ['\\', 'n', ..] => true,
            ['|' | ')', ..] => self.magic == Level::VeryMagic,
            ['\\', '|' | ')', ..] => self.magic < Level::VeryMagic,
            _ => false,
        }
    }

    fn literal(&mut self, c: char) {
        let mut buffer = [0u8; 4];
        self.out
            .push_str(&regex::escape(c.encode_utf8(&mut buffer)));
        self.atom();
    }

    fn atom(&mut self) {
        self.has_atom = true;
        self.branch_start = false;
    }

    fn push_atom(&mut self, text: &str) {
        self.out.push_str(text);
        self.atom();
    }

    fn special(&mut self, c: char) -> Result<(), String> {
        match c {
            '(' => self.open_group("("),
            ')' => {
                if self.depth == 0 {
                    return Err("E55: Unmatched \\)".to_string());
                }
                self.depth -= 1;
                self.push_atom(")");
            }
            '|' => {
                self.out.push('|');
                self.has_atom = false;
                self.branch_start = true;
            }
            '*' | '+' | '?' | '=' => {
                if !self.has_atom {
                    // vim: a multi with nothing before it is that character.
                    self.literal(c);
                } else {
                    self.out.push(if c == '=' { '?' } else { c });
                }
            }
            '{' => {
                if !self.has_atom {
                    return Err("E64: \\{ follows nothing".to_string());
                }
                self.counted_multi()?;
            }
            '.' => self.push_atom("."),
            '[' => self.class()?,
            '<' => self.push_atom(r"\b{start}"),
            '>' => self.push_atom(r"\b{end}"),
            // vim matches the last substitute string here; nobody types `~`
            // meaning that, and many type it meaning a tilde.
            '~' => self.literal('~'),
            '%' => self.percent()?,
            '@' => return Err("E869: \\@ look-around is not supported".to_string()),
            '&' => return Err("E869: \\& is not supported".to_string()),
            _ => self.literal(c),
        }
        Ok(())
    }

    fn open_group(&mut self, opening: &str) {
        self.out.push_str(opening);
        self.depth += 1;
        self.has_atom = false;
        self.branch_start = true;
    }

    /// `\{n,m}`, `\{-n,m}` and their short forms; `{` has been read.
    fn counted_multi(&mut self) -> Result<(), String> {
        let mut body = String::new();
        loop {
            match self.next() {
                Some('}') => break,
                Some('\\') if self.peek() == Some('}') => {
                    self.at += 1;
                    break;
                }
                Some(c) => body.push(c),
                None => return Err("E554: Syntax error in \\{...}".to_string()),
            }
        }
        let (lazy, counts) = match body.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, body.as_str()),
        };
        let (min, max) = match counts.split_once(',') {
            Some((min, max)) => (min, Some(max)),
            None => (counts, None),
        };
        let number = |text: &str| -> Result<Option<u32>, String> {
            if text.is_empty() {
                return Ok(None);
            }
            text.parse()
                .map(Some)
                .map_err(|_| "E554: Syntax error in \\{...}".to_string())
        };
        let repeat = match (number(min)?, max.map(number).transpose()?) {
            // `\{}` and `\{-}`: as many, or as few, as possible.
            (None, None) => "*".to_string(),
            (Some(exact), None) => format!("{{{exact}}}"),
            (min, Some(None)) => format!("{{{},}}", min.unwrap_or(0)),
            (min, Some(Some(max))) => format!("{{{},{max}}}", min.unwrap_or(0)),
        };
        self.out.push_str(&repeat);
        if lazy {
            self.out.push('?');
        }
        Ok(())
    }

    /// `\%(`, `\%d123`, `\%x2a`, `\%u20ac`, `\%[abc]`; `%` has been read.
    fn percent(&mut self) -> Result<(), String> {
        match self.next() {
            Some('(') => self.open_group("(?:"),
            Some('d') => self.code_point(10, usize::MAX)?,
            Some('x') => self.code_point(16, 2)?,
            Some('u') => self.code_point(16, 4)?,
            Some('U') => self.code_point(16, 8)?,
            Some('o') => self.code_point(8, 4)?,
            Some('[') => {
                // A sequence of optionally matched atoms: `r\%[ead]`.
                let mut optional = Vec::new();
                loop {
                    match self.next() {
                        Some(']') => break,
                        Some(c) => optional.push(c),
                        None => return Err("E69: Missing ] after \\%[".to_string()),
                    }
                }
                for c in &optional {
                    self.out.push_str("(?:");
                    let mut buffer = [0u8; 4];
                    self.out
                        .push_str(&regex::escape(c.encode_utf8(&mut buffer)));
                }
                for _ in &optional {
                    self.out.push_str(")?");
                }
                self.atom();
            }
            Some(other) => return Err(format!("E71: \\%{other} is not supported")),
            None => return Err("E71: Invalid character after \\%".to_string()),
        }
        Ok(())
    }

    /// Read up to `max_digits` digits in `radix` and emit that character.
    fn code_point(&mut self, radix: u32, max_digits: usize) -> Result<(), String> {
        let mut value: u32 = 0;
        let mut digits = 0;
        while digits < max_digits {
            let Some(digit) = self.peek().and_then(|c| c.to_digit(radix)) else {
                break;
            };
            value = value
                .checked_mul(radix)
                .and_then(|value| value.checked_add(digit))
                .ok_or("E678: Invalid character after \\%[dxouU]")?;
            self.at += 1;
            digits += 1;
        }
        let c = (digits > 0)
            .then(|| char::from_u32(value))
            .flatten()
            .ok_or("E678: Invalid character after \\%[dxouU]")?;
        self.literal(c);
        Ok(())
    }

    /// `\d`, `\s`, `\zs`, `\n`, `\_s`, ...: a backslash followed by a letter,
    /// a digit or `_` means the same at every magic level.
    fn letter_escape(&mut self, c: char) -> Result<(), String> {
        if let Some(class) = character_class(c) {
            self.push_atom(class);
            return Ok(());
        }
        match c {
            'e' => self.push_atom(r"\x1b"),
            't' => self.push_atom(r"\t"),
            'r' => self.push_atom(r"\r"),
            'b' => self.push_atom(r"\x08"),
            'n' => self.push_atom(r"\n"),
            'z' => {
                return Err(match self.next() {
                    Some(c @ ('s' | 'e')) => format!("E869: \\z{c} is not supported"),
                    _ => "E68: Invalid character after \\z".to_string(),
                })
            }
            '1'..='9' => {
                return Err("E869: back-references in a pattern are not supported".to_string())
            }
            // `\_x` is `\x` that also matches a line break. Lines are
            // matched one at a time here, so it is `\x`.
            '_' => match self.next() {
                Some('^') => self.out.push('^'),
                Some('$') => self.out.push('$'),
                Some('.') => self.push_atom("."),
                Some('[') => self.class()?,
                Some(c) => match character_class(c) {
                    Some(class) => self.push_atom(class),
                    None => return Err(format!("E63: Invalid use of \\_{c}")),
                },
                None => return Err("E63: Invalid use of \\_".to_string()),
            },
            // vim takes an unknown escaped letter as the letter.
            _ => self.literal(c),
        }
        Ok(())
    }

    /// A `[...]` collection; `[` has been read. Without a closing `]` it is
    /// a literal `[`, as in vim.
    fn class(&mut self) -> Result<(), String> {
        let Some((class, end)) = translate_class(&self.chars, self.at) else {
            self.literal('[');
            return Ok(());
        };
        self.at = end;
        self.push_atom(&class);
        Ok(())
    }
}

/// The class a letter stands for after a backslash, in vim's ASCII terms.
fn character_class(c: char) -> Option<&'static str> {
    Some(match c {
        'd' => "[0-9]",
        'D' => "[^0-9]",
        's' => r"[ \t]",
        'S' => r"[^ \t]",
        'w' => "[0-9A-Za-z_]",
        'W' => "[^0-9A-Za-z_]",
        'a' => "[A-Za-z]",
        'A' => "[^A-Za-z]",
        'l' => "[a-z]",
        'L' => "[^a-z]",
        'u' => "[A-Z]",
        'U' => "[^A-Z]",
        'x' => "[0-9A-Fa-f]",
        'X' => "[^0-9A-Fa-f]",
        'o' => "[0-7]",
        'O' => "[^0-7]",
        'h' => "[A-Za-z_]",
        'H' => "[^A-Za-z_]",
        // Identifier, keyword, file name and printable characters depend on
        // options in vim; these are the defaults.
        'i' | 'k' => r"[\p{Alphabetic}0-9_]",
        'I' | 'K' => r"[\p{Alphabetic}_]",
        'f' => r"[\p{Alphabetic}0-9_/.\-+,#$%~=]",
        'F' => r"[\p{Alphabetic}_/.\-+,#$%~=]",
        'p' => r"[^\p{Cc}]",
        'P' => r"[^\p{Cc}0-9]",
        _ => return None,
    })
}

/// Translate the collection whose body starts at `start` (after the `[`).
/// Returns the class and the index after its `]`, or `None` when it is not
/// closed.
fn translate_class(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut out = String::from("[");
    let mut at = start;
    if chars.get(at) == Some(&'^') {
        out.push('^');
        at += 1;
    }
    // A `]` directly after the opening is a member, not the end.
    if chars.get(at) == Some(&']') {
        out.push_str(r"\]");
        at += 1;
    }
    loop {
        let c = *chars.get(at)?;
        at += 1;
        match c {
            ']' => return Some((out + "]", at)),
            '[' if chars.get(at) == Some(&':') => {
                let close = chars[at + 1..]
                    .windows(2)
                    .position(|pair| pair == [':', ']'])?;
                let name: String = chars[at + 1..at + 1 + close].iter().collect();
                out.push_str(&named_class(&name)?);
                at += close + 3;
            }
            '\\' => match chars.get(at).copied() {
                Some('e') => {
                    out.push_str(r"\x1b");
                    at += 1;
                }
                Some('b') => {
                    out.push_str(r"\x08");
                    at += 1;
                }
                Some(c @ ('t' | 'r' | 'n' | '\\' | ']' | '^' | '-')) => {
                    out.push('\\');
                    out.push(c);
                    at += 1;
                }
                // Not vim (where `[\d]` is a backslash or a `d`), but what
                // everyone who types it means.
                Some(c @ ('d' | 'w' | 's'))
                    if !chars.get(at + 1).is_some_and(char::is_ascii_digit) =>
                {
                    let class = character_class(c)?;
                    out.push_str(&class[1..class.len() - 1]);
                    at += 1;
                }
                _ => out.push_str(r"\\"),
            },
            // Characters the `regex` crate gives a meaning inside a class.
            '[' | '&' | '~' | '^' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
}

/// `[:alpha:]` and friends, as text to put inside a class.
fn named_class(name: &str) -> Option<String> {
    Some(match name {
        "alpha" | "digit" | "alnum" | "lower" | "upper" | "punct" | "space" | "blank" | "cntrl"
        | "graph" | "print" | "xdigit" => format!("[:{name}:]"),
        "return" => r"\r".to_string(),
        "tab" => r"\t".to_string(),
        "escape" => r"\x1b".to_string(),
        "backspace" => r"\x08".to_string(),
        "ident" | "keyword" => r"\p{Alphabetic}0-9_".to_string(),
        "fname" => r"\p{Alphabetic}0-9_/.\-+,#$%=".to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::to_rust_regex;
    use regex::Regex;

    /// The text `pattern` matches first in `text`, as vim would highlight it.
    fn found<'t>(pattern: &str, text: &'t str) -> Option<&'t str> {
        let translated =
            to_rust_regex(pattern).unwrap_or_else(|error| panic!("{pattern}: {error}"));
        let regex = Regex::new(&translated)
            .unwrap_or_else(|error| panic!("{pattern} -> {translated}: {error}"));
        regex.find(text).map(|found| found.as_str())
    }

    // Every expectation below was checked in `nvim --clean` with
    // `:echo matchstr(text, pattern)`, except the bare `~`, which vim
    // matches against the last substitute string.

    #[test]
    fn punctuation_is_literal_in_a_magic_pattern() {
        assert_eq!(found("main(", "fn main() {}"), Some("main("));
        assert_eq!(found("a+b", "a+b aab"), Some("a+b"));
        assert_eq!(found("x|y", "x y x|y"), Some("x|y"));
        assert_eq!(found("{k}", "a{k}"), Some("{k}"));
        assert_eq!(found("what?", "what? wha"), Some("what?"));
        assert_eq!(found("a=b", "a=b ab"), Some("a=b"));
    }

    #[test]
    fn backslashed_punctuation_is_special_in_a_magic_pattern() {
        assert_eq!(found(r"\(ab\)\+", "xababx"), Some("abab"));
        assert_eq!(found(r"foo\|bar", "a bar foo"), Some("bar"));
        assert_eq!(found(r"colou\=r", "color"), Some("color"));
        assert_eq!(found(r"colou\?r", "colour"), Some("colour"));
        assert_eq!(found(r"a\{2,3}", "aaaa"), Some("aaa"));
        assert_eq!(found(r"a\{2}", "aaaa"), Some("aa"));
        assert_eq!(found(r"a\{,2}b", "aaab"), Some("aab"));
        assert_eq!(found(r"a\{2,}", "aaaa"), Some("aaaa"));
        assert_eq!(found(r"t.\{-}e", "two three"), Some("two thre"));
        assert_eq!(found(r"a\{-1,}", "aaaa"), Some("a"));
        assert_eq!(found(r"\%(ab\)*c", "ababc"), Some("ababc"));
    }

    #[test]
    fn star_dot_and_brackets_are_special_as_typed() {
        assert_eq!(found("a*b", "aaab"), Some("aaab"));
        assert_eq!(found("a.c", "abc"), Some("abc"));
        assert_eq!(found("[bc]at", "cat"), Some("cat"));
        assert_eq!(found(r"a\.c", "abc a.c"), Some("a.c"));
        assert_eq!(found(r"a\*", "aa a*"), Some("a*"));
        // A star with nothing to repeat is a star.
        assert_eq!(found("*a", "b*a"), Some("*a"));
        assert_eq!(found(r"\(*a\)", "b*a"), Some("*a"));
    }

    #[test]
    fn anchors_are_positional() {
        assert_eq!(found("^a", "ba"), None);
        assert_eq!(found("^a", "ab"), Some("a"));
        assert_eq!(found("a^b", "a^b"), Some("a^b"));
        assert_eq!(found("b$", "ab"), Some("b"));
        assert_eq!(found("a$b", "a$b"), Some("a$b"));
        assert_eq!(found(r"x\|^a", "ab"), Some("a"));
        assert_eq!(found(r"\(b$\)", "ab"), Some("b"));
        assert_eq!(found(r"\^a", "b^a"), Some("^a"));
        assert_eq!(found(r"a\$", "a$"), Some("a$"));
    }

    #[test]
    fn word_boundaries_and_classes() {
        assert_eq!(found(r"\<t\w\+", "at two"), Some("two"));
        assert_eq!(found(r"\w\+\>", "one two"), Some("one"));
        assert_eq!(found(r"\d\+", "ab 123"), Some("123"));
        assert_eq!(found(r"\s\+", "a  \tb"), Some("  \t"));
        assert_eq!(found(r"\S\+", "  ab cd"), Some("ab"));
        assert_eq!(found(r"\u\l\+", "a Word"), Some("Word"));
        assert_eq!(found(r"\x\+", "zz1fz"), Some("1f"));
        assert_eq!(found(r"\a\+", "12ab3"), Some("ab"));
        assert_eq!(found(r"\h\w*", "1 _a1"), Some("_a1"));
        // vim's classes are ASCII.
        assert_eq!(found(r"\w\+", "héllo"), Some("h"));
    }

    #[test]
    fn very_magic() {
        assert_eq!(
            found(r"\v(two) (three)", "one two three"),
            Some("two three")
        );
        assert_eq!(found(r"\va+|b", "caab"), Some("aa"));
        assert_eq!(found(r"\v<t\w+>", "at two"), Some("two"));
        assert_eq!(found(r"\va{2}", "aaa"), Some("aa"));
        assert_eq!(found(r"\vx=y", "y"), Some("y"));
        assert_eq!(found(r"\va\+", "aa a+"), Some("a+"));
        assert_eq!(found(r"\v%(ab)+", "abab"), Some("abab"));
        // Switching back.
        assert_eq!(found(r"\v(a)\m(b)", "a(b)"), Some("a(b)"));
    }

    #[test]
    fn nomagic_and_very_nomagic() {
        assert_eq!(found(r"\Ma.c*", "abc a.c*"), Some("a.c*"));
        assert_eq!(found(r"\Ma\.c", "abc"), Some("abc"));
        assert_eq!(found(r"\V[a].*", "x [a].*"), Some("[a].*"));
        assert_eq!(found(r"\V\<a\>", "ba a"), Some("a"));
        assert_eq!(found(r"\V^a", "ba"), None);
        assert_eq!(found(r"\Va$", "ab"), None);
    }

    #[test]
    fn collections() {
        assert_eq!(found("[[:alpha:]]\\+", "12ab3"), Some("ab"));
        assert_eq!(found("[^a-c]", "abcd"), Some("d"));
        assert_eq!(found("[]x]\\+", "a]x]"), Some("]x]"));
        assert_eq!(found("[^]]", "]a"), Some("a"));
        assert_eq!(found(r"[a\]]\+", "b]a"), Some("]a"));
        assert_eq!(found("[a-]\\+", "b-a"), Some("-a"));
        assert_eq!(found("[&~[]\\+", "a&~[b"), Some("&~["));
        assert_eq!(found(r"[\t ]\+x", "a \tx"), Some(" \tx"));
        // Not closed: a bracket.
        assert_eq!(found("a[1", "a[1]"), Some("a[1"));
        assert_eq!(found(r"baz\[1]", "baz[1]"), Some("baz[1]"));
    }

    #[test]
    fn escapes_and_code_points() {
        assert_eq!(found(r"a\tb", "a\tb"), Some("a\tb"));
        assert_eq!(found(r"a\/b", "a/b"), Some("a/b"));
        assert_eq!(found(r"a\\b", r"a\b"), Some(r"a\b"));
        assert_eq!(found(r"\%d65", "xAx"), Some("A"));
        assert_eq!(found(r"\%x41b", "xAb"), Some("Ab"));
        assert_eq!(found(r"\%u20ac", "5€"), Some("€"));
        assert_eq!(found(r"r\%[ead]", "re"), Some("re"));
        assert_eq!(found(r"r\%[ead]", "read"), Some("read"));
        assert_eq!(found("~", "a~b"), Some("~"));
    }

    #[test]
    fn unsupported_items_are_errors() {
        for pattern in [
            r"foo\zsbar",
            r"foo\zebar",
            r"\(a\)\1",
            r"a\@=",
            r"\(a\)\@<!b",
            r"a\&b",
            r"\%Vx",
            r"\%23l",
        ] {
            assert!(to_rust_regex(pattern).is_err(), "{pattern}");
        }
    }

    #[test]
    fn unbalanced_groups_are_errors() {
        assert!(to_rust_regex(r"\(a").is_err());
        assert!(to_rust_regex(r"a\)").is_err());
        assert!(to_rust_regex(r"\va)").is_err());
        assert!(to_rust_regex(r"a\{2").is_err());
    }
}
