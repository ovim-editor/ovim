//! Shell command expansion for Vim-style filename patterns.
//!
//! Supports the following patterns:
//! - `%` - current filename
//! - `#` - alternate filename
//! - `%:p` - full absolute path
//! - `%:h` - head (directory)
//! - `%:t` - tail (basename)
//! - `%:r` - root (without extension)
//! - `%:e` - extension only
//! - `\%` - literal `%`
//!
//! Modifiers can be chained: `%:p:h` = directory of absolute path

use std::path::Path;

/// A `%` or `#` reference with its modifiers applied.
struct Reference {
    /// The file name after the modifiers.
    text: String,
    /// The file name was empty before the modifiers.
    unnamed: bool,
    /// The `:p` modifier was applied, which makes an unnamed buffer usable
    /// (`%:p:h` is the working directory).
    absolute: bool,
}

/// Expands %, #, and modifiers in shell commands.
///
/// # Arguments
/// * `cmd` - The shell command containing patterns to expand
/// * `current_file` - The current file path (% register)
/// * `alternate_file` - The alternate file path (# register)
///
/// # Returns
/// The command with all patterns expanded
pub fn expand_shell_command(cmd: &str, current_file: &str, alternate_file: &str) -> String {
    let Ok(expanded) = expand_references(cmd, current_file, alternate_file, |_, reference| {
        Ok::<_, std::convert::Infallible>(shell_escape(&reference.text))
    });
    expanded
}

/// Expands `%`, `#` and their modifiers in a file argument (`:e %:h/x`,
/// `:w %:r.bak`, `:sp #`). The names are inserted as they are: unlike a
/// shell command, nothing downstream would unquote them. An unnamed buffer
/// is vim's E499 (unless `:p` made the name absolute) and a missing
/// alternate file E194.
pub fn expand_file_argument(
    arg: &str,
    current_file: &str,
    alternate_file: &str,
) -> Result<String, String> {
    expand_references(
        arg,
        current_file,
        alternate_file,
        |source, reference| match (source, reference.unnamed, reference.absolute) {
            ('#', true, _) => Err("E194: No alternate file name to substitute for '#'".to_string()),
            ('%', true, false) => {
                Err("E499: Empty file name for '%' or '#', only works with \":p:h\"".to_string())
            }
            _ => Ok(reference.text),
        },
    )
}

/// Replaces every unescaped `%` and `#` in `text` with what `insert` makes
/// of the file it names; `\%`, `\#` and `\\` are the literal characters.
fn expand_references<E>(
    text: &str,
    current_file: &str,
    alternate_file: &str,
    mut insert: impl FnMut(char, Reference) -> Result<String, E>,
) -> Result<String, E> {
    let mut result = String::with_capacity(text.len() * 2);
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Check for escaped % or #
            if let Some(&next) = chars.peek() {
                if next == '%' || next == '#' || next == '\\' {
                    result.push(chars.next().unwrap());
                    continue;
                }
            }
            result.push(ch);
        } else if ch == '%' || ch == '#' {
            let file = if ch == '%' {
                current_file
            } else {
                alternate_file
            };
            let reference = expand_with_modifiers(file, &mut chars);
            result.push_str(&insert(ch, reference)?);
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

/// Expands a filename with optional modifiers (:p, :h, :t, :r, :e).
/// Modifiers can be chained.
fn expand_with_modifiers(
    filename: &str,
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> Reference {
    let mut result = filename.to_string();
    let mut absolute = false;

    // Consume and apply modifiers
    while chars.peek() == Some(&':') {
        chars.next(); // consume ':'

        if let Some(&modifier) = chars.peek() {
            match modifier {
                'p' => {
                    chars.next();
                    result = make_absolute(&result);
                    absolute = true;
                }
                'h' => {
                    chars.next();
                    result = get_head(&result);
                }
                't' => {
                    chars.next();
                    result = get_tail(&result);
                }
                'r' => {
                    chars.next();
                    result = get_root(&result);
                }
                'e' => {
                    chars.next();
                    result = get_extension(&result);
                }
                _ => {
                    // Unknown modifier, put the ':' back conceptually
                    // by not consuming the next char
                    result.push(':');
                }
            }
        } else {
            // Trailing ':', just add it
            result.push(':');
        }
    }

    Reference {
        text: result,
        unnamed: filename.is_empty(),
        absolute,
    }
}

/// Convert to absolute path.
fn make_absolute(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let path = Path::new(path);
    if path.is_absolute() {
        path.to_string_lossy().to_string()
    } else {
        // Try to make it absolute
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path).to_string_lossy().to_string(),
            Err(_) => path.to_string_lossy().to_string(),
        }
    }
}

/// Get the head (directory) of a path.
fn get_head(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }

    let path = Path::new(path);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().to_string(),
        _ => ".".to_string(),
    }
}

