//! Client-side matcher for the completion menu.
//!
//! The server already ranked the list (`sortText`); as the user keeps typing
//! the client narrows and re-orders it with the tiers below, IntelliJ / VS
//! Code style. Within one tier the server's order is kept, so this matcher
//! only has to be *coarse*: it never lets a fine-grained score override the
//! server's ranking among equally good matches.
//!
//! Tiers, best first:
//! 1. `Exact` - the whole text equals the pattern.
//! 2. `Prefix` - the text starts with the pattern.
//! 3. `PrefixInsensitive` - prefix match that only holds ignoring case (only
//!    distinct from `Prefix` when the pattern contains an uppercase letter).
//! 4. `Hump` - every pattern char is either consecutive with the previous
//!    match or sits on a word boundary (`gEm` -> `getEmail`, `NPE` ->
//!    `NullPointerException`, `fb` -> `foo_bar`).
//! 5. `Subsequence` - the pattern chars appear in order somewhere.
//!
//! Case is "smart": an all-lowercase pattern ignores case; a pattern with an
//! uppercase letter demands that letter's case (except after a `_`/`-`
//! separator where a lowercase letter is the natural spelling).

/// How well the pattern matched. Higher is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchTier {
    Subsequence = 1,
    Hump = 2,
    PrefixInsensitive = 3,
    Prefix = 4,
    Exact = 5,
}

/// A successful match: its tier and the matched char indices in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub tier: MatchTier,
    /// Char (not byte) indices into the matched text, ascending.
    pub positions: Vec<usize>,
}

/// Longest pattern/text the hump search will consider; beyond it only the
/// linear tiers are tried (keeps the backtracking search bounded).
const MAX_HUMP_TEXT: usize = 160;
const MAX_HUMP_PATTERN: usize = 32;

/// Matches `pattern` against `text`. An empty pattern matches everything
/// (as `Prefix`, with no highlighted positions).
pub fn fuzzy_match(pattern: &str, text: &str) -> Option<FuzzyMatch> {
    if pattern.is_empty() {
        return Some(FuzzyMatch {
            tier: MatchTier::Prefix,
            positions: Vec::new(),
        });
    }
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    if pat.len() > txt.len() {
        return None;
    }
    let smart_upper = pat.iter().any(|c| c.is_uppercase());

    // Exact / prefix tiers.
    if chars_eq_prefix(&pat, &txt, true) {
        let tier = if pat.len() == txt.len() {
            MatchTier::Exact
        } else {
            MatchTier::Prefix
        };
        return Some(FuzzyMatch {
            tier,
            positions: (0..pat.len()).collect(),
        });
    }
    if chars_eq_prefix(&pat, &txt, false) {
        let tier = if !smart_upper {
            // An all-lowercase pattern ignores case: as good as exact-case.
            if pat.len() == txt.len() {
                MatchTier::Exact
            } else {
                MatchTier::Prefix
            }
        } else {
            MatchTier::PrefixInsensitive
        };
        return Some(FuzzyMatch {
            tier,
            positions: (0..pat.len()).collect(),
        });
    }

    if pat.len() <= MAX_HUMP_PATTERN && txt.len() <= MAX_HUMP_TEXT {
        let mut failed = vec![false; (pat.len() + 1) * (txt.len() + 1)];
        let mut positions = Vec::with_capacity(pat.len());
        if hump_search(&pat, &txt, 0, 0, None, &mut positions, &mut failed) {
            return Some(FuzzyMatch {
                tier: MatchTier::Hump,
                positions,
            });
        }
    }

    subsequence(&pat, &txt).map(|positions| FuzzyMatch {
        tier: MatchTier::Subsequence,
        positions,
    })
}

fn chars_eq_prefix(pat: &[char], txt: &[char], case_sensitive: bool) -> bool {
    pat.iter().zip(txt.iter()).all(|(p, t)| {
        if case_sensitive {
            p == t
        } else {
            chars_eq_ci(*p, *t)
        }
    })
}

