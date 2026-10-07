//! Parsing one ex command line: `[range]name[!] [args] [| next]`.
//!
//! Follows vim's `do_one_cmd`: leading colons and blanks are skipped, the
//! range is a list of addresses separated by `,` or `;`, the name is either a
//! run of letters (commands starting with an upper-case letter may also
//! contain digits, like user commands) or one of vim's one-character
//! commands, `!` right after the name is the bang, and whether a `|` ends the
//! arguments depends on the command ([`ArgKind`]).

use super::table::{self, ArgKind, ExCommand};

/// A parsed command.
#[derive(Debug, Clone)]
pub(crate) struct ParsedCmd<'a> {
    pub range: Option<RangeSpec>,
    pub command: &'static ExCommand,
    pub bang: bool,
    pub args: &'a str,
    /// The command line after a `|` that ended this command.
    pub next: Option<&'a str>,
}

/// One line address: a base and a list of `+N`/`-N` offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub base: Base,
    pub offset: isize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// `.`, or an address that is only offsets (`+2`).
    Current,
    /// `$`
    Last,
    /// A 1-based line number; 0 is "before the first line".
    Number(usize),
    /// `'x`
    Mark(char),
    /// `/pat/` (forward) or `?pat?` (backward); an empty pattern reuses the
    /// last search.
    Search { pattern: String, forward: bool },
}

/// The addresses of a range with the separator before each one after the
/// first (`;` makes the previous address the cursor for the next).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSpec {
    pub addresses: Vec<(Address, bool)>,
}

impl RangeSpec {
    /// `%` — every line.
    fn whole() -> Self {
        RangeSpec {
            addresses: vec![
                (
                    Address {
                        base: Base::Number(1),
                        offset: 0,
                    },
                    false,
                ),
                (
                    Address {
                        base: Base::Last,
                        offset: 0,
                    },
                    false,
                ),
            ],
        }
    }
}

/// Parse errors, reported like vim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParseError {
    /// Unknown command name (E492).
    Unknown,
    /// `!` after a command that takes none (E477).
    NoBang,
    /// A malformed address such as `'` without a mark name (E14).
    InvalidAddress,
    /// Text after a complete argument (E488).
    TrailingCharacters(String),
}

impl ParseError {
    pub fn message(&self, line: &str) -> String {
        match self {
            ParseError::Unknown => format!("E492: Not an editor command: {line}"),
            ParseError::NoBang => "E477: No ! allowed".to_string(),
            ParseError::InvalidAddress => format!("E14: Invalid address: {line}"),
            ParseError::TrailingCharacters(rest) => format!("E488: Trailing characters: {rest}"),
        }
    }
}

/// Parse the first command of `line`.
pub(crate) fn parse(line: &str) -> Result<ParsedCmd<'_>, ParseError> {
    let rest = line.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    let (range, rest) = parse_range(rest)?;
    let rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());

    let name_len = name_length(rest);
    let name = &rest[..name_len];
    let after_name = &rest[name_len..];
    if name.is_empty()
        && !after_name.trim_start().is_empty()
        && !after_name.trim_start().starts_with('|')
    {
        return Err(ParseError::Unknown);
    }
    // A bare range (`:42`) resolves to the "" entry.
    let command = table::lookup(name).ok_or(ParseError::Unknown)?;

    let (bang, after_bang) = match after_name.strip_prefix('!') {
        Some(rest) if name != "!" => {
            if !command.bang {
                return Err(ParseError::NoBang);
            }
            (true, rest)
        }
        _ => (false, after_name),
    };
    let args = after_bang.trim_start();
    let (args, next) = split_args(command.args, args);
    Ok(ParsedCmd {
        range,
        command,
        bang,
        args: args.trim_end(),
        next,
    })
}

/// Length of the command name at the start of `text` (vim's
/// `find_ex_command`): one of the one-character commands, a letter run, or
/// for names starting with an upper-case letter a letter-and-digit run.
fn name_length(text: &str) -> usize {
    let Some(first) = text.chars().next() else {
        return 0;
    };
    if matches!(first, '!' | '&' | '<' | '>' | '=' | '@' | '~' | '#') {
        return first.len_utf8();
    }
    if !first.is_ascii_alphabetic() {
        return 0;
    }
    let user_style = first.is_ascii_uppercase();
    text.find(|c: char| !(c.is_ascii_alphabetic() || (user_style && c.is_ascii_digit())))
        .unwrap_or(text.len())
}

/// Split the arguments from the next command according to how the command
/// treats `|`.
fn split_args(kind: ArgKind, args: &str) -> (&str, Option<&str>) {
    match kind {
        ArgKind::Rest => (args, None),
        ArgKind::Substitute => split_after_replacement(args),
        // `:r !cmd` and `:w !cmd` hand the whole rest to the shell.
        ArgKind::File if args.starts_with('!') => (args, None),
        ArgKind::None | ArgKind::Text | ArgKind::File => split_at_bar(args),
    }
}