/// Get the tail (basename) of a path.
fn get_tail(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let path = Path::new(path);
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Get the root (without extension) of a path.
fn get_root(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let path = Path::new(path);

    // Get the stem (filename without extension)
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Combine with parent directory
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            format!("{}/{}", parent.to_string_lossy(), stem)
        }
        _ => stem,
    }
}

/// Get the extension of a path.
fn get_extension(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let path = Path::new(path);
    path.extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Shell-escape a filename for safe use in shell commands.
/// This handles spaces, special characters, etc.
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    // Check if the string needs quoting
    let needs_quoting = s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '!'
                | '"'
                | '#'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | '\\'
                | ']'
                | '^'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
        )
    });

    if !needs_quoting {
        return s.to_string();
    }

    // Use single quotes and escape single quotes within
    let mut result = String::with_capacity(s.len() + 4);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            // End quote, add escaped quote, start quote again
            result.push_str("'\\''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_percent_basic() {
        let result = expand_shell_command("echo %", "file.rs", "");
        assert_eq!(result, "echo file.rs");
    }

    #[test]
    fn test_expand_hash_basic() {
        let result = expand_shell_command("echo #", "file.rs", "other.rs");
        assert_eq!(result, "echo other.rs");
    }

    #[test]
    fn test_expand_percent_tail() {
        let result = expand_shell_command("echo %:t", "src/main.rs", "");
        assert_eq!(result, "echo main.rs");
    }

    #[test]
    fn test_expand_percent_head() {
        let result = expand_shell_command("echo %:h", "src/main.rs", "");
        assert_eq!(result, "echo src");
    }

    #[test]
    fn test_expand_percent_root() {
        let result = expand_shell_command("echo %:r", "src/main.rs", "");
        assert_eq!(result, "echo src/main");
    }

    #[test]
    fn test_expand_percent_extension() {
        let result = expand_shell_command("echo %:e", "src/main.rs", "");
        assert_eq!(result, "echo rs");
    }

    #[test]
    fn test_expand_modifiers_chain() {
        // :p:h = directory of absolute path
        let result = expand_shell_command("echo %:t:r", "src/main.rs", "");
        assert_eq!(result, "echo main");
    }

    #[test]
    fn test_escape_literal_percent() {
        let result = expand_shell_command("echo \\% and %", "file.rs", "");
        assert_eq!(result, "echo % and file.rs");
    }

    #[test]
    fn test_escape_literal_hash() {
        let result = expand_shell_command("echo \\# and #", "file.rs", "other.rs");
        assert_eq!(result, "echo # and other.rs");
    }

    #[test]
    fn test_shell_escape_spaces() {
        let result = expand_shell_command("echo %", "my file.rs", "");
        assert_eq!(result, "echo 'my file.rs'");
    }

    #[test]
    fn test_no_expansion_needed() {
        let result = expand_shell_command("echo hello world", "file.rs", "other.rs");
        assert_eq!(result, "echo hello world");
    }

    #[test]
    fn test_empty_filename() {
        let result = expand_shell_command("echo %:t", "", "");
        assert_eq!(result, "echo ''");
    }

    #[test]
    fn test_head_of_simple_filename() {
        // Head of a simple filename should be "."
        let result = expand_shell_command("echo %:h", "file.rs", "");
        assert_eq!(result, "echo .");
    }

    #[test]
    fn file_argument_inserts_names_without_shell_quoting() {
        // vim: `:e %:h/x` and `:e #` take the names verbatim.
        let expand = |arg: &str| expand_file_argument(arg, "my dir/main.rs", "other.rs");
        assert_eq!(expand("%"), Ok("my dir/main.rs".to_string()));
        assert_eq!(expand("%:h/x"), Ok("my dir/x".to_string()));
        assert_eq!(expand("%:r.bak"), Ok("my dir/main.bak".to_string()));
        assert_eq!(expand("#"), Ok("other.rs".to_string()));
        assert_eq!(expand("a\\%b\\#c"), Ok("a%b#c".to_string()));
        assert_eq!(expand("plain.txt"), Ok("plain.txt".to_string()));
    }

    #[test]
    fn file_argument_reports_e499_and_e194() {
        // nvim --clean: an unnamed buffer makes `:e %` E499 (even with `:h`),
        // `:e %:p:h/x` works, and `:e #` without an alternate is E194.
        let unnamed = |arg: &str| expand_file_argument(arg, "", "other.rs");
        assert!(unnamed("%").unwrap_err().starts_with("E499"));
        assert!(unnamed("%:h/a.txt").unwrap_err().starts_with("E499"));
        assert!(unnamed("%:p:h/a.txt").is_ok());
        let no_alternate = expand_file_argument("#", "a.txt", "");
        assert!(no_alternate.unwrap_err().starts_with("E194"));
        let no_alternate = expand_file_argument("#:p:h", "a.txt", "");
        assert!(no_alternate.unwrap_err().starts_with("E194"));
    }

    #[test]
    fn test_complex_command() {
        let result = expand_shell_command("echo '%' | pbcopy", "src/main.rs", "");
        assert_eq!(result, "echo 'src/main.rs' | pbcopy");
    }
}
