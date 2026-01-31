/// Simple glob pattern matching supporting `*` and `?` wildcards.
/// Matches against the given string case-insensitively.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    glob_match_inner(&pattern, &text)
}

fn glob_match_inner(pattern: &[char], text: &[char]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = None;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }

    pi == pattern.len()
}

/// Checks if a result's file path matches the file filter.
/// The filter is space-separated tokens; all must match.
/// Tokens containing `*` or `?` are glob-matched against the basename
/// (or full path if token contains `/`). Otherwise, substring match (case-insensitive).
pub fn matches_file_filter(filter: &str, path: &str) -> bool {
    if filter.is_empty() {
        return true;
    }

    let tokens: Vec<&str> = filter.split_whitespace().collect();
    if tokens.is_empty() {
        return true;
    }

    let basename = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());

    let path_lower = path.to_lowercase();
    let basename_lower = basename.to_lowercase();

    for token in &tokens {
        let is_glob = token.contains('*') || token.contains('?');
        let has_slash = token.contains('/');

        if is_glob {
            let target = if has_slash {
                &path_lower
            } else {
                &basename_lower
            };
            if !glob_match(token, target) {
                return false;
            }
        } else {
            let token_lower = token.to_lowercase();
            let target = if has_slash {
                &path_lower
            } else {
                &basename_lower
            };
            if !target.contains(&token_lower) {
                return false;
            }
        }
    }

    true
}

/// Truncates a path in the middle if it's too long
/// Prioritizes showing the filename and immediate parent directories
pub fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }

    let parts: Vec<&str> = path.split('/').collect();

    if parts.is_empty() {
        return path.to_string();
    }

    if parts.len() == 1 {
        if max_len < 4 {
            return "...".to_string();
        }
        let chars: Vec<char> = path.chars().collect();
        let start_len = (max_len - 3) / 2;
        let end_len = max_len - 3 - start_len;
        let start: String = chars.iter().take(start_len).collect();
        let end: String = chars
            .iter()
            .skip(chars.len().saturating_sub(end_len))
            .collect();
        return format!("{}...{}", start, end);
    }

    let last = parts[parts.len() - 1];
    let reserved = 4 + last.len();

    if reserved >= max_len {
        if max_len < 4 {
            return "...".to_string();
        }
        let available = max_len - 3;
        let chars: Vec<char> = last.chars().collect();
        let skip_count = chars.len().saturating_sub(available);
        let suffix: String = chars.iter().skip(skip_count).collect();
        return format!("...{}", suffix);
    }

    let mut included_parts = vec![last];
    let mut current_len = last.len();

    for i in (0..parts.len() - 1).rev() {
        let part = parts[i];
        let needed = part.len() + 1;

        if current_len + needed + 4 <= max_len {
            included_parts.insert(0, part);
            current_len += needed;
        } else {
            if i > 0 && current_len + needed + 4 <= max_len {
                included_parts.insert(0, part);
                let current_len = current_len + needed;
                let _ = current_len;
            }
            break;
        }
    }

    if included_parts.len() == parts.len() {
        return path.to_string();
    }

    if included_parts.len() < parts.len() && included_parts[0] != parts[0] {
        let mut result = String::from(".../");
        result.push_str(&included_parts.join("/"));
        return result;
    }

    included_parts.join("/")
}