fn chars_eq_ci(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Case-smart single char comparison used by the hump search.
fn hump_char_eq(pattern_char: char, text: &[char], t: usize) -> bool {
    let tc = text[t];
    if pattern_char == tc {
        return true;
    }
    if pattern_char.is_uppercase() {
        // Uppercase demands uppercase, except right after a separator, where
        // the segment is naturally lowercase (`sE` -> `snake_email`).
        t > 0 && !text[t - 1].is_alphanumeric() && chars_eq_ci(pattern_char, tc)
    } else {
        chars_eq_ci(pattern_char, tc)
    }
}

fn is_boundary(text: &[char], t: usize) -> bool {
    if t == 0 {
        return true;
    }
    let prev = text[t - 1];
    let cur = text[t];
    if !prev.is_alphanumeric() {
        return cur.is_alphanumeric();
    }
    if cur.is_uppercase() && (prev.is_lowercase() || prev.is_ascii_digit()) {
        return true;
    }
    // Acronym followed by a word: the `S` in `HTTPServer`.
    cur.is_uppercase()
        && prev.is_uppercase()
        && text.get(t + 1).is_some_and(|next| next.is_lowercase())
}

fn hump_search(
    pat: &[char],
    txt: &[char],
    pi: usize,
    from: usize,
    prev: Option<usize>,
    positions: &mut Vec<usize>,
    failed: &mut [bool],
) -> bool {
    if pi == pat.len() {
        return true;
    }
    let width = txt.len() + 1;
    if failed[pi * width + from] {
        return false;
    }
    // Not enough text left for the remaining pattern.
    if txt.len() - from.min(txt.len()) < pat.len() - pi {
        failed[pi * width + from] = true;
        return false;
    }
    for t in from..txt.len() {
        let consecutive = prev.is_some_and(|p| p + 1 == t);
        if !(consecutive || is_boundary(txt, t)) {
            continue;
        }
        if !hump_char_eq(pat[pi], txt, t) {
            continue;
        }
        positions.push(t);
        if hump_search(pat, txt, pi + 1, t + 1, Some(t), positions, failed) {
            return true;
        }
        positions.pop();
    }
    failed[pi * width + from] = true;
    false
}

fn subsequence(pat: &[char], txt: &[char]) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(pat.len());
    let mut t = 0;
    for &p in pat {
        while t < txt.len() && !chars_eq_ci(p, txt[t]) {
            t += 1;
        }
        if t == txt.len() {
            return None;
        }
        positions.push(t);
        t += 1;
    }
    Some(positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(pattern: &str, text: &str) -> Option<MatchTier> {
        fuzzy_match(pattern, text).map(|m| m.tier)
    }

    #[test]
    fn empty_pattern_matches_everything() {
        assert_eq!(tier("", "anything"), Some(MatchTier::Prefix));
    }

    #[test]
    fn prefix_beats_hump_beats_subsequence() {
        assert_eq!(tier("get", "getEmail"), Some(MatchTier::Prefix));
        assert_eq!(tier("gEm", "getEmail"), Some(MatchTier::Hump));
        assert_eq!(tier("gml", "getEmail"), Some(MatchTier::Subsequence));
        assert_eq!(tier("xyz", "getEmail"), None);
    }

    #[test]
    fn exact_is_the_top_tier() {
        assert_eq!(tier("list", "list"), Some(MatchTier::Exact));
        assert_eq!(tier("list", "List"), Some(MatchTier::Exact));
        assert_eq!(tier("List", "list"), Some(MatchTier::PrefixInsensitive));
    }

    #[test]
    fn lowercase_pattern_is_case_insensitive() {
        assert_eq!(tier("str", "String"), Some(MatchTier::Prefix));
        assert_eq!(tier("gem", "getEmail"), Some(MatchTier::Hump));
    }

    #[test]
    fn uppercase_pattern_letter_demands_uppercase() {
        // `Str` typed with a capital: exact-case prefix beats case-folded.
        assert_eq!(tier("Str", "String"), Some(MatchTier::Prefix));
        assert_eq!(tier("Str", "string"), Some(MatchTier::PrefixInsensitive));
        // `E` is a hump letter; it must not match the lowercase `e` mid-word.
        assert_eq!(tier("gEl", "gettel"), Some(MatchTier::Subsequence));
    }

    #[test]
    fn acronyms_match_word_starts() {
        assert_eq!(tier("NPE", "NullPointerException"), Some(MatchTier::Hump));
        assert_eq!(tier("npe", "NullPointerException"), Some(MatchTier::Hump));
        assert_eq!(tier("HS", "HTTPServer"), Some(MatchTier::Hump));
    }

    #[test]
    fn snake_case_words_are_humps() {
        assert_eq!(tier("fb", "foo_bar"), Some(MatchTier::Hump));
        assert_eq!(tier("sE", "snake_email"), Some(MatchTier::Hump));
    }

    #[test]
    fn hump_may_start_at_a_later_word() {
        assert_eq!(tier("Em", "getEmail"), Some(MatchTier::Hump));
    }

    #[test]
    fn positions_index_chars_not_bytes() {
        let m = fuzzy_match("gEm", "getEmail").unwrap();
        assert_eq!(m.positions, vec![0, 3, 4]);
        let m = fuzzy_match("é", "éa").unwrap();
        assert_eq!(m.positions, vec![0]);
    }

    #[test]
    fn hump_search_is_bounded_on_adversarial_input() {
        let text = "a".repeat(150);
        let pattern = format!("{}b", "a".repeat(20));
        assert_eq!(tier(&pattern, &text), None);
    }
}
