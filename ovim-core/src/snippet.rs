//! LSP / TextMate snippet syntax.
//!
//! `$1`, `${1:default}`, `${1|one,two|}`, `$0`, variables (`$TM_FILENAME`,
//! `${TM_SELECTED_TEXT:fallback}`), nested placeholders and `\$` `\}` `\\`
//! escapes, as specified by LSP 3.17 "Snippet Syntax".
//!
//! [`Snippet::parse`] never fails: text that is not valid snippet syntax is
//! inserted literally, like VS Code does.

use std::collections::HashMap;

/// One tab stop after expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabstop {
    pub index: u32,
    /// Char ranges (half-open, offsets into [`Snippet::text`]). The first is
    /// the primary occurrence; the rest are mirrors of it.
    pub ranges: Vec<(usize, usize)>,
    /// Choices for a `${1|a,b|}` stop (the first one is already inserted).
    pub choices: Vec<String>,
}

/// An expanded snippet: plain text plus the tab stops inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// Ordered `1, 2, ...` with `$0` (final cursor position) last. Always
    /// ends with index 0: an implicit `$0` is appended at the end of text.
    pub tabstops: Vec<Tabstop>,
}

impl Snippet {
    /// True when the snippet has an interactive tab stop besides `$0`.
    pub fn has_interactive_stops(&self) -> bool {
        self.tabstops.iter().any(|stop| stop.index != 0)
    }

    /// Parses `input`, resolving variables with `vars`.
    pub fn parse(input: &str, vars: &dyn Fn(&str) -> Option<String>) -> Snippet {
        let chars: Vec<char> = input.chars().collect();
        let mut parser = Parser {
            chars: &chars,
            pos: 0,
        };
        let nodes = match parser.parse_nodes(false) {
            Some(nodes) if parser.pos == chars.len() => nodes,
            _ => return Snippet::literal(input),
        };

        // The first occurrence that carries content defines the text of every
        // other occurrence of the same index (mirrors).
        let mut defaults: HashMap<u32, Vec<Node>> = HashMap::new();
        collect_defaults(&nodes, &mut defaults);

        let mut out = Emitter {
            text: String::new(),
            len: 0,
            stops: Vec::new(),
            defaults: &defaults,
            vars,
        };
        out.emit(&nodes, true);

        let mut stops: Vec<Tabstop> = Vec::new();
        for (index, range, choices) in out.stops {
            match stops.iter_mut().find(|stop| stop.index == index) {
                Some(stop) => stop.ranges.push(range),
                None => stops.push(Tabstop {
                    index,
                    ranges: vec![range],
                    choices,
                }),
            }
        }
        // $1, $2, ... then $0.
        stops.sort_by_key(|stop| {
            if stop.index == 0 {
                u32::MAX
            } else {
                stop.index
            }
        });
        if stops.last().is_none_or(|stop| stop.index != 0) {
            stops.push(Tabstop {
                index: 0,
                ranges: vec![(out.len, out.len)],
                choices: Vec::new(),
            });
        }
        Snippet {
            text: out.text,
            tabstops: stops,
        }
    }

    /// Prefixes every line after the first with `indent` (the whitespace of
    /// the line the snippet is inserted into) and shifts the tab stops along.
    pub fn indent_continuation_lines(&mut self, indent: &str) {
        let width = indent.chars().count();
        if width == 0 || !self.text.contains('\n') {
            return;
        }
        let newline_positions: Vec<usize> = self
            .text
            .chars()
            .enumerate()
            .filter(|(_, c)| *c == '\n')
            .map(|(i, _)| i)
            .collect();
        let shift = |offset: usize| {
            offset + width * newline_positions.iter().filter(|&&nl| nl < offset).count()
        };
        let mut text = String::new();
        for (i, line) in self.text.split('\n').enumerate() {
            if i > 0 {
                text.push('\n');
                text.push_str(indent);
            }
            text.push_str(line);
        }
        self.text = text;
        for stop in &mut self.tabstops {
            for range in &mut stop.ranges {
                *range = (shift(range.0), shift(range.1));
            }
        }
    }