/// Split at the first `|` not escaped with a backslash.
fn split_at_bar(args: &str) -> (&str, Option<&str>) {
    let mut escaped = false;
    for (index, c) in args.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '|' {
            return (args[..index].trim_end(), Some(&args[index + 1..]));
        }
    }
    (args, None)
}

/// `:s/pat/rep/flags | next`: a bar inside the pattern or the replacement is
/// literal (regex alternation, replacement text); only after the closing
/// delimiter of the replacement does it separate commands. An unterminated
/// replacement owns the rest of the line (vim: `:s/a/b|c` inserts `b|c`).
fn split_after_replacement(args: &str) -> (&str, Option<&str>) {
    let Some(delimiter) = args.chars().next() else {
        return (args, None);
    };
    if delimiter.is_alphanumeric() || matches!(delimiter, '"' | '|' | '\\' | ' ') {
        // `:s` with flags only (`:s g`, `:&&`) — an ordinary argument.
        return split_at_bar(args);
    }
    // The opening delimiter counts: pattern and replacement end at the 2nd
    // and 3rd.
    let mut seen = 0;
    let mut escaped = false;
    for (index, c) in args.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter {
            seen += 1;
        } else if c == '|' && seen >= 3 {
            return (args[..index].trim_end(), Some(&args[index + 1..]));
        }
    }
    (args, None)
}

/// Parse a whole string as one address (the destination of `:t` / `:m`).
pub fn address(text: &str) -> Result<Option<Address>, ParseError> {
    let (address, rest) = parse_address(text.trim())?;
    let rest = rest.trim();
    if !rest.is_empty() {
        return Err(ParseError::TrailingCharacters(rest.to_string()));
    }
    Ok(address)
}

/// Parse `[range]` at the start of `text`. Returns the range (if any) and the
/// remaining text.
fn parse_range(text: &str) -> Result<(Option<RangeSpec>, &str), ParseError> {
    if let Some(rest) = text.strip_prefix('%') {
        return Ok((Some(RangeSpec::whole()), rest));
    }
    let mut addresses = Vec::new();
    let mut rest = text;
    let mut semicolon = false;
    loop {
        rest = rest.trim_start();
        let (address, after) = parse_address(rest)?;
        match address {
            Some(address) => {
                addresses.push((address, semicolon));
                rest = after;
            }
            None if addresses.is_empty() && !rest.starts_with([',', ';']) => break,
            // `,5` / `3,` default the missing side to the current line.
            None => addresses.push((
                Address {
                    base: Base::Current,
                    offset: 0,
                },
                semicolon,
            )),
        }
        rest = rest.trim_start();
        match rest.chars().next() {
            Some(',') => semicolon = false,
            Some(';') => semicolon = true,
            _ => break,
        }
        rest = &rest[1..];
    }
    Ok((
        (!addresses.is_empty()).then_some(RangeSpec { addresses }),
        rest,
    ))
}

/// Parse one address (vim's `get_address`). Returns `None` when `text` does
/// not start with one.
fn parse_address(text: &str) -> Result<(Option<Address>, &str), ParseError> {
    let mut rest = text;
    let base = match rest.chars().next() {
        Some('.') => {
            rest = &rest[1..];
            Some(Base::Current)
        }
        Some('$') => {
            rest = &rest[1..];
            Some(Base::Last)
        }
        Some('\'') => {
            let mut chars = rest[1..].chars();
            let mark = chars.next().ok_or(ParseError::InvalidAddress)?;
            rest = chars.as_str();
            Some(Base::Mark(mark))
        }
        Some(c @ ('/' | '?')) => {
            let (pattern, after) = take_pattern(&rest[1..], c);
            rest = after;
            Some(Base::Search {
                pattern,
                forward: c == '/',
            })
        }
        Some(c) if c.is_ascii_digit() => {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let number = rest[..end]
                .parse()
                .map_err(|_| ParseError::InvalidAddress)?;
            rest = &rest[end..];
            Some(Base::Number(number))
        }
        _ => None,
    };
    let mut offset: isize = 0;
    let mut any_offset = false;
    loop {
        let sign = match rest.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            // `.5` is `.+5`.
            Some(c) if c.is_ascii_digit() && base.is_some() => {
                let end = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                offset += rest[..end]
                    .parse::<isize>()
                    .map_err(|_| ParseError::InvalidAddress)?;
                rest = &rest[end..];
                any_offset = true;
                continue;
            }
            _ => break,
        };
        rest = &rest[1..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let amount = if end == 0 {
            1
        } else {
            rest[..end]
                .parse::<isize>()
                .map_err(|_| ParseError::InvalidAddress)?
        };
        rest = &rest[end..];
        offset += sign * amount;
        any_offset = true;
    }
    if base.is_none() && !any_offset {
        return Ok((None, text));
    }
    Ok((
        Some(Address {
            base: base.unwrap_or(Base::Current),
            offset,
        }),
        rest,
    ))
}

