//! The replacement string of `:s/pat/rep/`, parsed once into tokens.
//!
//! Magic characters, as in vim: `&` and `\0` are the whole match, `\1` to
//! `\9` the groups, `\r` a line break, `\t` a tab, `\u` / `\l` change the
//! case of the next character, `\U` / `\L` of the following ones until `\e`
//! or `\E`. A backslash before anything else is dropped, so `\&`, `\\`, `\/`
//! and `\~` insert the character itself.

use regex::Captures;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    Upper,
    Lower,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Text(String),
    /// `&`, `\0`, `\1` ... `\9`.
    Group(usize),
    /// `\u` or `\l`: the next character only.
    Next(Case),
    /// `\U` or `\L`: until the end of the replacement or `\e` / `\E`.
    Following(Case),
    /// `\e` or `\E`.
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Replacement {
    tokens: Vec<Token>,
}

impl Replacement {
    /// Parse the replacement as typed (delimiter escapes already removed).
    pub(super) fn parse(typed: &str) -> Self {
        let mut tokens = Vec::new();
        let mut chars = typed.chars();
        while let Some(c) = chars.next() {
            let token = match c {
                '&' => Token::Group(0),
                '\\' => match chars.next() {
                    Some(digit @ '0'..='9') => Token::Group(digit as usize - '0' as usize),
                    Some('r') => Token::Text("\n".to_string()),
                    Some('t') => Token::Text("\t".to_string()),
                    // vim inserts a NUL byte for `\n`, a trap we do not
                    // emulate: it stays the two characters typed.
                    Some('n') => Token::Text("\\n".to_string()),
                    Some('u') => Token::Next(Case::Upper),
                    Some('l') => Token::Next(Case::Lower),
                    Some('U') => Token::Following(Case::Upper),
                    Some('L') => Token::Following(Case::Lower),
                    Some('e' | 'E') => Token::End,
                    Some(other) => Token::Text(other.to_string()),
                    None => Token::Text("\\".to_string()),
                },
                other => Token::Text(other.to_string()),
            };
            match (token, tokens.last_mut()) {
                (Token::Text(text), Some(Token::Text(previous))) => previous.push_str(&text),
                (token, _) => tokens.push(token),
            }
        }
        Self { tokens }
    }

    /// The replacement for one match.
    pub(super) fn expand(&self, captures: &Captures) -> String {
        let mut out = String::new();
        let mut next: Option<Case> = None;
        let mut following: Option<Case> = None;
        for token in &self.tokens {
            let text = match token {
                Token::Text(text) => text.as_str(),
                Token::Group(group) => captures.get(*group).map_or("", |found| found.as_str()),
                Token::Next(case) => {
                    next = Some(*case);
                    continue;
                }
                Token::Following(case) => {
                    following = Some(*case);
                    continue;
                }
                Token::End => {
                    following = None;
                    continue;
                }
            };
            for c in text.chars() {
                match next.take().or(following) {
                    Some(Case::Upper) => out.extend(c.to_uppercase()),
                    Some(Case::Lower) => out.extend(c.to_lowercase()),
                    None => out.push(c),
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    /// Replace the first match of `pattern` in `text`.
    fn replace(pattern: &str, typed: &str, text: &str) -> String {
        let regex = Regex::new(pattern).unwrap();
        let replacement = Replacement::parse(typed);
        regex
            .replace(text, |captures: &Captures| replacement.expand(captures))
            .into_owned()
    }

    #[test]
    fn groups_and_the_whole_match() {
        // `${1}x` must not read a group named "1x".
        assert_eq!(replace("(a)(b)", r"\2\1x", "ab"), "bax");
        assert_eq!(replace("a", r"[&]", "a"), "[a]");
        assert_eq!(replace("a", r"\0\0", "a"), "aa");
        assert_eq!(replace("(a)", r"\2.", "a"), ".");
    }

    #[test]
    fn escaped_characters_are_inserted_as_themselves() {
        // nvim --clean: `:s/a/\&x/` gives "&x", `\\` one backslash, `\z` "z",
        // `\|` a bar; `$1` is no capture reference.
        assert_eq!(replace("a", r"\&x", "a"), "&x");
        assert_eq!(replace("a", r"\\", "a"), "\\");
        assert_eq!(replace("a", r"\z", "a"), "z");
        assert_eq!(replace("a", r"\|", "a"), "|");
        assert_eq!(replace("a", r"\~", "a"), "~");
        assert_eq!(replace("a", "$1", "a"), "$1");
        assert_eq!(replace("a", "x\\", "a"), "x\\");
    }

    #[test]
    fn line_break_and_tab() {
        assert_eq!(replace(",", r"\r", "a,b"), "a\nb");
        assert_eq!(replace(",", r"\t", "a,b"), "a\tb");
        // vim inserts a NUL for `\n`; here it stays as typed.
        assert_eq!(replace(",", r"\n", "a,b"), "a\\nb");
    }

    #[test]
    fn case_conversion() {
        // nvim --clean results for each of these.
        assert_eq!(replace(r"\w+", r"\u&", "hello world"), "Hello world");
        assert_eq!(replace(r"\w+", r"\U&", "hello world"), "HELLO world");
        assert_eq!(replace(r"\w+", r"\L&", "HELLO WORLD"), "hello WORLD");
        assert_eq!(replace(r"\w+", r"\l&", "HELLO WORLD"), "hELLO WORLD");
        assert_eq!(replace(r"\w+", r"\L\u&", "HELLO WORLD"), "Hello WORLD");
        assert_eq!(replace("hello", r"\Ufoo\ebar", "hello"), "FOObar");
        assert_eq!(replace("ab", r"\U&x\Ey", "ab"), "ABXy");
        assert_eq!(replace("a", r"\u\L&X", "a"), "Ax");
        assert_eq!(
            replace(r"(\w+) (\w+)", r"\U\1\E \2", "hello world"),
            "HELLO world"
        );
        // `\U` with nothing after it inserts nothing.
        assert_eq!(replace("b", r"\U", "abc"), "ac");
    }
}