    /// `text` inserted as-is; the cursor ends after it.
    pub fn literal(text: &str) -> Snippet {
        let len = text.chars().count();
        Snippet {
            text: text.to_string(),
            tabstops: vec![Tabstop {
                index: 0,
                ranges: vec![(len, len)],
                choices: Vec::new(),
            }],
        }
    }
}

#[derive(Debug, Clone)]
enum Node {
    Text(String),
    Tabstop {
        index: u32,
        children: Vec<Node>,
        choices: Vec<String>,
        /// A transform (`${1/re/fmt/}`) was present: the occurrence is dropped.
        transformed: bool,
    },
    Variable {
        name: String,
        children: Vec<Node>,
    },
}

fn collect_defaults(nodes: &[Node], defaults: &mut HashMap<u32, Vec<Node>>) {
    for node in nodes {
        match node {
            Node::Tabstop {
                index,
                children,
                choices,
                ..
            } => {
                if !children.is_empty() {
                    defaults.entry(*index).or_insert_with(|| children.clone());
                } else if let Some(first) = choices.first() {
                    defaults
                        .entry(*index)
                        .or_insert_with(|| vec![Node::Text(first.clone())]);
                }
                collect_defaults(children, defaults);
            }
            Node::Variable { children, .. } => collect_defaults(children, defaults),
            Node::Text(_) => {}
        }
    }
}

struct Emitter<'a> {
    text: String,
    len: usize,
    stops: Vec<(u32, (usize, usize), Vec<String>)>,
    defaults: &'a HashMap<u32, Vec<Node>>,
    vars: &'a dyn Fn(&str) -> Option<String>,
}