/// Read a search pattern up to an unescaped `delimiter` (consumed) or the end.
/// `\delimiter` becomes a literal delimiter.
fn take_pattern(text: &str, delimiter: char) -> (String, &str) {
    let mut pattern = String::new();
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if escaped {
            if c != delimiter {
                pattern.push('\\');
            }
            pattern.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter {
            return (pattern, &text[index + c.len_utf8()..]);
        } else {
            pattern.push(c);
        }
    }
    if escaped {
        pattern.push('\\');
    }
    (pattern, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(base: Base, offset: isize) -> Address {
        Address { base, offset }
    }

    fn head(line: &str) -> (Option<RangeSpec>, String, bool, &str, Option<&str>) {
        let parsed = parse(line).unwrap_or_else(|error| panic!("{line:?}: {error:?}"));
        (
            parsed.range,
            parsed.command.name(),
            parsed.bang,
            parsed.args,
            parsed.next,
        )
    }

    #[test]
    fn splits_range_name_bang_and_args() {
        let (range, name, bang, args, next) = head("2,4d");
        assert_eq!(
            range.unwrap().addresses,
            vec![
                (addr(Base::Number(2), 0), false),
                (addr(Base::Number(4), 0), false)
            ]
        );
        assert_eq!(
            (name, bang, args, next),
            ("delete".to_string(), false, "", None)
        );

        let (_, name, bang, args, _) = head("g!/a/d");
        assert_eq!((name, bang, args), ("global".to_string(), true, "/a/d"));

        let (_, name, bang, args, _) = head("w !cat");
        assert_eq!((name, bang, args), ("write".to_string(), false, "!cat"));

        let (_, name, bang, args, _) = head("w!file.txt");
        assert_eq!((name, bang, args), ("write".to_string(), true, "file.txt"));

        let (_, name, _, args, _) = head("t0");
        assert_eq!((name, args), ("t".to_string(), "0"));

        let (_, name, _, args, _) = head("r ~/x y.txt");
        assert_eq!((name, args), ("read".to_string(), "~/x y.txt"));
    }

    #[test]
    fn parses_vim_addresses() {
        let range = |line| head(line).0.unwrap().addresses;
        assert_eq!(range("%d").len(), 2);
        assert_eq!(
            range(".,+2y"),
            vec![
                (addr(Base::Current, 0), false),
                (addr(Base::Current, 2), false)
            ]
        );
        assert_eq!(range("$-1d"), vec![(addr(Base::Last, -1), false)]);
        assert_eq!(range("'a,'bd")[1], (addr(Base::Mark('b'), 0), false));
        assert_eq!(
            range("1;/x\\/y/d")[1],
            (
                addr(
                    Base::Search {
                        pattern: "x/y".into(),
                        forward: true
                    },
                    0
                ),
                true
            )
        );
        assert_eq!(range("+d"), vec![(addr(Base::Current, 1), false)]);
        assert_eq!(range("::3 d"), vec![(addr(Base::Number(3), 0), false)]);
    }

    #[test]
    fn bars_end_ordinary_commands_but_not_pattern_or_shell_commands() {
        assert_eq!(head("d x | y").3, "x");
        assert_eq!(head("d x | y").4, Some(" y"));
        assert_eq!(head("s/a|b/c/ | update").3, "/a|b/c/");
        assert_eq!(head("s/a|b/c/ | update").4, Some(" update"));
        // A bar in the replacement is text; only the closing delimiter ends it.
        assert_eq!(head("s/,/ | /g").3, "/,/ | /g");
        assert_eq!(head("s/,/ | /g").4, None);
        assert_eq!(head("s/a/b|c").3, "/a/b|c");
        assert_eq!(head("s/a/b/g | update").4, Some(" update"));
        assert_eq!(head("g/a|b/d|3d").3, "/a|b/d|3d");
        assert_eq!(head("!echo a | wc").3, "echo a | wc");
        assert_eq!(head("r !echo a | wc").4, None);
        assert_eq!(head("normal Ax|y").3, "Ax|y");
        assert_eq!(head("3|4").4, Some("4"));
    }

    #[test]
    fn reports_vim_errors() {
        assert_eq!(parse("foo").unwrap_err(), ParseError::Unknown);
        assert_eq!(parse("u!").unwrap_err(), ParseError::NoBang);
        assert_eq!(parse("3zz").unwrap_err(), ParseError::Unknown);
    }
}
