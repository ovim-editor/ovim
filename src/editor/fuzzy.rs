//! Fuzzy matching and scoring algorithms for the picker.
//!
//! Extracted from `Picker` to keep scoring logic reusable and testable
//! independently of picker state.

/// Fuzzy match that also returns the matched character positions in the target.
/// Case-insensitive. Prefers exact substring matches over fuzzy ones.
pub(crate) fn fuzzy_match_with_positions(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }

    let query_lower = query.to_lowercase();
    let target_lower = target.to_lowercase();

    let query_chars: Vec<char> = query_lower.chars().collect();
    let target_chars: Vec<char> = target_lower.chars().collect();

    if query_chars.is_empty() {
        return Some((0, Vec::new()));
    }

    // Prefer exact substring matches — find the best occurrence
    if let Some(result) = exact_substring_match(&query_chars, &target_chars) {
        return Some(result);
    }

    // Fall back to fuzzy matching
    let mut query_idx = 0;
    let mut target_idx = 0;
    let mut score: i32 = 0;
    let mut consecutive_matches = 0;
    let mut last_match_idx: Option<usize> = None;
    let mut positions = Vec::with_capacity(query_chars.len());

    while query_idx < query_chars.len() && target_idx < target_chars.len() {
        if query_chars[query_idx] == target_chars[target_idx] {
            // Base score for match
            score += 1;

            // Bonus for consecutive matches
            if let Some(last_idx) = last_match_idx {
                if target_idx == last_idx + 1 {
                    consecutive_matches += 1;
                    score += consecutive_matches * 5;
                } else {
                    consecutive_matches = 0;
                    let gap = target_idx - last_idx - 1;
                    score -= (gap as i32).min(3);
                }
            } else {
                consecutive_matches = 0;
            }

            // Bonus for matching at start of target
            if target_idx == 0 {
                score += 10;
            }

            // Bonus for matching after path separator or start of word
            if target_idx > 0 {
                let prev_char = target_chars[target_idx - 1];
                if prev_char == '/' || prev_char == '_' || prev_char == '-' || prev_char == ' ' {
                    score += 8;
                }
            }

            positions.push(target_idx);
            last_match_idx = Some(target_idx);
            query_idx += 1;
        }
        target_idx += 1;
    }

    if query_idx == query_chars.len() {
        score += 100 - (target_chars.len() as i32).min(100);
        Some((score, positions))
    } else {
        None
    }
}

/// Find the best exact substring match, preferring word boundaries and start of string.
/// Returns a high score so exact matches always rank above fuzzy matches.
pub(crate) fn exact_substring_match(
    query_chars: &[char],
    target_chars: &[char],
) -> Option<(i32, Vec<usize>)> {
    let query_len = query_chars.len();
    if query_len == 0 || target_chars.len() < query_len {
        return None;
    }

    let mut best: Option<(i32, usize)> = None;

    for start in 0..=(target_chars.len() - query_len) {
        let matches = (0..query_len).all(|i| target_chars[start + i] == query_chars[i]);
        if !matches {
            continue;
        }

        // Base: large bonus for being an exact substring
        let mut score: i32 = 200;

        // Bonus for matching at start of target
        if start == 0 {
            score += 20;
        }

        // Bonus for matching at a word boundary
        if start > 0 {
            let prev = target_chars[start - 1];
            if prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' {
                score += 15;
            }
        }

        // Prefer shorter targets (more specific match)
        score += 100 - (target_chars.len() as i32).min(100);

        match best {
            Some((best_score, _)) if score <= best_score => {}
            _ => best = Some((score, start)),
        }
    }

    best.map(|(score, start)| {
        let positions: Vec<usize> = (start..start + query_len).collect();
        (score, positions)
    })
}

/// Filename-preferential fuzzy scoring.
/// Splits query on whitespace (all tokens must match), prefers filename matches.
/// Returns (total_score, matched_positions_in_full_path).
pub(crate) fn fuzzy_score(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }

    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return Some((0, Vec::new()));
    }

    // Extract filename and its char offset in the full path
    let filename_start = target.rfind('/').map(|i| i + 1).unwrap_or(0);
    let filename = &target[filename_start..];
    // Convert byte offset to char offset
    let filename_char_offset = target[..filename_start].chars().count();

    let mut total_score: i32 = 0;
    let mut all_positions = Vec::new();

    for token in &tokens {
        // Try matching against filename first (with bonus)
        if let Some((score, positions)) = fuzzy_match_with_positions(token, filename) {
            total_score += score + 50; // Filename match bonus
                                       // Offset positions to full-path indices
            for pos in positions {
                all_positions.push(pos + filename_char_offset);
            }
        } else if let Some((score, positions)) = fuzzy_match_with_positions(token, target) {
            // Fall back to full path match (no bonus)
            total_score += score;
            all_positions.extend(positions);
        } else {
            // Token didn't match at all — entire query fails
            return None;
        }
    }

    Some((total_score, all_positions))
}

/// Re-derive highlight positions for a query against a display string.
/// Used by the picker UI when nucleo provides results without position info.
/// Handles space-separated multi-token queries.
pub(crate) fn rematch_positions(query: &str, display: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let display_lower: Vec<char> = display.to_lowercase().chars().collect();

    // Handle space-separated tokens: each token must match independently
    let token_ranges: Vec<(usize, usize)> = {
        let mut ranges = Vec::new();
        let mut start = 0;
        for (i, &c) in query_lower.iter().enumerate() {
            if c == ' ' {
                if i > start {
                    ranges.push((start, i));
                }
                start = i + 1;
            }
        }
        if start < query_lower.len() {
            ranges.push((start, query_lower.len()));
        }
        ranges
    };

    for (token_start, token_end) in token_ranges {
        let token = &query_lower[token_start..token_end];
        let mut qi = 0;
        for (di, &dc) in display_lower.iter().enumerate() {
            if qi < token.len() && token[qi] == dc {
                positions.push(di);
                qi += 1;
            }
        }
    }

    positions.sort_unstable();
    positions.dedup();
    positions
}