impl Emitter<'_> {
    fn push(&mut self, s: &str) {
        self.text.push_str(s);
        self.len += s.chars().count();
    }

    /// `register` is false while flattening a default that is only text
    /// (a mirror's copy of a nested placeholder must not register stops).
    fn emit(&mut self, nodes: &[Node], register: bool) {
        for node in nodes {
            match node {
                Node::Text(t) => self.push(t),
                Node::Tabstop {
                    index,
                    children,
                    choices,
                    transformed,
                } => {
                    if *transformed {
                        continue;
                    }
                    let start = self.len;
                    if !children.is_empty() {
                        self.emit(children, register);
                    } else if let Some(default) = self.defaults.get(index) {
                        let default = default.clone();
                        self.emit(&default, false);
                    }
                    if register {
                        self.stops
                            .push((*index, (start, self.len), choices.clone()));
                    }
                }
                Node::Variable { name, children } => {
                    match (self.vars)(name) {
                        Some(value) if !value.is_empty() => self.push(&value),
                        // Known but empty (no selection, empty clipboard): the
                        // default applies, as in VS Code.
                        Some(_) | None if !children.is_empty() => self.emit(children, register),
                        Some(_) => {}
                        // VS Code inserts the variable name for unknown ones.
                        None => self.push(name),
                    }
                }
            }
        }
    }
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Parses until end of input, or (when `nested`) until an unescaped `}`.
    fn parse_nodes(&mut self, nested: bool) -> Option<Vec<Node>> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        let flush = |text: &mut String, nodes: &mut Vec<Node>| {
            if !text.is_empty() {
                nodes.push(Node::Text(std::mem::take(text)));
            }
        };
        while let Some(c) = self.peek() {
            match c {
                '}' if nested => break,
                '\\' => match self.chars.get(self.pos + 1) {
                    Some(&next) if matches!(next, '$' | '}' | '\\') => {
                        text.push(next);
                        self.pos += 2;
                    }
                    _ => {
                        text.push('\\');
                        self.pos += 1;
                    }
                },
                '$' => {
                    let save = self.pos;
                    match self.parse_dollar()? {
                        Some(node) => {
                            flush(&mut text, &mut nodes);
                            nodes.push(node);
                        }
                        None => {
                            // A lone `$` that starts nothing: literal.
                            self.pos = save + 1;
                            text.push('$');
                        }
                    }
                }
                _ => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
        flush(&mut text, &mut nodes);
        Some(nodes)
    }

    /// At a `$`. `Some(None)` means "not a snippet construct"; `None` means
    /// malformed syntax (the whole input becomes literal text).
    fn parse_dollar(&mut self) -> Option<Option<Node>> {
        self.pos += 1; // '$'
        match self.peek() {
            Some(c) if c.is_ascii_digit() => {
                let index = self.parse_int()?;
                Some(Some(Node::Tabstop {
                    index,
                    children: Vec::new(),
                    choices: Vec::new(),
                    transformed: false,
                }))
            }
            Some(c) if is_var_start(c) => {
                let name = self.parse_var_name();
                Some(Some(Node::Variable {
                    name,
                    children: Vec::new(),
                }))
            }
            Some('{') => {
                self.pos += 1;
                self.parse_braced().map(Some)
            }
            _ => Some(None),
        }
    }

    fn parse_int(&mut self) -> Option<u32> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn parse_var_name(&mut self) -> String {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// After `${`.
    fn parse_braced(&mut self) -> Option<Node> {
        let is_tabstop = self.peek().is_some_and(|c| c.is_ascii_digit());
        let (index, name) = if is_tabstop {
            (Some(self.parse_int()?), String::new())
        } else if self.peek().is_some_and(is_var_start) {
            (None, self.parse_var_name())
        } else {
            return None;
        };

        match self.peek()? {
            '}' => {
                self.pos += 1;
                Some(match index {
                    Some(index) => Node::Tabstop {
                        index,
                        children: Vec::new(),
                        choices: Vec::new(),
                        transformed: false,
                    },
                    None => Node::Variable {
                        name,
                        children: Vec::new(),
                    },
                })
            }
            ':' => {
                self.pos += 1;
                let children = self.parse_nodes(true)?;
                if self.peek()? != '}' {
                    return None;
                }
                self.pos += 1;
                Some(match index {
                    Some(index) => Node::Tabstop {
                        index,
                        children,
                        choices: Vec::new(),
                        transformed: false,
                    },
                    None => Node::Variable { name, children },
                })
            }
            '|' if index.is_some() => {
                self.pos += 1;
                let choices = self.parse_choices()?;
                Some(Node::Tabstop {
                    index: index?,
                    children: Vec::new(),
                    choices,
                    transformed: false,
                })
            }
            '/' => {
                self.skip_transform()?;
                Some(match index {
                    Some(index) => Node::Tabstop {
                        index,
                        children: Vec::new(),
                        choices: Vec::new(),
                        transformed: true,
                    },
                    None => Node::Variable {
                        name,
                        children: Vec::new(),
                    },
                })
            }
            _ => None,
        }
    }

    /// After `|`, through the closing `|}`.
    fn parse_choices(&mut self) -> Option<Vec<String>> {
        let mut choices = Vec::new();
        let mut current = String::new();
        loop {
            match self.peek()? {
                '\\' => match self.chars.get(self.pos + 1) {
                    Some(&next) if matches!(next, '$' | '}' | '\\' | ',' | '|') => {
                        current.push(next);
                        self.pos += 2;
                    }
                    _ => {
                        current.push('\\');
                        self.pos += 1;
                    }
                },
                ',' => {
                    choices.push(std::mem::take(&mut current));
                    self.pos += 1;
                }
                '|' if self.chars.get(self.pos + 1) == Some(&'}') => {
                    choices.push(current);
                    self.pos += 2;
                    return Some(choices);
                }
                c => {
                    current.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// At `/` of `/regex/format/options}`; consumes through the `}`.
    fn skip_transform(&mut self) -> Option<()> {
        let mut slashes = 0;
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '\\' => self.pos += 1,
                '/' => slashes += 1,
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                '}' if slashes >= 3 => return Some(()),
                _ => {}
            }
        }
        None
    }
}

fn is_var_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Variable values for the common `TM_*` names; everything else is unknown
/// (and therefore falls back to its default or its own name).
pub fn standard_variable(
    name: &str,
    file_path: Option<&str>,
    current_line: &str,
    line_index: usize,
    current_word: &str,
) -> Option<String> {
    let path = file_path.map(std::path::Path::new);
    match name {
        "TM_FILENAME" => path?.file_name().map(|n| n.to_string_lossy().into_owned()),
        "TM_FILENAME_BASE" => path?.file_stem().map(|n| n.to_string_lossy().into_owned()),
        "TM_DIRECTORY" => path?.parent().map(|p| p.to_string_lossy().into_owned()),
        "TM_FILEPATH" => file_path.map(str::to_string),
        "RELATIVE_FILEPATH" => file_path.map(str::to_string),
        "TM_LINE_INDEX" => Some(line_index.to_string()),
        "TM_LINE_NUMBER" => Some((line_index + 1).to_string()),
        "TM_CURRENT_LINE" => Some(current_line.to_string()),
        "TM_CURRENT_WORD" => Some(current_word.to_string()),
        "TM_SELECTED_TEXT" | "CLIPBOARD" => Some(String::new()),
        "LINE_COMMENT" => Some("//".to_string()),
        "BLOCK_COMMENT_START" => Some("/*".to_string()),
        "BLOCK_COMMENT_END" => Some("*/".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Snippet {
        Snippet::parse(input, &|name| match name {
            "TM_FILENAME" => Some("Main.java".to_string()),
            _ => None,
        })
    }

    fn stop(snippet: &Snippet, index: u32) -> &Tabstop {
        snippet.tabstops.iter().find(|s| s.index == index).unwrap()
    }

    #[test]
    fn plain_text_has_only_the_implicit_final_stop() {
        let s = parse("hello");
        assert_eq!(s.text, "hello");
        assert_eq!(s.tabstops.len(), 1);
        assert_eq!(s.tabstops[0].index, 0);
        assert_eq!(s.tabstops[0].ranges, vec![(5, 5)]);
        assert!(!s.has_interactive_stops());
    }

    #[test]
    fn tabstops_and_placeholders() {
        let s = parse("foo(${1:a}, ${2:b})$0");
        assert_eq!(s.text, "foo(a, b)");
        assert_eq!(stop(&s, 1).ranges, vec![(4, 5)]);
        assert_eq!(stop(&s, 2).ranges, vec![(7, 8)]);
        assert_eq!(stop(&s, 0).ranges, vec![(9, 9)]);
        let order: Vec<u32> = s.tabstops.iter().map(|t| t.index).collect();
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn bare_and_braced_tabstops_without_default() {
        let s = parse("a$1b${2}c");
        assert_eq!(s.text, "abc");
        assert_eq!(stop(&s, 1).ranges, vec![(1, 1)]);
        assert_eq!(stop(&s, 2).ranges, vec![(2, 2)]);
        assert_eq!(stop(&s, 0).ranges, vec![(3, 3)]);
    }

    #[test]
    fn implicit_final_stop_goes_at_the_end() {
        let s = parse("f(${1:x})");
        assert_eq!(stop(&s, 0).ranges, vec![(4, 4)]);
    }

    #[test]
    fn nested_placeholders() {
        let s = parse("${1:foo ${2:bar}} baz");
        assert_eq!(s.text, "foo bar baz");
        assert_eq!(stop(&s, 1).ranges, vec![(0, 7)]);
        assert_eq!(stop(&s, 2).ranges, vec![(4, 7)]);
    }

    #[test]
    fn mirrors_copy_the_primary_default() {
        let s = parse("${1:name} = $1;");
        assert_eq!(s.text, "name = name;");
        assert_eq!(stop(&s, 1).ranges, vec![(0, 4), (7, 11)]);
        // The default may come after the bare mirror too.
        let s = parse("$1 = ${1:name};");
        assert_eq!(s.text, "name = name;");
        assert_eq!(stop(&s, 1).ranges, vec![(0, 4), (7, 11)]);
    }

    #[test]
    fn choices_insert_the_first_and_keep_the_rest() {
        let s = parse("${1|public,private,protected|} class");
        assert_eq!(s.text, "public class");
        let one = stop(&s, 1);
        assert_eq!(one.ranges, vec![(0, 6)]);
        assert_eq!(one.choices, vec!["public", "private", "protected"]);
    }

    #[test]
    fn variables_resolve_or_fall_back() {
        assert_eq!(parse("$TM_FILENAME").text, "Main.java");
        assert_eq!(parse("${TM_FILENAME}").text, "Main.java");
        assert_eq!(parse("${UNKNOWN:fallback}").text, "fallback");
        assert_eq!(parse("$UNKNOWN").text, "UNKNOWN");
        assert_eq!(parse("${TM_FILENAME:fallback}").text, "Main.java");
    }

    /// A variable that is known but empty (no selection) uses its default.
    #[test]
    fn empty_variables_fall_back_to_their_default() {
        let snippet = |input: &str| {
            Snippet::parse(input, &|name| match name {
                "TM_SELECTED_TEXT" | "CLIPBOARD" => Some(String::new()),
                "TM_FILENAME" => Some("Main.java".to_string()),
                _ => None,
            })
        };
        assert_eq!(snippet("${TM_SELECTED_TEXT:default}").text, "default");
        assert_eq!(snippet("${CLIPBOARD:$TM_FILENAME}").text, "Main.java");
        // No default: empty stays empty, it is not replaced by the name.
        assert_eq!(snippet("[$TM_SELECTED_TEXT]").text, "[]");
        // A non-empty value still beats the default.
        assert_eq!(snippet("${TM_FILENAME:default}").text, "Main.java");
    }

    #[test]
    fn escapes_produce_literals() {
        let s = parse(r"cost: \$1 \} \\ \n");
        assert_eq!(s.text, r"cost: $1 } \ \n");
        assert!(!s.has_interactive_stops());
    }

    #[test]
    fn a_dollar_that_starts_nothing_is_literal() {
        assert_eq!(parse("a $ b").text, "a $ b");
        assert_eq!(parse("$").text, "$");
        assert_eq!(parse("$-").text, "$-");
    }

    #[test]
    fn malformed_snippets_are_inserted_literally() {
        for bad in ["${1:oops", "${", "${1|a,b}", "${}", "${1:a${2:b}"] {
            let s = parse(bad);
            assert_eq!(s.text, bad, "{bad}");
            assert!(!s.has_interactive_stops());
        }
    }

    #[test]
    fn transforms_are_dropped() {
        let s = parse("${1:x} ${1/(.*)/${1:/upcase}/}");
        assert_eq!(s.text, "x ");
        assert_eq!(stop(&s, 1).ranges, vec![(0, 1)]);
    }

    #[test]
    fn multiline_and_unicode_offsets_are_char_based() {
        let s = parse("é${1:ü}\n\t$0");
        assert_eq!(s.text, "éü\n\t");
        assert_eq!(stop(&s, 1).ranges, vec![(1, 2)]);
        assert_eq!(stop(&s, 0).ranges, vec![(4, 4)]);
    }

    #[test]
    fn placeholder_with_escaped_close_brace() {
        let s = parse(r"${1:a\}b}");
        assert_eq!(s.text, "a}b");
        assert_eq!(stop(&s, 1).ranges, vec![(0, 3)]);
    }

    #[test]
    fn continuation_lines_get_the_indent_and_stops_follow() {
        let mut s = parse("if (${1:cond}) {\n\t$0\n}");
        s.indent_continuation_lines("    ");
        assert_eq!(s.text, "if (cond) {\n    \t\n    }");
        assert_eq!(stop(&s, 1).ranges, vec![(4, 8)]);
        // `$0` sits after the continuation line's indent and tab.
        assert_eq!(stop(&s, 0).ranges, vec![(17, 17)]);
    }

    #[test]
    fn method_call_snippet_like_a_language_server_sends() {
        let s = parse("filter(${1:predicate})$0");
        assert_eq!(s.text, "filter(predicate)");
        assert_eq!(stop(&s, 1).ranges, vec![(7, 16)]);
        assert_eq!(stop(&s, 0).ranges, vec![(17, 17)]);
    }
}
